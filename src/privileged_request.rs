use crate::engine::{EngineCommittedDocumentAuthority, EngineHost, EngineHostError};

/// Browser-product taxonomy for future privileged capability requests.
///
/// These values are request labels only. They are not Rarog capability classes or grants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnginePrivilegedRequestKind {
    Network,
    Clipboard,
}

/// Fail-closed result of the privileged-request source preflight.
///
/// There is intentionally no allow/authorized variant in this Z4 slice. A current committed
/// authority only proves that the request source is not stale; the requested operation remains
/// unsupported until a separately reviewed capability-brokering policy exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnginePrivilegedRequestDecision {
    DeniedStaleAuthority,
    DeniedUnsupported,
}

impl EngineHost {
    /// Revalidates the committed remote-document source of a future privileged request and then
    /// denies the request because capability brokering is not implemented yet.
    ///
    /// Replayed or replaced authority is distinguished from a current-but-unsupported request.
    /// Missing or divergent live Host state continues to fail closed through
    /// `validate_committed_document_authority`.
    pub fn preflight_privileged_request(
        &self,
        authority: EngineCommittedDocumentAuthority,
        _kind: EnginePrivilegedRequestKind,
    ) -> Result<EnginePrivilegedRequestDecision, EngineHostError> {
        if !self.validate_committed_document_authority(authority)? {
            return Ok(EnginePrivilegedRequestDecision::DeniedStaleAuthority);
        }

        Ok(EnginePrivilegedRequestDecision::DeniedUnsupported)
    }
}

#[cfg(test)]
mod tests {
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
        let listener = TcpListener::bind((bind_host, 0)).expect("bind privileged fixture server");
        let port = listener.local_addr().expect("fixture address").port();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept privileged fixture request");
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
    ) -> EngineCommittedDocumentAuthority {
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
                EngineNavigationPoll::Pending => panic!("privileged fixture timed out"),
                EngineNavigationPoll::Committed { .. } => break,
                EngineNavigationPoll::Failed { message } => {
                    panic!("privileged fixture navigation failed: {message}")
                }
                EngineNavigationPoll::Stale => panic!("privileged fixture navigation went stale"),
            }
        }

        host.committed_document_authority(tab)
            .expect("committed authority query")
            .expect("committed remote authority")
    }

    #[test]
    fn current_authority_is_still_denied_for_all_unsupported_privileged_kinds() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/current"),
        );

        assert_eq!(
            host.preflight_privileged_request(current, EnginePrivilegedRequestKind::Network),
            Ok(EnginePrivilegedRequestDecision::DeniedUnsupported)
        );
        assert_eq!(
            host.preflight_privileged_request(current, EnginePrivilegedRequestKind::Clipboard),
            Ok(EnginePrivilegedRequestDecision::DeniedUnsupported)
        );

        assert_eq!(
            host.begin_navigation(tab, "mailto:zorya@example.invalid")
                .expect("blocked external protocol"),
            None
        );
        assert_eq!(
            host.preflight_privileged_request(current, EnginePrivilegedRequestKind::Clipboard),
            Ok(EnginePrivilegedRequestDecision::DeniedUnsupported)
        );
    }

    #[test]
    fn replacement_and_local_document_turn_old_request_source_into_stale_denial() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let first = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/first"),
        );
        let same_site = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/second"),
        );

        assert_eq!(same_site.site_process(), first.site_process());
        assert_ne!(same_site.navigation_context(), first.navigation_context());
        assert_eq!(
            host.preflight_privileged_request(first, EnginePrivilegedRequestKind::Network),
            Ok(EnginePrivilegedRequestDecision::DeniedStaleAuthority)
        );

        let cross_site = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "localhost", "/cross-site"),
        );
        assert_ne!(cross_site.site_process(), same_site.site_process());
        assert_eq!(
            host.preflight_privileged_request(same_site, EnginePrivilegedRequestKind::Clipboard),
            Ok(EnginePrivilegedRequestDecision::DeniedStaleAuthority)
        );

        host.load_local_html(tab, "<main>local</main>")
            .expect("replace with local document");
        assert_eq!(
            host.preflight_privileged_request(cross_site, EnginePrivilegedRequestKind::Network),
            Ok(EnginePrivilegedRequestDecision::DeniedStaleAuthority)
        );
    }

    #[test]
    fn closed_or_recreated_view_cannot_reuse_old_request_source() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("first view");
        let stale = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/first-view"),
        );

        assert!(host.close_view(tab).expect("close first view"));
        assert_eq!(
            host.preflight_privileged_request(stale, EnginePrivilegedRequestKind::Clipboard),
            Ok(EnginePrivilegedRequestDecision::DeniedStaleAuthority)
        );

        host.create_view(tab).expect("replacement view");
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/replacement-view"),
        );
        assert_ne!(current.view_generation(), stale.view_generation());
        assert_eq!(
            host.preflight_privileged_request(stale, EnginePrivilegedRequestKind::Network),
            Ok(EnginePrivilegedRequestDecision::DeniedStaleAuthority)
        );
        assert_eq!(
            host.preflight_privileged_request(current, EnginePrivilegedRequestKind::Network),
            Ok(EnginePrivilegedRequestDecision::DeniedUnsupported)
        );
    }
}
