use rarog_fetch::{
    FetchError, FetchErrorKind, FetchResponse, HeaderList, NetworkCapability, NetworkPoll,
    NetworkRequest, NetworkTicket,
};
use reqwest::{Client, Method};
use std::collections::BTreeMap;
use std::num::NonZeroU64;
use std::thread::{self, JoinHandle};
use tokio::runtime::Builder;
use tokio::sync::{mpsc, watch};

const MAX_IN_FLIGHT: usize = 64;
const COMMAND_QUEUE_CAPACITY: usize = MAX_IN_FLIGHT;
const COMPLETION_QUEUE_CAPACITY: usize = MAX_IN_FLIGHT;

struct TransportCommand {
    ticket: NetworkTicket,
    request: NetworkRequest,
    cancellation: watch::Receiver<bool>,
}

struct TransportCompletion {
    ticket: NetworkTicket,
    result: Result<FetchResponse, FetchError>,
}

pub(crate) struct HttpTransport {
    commands: mpsc::Sender<TransportCommand>,
    completions: mpsc::Receiver<TransportCompletion>,
    active: BTreeMap<NetworkTicket, watch::Sender<bool>>,
    ready: BTreeMap<NetworkTicket, Result<FetchResponse, FetchError>>,
    next_ticket: u64,
    _thread: JoinHandle<()>,
}

impl HttpTransport {
    pub(crate) fn new() -> Result<Self, FetchError> {
        let (commands_tx, commands_rx) = mpsc::channel(COMMAND_QUEUE_CAPACITY);
        let (completion_tx, completion_rx) = mpsc::channel(COMPLETION_QUEUE_CAPACITY);
        let thread = thread::Builder::new()
            .name("zorya-http".into())
            .spawn(move || transport_worker_main(commands_rx, completion_tx))
            .map_err(|error| FetchError::network(format!("failed to start HTTP worker: {error}")))?;

        Ok(Self {
            commands: commands_tx,
            completions: completion_rx,
            active: BTreeMap::new(),
            ready: BTreeMap::new(),
            next_ticket: 1,
            _thread: thread,
        })
    }

    fn allocate_ticket(&mut self) -> Result<NetworkTicket, FetchError> {
        let raw = self.next_ticket;
        let value = NonZeroU64::new(raw).ok_or_else(|| {
            FetchError::new(
                FetchErrorKind::InvalidNetworkTicket,
                "HTTP transport ticket identity space is exhausted",
            )
        })?;
        self.next_ticket = raw.checked_add(1).unwrap_or(0);
        Ok(NetworkTicket::new(value))
    }

    fn drain_completions(&mut self) {
        while let Ok(completion) = self.completions.try_recv() {
            if self.active.contains_key(&completion.ticket) {
                self.ready.insert(completion.ticket, completion.result);
            }
        }
    }

    fn invalid_ticket(ticket: NetworkTicket) -> FetchError {
        FetchError::new(
            FetchErrorKind::InvalidNetworkTicket,
            format!("unknown or completed HTTP transport ticket {}", ticket.get()),
        )
    }
}

impl NetworkCapability for HttpTransport {
    fn start(&mut self, request: NetworkRequest) -> Result<NetworkTicket, FetchError> {
        self.drain_completions();
        if self.active.len() >= MAX_IN_FLIGHT {
            return Err(FetchError::network(format!(
                "HTTP transport in-flight limit {MAX_IN_FLIGHT} reached"
            )));
        }

        let ticket = self.allocate_ticket()?;
        let (cancel_tx, cancel_rx) = watch::channel(false);
        self.commands
            .try_send(TransportCommand {
                ticket,
                request,
                cancellation: cancel_rx,
            })
            .map_err(|error| FetchError::network(format!("HTTP worker queue unavailable: {error}")))?;
        self.active.insert(ticket, cancel_tx);
        Ok(ticket)
    }

    fn poll(&mut self, ticket: NetworkTicket) -> Result<NetworkPoll, FetchError> {
        if !self.active.contains_key(&ticket) {
            return Err(Self::invalid_ticket(ticket));
        }

        self.drain_completions();
        let Some(result) = self.ready.remove(&ticket) else {
            return Ok(NetworkPoll::Pending);
        };

        self.active.remove(&ticket);
        result.map(NetworkPoll::Complete)
    }

    fn cancel(&mut self, ticket: NetworkTicket) -> Result<(), FetchError> {
        let cancellation = self
            .active
            .remove(&ticket)
            .ok_or_else(|| Self::invalid_ticket(ticket))?;
        self.ready.remove(&ticket);
        cancellation.send_replace(true);
        Ok(())
    }
}

impl Drop for HttpTransport {
    fn drop(&mut self) {
        for cancellation in self.active.values() {
            cancellation.send_replace(true);
        }
        self.active.clear();
        self.ready.clear();
    }
}

fn transport_worker_main(
    mut commands: mpsc::Receiver<TransportCommand>,
    completions: mpsc::Sender<TransportCompletion>,
) {
    let runtime = match Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(_) => return,
    };

    runtime.block_on(async move {
        let client = build_client().map_err(|error| error.to_string());

        while let Some(command) = commands.recv().await {
            let completions = completions.clone();
            match &client {
                Ok(client) => {
                    let client = client.clone();
                    tokio::spawn(async move {
                        run_request(client, command, completions).await;
                    });
                }
                Err(message) => {
                    let _ = completions
                        .send(TransportCompletion {
                            ticket: command.ticket,
                            result: Err(FetchError::network(format!(
                                "failed to initialize HTTP client: {message}"
                            ))),
                        })
                        .await;
                }
            }
        }
    });
}

