from pathlib import Path
import re

path = Path("src/privileged_request.rs")
text = path.read_text()


def once(old: str, new: str) -> None:
    global text
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"expected one match, got {count}: {old[:80]!r}")
    text = text.replace(old, new, 1)


once(
    "    CredentialsMode, FetchMethod, HeaderList, RedirectMode, RequestDestination, RequestMode,\n",
    "    CredentialsMode, DEFAULT_MAX_RESPONSE_BODY_BYTES, FetchMethod, HeaderList, RedirectMode,\n"
    "    RequestDestination, RequestMode,\n",
)
once(
    "/// canonical headers, exact optional bounded body, canonical Fetch envelope metadata and raw target.\n",
    "/// canonical headers, exact optional bounded body, canonical Fetch envelope metadata, exact response-body\n"
    "/// byte limit and raw target.\n",
)
once(
    "/// Method, headers, mode, credentials, redirect and destination use canonical Rarog Fetch types;\n"
    "/// Zorya does not parse or normalize them independently. Registration does not parse or authorize\n",
    "/// Method, headers, mode, credentials, redirect and destination use canonical Rarog Fetch types;\n"
    "/// the response-body limit follows the pinned Rarog non-zero byte-limit contract. Zorya does not\n"
    "/// parse or normalize them independently. Registration does not parse or authorize\n",
)
once(
    "    destination: RequestDestination,\n    target: String,\n",
    "    destination: RequestDestination,\n    max_response_body_bytes: usize,\n    target: String,\n",
)
once(
    "    pub fn destination(&self) -> &RequestDestination {\n"
    "        &self.destination\n"
    "    }\n\n"
    "    pub fn target(&self) -> &str {\n",
    "    pub fn destination(&self) -> &RequestDestination {\n"
    "        &self.destination\n"
    "    }\n\n"
    "    pub const fn max_response_body_bytes(&self) -> usize {\n"
    "        self.max_response_body_bytes\n"
    "    }\n\n"
    "    pub fn target(&self) -> &str {\n",
)
once(
    '            .field("destination_other_bytes", &destination_other_bytes)\n            .finish()\n',
    '            .field("destination_other_bytes", &destination_other_bytes)\n'
    '            .field("max_response_body_bytes", &self.max_response_body_bytes)\n'
    "            .finish()\n",
)
once(
    "    NetworkDestinationTooLong {\n        bytes: usize,\n        max: usize,\n    },\n"
    "    NetworkBodyBudgetExceeded {\n",
    "    NetworkDestinationTooLong {\n        bytes: usize,\n        max: usize,\n    },\n"
    "    InvalidNetworkResponseBodyLimit,\n"
    "    NetworkBodyBudgetExceeded {\n",
)
once(
    "            Self::NetworkDestinationTooLong { bytes, max } => write!(\n"
    "                formatter,\n"
    '                "Network privileged request destination is {bytes} bytes; maximum is {max} bytes"\n'
    "            ),\n"
    "            Self::NetworkBodyBudgetExceeded {\n",
    "            Self::NetworkDestinationTooLong { bytes, max } => write!(\n"
    "                formatter,\n"
    '                "Network privileged request destination is {bytes} bytes; maximum is {max} bytes"\n'
    "            ),\n"
    "            Self::InvalidNetworkResponseBodyLimit => formatter.write_str(\n"
    '                "Network privileged request response-body limit must be non-zero",\n'
    "            ),\n"
    "            Self::NetworkBodyBudgetExceeded {\n",
)

marker = (
    "    /// Registers the exact bounded current Network request envelope without target parsing,\n"
    "    /// source revalidation or authorization.\n"
)
if text.count(marker) != 1:
    raise SystemExit("request-parts marker mismatch")
