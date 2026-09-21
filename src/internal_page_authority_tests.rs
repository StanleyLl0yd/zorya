use crate::engine::{
    EngineCommittedInternalPageAuthority, EngineHost, EngineInternalPage, EngineNavigationPoll,
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

fn serve_once(response: Vec<u8>) -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind internal-page fixture");
    let port = listener.local_addr().expect("fixture address").port();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept fixture request");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("fixture read timeout");
        let mut request = [0u8; 4096];
        let _ = stream.read(&mut request);
        let _ = stream.write_all(&response);
        let _ = stream.flush();
    });
    format!("http://127.0.0.1:{port}/")
}

fn serve_html_once(body: &str) -> String {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .into_bytes();
    serve_once(response)
}

fn serve_invalid_utf8_once() -> String {
    let mut response =
        b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 1\r\nConnection: close\r\n\r\n"
            .to_vec();
    response.push(0xE9);
    serve_once(response)
}

fn serve_stalled_once() -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind stalled fixture");
    let port = listener.local_addr().expect("stalled fixture address").port();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept stalled fixture");
        let mut request = [0u8; 4096];
        let _ = stream.read(&mut request);
        thread::sleep(Duration::from_secs(2));
    });
    format!("http://127.0.0.1:{port}/stall")
}

fn poll_terminal(host: &mut EngineHost, request: crate::engine::EngineNavigationRequest) -> EngineNavigationPoll {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match host.poll_navigation(request).expect("poll navigation") {
            EngineNavigationPoll::Pending if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(5));
            }
            EngineNavigationPoll::Pending => panic!("navigation fixture timed out"),
            terminal => return terminal,
        }
    }
}

fn load_start(host: &mut EngineHost, tab: TabId) -> EngineCommittedInternalPageAuthority {
    host.load_internal_page(tab, EngineInternalPage::Start)
        .expect("load bundled Start page")
}

#[test]
fn bundled_internal_page_mints_authority_but_generic_local_html_never_does() {
    let tab = initial_tab();
    let mut host = EngineHost::new().expect("engine host");
    host.create_view(tab).expect("view");

    assert_eq!(host.committed_internal_page_authority(tab), Ok(None));
    assert_eq!(host.committed_document_authority(tab), Ok(None));

    let authority = load_start(&mut host, tab);
    assert_eq!(authority.page(), EngineInternalPage::Start);
    assert_ne!(authority.document().get(), 0);
    assert_eq!(
        host.committed_internal_page_authority(tab),
        Ok(Some(authority))
    );
    assert_eq!(
        host.validate_committed_internal_page_authority(authority),
        Ok(true)
    );
    assert_eq!(host.committed_document_authority(tab), Ok(None));

    host.load_local_html(tab, include_str!("../assets/z1-start.html"))
        .expect("generic local copy of exact bundled source");
    assert_eq!(host.committed_internal_page_authority(tab), Ok(None));
    assert_eq!(
        host.validate_committed_internal_page_authority(authority),
        Ok(false)
    );
    assert_eq!(host.committed_document_authority(tab), Ok(None));
}

#[test]
fn same_internal_page_reload_rotates_document_token_and_blocked_navigation_preserves_current() {
    let tab = initial_tab();
    let mut host = EngineHost::new().expect("engine host");
    host.create_view(tab).expect("view");

    let first = load_start(&mut host, tab);
    assert_eq!(
        host.begin_navigation(tab, "file:///tmp/not-internal")
            .expect("blocked navigation result"),
        None
    );
    assert_eq!(
        host.committed_internal_page_authority(tab),
        Ok(Some(first))
    );

    let second = load_start(&mut host, tab);
    assert_eq!(second.host_instance(), first.host_instance());
    assert_eq!(second.tab(), first.tab());
    assert_eq!(second.view_generation(), first.view_generation());
    assert_eq!(second.page(), first.page());
    assert!(second.document().get() > first.document().get());
    assert_eq!(
        host.validate_committed_internal_page_authority(first),
        Ok(false)
    );
    assert_eq!(
        host.validate_committed_internal_page_authority(second),
        Ok(true)
    );
}

