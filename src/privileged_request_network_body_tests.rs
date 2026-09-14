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

fn serve_once(bind_host: &str, url_host: &str, path: &str) -> String {
    let listener = TcpListener::bind((bind_host, 0)).expect("bind privileged body fixture server");
    let port = listener.local_addr().expect("fixture address").port();
    thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .expect("accept privileged body fixture request");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("fixture read timeout");
        let mut request = [0u8; 4096];
        let _ = stream.read(&mut request);
        let response = b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: 18\r\nConnection: close\r\n\r\n<main>Zorya</main>";
        let _ = stream.write_all(response);
        let _ = stream.flush();
    });
    format!("http://{url_host}:{port}{path}")
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
            EngineNavigationPoll::Pending => panic!("privileged body fixture timed out"),
            EngineNavigationPoll::Committed { .. } => break,
            EngineNavigationPoll::Failed { message } => {
                panic!("privileged body fixture navigation failed: {message}")
            }
            EngineNavigationPoll::Stale => panic!("privileged body fixture navigation went stale"),
        }
    }

    host.committed_document_source(tab)
        .expect("committed authority query")
        .expect("committed remote authority")
}

fn current_authority(path: &str) -> (TabId, EngineHost, EngineCommittedDocumentSource) {
    let tab = initial_tab();
    let mut host = EngineHost::new().expect("engine host");
    host.create_view(tab).expect("view");
    let authority = commit_remote(&mut host, tab, serve_once("127.0.0.1", "127.0.0.1", path));
    (tab, host, authority)
}

#[test]
fn exact_network_body_is_shared_consumed_once_and_remains_unsupported() {
    let (_tab, host, current) = current_authority("/body-bound");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(2).expect("tracker");
    let body = b"exact-request-body".to_vec();
    let request = tracker
        .register_network_with_method_headers_and_body(
            &current,
            FetchMethod::post(),
            "http://127.0.0.1:1/body-bound",
            HeaderList::default(),
            Some(body.clone()),
        )
        .expect("register body-bound request");
    let cloned = request.clone();

    assert_eq!(request.body(), Some(body.as_slice()));
    assert_eq!(tracker.pending_network_body_bytes(), body.len());
    assert!(std::ptr::eq(
        request.body().expect("body"),
        cloned.body().expect("cloned body")
    ));
    assert_eq!(
        tracker.preflight_network_once(&host, cloned),
        Ok(EngineNetworkTargetDecision::DeniedUnsupported)
    );
    assert_eq!(tracker.pending_network_body_bytes(), 0);
    assert_eq!(tracker.pending_requests(), 0);
    assert_eq!(
        tracker.preflight_network_once(&host, request.clone()),
        Err(EnginePrivilegedRequestError::UnknownRequest(request.id()))
    );
}