old_block_start = text.index(marker)
old_fn_start = text.index(
    "    #[allow(clippy::too_many_arguments)]\n    pub fn register_network_with_request_parts(",
    old_block_start,
)
next_marker = text.index(
    "    /// Consumes an exact generic request before checking its source authority.",
    old_fn_start,
)
new_block = '''    /// Registers the exact bounded current Network request envelope with the pinned Rarog
    /// default response-body limit, without target parsing, source revalidation or authorization.
    ///
    /// This compatibility path binds `DEFAULT_MAX_RESPONSE_BODY_BYTES` exactly; callers that need
    /// an explicit canonical response limit use
    /// `register_network_with_request_parts_and_response_limit`.
    #[allow(clippy::too_many_arguments)]
    pub fn register_network_with_request_parts(
        &mut self,
        source: &EngineCommittedDocumentSource,
        method: FetchMethod,
        target: impl Into<String>,
        headers: HeaderList,
        body: Option<Vec<u8>>,
        mode: RequestMode,
        credentials: CredentialsMode,
        redirect: RedirectMode,
        destination: RequestDestination,
    ) -> Result<EngineNetworkPrivilegedRequest, EnginePrivilegedRequestError> {
        self.register_network_with_request_parts_and_response_limit(
            source,
            method,
            target,
            headers,
            body,
            mode,
            credentials,
            redirect,
            destination,
            DEFAULT_MAX_RESPONSE_BODY_BYTES,
        )
    }

    /// Registers the exact bounded current Network request envelope and response-body byte limit
    /// without target parsing, source revalidation or authorization.
    ///
    /// Incoming headers are rebound through Rarog `HeaderList::append` into fixed product limits.
    /// Body/method compatibility reuses Rarog `FetchMethod::permits_body`; mode, credentials,
    /// redirect and destination remain canonical pinned-Rarog Fetch values. The response-body limit
    /// is retained exactly under Rarog's non-zero `FetchLimits` contract; Zorya does not impose a
    /// second cap because retaining this fixed-size scalar allocates no response storage.
    /// `RequestDestination::Other` is retained byte-for-byte but capped before request-ID,
    /// pending-slot or tracker body-budget consumption. Deferring target parsing until consumption
    /// preserves stale-source-before-target classification.
    #[allow(clippy::too_many_arguments)]
    pub fn register_network_with_request_parts_and_response_limit(
        &mut self,
        source: &EngineCommittedDocumentSource,
        method: FetchMethod,
        target: impl Into<String>,
        headers: HeaderList,
        body: Option<Vec<u8>>,
        mode: RequestMode,
        credentials: CredentialsMode,
        redirect: RedirectMode,
        destination: RequestDestination,
        max_response_body_bytes: usize,
    ) -> Result<EngineNetworkPrivilegedRequest, EnginePrivilegedRequestError> {
        let target = target.into();
        if target.len() > MAX_PRIVILEGED_NETWORK_TARGET_BYTES {
            return Err(EnginePrivilegedRequestError::NetworkTargetTooLong {
                bytes: target.len(),
                max: MAX_PRIVILEGED_NETWORK_TARGET_BYTES,
            });
        }
        if max_response_body_bytes == 0 {
            return Err(EnginePrivilegedRequestError::InvalidNetworkResponseBodyLimit);
        }
        let destination = bind_network_destination(destination)?;
        let headers = bind_network_headers(&headers)?;
        let body = bind_network_body(&method, body)?;
        let body_bytes = body.as_ref().map_or(0, |body| body.len());
        let next_body_bytes = self
            .pending_network_body_bytes
            .checked_add(body_bytes)
            .ok_or(EnginePrivilegedRequestError::NetworkBodyBudgetExceeded {
                pending: self.pending_network_body_bytes,
                requested: body_bytes,
                max: self.max_pending_network_body_bytes,
            })?;
        if next_body_bytes > self.max_pending_network_body_bytes {
            return Err(EnginePrivilegedRequestError::NetworkBodyBudgetExceeded {
                pending: self.pending_network_body_bytes,
                requested: body_bytes,
                max: self.max_pending_network_body_bytes,
            });
        }
        self.ensure_capacity()?;

        let id = allocate_privileged_request_id()?;
        let request = EngineNetworkPrivilegedRequest {
            id,
            source: source.clone(),
            method,
            headers,
            body,
            mode,
            credentials,
            redirect,
            destination,
            max_response_body_bytes,
            target,
        };
        self.insert_pending(id, PendingPrivilegedRequest::Network(request.clone()));
        self.pending_network_body_bytes = next_body_bytes;
        Ok(request)
    }

'''
text = text[:old_block_start] + new_block + text[next_marker:]
text = text.replace(
    "/// Same-ID source, method, headers, body, mode, credentials, redirect, destination or target\n",
    "/// Same-ID source, method, headers, body, mode, credentials, redirect, destination, response limit or target\n",
    1,
)
text = text.replace(
    "/// metadata are correlation data for a later reviewed Fetch/broker path and are not executed here.\n",
    "/// metadata and response limit are correlation data for a later reviewed Fetch/broker path and are\n"
    "    /// not executed or enforced here.\n",
    1,
)
text = text.replace(
    '#[cfg(test)]\n#[path = "privileged_request_network_origin_tests.rs"]\nmod network_origin_tests;\n',
    '#[cfg(test)]\n#[path = "privileged_request_network_origin_tests.rs"]\nmod network_origin_tests;\n\n'
    '#[cfg(test)]\n#[path = "privileged_request_network_response_limit_tests.rs"]\nmod network_response_limit_tests;\n',
    1,
)
path.write_text(text)