fn build_client() -> Result<Client, reqwest::Error> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .no_zstd()
        .build()
}

async fn run_request(
    client: Client,
    mut command: TransportCommand,
    completions: mpsc::Sender<TransportCompletion>,
) {
    if *command.cancellation.borrow() {
        return;
    }

    let request = execute_request(client, command.request);
    let result = tokio::select! {
        biased;
        _ = command.cancellation.changed() => return,
        result = request => result,
    };

    if *command.cancellation.borrow() {
        return;
    }

    let _ = completions
        .send(TransportCompletion {
            ticket: command.ticket,
            result,
        })
        .await;
}

async fn execute_request(client: Client, request: NetworkRequest) -> Result<FetchResponse, FetchError> {
    let response_url = request.url().clone();
    let max_body_bytes = request.max_response_body_bytes();
    let method = Method::from_bytes(request.method().as_str().as_bytes())
        .map_err(|error| FetchError::network(format!("HTTP method conversion failed: {error}")))?;
    let mut builder = client.request(method, response_url.as_str());

    for header in request.headers().iter() {
        builder = builder.header(header.name(), header.value());
    }
    if let Some(body) = request.body() {
        builder = builder.body(body.to_vec());
    }

    let mut response = builder
        .send()
        .await
        .map_err(|error| FetchError::network(format!("HTTP request failed: {error}")))?;
    if response.url().as_str() != response_url.as_str() {
        return Err(FetchError::network(
            "HTTP backend changed the response URL while redirects are disabled",
        ));
    }

    let status = response.status().as_u16();
    let mut headers = HeaderList::default();
    for (name, value) in response.headers() {
        let value = value.to_str().map_err(|error| {
            FetchError::network(format!(
                "HTTP response header '{}' is not representable by the Rarog header contract: {error}",
                name.as_str()
            ))
        })?;
        headers.append(name.as_str(), value)?;
    }

    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| FetchError::network(format!("HTTP response body failed: {error}")))?
    {
        let required = body.len().checked_add(chunk.len()).ok_or_else(|| {
            FetchError::new(
                FetchErrorKind::ResponseBodyLimitExceeded,
                "HTTP response body size overflowed",
            )
        })?;
        if required > max_body_bytes {
            return Err(FetchError::new(
                FetchErrorKind::ResponseBodyLimitExceeded,
                format!(
                    "HTTP response body requires more than {max_body_bytes} bytes"
                ),
            ));
        }
        body.extend_from_slice(&chunk);
    }

    FetchResponse::try_new(
        Some(response_url),
        status,
        headers,
        body,
        max_body_bytes,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rarog_fetch::{FetchLimits, FetchRequest};
    use rarog_url::WebUrl;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::{Duration, Instant};

    fn request(url: &str, max_response_body_bytes: usize) -> NetworkRequest {
        let url = WebUrl::parse(url).expect("request URL");
        let origin = WebUrl::parse("https://browser.example/")
            .expect("origin URL")
            .origin()
            .expect("origin");
        let limits = FetchLimits {
            max_response_body_bytes,
            ..FetchLimits::default()
        };
        FetchRequest::try_new(url, origin, limits)
            .expect("fetch request")
            .network_request()
    }

    fn serve_once(response: Vec<u8>, delay: Duration) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture server");
        let address = listener.local_addr().expect("fixture address");
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept fixture request");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("read timeout");
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            if !delay.is_zero() {
                thread::sleep(delay);
            }
            let _ = stream.write_all(&response);
            let _ = stream.flush();
        });
        format!("http://{address}/")
    }

    fn wait_for_completion(
        transport: &mut HttpTransport,
        ticket: NetworkTicket,
    ) -> Result<FetchResponse, FetchError> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match transport.poll(ticket)? {
                NetworkPoll::Pending if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(5));
                }
                NetworkPoll::Pending => panic!("HTTP fixture timed out"),
                NetworkPoll::Complete(response) => return Ok(response),
            }
        }
    }

    #[test]
    fn redirects_and_content_decoding_stay_disabled() {
        let response = b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/redirected\r\nContent-Encoding: gzip\r\nContent-Length: 8\r\nConnection: close\r\n\r\nraw-body".to_vec();
        let url = serve_once(response, Duration::ZERO);
        let mut transport = HttpTransport::new().expect("transport");
        let ticket = transport.start(request(&url, 1024)).expect("start");
        let response = wait_for_completion(&mut transport, ticket).expect("response");

        assert_eq!(response.status(), 302);
        assert_eq!(response.url().map(WebUrl::as_str), Some(url.as_str()));
        assert_eq!(response.headers().get_first("content-encoding"), Some("gzip"));
        assert_eq!(response.body(), b"raw-body");
    }

    #[test]
    fn streamed_body_is_rejected_at_rarog_request_limit() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: 10\r\nConnection: close\r\n\r\n0123456789".to_vec();
        let url = serve_once(response, Duration::ZERO);
        let mut transport = HttpTransport::new().expect("transport");
        let ticket = transport.start(request(&url, 4)).expect("start");

        let error = wait_for_completion(&mut transport, ticket).unwrap_err();
        assert_eq!(error.kind, FetchErrorKind::ResponseBodyLimitExceeded);
    }

    #[test]
    fn cancellation_invalidates_ticket_without_waiting_for_network() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec();
        let url = serve_once(response, Duration::from_millis(250));
        let mut transport = HttpTransport::new().expect("transport");
        let ticket = transport.start(request(&url, 1024)).expect("start");

        transport.cancel(ticket).expect("cancel");
        let error = transport.poll(ticket).unwrap_err();
        assert_eq!(error.kind, FetchErrorKind::InvalidNetworkTicket);
    }
}
