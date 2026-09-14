use crate::engine::{EngineCommittedDocumentAuthority, EngineHost, EngineHostError};
use rarog_url::WebUrl;

/// Fail-closed result of the browser-product Network target policy.
///
/// There is intentionally no allow/authorized variant. `DeniedUnsupported` means only that a
/// current committed remote document requested an HTTP(S) target in the same canonical Rarog Site;
/// actual capability brokering remains unsupported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineNetworkTargetDecision {
    DeniedStaleAuthority,
    DeniedInvalidTarget,
    DeniedUnsupportedScheme,
    DeniedCrossSite,
    DeniedUnsupported,
}

impl EngineHost {
    /// Applies target-aware browser policy for a future privileged Network request.
    ///
    /// Source authority is revalidated before target parsing. Source Site identity comes only from
    /// the live Rarog Host navigation context, and target Site identity comes only from the pinned
    /// Rarog URL contract. This method grants no capability and starts no network operation.
    pub fn preflight_network_target(
        &self,
        authority: EngineCommittedDocumentAuthority,
        target: &str,
    ) -> Result<EngineNetworkTargetDecision, EngineHostError> {
        if !self.validate_committed_document_authority(authority)? {
            return Ok(EngineNetworkTargetDecision::DeniedStaleAuthority);
        }

        let Some(source_site) = self.committed_document_site_identity(authority.tab())? else {
            return Ok(EngineNetworkTargetDecision::DeniedStaleAuthority);
        };

        let target = match WebUrl::parse(target) {
            Ok(target) => target,
            Err(_) => return Ok(EngineNetworkTargetDecision::DeniedInvalidTarget),
        };
        if !matches!(target.scheme(), "http" | "https") {
            return Ok(EngineNetworkTargetDecision::DeniedUnsupportedScheme);
        }
        let target_site = match target.site_identity() {
            Ok(site) => site,
            Err(_) => return Ok(EngineNetworkTargetDecision::DeniedInvalidTarget),
        };
        if !source_site.same_site(&target_site) {
            return Ok(EngineNetworkTargetDecision::DeniedCrossSite);
        }

        Ok(EngineNetworkTargetDecision::DeniedUnsupported)
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
        let listener =
            TcpListener::bind((bind_host, 0)).expect("bind target-policy fixture server");
        let port = listener.local_addr().expect("fixture address").port();
        thread::spawn(move || {
            let (mut stream, _) = listener
                .accept()
                .expect("accept target-policy fixture request");
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
                EngineNavigationPoll::Pending => panic!("target-policy fixture timed out"),
                EngineNavigationPoll::Committed { .. } => break,
                EngineNavigationPoll::Failed { message } => {
                    panic!("target-policy fixture navigation failed: {message}")
                }
                EngineNavigationPoll::Stale => {
                    panic!("target-policy fixture navigation went stale")
                }
            }
        }

        host.committed_document_authority(tab)
            .expect("committed authority query")
            .expect("committed remote authority")
    }

    #[test]
    fn current_source_uses_canonical_schemeful_same_site_policy() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/source"),
        );

        assert_eq!(
            host.preflight_network_target(current, "http://127.0.0.1:1/resource"),
            Ok(EngineNetworkTargetDecision::DeniedUnsupported)
        );
        assert_eq!(
            host.preflight_network_target(current, "https://127.0.0.1/resource"),
            Ok(EngineNetworkTargetDecision::DeniedCrossSite)
        );
        assert_eq!(
            host.preflight_network_target(current, "http://localhost/resource"),
            Ok(EngineNetworkTargetDecision::DeniedCrossSite)
        );
    }

    #[test]
    fn invalid_and_non_http_targets_fail_closed() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/invalid-targets"),
        );

        for target in ["../relative", "http://"] {
            assert_eq!(
                host.preflight_network_target(current, target),
                Ok(EngineNetworkTargetDecision::DeniedInvalidTarget),
                "{target}"
            );
        }
        for target in [
            "file:///tmp/zorya",
            "data:text/plain,zorya",
            "mailto:zorya@example.invalid",
            "zorya-external:payload",
        ] {
            assert_eq!(
                host.preflight_network_target(current, target),
                Ok(EngineNetworkTargetDecision::DeniedUnsupportedScheme),
                "{target}"
            );
        }
    }

    #[test]
    fn stale_source_is_rejected_before_target_policy() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let first = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/first"),
        );
        let replacement = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/replacement"),
        );
        assert_ne!(replacement.navigation_context(), first.navigation_context());

        assert_eq!(
            host.preflight_network_target(first, "../relative"),
            Ok(EngineNetworkTargetDecision::DeniedStaleAuthority)
        );

        host.load_local_html(tab, "<main>local</main>")
            .expect("replace with local document");
        assert_eq!(
            host.preflight_network_target(replacement, "http://127.0.0.1/"),
            Ok(EngineNetworkTargetDecision::DeniedStaleAuthority)
        );
    }

    #[test]
    fn closed_recreated_or_replaced_host_cannot_reuse_source_authority() {
        let tab = initial_tab();
        let mut first_host = EngineHost::new().expect("first engine host");
        first_host.create_view(tab).expect("first view");
        let stale = commit_remote(
            &mut first_host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/first-host"),
        );

        assert!(first_host.close_view(tab).expect("close first view"));
        assert_eq!(
            first_host.preflight_network_target(stale, "http://127.0.0.1/"),
            Ok(EngineNetworkTargetDecision::DeniedStaleAuthority)
        );
        first_host.create_view(tab).expect("recreated view");
        assert_eq!(
            first_host.preflight_network_target(stale, "http://127.0.0.1/"),
            Ok(EngineNetworkTargetDecision::DeniedStaleAuthority)
        );

        let mut replacement_host = EngineHost::new().expect("replacement engine host");
        replacement_host.create_view(tab).expect("replacement view");
        let replacement = commit_remote(
            &mut replacement_host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/replacement-host"),
        );
        assert_eq!(replacement.tab(), stale.tab());
        assert_eq!(replacement.view_generation(), stale.view_generation());
        assert_eq!(replacement.navigation_context(), stale.navigation_context());
        assert_eq!(replacement.site_process(), stale.site_process());
        assert_ne!(replacement.host_instance(), stale.host_instance());
        assert_eq!(
            replacement_host.preflight_network_target(stale, "http://127.0.0.1/"),
            Ok(EngineNetworkTargetDecision::DeniedStaleAuthority)
        );
        assert_eq!(
            replacement_host.preflight_network_target(replacement, "http://127.0.0.1:1/"),
            Ok(EngineNetworkTargetDecision::DeniedUnsupported)
        );
    }
}
