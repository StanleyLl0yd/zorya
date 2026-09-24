use super::*;
use crate::engine::{EngineHost, EngineHostError, EngineNavigationPoll};
use crate::network_target_policy::{
    EngineNetworkAuthorizationResult, EngineNetworkTargetDecision,
};
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
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind authorization fixture");
    let port = listener.local_addr().expect("fixture address").port();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept authorization fixture");
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
            EngineNavigationPoll::Pending => panic!("authorization fixture timed out"),
            EngineNavigationPoll::Committed { .. } => break,
            EngineNavigationPoll::Failed { message } => {
                panic!("authorization fixture navigation failed: {message}")
            }
            EngineNavigationPoll::Stale => panic!("authorization fixture navigation went stale"),
        }
    }

    host.committed_document_source(tab)
        .expect("source query")
        .expect("remote source")
}

fn current_source(path: &str) -> (TabId, EngineHost, EngineCommittedDocumentSource) {
    let tab = initial_tab();
    let mut host = EngineHost::new().expect("engine host");
    host.create_view(tab).expect("view");
    let source = commit_remote(&mut host, tab, serve_once(path));
    (tab, host, source)
}

#[test]
fn exact_same_site_request_grants_one_context_capability_and_revokes_explicitly() {
    let (_tab, mut host, source) = current_source("/authorization-current");
    assert_eq!(host.active_privileged_capabilities(), 0);

    let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let request = tracker
        .register_network(&source, "http://127.0.0.1:1/authorized")
        .expect("register request");

    let authorization = match tracker
        .authorize_network_once(&mut host, request)
        .expect("authorize request")
    {
        EngineNetworkAuthorizationResult::Authorized(authorization) => authorization,
        EngineNetworkAuthorizationResult::Denied(decision) => {
            panic!("exact current same-site request was denied: {decision:?}")
        }
    };

    assert_eq!(authorization.authority(), source.authority());
    assert_eq!(authorization.source(), &source);
    assert_eq!(host.active_privileged_capabilities(), 1);
    host.revoke_network_authorization(authorization)
        .expect("explicit revoke");
    assert_eq!(host.active_privileged_capabilities(), 0);
    assert_eq!(tracker.pending_requests(), 0);
}

#[test]
fn authorization_is_one_shot_and_replay_fails_after_explicit_revoke() {
    let (_tab, mut host, source) = current_source("/authorization-replay");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let request = tracker
        .register_network(&source, "http://127.0.0.1:1/replay")
        .expect("register request");

    let authorization = match tracker
        .authorize_network_once(&mut host, request.clone())
        .expect("authorize request")
    {
        EngineNetworkAuthorizationResult::Authorized(authorization) => authorization,
        EngineNetworkAuthorizationResult::Denied(decision) => {
            panic!("exact current same-site request was denied: {decision:?}")
        }
    };
    host.revoke_network_authorization(authorization)
        .expect("explicit revoke");

    assert!(matches!(
        tracker.authorize_network_once(&mut host, request.clone()),
        Err(EnginePrivilegedRequestError::UnknownRequest(id)) if id == request.id()
    ));
    assert_eq!(host.active_privileged_capabilities(), 0);
}

#[test]
fn target_denials_never_grant_host_capability() {
    let (_tab, mut host, source) = current_source("/authorization-denials");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(4).expect("tracker");

    for (target, expected) in [
        ("../relative", EngineNetworkTargetDecision::DeniedInvalidTarget),
        (
            "file:///tmp/zorya-authorization",
            EngineNetworkTargetDecision::DeniedUnsupportedScheme,
        ),
        (
            "https://127.0.0.1/resource",
            EngineNetworkTargetDecision::DeniedCrossSite,
        ),
    ] {
        let request = tracker
            .register_network(&source, target)
            .expect("register denied request");
        match tracker
            .authorize_network_once(&mut host, request)
            .expect("policy result")
        {
            EngineNetworkAuthorizationResult::Denied(decision) => {
                assert_eq!(decision, expected, "{target}");
            }
            EngineNetworkAuthorizationResult::Authorized(_) => {
                panic!("denied target unexpectedly received capability: {target}")
            }
        }
        assert_eq!(host.active_privileged_capabilities(), 0, "{target}");
    }
}

#[test]
fn stale_source_is_denied_before_capability_grant() {
    let (tab, mut host, first) = current_source("/authorization-stale-first");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let request = tracker
        .register_network(&first, "http://127.0.0.1:1/stale")
        .expect("register request");

    let replacement = commit_remote(&mut host, tab, serve_once("/authorization-stale-second"));
    assert_ne!(replacement.navigation_context(), first.navigation_context());
    assert!(matches!(
        tracker
            .authorize_network_once(&mut host, request)
            .expect("stale policy result"),
        EngineNetworkAuthorizationResult::Denied(EngineNetworkTargetDecision::DeniedStaleAuthority)
    ));
    assert_eq!(host.active_privileged_capabilities(), 0);
}

#[test]
fn authorization_debug_redacts_origin_and_raw_capability_identity() {
    let (_tab, mut host, source) = current_source("/authorization-debug");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let request = tracker
        .register_network(&source, "http://127.0.0.1:1/debug")
        .expect("register request");

    let authorization = match tracker
        .authorize_network_once(&mut host, request)
        .expect("authorize request")
    {
        EngineNetworkAuthorizationResult::Authorized(authorization) => authorization,
        EngineNetworkAuthorizationResult::Denied(decision) => {
            panic!("exact current same-site request was denied: {decision:?}")
        }
    };

    let debug = format!("{authorization:?}");
    assert!(!debug.contains("127.0.0.1"));
    assert!(!debug.contains("Capability"));
    assert!(debug.contains("authority"));
    host.revoke_network_authorization(authorization)
        .expect("explicit revoke");
}

#[test]
fn foreign_engine_host_rejects_authorization_without_touching_its_broker() {
    let (tab, mut first_host, source) = current_source("/authorization-first-host");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let request = tracker
        .register_network(&source, "http://127.0.0.1:1/foreign")
        .expect("register request");
    let authorization = match tracker
        .authorize_network_once(&mut first_host, request)
        .expect("authorize request")
    {
        EngineNetworkAuthorizationResult::Authorized(authorization) => authorization,
        EngineNetworkAuthorizationResult::Denied(decision) => {
            panic!("exact current same-site request was denied: {decision:?}")
        }
    };
    assert_eq!(first_host.active_privileged_capabilities(), 1);
    assert!(first_host.close_view(tab).expect("close source view"));
    assert_eq!(first_host.active_privileged_capabilities(), 0);

    let mut replacement_host = EngineHost::new().expect("replacement host");
    replacement_host.create_view(tab).expect("replacement view");
    assert_eq!(replacement_host.active_privileged_capabilities(), 0);
    let error = replacement_host
        .revoke_network_authorization(authorization)
        .expect_err("foreign authorization must be rejected");
    assert!(matches!(
        error,
        EngineHostError::ForeignNetworkAuthorization { .. }
    ));
    assert_eq!(replacement_host.active_privileged_capabilities(), 0);
}