for test_path in [
    Path("src/privileged_request.rs"),
    Path("src/privileged_request_network_body_tests.rs"),
    Path("src/privileged_request_network_origin_tests.rs"),
]:
    data = test_path.read_text()
    data, count = re.subn(
        r"(\s+)destination: RequestDestination::Empty,\n(\s+)target:",
        r"\1destination: RequestDestination::Empty,\n\2max_response_body_bytes: request.max_response_body_bytes,\n\2target:",
        data,
    )
    test_path.write_text(data)
    print(f"{test_path}: updated {count} direct request literals")

Path("src/privileged_request_network_response_limit_tests.rs").write_text(r'''use super::*;
use crate::engine::EngineNavigationPoll;
use crate::{BrowserApp, BrowserWindow, TabId};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

fn initial_tab() -> TabId {
    let app = BrowserApp::bootstrap().expect("browser bootstrap");
    app.windows()
        .next()
        .and_then(BrowserWindow::active_tab_id)
        .expect("bootstrap creates an active tab")
}

fn serve_once(path: &str) -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind response-limit fixture");
    let port = listener.local_addr().expect("fixture address").port();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept response-limit fixture");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("fixture read timeout");
        let mut request = [0u8; 4096];
        let _ = stream.read(&mut request);
        let response = b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: 18\r\nConnection: close\r\n\r\n<main>Zorya</main>";
        let _ = stream.write_all(response);
        let _ = stream.flush();
    });
    format!("http://127.0.0.1:{port}{path}")
}

fn commit_remote(
    host: &mut EngineHost,
    tab: TabId,
    location: String,
) -> EngineCommittedDocumentSource {
    let request = host
        .begin_navigation(tab, location)
        .expect("begin remote navigation")
        .expect("remote navigation forwarded");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match host.poll_navigation(request).expect("poll remote navigation") {
            EngineNavigationPoll::Pending if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(5));
            }
            EngineNavigationPoll::Pending => panic!("response-limit fixture timed out"),
            EngineNavigationPoll::Committed { .. } => break,
            EngineNavigationPoll::Failed { message } => {
                panic!("response-limit fixture navigation failed: {message}")
            }
            EngineNavigationPoll::Stale => panic!("response-limit fixture navigation went stale"),
        }
    }
    host.committed_document_source(tab)
        .expect("committed source query")
        .expect("committed remote source")
}

fn current_source(path: &str) -> (TabId, EngineHost, EngineCommittedDocumentSource) {
    let tab = initial_tab();
    let mut host = EngineHost::new().expect("engine host");
    host.create_view(tab).expect("view");
    let source = commit_remote(&mut host, tab, serve_once(path));
    (tab, host, source)
}

#[test]
fn compatibility_default_and_explicit_response_limit_are_bound_exactly() {
    let (_tab, host, current) = current_source("/response-limit-binding");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(2).expect("tracker");

    let defaulted = tracker
        .register_network(&current, "http://127.0.0.1:1/default-limit")
        .expect("register default response limit");
    assert_eq!(
        defaulted.max_response_body_bytes(),
        DEFAULT_MAX_RESPONSE_BODY_BYTES
    );
    assert_eq!(
        tracker.preflight_network_once(&host, defaulted),
        Ok(EngineNetworkTargetDecision::DeniedUnsupported)
    );

    let request = tracker
        .register_network_with_request_parts_and_response_limit(
            &current,
            FetchMethod::post(),
            "http://127.0.0.1:1/explicit-limit",
            HeaderList::default(),
            Some(vec![1]),
            RequestMode::Cors,
            CredentialsMode::SameOrigin,
            RedirectMode::Follow,
            RequestDestination::Empty,
            12_345,
        )
        .expect("register explicit response limit");
    assert_eq!(request.max_response_body_bytes(), 12_345);
    assert_eq!(
        tracker.preflight_network_once(&host, request),
        Ok(EngineNetworkTargetDecision::DeniedUnsupported)
    );
}

#[test]
fn zero_response_limit_is_rejected_before_slot_or_body_budget_use() {
    let (_tab, host, current) = current_source("/response-limit-zero");
    let mut tracker =
        EnginePrivilegedRequestTracker::try_new_with_body_budget(1, 4).expect("tracker");

    assert_eq!(
        tracker.register_network_with_request_parts_and_response_limit(
            &current,
            FetchMethod::post(),
            "http://127.0.0.1:1/zero-limit",
            HeaderList::default(),
            Some(vec![1, 2, 3, 4]),
            RequestMode::Cors,
            CredentialsMode::SameOrigin,
            RedirectMode::Follow,
            RequestDestination::Empty,
            0,
        ),
        Err(EnginePrivilegedRequestError::InvalidNetworkResponseBodyLimit)
    );
    assert_eq!(tracker.pending_requests(), 0);
    assert_eq!(tracker.pending_network_body_bytes(), 0);

    let request = tracker
        .register_network_with_request_parts_and_response_limit(
            &current,
            FetchMethod::post(),
            "http://127.0.0.1:1/valid-after-zero",
            HeaderList::default(),
            Some(vec![1, 2, 3, 4]),
            RequestMode::Cors,
            CredentialsMode::SameOrigin,
            RedirectMode::Follow,
            RequestDestination::Empty,
            1,
        )
        .expect("zero limit consumed no slot or body budget");
    assert_eq!(tracker.pending_network_body_bytes(), 4);
    assert_eq!(
        tracker.preflight_network_once(&host, request),
        Ok(EngineNetworkTargetDecision::DeniedUnsupported)
    );
}

#[test]
fn response_limit_substitution_burns_once_and_releases_body_budget() {
    let (_tab, host, current) = current_source("/response-limit-mismatch");
    let mut tracker =
        EnginePrivilegedRequestTracker::try_new_with_body_budget(1, 4).expect("tracker");
    let request = tracker
        .register_network_with_request_parts_and_response_limit(
            &current,
            FetchMethod::post(),
            "http://127.0.0.1:1/response-limit-mismatch",
            HeaderList::default(),
            Some(vec![1, 2, 3, 4]),
            RequestMode::Cors,
            CredentialsMode::SameOrigin,
            RedirectMode::Follow,
            RequestDestination::Empty,
            1024,
        )
        .expect("register response-limit request");
    assert_eq!(tracker.pending_network_body_bytes(), 4);

    let mut forged = request.clone();
    forged.max_response_body_bytes = 2048;
    assert_eq!(
        tracker.preflight_network_once(&host, forged),
        Err(EnginePrivilegedRequestError::MismatchedRequest(request.id()))
    );
    assert_eq!(tracker.pending_requests(), 0);
    assert_eq!(tracker.pending_network_body_bytes(), 0);
    assert_eq!(
        tracker.preflight_network_once(&host, request.clone()),
        Err(EnginePrivilegedRequestError::UnknownRequest(request.id()))
    );
}

#[test]
fn response_limit_debug_is_safe_metadata_and_target_remains_redacted() {
    let (_tab, _host, current) = current_source("/response-limit-debug");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let secret_target = "http://127.0.0.1:1/resource?secret=response-limit-target";
    let request = tracker
        .register_network_with_request_parts_and_response_limit(
            &current,
            FetchMethod::get(),
            secret_target,
            HeaderList::default(),
            None,
            RequestMode::Cors,
            CredentialsMode::SameOrigin,
            RedirectMode::Follow,
            RequestDestination::Empty,
            65_537,
        )
        .expect("register response-limit debug request");
    let debug = format!("{request:?}");

    assert!(!debug.contains(secret_target));
    assert!(!debug.contains("secret=response-limit-target"));
    assert!(debug.contains("max_response_body_bytes"));
    assert!(debug.contains("65537"));
}

#[test]
fn stale_source_still_precedes_target_classification_with_explicit_response_limit() {
    let (tab, mut host, first) = current_source("/response-limit-stale-first");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let request = tracker
        .register_network_with_request_parts_and_response_limit(
            &first,
            FetchMethod::get(),
            "../relative",
            HeaderList::default(),
            None,
            RequestMode::Cors,
            CredentialsMode::SameOrigin,
            RedirectMode::Follow,
            RequestDestination::Document,
            4096,
        )
        .expect("register stale-source response-limit request");
    let replacement = commit_remote(&mut host, tab, serve_once("/response-limit-stale-second"));
    assert_ne!(replacement.navigation_context(), first.navigation_context());

    assert_eq!(
        tracker.preflight_network_once(&host, request),
        Ok(EngineNetworkTargetDecision::DeniedStaleAuthority)
    );
    assert_eq!(tracker.pending_requests(), 0);
}
''')
