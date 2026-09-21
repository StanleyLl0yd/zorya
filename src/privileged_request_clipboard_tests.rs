use super::*;
use crate::engine::EngineNavigationPoll;
use crate::{BrowserApp, BrowserWindow, TabId};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
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
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind Clipboard fixture");
    let port = listener.local_addr().expect("fixture address").port();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept Clipboard fixture");
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
            EngineNavigationPoll::Pending => panic!("Clipboard fixture timed out"),
            EngineNavigationPoll::Committed { .. } => break,
            EngineNavigationPoll::Failed { message } => {
                panic!("Clipboard navigation failed: {message}")
            }
            EngineNavigationPoll::Stale => panic!("Clipboard navigation went stale"),
        }
    }
    host.committed_document_source(tab)
        .expect("committed source query")
        .expect("committed remote source")
}

fn current_authority(path: &str) -> (TabId, EngineHost, EngineCommittedDocumentSource) {
    let tab = initial_tab();
    let mut host = EngineHost::new().expect("engine host");
    host.create_view(tab).expect("view");
    let current = commit_remote(&mut host, tab, serve_once(path));
    (tab, host, current)
}

#[test]
fn clipboard_read_binds_exact_default_and_explicit_limits() {
    let (_tab, host, current) = current_authority("/clipboard-read");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(4).expect("tracker");
    let default = tracker
        .register_clipboard_read(&current)
        .expect("default read");
    assert_eq!(
        default.operation_kind(),
        EngineClipboardOperationKind::ReadText
    );
    assert_eq!(default.source(), &current);
    assert_eq!(default.authority(), current.authority());
    assert_eq!(default.source_origin(), current.origin());
    let default_clone = default.clone();
    assert_eq!(default_clone.source(), default.source());
    assert!(std::ptr::eq(
        default_clone.source_origin(),
        default.source_origin()
    ));
    assert_eq!(
        default.max_read_text_bytes(),
        Some(MAX_PRIVILEGED_CLIPBOARD_TEXT_BYTES)
    );
    assert_eq!(default.write_text(), None);
    let minimum = tracker
        .register_clipboard_read_with_limit(&current, 1)
        .expect("minimum read");
    let middle = tracker
        .register_clipboard_read_with_limit(&current, 4096)
        .expect("explicit read");
    let maximum = tracker
        .register_clipboard_read_with_limit(&current, MAX_PRIVILEGED_CLIPBOARD_TEXT_BYTES)
        .expect("explicit maximum read");
    assert_eq!(minimum.max_read_text_bytes(), Some(1));
    assert_eq!(middle.max_read_text_bytes(), Some(4096));
    assert_eq!(
        maximum.max_read_text_bytes(),
        Some(MAX_PRIVILEGED_CLIPBOARD_TEXT_BYTES)
    );
    for request in [default, minimum, middle, maximum] {
        assert_eq!(
            tracker.preflight_clipboard_once(&host, request),
            Ok(EngineClipboardRequestDecision::DeniedUnsupported)
        );
    }
}

#[test]
fn clipboard_read_rejects_zero_and_over_max_without_slot_use() {
    let (_tab, _host, current) = current_authority("/clipboard-read-bounds");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    for bytes in [0, MAX_PRIVILEGED_CLIPBOARD_TEXT_BYTES + 1] {
        assert_eq!(
            tracker.register_clipboard_read_with_limit(&current, bytes),
            Err(
                EnginePrivilegedRequestError::ClipboardReadTextLimitOutOfRange {
                    bytes,
                    max: MAX_PRIVILEGED_CLIPBOARD_TEXT_BYTES,
                }
            )
        );
        assert_eq!(tracker.pending_requests(), 0);
    }
}

#[test]
fn clipboard_write_is_shared_redacted_consumed_once_and_releases_budget() {
    let (_tab, host, current) = current_authority("/clipboard-write");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(2).expect("tracker");
    let secret = "clipboard-secret-do-not-log";
    let request = tracker
        .register_clipboard_write(&current, secret)
        .expect("write request");
    let clone = request.clone();
    assert_eq!(
        request.operation_kind(),
        EngineClipboardOperationKind::WriteText
    );
    assert_eq!(request.write_text(), Some(secret));
    assert_eq!(tracker.pending_clipboard_text_bytes(), secret.len());
    match (&request.operation, &clone.operation) {
        (
            EngineClipboardOperation::WriteText { text: first },
            EngineClipboardOperation::WriteText { text: second },
        ) => assert!(Arc::ptr_eq(first, second)),
        _ => panic!("expected write operations"),
    }
    let debug = format!("{request:?}");
    assert!(!debug.contains(secret));
    assert!(!debug.contains("127.0.0.1"));
    assert!(debug.contains("WriteText"));
    assert!(debug.contains("write_text_bytes"));
    assert_eq!(
        tracker.preflight_clipboard_once(&host, clone),
        Ok(EngineClipboardRequestDecision::DeniedUnsupported)
    );
    assert_eq!(tracker.pending_clipboard_text_bytes(), 0);
    assert_eq!(
        tracker.preflight_clipboard_once(&host, request.clone()),
        Err(EnginePrivilegedRequestError::UnknownRequest(request.id()))
    );
}