#[test]
fn absent_and_present_empty_bodies_are_distinct_and_mismatch_burns_slot() {
    let (_tab, host, current) = current_authority("/empty-body-identity");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let request = tracker
        .register_network_with_method_headers_and_body(
            &current,
            FetchMethod::post(),
            "http://127.0.0.1:1/empty-body",
            HeaderList::default(),
            Some(Vec::new()),
        )
        .expect("register present empty body");
    assert_eq!(request.body(), Some(&[][..]));
    assert_eq!(tracker.pending_network_body_bytes(), 0);

    let forged = EngineNetworkPrivilegedRequest {
        id: request.id,
        source: request.source.clone(),
        method: request.method.clone(),
        headers: request.headers.clone(),
        body: None,
        mode: RequestMode::Cors,
        credentials: CredentialsMode::SameOrigin,
        redirect: RedirectMode::Follow,
        destination: RequestDestination::Empty,
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
fn rarog_method_body_semantics_reject_get_and_head_before_registration() {
    let (_tab, host, current) = current_authority("/body-method-policy");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(2).expect("tracker");

    for method in [FetchMethod::get(), FetchMethod::head()] {
        let expected = method.clone();
        assert_eq!(
            tracker.register_network_with_method_headers_and_body(
                &current,
                method,
                "http://127.0.0.1:1/body-method",
                HeaderList::default(),
                Some(Vec::new()),
            ),
            Err(EnginePrivilegedRequestError::NetworkBodyNotPermitted { method: expected })
        );
        assert_eq!(tracker.pending_requests(), 0);
        assert_eq!(tracker.pending_network_body_bytes(), 0);
    }

    let custom = FetchMethod::try_new("PATCH").expect("Rarog custom method");
    assert!(custom.permits_body());
    let request = tracker
        .register_network_with_method_headers_and_body(
            &current,
            custom,
            "http://127.0.0.1:1/custom-body-method",
            HeaderList::default(),
            Some(vec![1]),
        )
        .expect("body-permitting custom method");
    assert_eq!(tracker.pending_network_body_bytes(), 1);
    assert_eq!(
        tracker.preflight_network_once(&host, request),
        Ok(EngineNetworkTargetDecision::DeniedUnsupported)
    );
    assert_eq!(tracker.pending_network_body_bytes(), 0);
}

#[test]
fn per_request_body_bound_is_enforced_without_consuming_slot_or_budget() {
    let (_tab, host, current) = current_authority("/body-size-bound");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let oversized = vec![0u8; MAX_PRIVILEGED_NETWORK_BODY_BYTES + 1];

    assert_eq!(
        tracker.register_network_with_method_headers_and_body(
            &current,
            FetchMethod::post(),
            "http://127.0.0.1:1/oversized-body",
            HeaderList::default(),
            Some(oversized),
        ),
        Err(EnginePrivilegedRequestError::NetworkBodyTooLong {
            bytes: MAX_PRIVILEGED_NETWORK_BODY_BYTES + 1,
            max: MAX_PRIVILEGED_NETWORK_BODY_BYTES,
        })
    );
    assert_eq!(tracker.pending_requests(), 0);
    assert_eq!(tracker.pending_network_body_bytes(), 0);

    let request = tracker
        .register_network_with_method_headers_and_body(
            &current,
            FetchMethod::post(),
            "http://127.0.0.1:1/max-body",
            HeaderList::default(),
            Some(vec![0u8; MAX_PRIVILEGED_NETWORK_BODY_BYTES]),
        )
        .expect("maximum body accepted");
    assert_eq!(
        tracker.pending_network_body_bytes(),
        MAX_PRIVILEGED_NETWORK_BODY_BYTES
    );
    assert_eq!(
        tracker.preflight_network_once(&host, request),
        Ok(EngineNetworkTargetDecision::DeniedUnsupported)
    );
    assert_eq!(tracker.pending_network_body_bytes(), 0);
}

#[test]
fn aggregate_body_budget_is_bounded_and_released_after_consume() {
    let (_tab, host, current) = current_authority("/aggregate-body-budget");
    let mut tracker =
        EnginePrivilegedRequestTracker::try_new_with_body_budget(4, 4).expect("tracker");
    let first = tracker
        .register_network_with_method_headers_and_body(
            &current,
            FetchMethod::post(),
            "http://127.0.0.1:1/body-one",
            HeaderList::default(),
            Some(vec![1, 2]),
        )
        .expect("first body");
    let second = tracker
        .register_network_with_method_headers_and_body(
            &current,
            FetchMethod::post(),
            "http://127.0.0.1:1/body-two",
            HeaderList::default(),
            Some(vec![3, 4]),
        )
        .expect("second body");
    assert_eq!(tracker.pending_network_body_bytes(), 4);

    assert_eq!(
        tracker.register_network_with_method_headers_and_body(
            &current,
            FetchMethod::post(),
            "http://127.0.0.1:1/body-over-budget",
            HeaderList::default(),
            Some(vec![5]),
        ),
        Err(EnginePrivilegedRequestError::NetworkBodyBudgetExceeded {
            pending: 4,
            requested: 1,
            max: 4,
        })
    );
    assert_eq!(tracker.pending_requests(), 2);
    assert_eq!(tracker.pending_network_body_bytes(), 4);

    assert_eq!(
        tracker.preflight_network_once(&host, first),
        Ok(EngineNetworkTargetDecision::DeniedUnsupported)
    );
    assert_eq!(tracker.pending_network_body_bytes(), 2);
    let third = tracker
        .register_network_with_method_headers_and_body(
            &current,
            FetchMethod::post(),
            "http://127.0.0.1:1/body-after-release",
            HeaderList::default(),
            Some(vec![5]),
        )
        .expect("released budget is reusable");
    assert_eq!(tracker.pending_network_body_bytes(), 3);

    assert_eq!(
        tracker.preflight_network_once(&host, second),
        Ok(EngineNetworkTargetDecision::DeniedUnsupported)
    );
    assert_eq!(
        tracker.preflight_network_once(&host, third),
        Ok(EngineNetworkTargetDecision::DeniedUnsupported)
    );
    assert_eq!(tracker.pending_network_body_bytes(), 0);
}

#[test]
fn body_mismatch_burns_slot_and_releases_aggregate_budget() {
    let (_tab, host, current) = current_authority("/body-mismatch");
    let mut tracker =
        EnginePrivilegedRequestTracker::try_new_with_body_budget(1, 8).expect("tracker");
    let request = tracker
        .register_network_with_method_headers_and_body(
            &current,
            FetchMethod::post(),
            "http://127.0.0.1:1/body-mismatch",
            HeaderList::default(),
            Some(b"original".to_vec()),
        )
        .expect("register body request");
    assert_eq!(tracker.pending_network_body_bytes(), 8);
    let forged = EngineNetworkPrivilegedRequest {
        id: request.id,
        source: request.source.clone(),
        method: request.method.clone(),
        headers: request.headers.clone(),
        body: Some(Arc::<[u8]>::from(b"forged!!".to_vec())),
        mode: RequestMode::Cors,
        credentials: CredentialsMode::SameOrigin,
        redirect: RedirectMode::Follow,
        destination: RequestDestination::Empty,
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
fn cross_kind_mismatch_burns_network_slot_and_releases_body_budget() {
    let (_tab, host, current) = current_authority("/cross-kind-body-burn");
    let mut tracker =
        EnginePrivilegedRequestTracker::try_new_with_body_budget(1, 4).expect("tracker");
    let request = tracker
        .register_network_with_method_headers_and_body(
            &current,
            FetchMethod::post(),
            "http://127.0.0.1:1/cross-kind-body",
            HeaderList::default(),
            Some(vec![1, 2, 3, 4]),
        )
        .expect("register body request");
    let forged = EnginePrivilegedRequest {
        id: request.id,
        authority: request.authority(),
        kind: EnginePrivilegedRequestKind::Clipboard,
    };

    assert_eq!(
        tracker.preflight_once(&host, forged),
        Err(EnginePrivilegedRequestError::MismatchedRequest(
            request.id()
        ))
    );
    assert_eq!(tracker.pending_requests(), 0);
    assert_eq!(tracker.pending_network_body_bytes(), 0);
}

#[test]
fn body_debug_is_redacted_but_safe_body_metadata_is_visible() {
    let (_tab, _host, current) = current_authority("/body-debug");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let secret = b"body-secret-do-not-log".to_vec();
    let request = tracker
        .register_network_with_method_headers_and_body(
            &current,
            FetchMethod::post(),
            "http://127.0.0.1:1/body-debug",
            HeaderList::default(),
            Some(secret.clone()),
        )
        .expect("register body request");
    let debug = format!("{request:?}");

    assert!(!debug.contains("body-secret-do-not-log"));
    assert!(debug.contains("body_present"));
    assert!(debug.contains("true"));
    assert!(debug.contains("body_bytes"));
    assert!(debug.contains(&secret.len().to_string()));
}

#[test]
fn stale_source_still_precedes_target_classification_for_body_request() {
    let (tab, mut host, first) = current_authority("/body-stale-first");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let request = tracker
        .register_network_with_method_headers_and_body(
            &first,
            FetchMethod::post(),
            "../relative",
            HeaderList::default(),
            Some(vec![1, 2, 3]),
        )
        .expect("register raw target with body");
    assert_eq!(tracker.pending_network_body_bytes(), 3);

    let replacement = commit_remote(
        &mut host,
        tab,
        serve_once("127.0.0.1", "127.0.0.1", "/body-stale-replacement"),
    );
    assert_ne!(replacement.navigation_context(), first.navigation_context());
    assert_eq!(
        tracker.preflight_network_once(&host, request),
        Ok(EngineNetworkTargetDecision::DeniedStaleAuthority)
    );
    assert_eq!(tracker.pending_network_body_bytes(), 0);
}

#[test]
fn body_budget_constructor_and_checked_add_fail_closed() {
    assert!(matches!(
        EnginePrivilegedRequestTracker::try_new_with_body_budget(1, 0),
        Err(EnginePrivilegedRequestError::InvalidNetworkBodyBudget)
    ));

    let (_tab, _host, current) = current_authority("/body-overflow");
    let mut tracker =
        EnginePrivilegedRequestTracker::try_new_with_body_budget(2, usize::MAX).expect("tracker");
    tracker.pending_network_body_bytes = usize::MAX;
    assert_eq!(
        tracker.register_network_with_method_headers_and_body(
            &current,
            FetchMethod::post(),
            "http://127.0.0.1:1/body-overflow",
            HeaderList::default(),
            Some(vec![1]),
        ),
        Err(EnginePrivilegedRequestError::NetworkBodyBudgetExceeded {
            pending: usize::MAX,
            requested: 1,
            max: usize::MAX,
        })
    );
    assert_eq!(tracker.pending_requests(), 0);
}

#[test]
fn body_accounting_underflow_fails_closed_after_burning_slot() {
    let (_tab, host, current) = current_authority("/body-accounting-underflow");
    let mut tracker =
        EnginePrivilegedRequestTracker::try_new_with_body_budget(1, 4).expect("tracker");
    let request = tracker
        .register_network_with_method_headers_and_body(
            &current,
            FetchMethod::post(),
            "http://127.0.0.1:1/body-accounting-underflow",
            HeaderList::default(),
            Some(vec![1]),
        )
        .expect("register body request");
    tracker.pending_network_body_bytes = 0;

    assert_eq!(
        tracker.preflight_network_once(&host, request),
        Err(EnginePrivilegedRequestError::NetworkBodyAccountingInvariant)
    );
    assert_eq!(tracker.pending_requests(), 0);
    assert_eq!(tracker.pending_network_body_bytes(), 0);
}
