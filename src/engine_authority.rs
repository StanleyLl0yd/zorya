#[cfg(test)]
use crate::engine::EngineCommittedInternalPageAuthority;
use crate::engine::{
    EngineCommittedDocumentAuthority, EngineCommittedDocumentSource, EngineHost, EngineHostError,
};

impl EngineHost {
    /// Revalidates an ephemeral committed remote-document authority snapshot against the
    /// current EngineHost incarnation, product View and live Rarog Host state.
    ///
    /// A stale snapshot is not authorization: Host replacement, replacement navigation, a local
    /// document, or a closed/recreated View returns `false`. Unexpected loss or divergence in the
    /// live Host authority remains an `InconsistentNavigationState` error through
    /// `committed_document_authority`.
    pub(crate) fn validate_committed_document_authority(
        &self,
        authority: EngineCommittedDocumentAuthority,
    ) -> Result<bool, EngineHostError> {
        match self.committed_document_authority(authority.tab()) {
            Ok(Some(current)) => Ok(current.rarog_navigation_context()
                == authority.rarog_navigation_context()
                && current == authority),
            Ok(None) => Ok(false),
            Err(EngineHostError::UnknownTab(tab)) if tab == authority.tab() => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Revalidates an exact Host-minted committed remote-document source against current live
    /// EngineHost/Rarog state.
    ///
    /// This is crate-private policy plumbing, not a public admission primitive. Authority is
    /// checked first so replaced Hosts/documents/Views fail stale before Origin/source comparison.
    /// The current source is then re-minted from live Rarog View + Host Site/process state and must
    /// match exactly, including canonical Origin. Impossible live Host divergence remains an error.
    pub(crate) fn validate_committed_document_source(
        &self,
        source: &EngineCommittedDocumentSource,
    ) -> Result<bool, EngineHostError> {
        if !self.validate_committed_document_authority(source.authority())? {
            return Ok(false);
        }

        Ok(self
            .committed_document_source(source.tab())?
            .as_ref()
            .is_some_and(|current| current == source))
    }

    /// Revalidates an opaque committed internal-page authority against the exact current
    /// EngineHost incarnation, View generation, internal-document token and page kind.
    ///
    /// Browser history/address text is not consulted. A generic local document, committed remote
    /// document, reloaded internal page, closed/recreated View or replacement EngineHost returns
    /// `false`; impossible live View/authority divergence remains fail-closed through
    /// `committed_internal_page_authority`.
    #[cfg(test)]
    pub(crate) fn validate_committed_internal_page_authority(
        &self,
        authority: EngineCommittedInternalPageAuthority,
    ) -> Result<bool, EngineHostError> {
        match self.committed_internal_page_authority(authority.tab()) {
            Ok(current) => Ok(current == Some(authority)),
            Err(EngineHostError::UnknownTab(tab)) if tab == authority.tab() => Ok(false),
            Err(error) => Err(error),
        }
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
        let listener = TcpListener::bind((bind_host, 0)).expect("bind authority fixture server");
        let port = listener.local_addr().expect("fixture address").port();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept authority fixture request");
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
                EngineNavigationPoll::Pending => panic!("authority fixture timed out"),
                EngineNavigationPoll::Committed { .. } => break,
                EngineNavigationPoll::Failed { message } => {
                    panic!("authority fixture navigation failed: {message}")
                }
                EngineNavigationPoll::Stale => panic!("authority fixture navigation went stale"),
            }
        }

        host.committed_document_authority(tab)
            .expect("committed authority query")
            .expect("committed remote authority")
    }

    #[test]
    fn revalidation_rejects_replaced_or_local_authority_and_preserves_policy_blocked_current() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");

        let first = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/first"),
        );
        assert_eq!(
            first.rarog_navigation_context().get(),
            first.navigation_context().get()
        );
        assert_eq!(host.validate_committed_document_authority(first), Ok(true));

        assert_eq!(
            host.begin_navigation(tab, "file:///tmp/zorya-authority")
                .expect("blocked navigation result"),
            None
        );
        assert_eq!(host.validate_committed_document_authority(first), Ok(true));

        let same_site = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/second"),
        );
        assert_eq!(same_site.host_instance(), first.host_instance());
        assert_ne!(same_site.navigation_context(), first.navigation_context());
        assert_ne!(
            same_site.rarog_navigation_context(),
            first.rarog_navigation_context()
        );
        assert_eq!(same_site.site_process(), first.site_process());
        assert_eq!(host.validate_committed_document_authority(first), Ok(false));
        assert_eq!(
            host.validate_committed_document_authority(same_site),
            Ok(true)
        );

        let cross_site = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "localhost", "/cross-site"),
        );
        assert_eq!(cross_site.host_instance(), first.host_instance());
        assert_ne!(
            cross_site.navigation_context(),
            same_site.navigation_context()
        );
        assert_ne!(cross_site.site_process(), same_site.site_process());
        assert_eq!(
            host.validate_committed_document_authority(same_site),
            Ok(false)
        );
        assert_eq!(
            host.validate_committed_document_authority(cross_site),
            Ok(true)
        );

        host.load_local_html(tab, "<main>local</main>")
            .expect("replace with local document");
        assert_eq!(
            host.validate_committed_document_authority(cross_site),
            Ok(false)
        );
    }

    #[test]
    fn revalidation_rejects_closed_and_recreated_view_generation() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("first view");
        let stale = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/first-view"),
        );
        assert_eq!(host.validate_committed_document_authority(stale), Ok(true));

        assert!(host.close_view(tab).expect("close first view"));
        assert_eq!(host.validate_committed_document_authority(stale), Ok(false));

        host.create_view(tab).expect("replacement view");
        assert_eq!(host.validate_committed_document_authority(stale), Ok(false));
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/replacement-view"),
        );
        assert_eq!(current.host_instance(), stale.host_instance());
        assert_ne!(current.view_generation(), stale.view_generation());
        assert_eq!(host.validate_committed_document_authority(stale), Ok(false));
        assert_eq!(
            host.validate_committed_document_authority(current),
            Ok(true)
        );
    }

    #[test]
    fn revalidation_rejects_snapshot_from_replaced_engine_host_even_when_local_ids_collide() {
        let tab = initial_tab();
        let mut first_host = EngineHost::new().expect("first engine host");
        first_host.create_view(tab).expect("first view");
        let stale = commit_remote(
            &mut first_host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/first-host"),
        );
        assert_eq!(
            first_host.validate_committed_document_authority(stale),
            Ok(true)
        );

        let mut replacement_host = EngineHost::new().expect("replacement engine host");
        replacement_host.create_view(tab).expect("replacement view");
        let current = commit_remote(
            &mut replacement_host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/replacement-host"),
        );

        assert_eq!(current.tab(), stale.tab());
        assert_eq!(current.view_generation(), stale.view_generation());
        assert_eq!(current.navigation_context(), stale.navigation_context());
        assert_eq!(current.site_process(), stale.site_process());
        assert_ne!(current.host_instance(), stale.host_instance());
        assert_eq!(
            replacement_host.validate_committed_document_authority(stale),
            Ok(false)
        );
        assert_eq!(
            replacement_host.validate_committed_document_authority(current),
            Ok(true)
        );
    }
}