#[test]
fn clipboard_write_rejects_per_request_and_aggregate_overflow_before_slot_use() {
    let (_tab, host, current) = current_authority("/clipboard-write-bounds");
    let mut per_request = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let oversized = "x".repeat(MAX_PRIVILEGED_CLIPBOARD_TEXT_BYTES + 1);
    assert_eq!(
        per_request.register_clipboard_write(&current, oversized),
        Err(EnginePrivilegedRequestError::ClipboardWriteTextTooLong {
            bytes: MAX_PRIVILEGED_CLIPBOARD_TEXT_BYTES + 1,
            max: MAX_PRIVILEGED_CLIPBOARD_TEXT_BYTES,
        })
    );
    assert_eq!(per_request.pending_requests(), 0);

    let mut aggregate = EnginePrivilegedRequestTracker::try_new_with_budgets(
        2,
        DEFAULT_MAX_PENDING_PRIVILEGED_NETWORK_BODY_BYTES,
        5,
    )
    .expect("bounded tracker");
    let first = aggregate
        .register_clipboard_write(&current, "12345")
        .expect("first write");
    assert_eq!(
        aggregate.register_clipboard_write(&current, "x"),
        Err(EnginePrivilegedRequestError::ClipboardTextBudgetExceeded {
            pending: 5,
            requested: 1,
            max: 5,
        })
    );
    assert_eq!(aggregate.pending_clipboard_text_bytes(), 5);
    assert_eq!(
        aggregate.preflight_clipboard_once(&host, first),
        Ok(EngineClipboardRequestDecision::DeniedUnsupported)
    );
    assert_eq!(aggregate.pending_clipboard_text_bytes(), 0);
}

#[test]
fn invalid_clipboard_budget_is_rejected_at_tracker_construction() {
    assert!(matches!(
        EnginePrivilegedRequestTracker::try_new_with_budgets(
            1,
            DEFAULT_MAX_PENDING_PRIVILEGED_NETWORK_BODY_BYTES,
            0,
        ),
        Err(EnginePrivilegedRequestError::InvalidClipboardTextBudget)
    ));
}

#[test]
fn clipboard_operation_and_payload_substitution_burn_once_and_release_budget() {
    let (_tab, host, current) = current_authority("/clipboard-mismatch");
    let mut read_tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let read = read_tracker
        .register_clipboard_read_with_limit(&current, 64)
        .expect("read request");
    let forged_limit = EngineClipboardPrivilegedRequest {
        id: read.id,
        source: read.source.clone(),
        operation: EngineClipboardOperation::ReadText {
            max_result_bytes: 63,
        },
    };
    assert_eq!(
        read_tracker.preflight_clipboard_once(&host, forged_limit),
        Err(EnginePrivilegedRequestError::MismatchedRequest(read.id()))
    );

    let mut write_tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let write = write_tracker
        .register_clipboard_write(&current, "original")
        .expect("write request");
    let forged_text = EngineClipboardPrivilegedRequest {
        id: write.id,
        source: write.source.clone(),
        operation: EngineClipboardOperation::WriteText {
            text: Arc::<str>::from("substitute"),
        },
    };
    assert_eq!(
        write_tracker.preflight_clipboard_once(&host, forged_text),
        Err(EnginePrivilegedRequestError::MismatchedRequest(write.id()))
    );
    assert_eq!(write_tracker.pending_clipboard_text_bytes(), 0);

    let mut operation_tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let write = operation_tracker
        .register_clipboard_write(&current, "operation")
        .expect("write request");
    let forged_operation = EngineClipboardPrivilegedRequest {
        id: write.id,
        source: write.source.clone(),
        operation: EngineClipboardOperation::ReadText {
            max_result_bytes: 64,
        },
    };
    assert_eq!(
        operation_tracker.preflight_clipboard_once(&host, forged_operation),
        Err(EnginePrivilegedRequestError::MismatchedRequest(write.id()))
    );
    assert_eq!(operation_tracker.pending_clipboard_text_bytes(), 0);
}