#[test]
fn pending_cancelled_and_failed_remote_navigation_preserve_committed_internal_authority() {
    let tab = initial_tab();
    let mut host = EngineHost::new().expect("engine host");
    host.create_view(tab).expect("view");
    let authority = load_start(&mut host, tab);

    let pending = host
        .begin_navigation(tab, serve_stalled_once())
        .expect("begin stalled remote")
        .expect("stalled remote forwarded");
    assert_eq!(
        host.committed_internal_page_authority(tab),
        Ok(Some(authority))
    );
    assert!(host.cancel_navigation(pending).expect("cancel stalled remote"));
    assert_eq!(
        host.committed_internal_page_authority(tab),
        Ok(Some(authority))
    );
    assert_eq!(
        host.validate_committed_internal_page_authority(authority),
        Ok(true)
    );

    let failing = host
        .begin_navigation(tab, serve_invalid_utf8_once())
        .expect("begin invalid remote")
        .expect("invalid remote forwarded");
    assert!(matches!(
        poll_terminal(&mut host, failing),
        EngineNavigationPoll::Failed { .. }
    ));
    assert_eq!(
        host.committed_internal_page_authority(tab),
        Ok(Some(authority))
    );
    assert_eq!(
        host.validate_committed_internal_page_authority(authority),
        Ok(true)
    );
}

#[test]
fn committed_remote_document_invalidates_internal_authority_and_establishes_remote_authority() {
    let tab = initial_tab();
    let mut host = EngineHost::new().expect("engine host");
    host.create_view(tab).expect("view");
    let internal = load_start(&mut host, tab);

    let remote = host
        .begin_navigation(tab, serve_html_once("<main>remote</main>"))
        .expect("begin remote")
        .expect("remote forwarded");
    assert!(matches!(
        poll_terminal(&mut host, remote),
        EngineNavigationPoll::Committed { .. }
    ));

    assert_eq!(host.committed_internal_page_authority(tab), Ok(None));
    assert_eq!(
        host.validate_committed_internal_page_authority(internal),
        Ok(false)
    );
    assert!(
        host.committed_document_authority(tab)
            .expect("remote authority query")
            .is_some()
    );
}

#[test]
fn closed_recreated_view_and_replacement_host_reject_stale_internal_authority() {
    let tab = initial_tab();
    let mut first_host = EngineHost::new().expect("first host");
    first_host.create_view(tab).expect("first view");
    let stale = load_start(&mut first_host, tab);
    assert_eq!(
        first_host.validate_committed_internal_page_authority(stale),
        Ok(true)
    );

    assert!(first_host.close_view(tab).expect("close first view"));
    assert_eq!(
        first_host.validate_committed_internal_page_authority(stale),
        Ok(false)
    );

    first_host.create_view(tab).expect("recreated view");
    let recreated = load_start(&mut first_host, tab);
    assert_eq!(recreated.host_instance(), stale.host_instance());
    assert_ne!(recreated.view_generation(), stale.view_generation());
    assert_eq!(
        first_host.validate_committed_internal_page_authority(stale),
        Ok(false)
    );
    assert_eq!(
        first_host.validate_committed_internal_page_authority(recreated),
        Ok(true)
    );

    let mut replacement_host = EngineHost::new().expect("replacement host");
    replacement_host.create_view(tab).expect("replacement view");
    let replacement = load_start(&mut replacement_host, tab);
    assert_eq!(replacement.tab(), stale.tab());
    assert_eq!(replacement.view_generation(), stale.view_generation());
    assert_eq!(replacement.document(), stale.document());
    assert_ne!(replacement.host_instance(), stale.host_instance());
    assert_eq!(
        replacement_host.validate_committed_internal_page_authority(stale),
        Ok(false)
    );
    assert_eq!(
        replacement_host.validate_committed_internal_page_authority(replacement),
        Ok(true)
    );
}
