use super::*;
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
        match host
            .poll_navigation(request)
            .expect("poll remote navigation")
        {
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
        Err(EnginePrivilegedRequestError::MismatchedRequest(
            request.id()
        ))
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