#[test]
fn cross_kind_mismatch_releases_clipboard_and_network_accounting_before_equality() {
    let (_tab, host, current) = current_authority("/clipboard-cross-kind");
    let mut clipboard_tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let clipboard = clipboard_tracker
        .register_clipboard_write(&current, "cross-kind")
        .expect("Clipboard write");
    let forged_network = EngineNetworkPrivilegedRequest {
        id: clipboard.id,
        source: current.clone(),
        method: FetchMethod::get(),
        headers: HeaderList::default(),
        body: None,
        mode: RequestMode::Cors,
        credentials: CredentialsMode::SameOrigin,
        redirect: RedirectMode::Follow,
        destination: RequestDestination::Empty,
        max_response_body_bytes: MAX_PRIVILEGED_NETWORK_RESPONSE_BODY_BYTES,
        target: "http://127.0.0.1:1/cross-kind-forged".to_owned(),
    };
    assert_eq!(
        clipboard_tracker.preflight_network_once(&host, forged_network),
        Err(EnginePrivilegedRequestError::MismatchedRequest(
            clipboard.id()
        ))
    );
    assert_eq!(clipboard_tracker.pending_clipboard_text_bytes(), 0);

    let mut network_tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let network = network_tracker
        .register_network_with_method_headers_and_body(
            &current,
            FetchMethod::post(),
            "http://127.0.0.1:1/cross-kind",
            HeaderList::default(),
            Some(vec![1, 2, 3, 4]),
        )
        .expect("Network body request");
    let forged_clipboard = EngineClipboardPrivilegedRequest {
        id: network.id,
        source: network.source.clone(),
        operation: EngineClipboardOperation::ReadText {
            max_result_bytes: 1,
        },
    };
    assert_eq!(
        network_tracker.preflight_clipboard_once(&host, forged_clipboard),
        Err(EnginePrivilegedRequestError::MismatchedRequest(
            network.id()
        ))
    );
    assert_eq!(network_tracker.pending_network_body_bytes(), 0);
}

#[test]
fn clipboard_source_substitution_burns_once_and_releases_write_budget() {
    let (tab, mut host, first) = current_authority("/clipboard-source-original");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let request = tracker
        .register_clipboard_write(&first, "source-bound")
        .expect("Clipboard request");
    assert_eq!(tracker.pending_clipboard_text_bytes(), "source-bound".len());

    let replacement = commit_remote(&mut host, tab, serve_once("/clipboard-source-replacement"));
    assert_ne!(replacement, first);
    let forged = EngineClipboardPrivilegedRequest {
        id: request.id,
        source: replacement,
        operation: request.operation.clone(),
    };

    assert_eq!(
        tracker.preflight_clipboard_once(&host, forged),
        Err(EnginePrivilegedRequestError::MismatchedRequest(
            request.id()
        ))
    );
    assert_eq!(tracker.pending_requests(), 0);
    assert_eq!(tracker.pending_clipboard_text_bytes(), 0);
    assert_eq!(
        tracker.preflight_clipboard_once(&host, request.clone()),
        Err(EnginePrivilegedRequestError::UnknownRequest(request.id()))
    );
}

#[test]
fn clipboard_source_is_revalidated_only_at_consume_time() {
    let (tab, mut host, first) = current_authority("/clipboard-stale-first");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let request = tracker
        .register_clipboard_write(&first, "stale")
        .expect("Clipboard request");
    let replacement = commit_remote(&mut host, tab, serve_once("/clipboard-stale-replacement"));
    assert_eq!(
        replacement.host_instance(),
        first.authority().host_instance()
    );
    assert_ne!(
        replacement.navigation_context(),
        first.authority().navigation_context()
    );
    assert_eq!(
        tracker.preflight_clipboard_once(&host, request),
        Ok(EngineClipboardRequestDecision::DeniedStaleAuthority)
    );
    assert_eq!(tracker.pending_clipboard_text_bytes(), 0);
}

#[test]
fn local_document_cannot_mint_clipboard_remote_source() {
    let tab = initial_tab();
    let mut host = EngineHost::new().expect("engine host");
    host.create_view(tab).expect("view");
    host.load_local_html(tab, "<main>local</main>")
        .expect("local document");

    assert_eq!(host.committed_document_source(tab), Ok(None));
}

#[test]
fn clipboard_budget_is_independent_from_network_body_budget() {
    let (_tab, host, current) = current_authority("/clipboard-independent-budget");
    let mut tracker =
        EnginePrivilegedRequestTracker::try_new_with_budgets(2, 4, 4).expect("bounded tracker");
    let clipboard = tracker
        .register_clipboard_write(&current, "1234")
        .expect("Clipboard budget");
    let network = tracker
        .register_network_with_method_headers_and_body(
            &current,
            FetchMethod::post(),
            "http://127.0.0.1:1/independent",
            HeaderList::default(),
            Some(vec![1, 2, 3, 4]),
        )
        .expect("Network budget");
    assert_eq!(tracker.pending_clipboard_text_bytes(), 4);
    assert_eq!(tracker.pending_network_body_bytes(), 4);
    assert_eq!(
        tracker.preflight_clipboard_once(&host, clipboard),
        Ok(EngineClipboardRequestDecision::DeniedUnsupported)
    );
    assert_eq!(tracker.pending_clipboard_text_bytes(), 0);
    assert_eq!(tracker.pending_network_body_bytes(), 4);
    assert_eq!(
        tracker.preflight_network_once(&host, network),
        Ok(EngineNetworkTargetDecision::DeniedUnsupported)
    );
    assert_eq!(tracker.pending_network_body_bytes(), 0);
}
