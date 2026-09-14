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
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind envelope fixture");
    let port = listener.local_addr().expect("fixture address").port();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept envelope fixture request");
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
            EngineNavigationPoll::Pending => panic!("envelope fixture timed out"),
            EngineNavigationPoll::Committed { .. } => break,
            EngineNavigationPoll::Failed { message } => {
                panic!("envelope fixture navigation failed: {message}")
            }
            EngineNavigationPoll::Stale => panic!("envelope fixture navigation went stale"),
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
fn convenience_defaults_and_explicit_fetch_metadata_are_bound_exactly() {
    let (_tab, host, current) = current_source("/envelope-binding");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(2).expect("tracker");

    let defaults = tracker
        .register_network(&current, "http://127.0.0.1:1/default-envelope")
        .expect("register default envelope");
    assert_eq!(defaults.mode(), RequestMode::Cors);
    assert_eq!(defaults.credentials(), CredentialsMode::SameOrigin);
    assert_eq!(defaults.redirect(), RedirectMode::Follow);
    assert_eq!(defaults.destination(), &RequestDestination::Empty);
    assert_eq!(
        tracker.preflight_network_once(&host, defaults),
        Ok(EngineNetworkTargetDecision::DeniedUnsupported)
    );

    let destination = RequestDestination::Other(String::from("zorya-subresource"));
    let request = tracker
        .register_network_with_request_parts(
            &current,
            FetchMethod::post(),
            "http://127.0.0.1:1/explicit-envelope",
            HeaderList::default(),
            Some(vec![1]),
            RequestMode::NoCors,
            CredentialsMode::Include,
            RedirectMode::Manual,
            destination.clone(),
        )
        .expect("register explicit envelope");
    assert_eq!(request.mode(), RequestMode::NoCors);
    assert_eq!(request.credentials(), CredentialsMode::Include);
    assert_eq!(request.redirect(), RedirectMode::Manual);
    assert_eq!(request.destination(), &destination);
    assert_eq!(
        tracker.preflight_network_once(&host, request),
        Ok(EngineNetworkTargetDecision::DeniedUnsupported)
    );
}

#[test]
fn every_fetch_metadata_substitution_burns_once_and_releases_body_budget() {
    let (_tab, host, current) = current_source("/envelope-mismatch");

    for case in 0..4 {
        let mut tracker =
            EnginePrivilegedRequestTracker::try_new_with_body_budget(1, 4).expect("tracker");
        let request = tracker
            .register_network_with_request_parts(
                &current,
                FetchMethod::post(),
                "http://127.0.0.1:1/envelope-mismatch",
                HeaderList::default(),
                Some(vec![1, 2, 3, 4]),
                RequestMode::Cors,
                CredentialsMode::SameOrigin,
                RedirectMode::Follow,
                RequestDestination::Empty,
            )
            .expect("register envelope request");
        assert_eq!(tracker.pending_network_body_bytes(), 4);

        let mut forged = request.clone();
        match case {
            0 => forged.mode = RequestMode::SameOrigin,
            1 => forged.credentials = CredentialsMode::Include,
            2 => forged.redirect = RedirectMode::Error,
            3 => forged.destination = RequestDestination::Script,
            _ => unreachable!(),
        }
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
}

#[test]
fn other_destination_is_exact_and_bounded_before_slot_or_body_budget_use() {
    let (_tab, host, current) = current_source("/destination-bound");
    let mut tracker =
        EnginePrivilegedRequestTracker::try_new_with_body_budget(1, 1).expect("tracker");
    let maximum = "x".repeat(MAX_PRIVILEGED_NETWORK_DESTINATION_BYTES);
    let request = tracker
        .register_network_with_request_parts(
            &current,
            FetchMethod::post(),
            "http://127.0.0.1:1/destination-max",
            HeaderList::default(),
            Some(vec![1]),
            RequestMode::Cors,
            CredentialsMode::SameOrigin,
            RedirectMode::Follow,
            RequestDestination::Other(maximum.clone()),
        )
        .expect("maximum destination accepted");
    assert_eq!(request.destination(), &RequestDestination::Other(maximum));
    assert_eq!(tracker.pending_requests(), 1);
    assert_eq!(tracker.pending_network_body_bytes(), 1);
    assert_eq!(
        tracker.preflight_network_once(&host, request),
        Ok(EngineNetworkTargetDecision::DeniedUnsupported)
    );
    assert_eq!(tracker.pending_network_body_bytes(), 0);

    let oversized = "y".repeat(MAX_PRIVILEGED_NETWORK_DESTINATION_BYTES + 1);
    assert_eq!(
        tracker.register_network_with_request_parts(
            &current,
            FetchMethod::post(),
            "http://127.0.0.1:1/destination-oversized",
            HeaderList::default(),
            Some(vec![1]),
            RequestMode::Cors,
            CredentialsMode::SameOrigin,
            RedirectMode::Follow,
            RequestDestination::Other(oversized),
        ),
        Err(EnginePrivilegedRequestError::NetworkDestinationTooLong {
            bytes: MAX_PRIVILEGED_NETWORK_DESTINATION_BYTES + 1,
            max: MAX_PRIVILEGED_NETWORK_DESTINATION_BYTES,
        })
    );
    assert_eq!(tracker.pending_requests(), 0);
    assert_eq!(tracker.pending_network_body_bytes(), 0);
}

#[test]
fn destination_other_debug_contents_are_redacted() {
    let (_tab, _host, current) = current_source("/destination-debug");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let secret = "destination-secret-do-not-log";
    let request = tracker
        .register_network_with_request_parts(
            &current,
            FetchMethod::get(),
            "http://127.0.0.1:1/destination-debug",
            HeaderList::default(),
            None,
            RequestMode::Navigate,
            CredentialsMode::Omit,
            RedirectMode::Error,
            RequestDestination::Other(secret.to_owned()),
        )
        .expect("register debug request");
    let debug = format!("{request:?}");

    assert!(!debug.contains(secret));
    assert!(debug.contains("destination_kind"));
    assert!(debug.contains("other"));
    assert!(debug.contains("destination_other_bytes"));
    assert!(debug.contains(&secret.len().to_string()));
    assert!(debug.contains("Navigate"));
    assert!(debug.contains("Omit"));
    assert!(debug.contains("Error"));
}

#[test]
fn stale_source_still_precedes_target_classification_for_explicit_envelope() {
    let (tab, mut host, first) = current_source("/envelope-stale-first");
    let mut tracker =
        EnginePrivilegedRequestTracker::try_new_with_body_budget(1, 3).expect("tracker");
    let request = tracker
        .register_network_with_request_parts(
            &first,
            FetchMethod::post(),
            "../relative",
            HeaderList::default(),
            Some(vec![1, 2, 3]),
            RequestMode::SameOrigin,
            CredentialsMode::Include,
            RedirectMode::Manual,
            RequestDestination::Document,
        )
        .expect("register raw target with explicit envelope");
    let replacement = commit_remote(&mut host, tab, serve_once("/envelope-stale-second"));
    assert_ne!(replacement.navigation_context(), first.navigation_context());

    assert_eq!(
        tracker.preflight_network_once(&host, request),
        Ok(EngineNetworkTargetDecision::DeniedStaleAuthority)
    );
    assert_eq!(tracker.pending_network_body_bytes(), 0);
    assert_eq!(tracker.pending_requests(), 0);
}
