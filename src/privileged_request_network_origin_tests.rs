use super::*;
use crate::engine::EngineNavigationPoll;
use crate::{BrowserApp, BrowserWindow, TabId};
use rarog_url::WebUrl;
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
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind Origin fixture");
    let port = listener.local_addr().expect("fixture address").port();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept Origin fixture request");
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
            EngineNavigationPoll::Pending => panic!("Origin fixture timed out"),
            EngineNavigationPoll::Committed { .. } => break,
            EngineNavigationPoll::Failed { message } => {
                panic!("Origin fixture navigation failed: {message}")
            }
            EngineNavigationPoll::Stale => panic!("Origin fixture navigation went stale"),
        }
    }
    host.committed_document_source(tab)
        .expect("committed source query")
        .expect("committed remote source")
}

fn current_source(path: &str) -> (TabId, EngineHost, EngineCommittedDocumentSource, String) {
    let tab = initial_tab();
    let mut host = EngineHost::new().expect("engine host");
    host.create_view(tab).expect("view");
    let location = serve_once(path);
    let source = commit_remote(&mut host, tab, location.clone());
    (tab, host, source, location)
}

#[test]
fn source_uses_exact_canonical_rarog_origin_and_clone_shares_it() {
    let (_tab, _host, source, location) = current_source("/exact-origin");
    let expected = WebUrl::parse(&location)
        .expect("canonical source URL")
        .origin()
        .expect("canonical source Origin");
    assert_eq!(source.origin(), &expected);
    assert!(!source.origin().is_opaque());

    let cloned = source.clone();
    assert!(std::ptr::eq(source.origin(), cloned.origin()));
    assert_eq!(cloned.authority(), source.authority());
}

#[test]
fn same_site_different_origin_remains_target_policy_not_origin_policy() {
    let (_tab, host, source, _location) = current_source("/same-site-other-port");
    let other = WebUrl::parse("http://127.0.0.1:1/resource")
        .expect("other-port URL")
        .origin()
        .expect("other-port Origin");
    assert_ne!(source.origin(), &other);
    assert!(source.origin().site().same_site(&other.site()));
    assert_eq!(
        host.preflight_network_target(&source, "http://127.0.0.1:1/resource"),
        Ok(EngineNetworkTargetDecision::DeniedUnsupported)
    );
}

#[test]
fn local_document_cannot_mint_network_source() {
    let (tab, mut host, _source, _location) = current_source("/local-replacement");
    host.load_local_html(tab, "<main>local</main>")
        .expect("replace with local document");
    assert_eq!(host.committed_document_source(tab), Ok(None));
}

#[test]
fn source_and_request_debug_redact_origin_serialization() {
    let (_tab, _host, source, _location) = current_source("/origin-debug");
    let serialized = source.origin().ascii_serialization();
    assert!(!format!("{source:?}").contains(&serialized));

    let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let request = tracker
        .register_network(&source, "http://127.0.0.1:1/resource?secret=target")
        .expect("register source-bound request");
    let debug = format!("{request:?}");
    assert!(!debug.contains(&serialized));
    assert!(!debug.contains("secret=target"));
}

#[test]
fn source_substitution_burns_once_and_releases_body_budget() {
    let (tab, mut host, first, _location) = current_source("/origin-first");
    let mut tracker =
        EnginePrivilegedRequestTracker::try_new_with_body_budget(1, 4).expect("tracker");
    let request = tracker
        .register_network_with_method_headers_and_body(
            &first,
            FetchMethod::post(),
            "http://127.0.0.1:1/resource",
            HeaderList::default(),
            Some(vec![1, 2, 3, 4]),
        )
        .expect("register body request");
    assert_eq!(tracker.pending_network_body_bytes(), 4);

    let replacement = commit_remote(&mut host, tab, serve_once("/origin-replacement"));
    assert_ne!(replacement.navigation_context(), first.navigation_context());
    let forged = EngineNetworkPrivilegedRequest {
        id: request.id,
        source: replacement,
        method: request.method.clone(),
        headers: request.headers.clone(),
        body: request.body.clone(),
        mode: RequestMode::Cors,
        credentials: CredentialsMode::SameOrigin,
        redirect: RedirectMode::Follow,
        destination: RequestDestination::Empty,
        max_response_body_bytes: request.max_response_body_bytes,
        target: request.target.clone(),
    };
    assert_eq!(
        tracker.preflight_network_once(&host, forged),
        Err(EnginePrivilegedRequestError::MismatchedRequest(
            request.id()
        ))
    );
    assert_eq!(tracker.pending_requests(), 0);
    assert_eq!(tracker.pending_network_body_bytes(), 0);
}

#[test]
fn stale_source_precedes_target_classification() {
    let (tab, mut host, first, _location) = current_source("/origin-stale-first");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let request = tracker
        .register_network(&first, "../relative")
        .expect("register raw target");
    let replacement = commit_remote(&mut host, tab, serve_once("/origin-stale-replacement"));
    assert_ne!(replacement.navigation_context(), first.navigation_context());
    assert_eq!(
        tracker.preflight_network_once(&host, request),
        Ok(EngineNetworkTargetDecision::DeniedStaleAuthority)
    );
}

#[test]
fn closed_recreated_view_cannot_reuse_old_source() {
    let (tab, mut host, stale, _location) = current_source("/origin-old-view");
    assert!(host.close_view(tab).expect("close source view"));
    assert_eq!(
        host.preflight_network_target(&stale, "../relative"),
        Ok(EngineNetworkTargetDecision::DeniedStaleAuthority)
    );

    host.create_view(tab).expect("replacement view");
    let current = commit_remote(&mut host, tab, serve_once("/origin-new-view"));
    assert_ne!(current.view_generation(), stale.view_generation());
    assert_eq!(
        host.preflight_network_target(&stale, "http://127.0.0.1:1/resource"),
        Ok(EngineNetworkTargetDecision::DeniedStaleAuthority)
    );
}
