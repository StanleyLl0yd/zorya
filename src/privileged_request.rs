use crate::engine::{EngineCommittedDocumentAuthority, EngineHost, EngineHostError};
use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU64;

pub const DEFAULT_MAX_PENDING_PRIVILEGED_REQUESTS: usize = 4096;

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

/// Process-local identity for one pending privileged request attempt.
///
/// This is a product correlation identity only. It is not a Rarog capability ID and conveys no
/// authority on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EnginePrivilegedRequestId(NonZeroU64);

impl EnginePrivilegedRequestId {
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// One-shot request handle bound to the exact source authority snapshot and request kind.
///
/// Handles are process-local and must not be persisted. Copying a handle does not make it
/// reusable: the tracker consumes its identity before source preflight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnginePrivilegedRequest {
    id: EnginePrivilegedRequestId,
    authority: EngineCommittedDocumentAuthority,
    kind: EnginePrivilegedRequestKind,
}

impl EnginePrivilegedRequest {
    pub const fn id(self) -> EnginePrivilegedRequestId {
        self.id
    }

    pub const fn authority(self) -> EngineCommittedDocumentAuthority {
        self.authority
    }

    pub const fn kind(self) -> EnginePrivilegedRequestKind {
        self.kind
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnginePrivilegedRequestError {
    InvalidLimit,
    CapacityExceeded,
    IdentitySpaceExhausted,
    UnknownRequest(EnginePrivilegedRequestId),
    MismatchedRequest(EnginePrivilegedRequestId),
    Host(EngineHostError),
}

impl fmt::Display for EnginePrivilegedRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimit => {
                formatter.write_str("privileged request limit must be non-zero")
            }
            Self::CapacityExceeded => formatter.write_str("privileged request limit reached"),
            Self::IdentitySpaceExhausted => {
                formatter.write_str("privileged request identity space is exhausted")
            }
            Self::UnknownRequest(id) => write!(
                formatter,
                "unknown or already consumed privileged request {}",
                id.get()
            ),
            Self::MismatchedRequest(id) => write!(
                formatter,
                "privileged request {} does not match its registered source and kind",
                id.get()
            ),
            Self::Host(error) => write!(formatter, "privileged request source preflight failed: {error}"),
        }
    }
}

impl std::error::Error for EnginePrivilegedRequestError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Host(error) => Some(error),
            _ => None,
        }
    }
}

impl From<EngineHostError> for EnginePrivilegedRequestError {
    fn from(error: EngineHostError) -> Self {
        Self::Host(error)
    }
}

/// Bounded process-local lifecycle for future privileged request attempts.
///
/// Registration allocates only a one-shot correlation identity. It does not validate or grant
/// authority. `preflight_once` consumes the exact registered request first and then revalidates
/// its committed-document source through `EngineHost`.
#[derive(Debug)]
pub struct EnginePrivilegedRequestTracker {
    max_pending: usize,
    next_id: Option<NonZeroU64>,
    pending: BTreeMap<EnginePrivilegedRequestId, EnginePrivilegedRequest>,
}

impl EnginePrivilegedRequestTracker {
    pub fn try_new(max_pending: usize) -> Result<Self, EnginePrivilegedRequestError> {
        if max_pending == 0 {
            return Err(EnginePrivilegedRequestError::InvalidLimit);
        }

        Ok(Self {
            max_pending,
            next_id: NonZeroU64::new(1),
            pending: BTreeMap::new(),
        })
    }

    pub fn with_default_limit() -> Result<Self, EnginePrivilegedRequestError> {
        Self::try_new(DEFAULT_MAX_PENDING_PRIVILEGED_REQUESTS)
    }

    pub const fn max_pending(&self) -> usize {
        self.max_pending
    }

    pub fn pending_requests(&self) -> usize {
        self.pending.len()
    }

    pub fn register(
        &mut self,
        authority: EngineCommittedDocumentAuthority,
        kind: EnginePrivilegedRequestKind,
    ) -> Result<EnginePrivilegedRequest, EnginePrivilegedRequestError> {
        if self.pending.len() >= self.max_pending {
            return Err(EnginePrivilegedRequestError::CapacityExceeded);
        }

        let next = self
            .next_id
            .ok_or(EnginePrivilegedRequestError::IdentitySpaceExhausted)?;
        let id = EnginePrivilegedRequestId(next);
        self.next_id = NonZeroU64::new(next.get().wrapping_add(1));

        let request = EnginePrivilegedRequest {
            id,
            authority,
            kind,
        };
        let previous = self.pending.insert(id, request);
        debug_assert!(previous.is_none(), "monotonic request IDs cannot collide");
        Ok(request)
    }

    /// Consumes the exact registered request before checking its source authority.
    ///
    /// Replays are therefore rejected even when the source document is still current. A handle
    /// with the right ID but mismatched source/kind also burns that slot and fails closed.
    pub fn preflight_once(
        &mut self,
        host: &EngineHost,
        request: EnginePrivilegedRequest,
    ) -> Result<EnginePrivilegedRequestDecision, EnginePrivilegedRequestError> {
        let stored = self
            .pending
            .remove(&request.id)
            .ok_or(EnginePrivilegedRequestError::UnknownRequest(request.id))?;
        if stored != request {
            return Err(EnginePrivilegedRequestError::MismatchedRequest(request.id));
        }

        host.preflight_privileged_request(stored.authority, stored.kind)
            .map_err(EnginePrivilegedRequestError::from)
    }
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
            let (mut stream, _) = listener
                .accept()
                .expect("accept privileged fixture request");
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

    #[test]
    fn registered_current_request_is_consumed_once_and_remains_unsupported() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/one-shot"),
        );
        let mut tracker = EnginePrivilegedRequestTracker::try_new(2).expect("tracker");
        let request = tracker
            .register(current, EnginePrivilegedRequestKind::Network)
            .expect("register request");

        assert!(request.id().get() > 0);
        assert_eq!(request.authority(), current);
        assert_eq!(request.kind(), EnginePrivilegedRequestKind::Network);
        assert_eq!(tracker.pending_requests(), 1);
        assert_eq!(
            tracker.preflight_once(&host, request),
            Ok(EnginePrivilegedRequestDecision::DeniedUnsupported)
        );
        assert_eq!(tracker.pending_requests(), 0);
        assert_eq!(
            tracker.preflight_once(&host, request),
            Err(EnginePrivilegedRequestError::UnknownRequest(request.id()))
        );
    }

    #[test]
    fn registered_request_revalidates_authority_at_consume_time() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let first = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/registered-first"),
        );
        let mut tracker = EnginePrivilegedRequestTracker::try_new(2).expect("tracker");
        let request = tracker
            .register(first, EnginePrivilegedRequestKind::Clipboard)
            .expect("register request");

        let replacement = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/registered-replacement"),
        );
        assert_eq!(replacement.site_process(), first.site_process());
        assert_ne!(replacement.navigation_context(), first.navigation_context());
        assert_eq!(
            tracker.preflight_once(&host, request),
            Ok(EnginePrivilegedRequestDecision::DeniedStaleAuthority)
        );
        assert_eq!(tracker.pending_requests(), 0);
        assert_eq!(
            tracker.preflight_once(&host, request),
            Err(EnginePrivilegedRequestError::UnknownRequest(request.id()))
        );
    }

    #[test]
    fn mismatched_request_burns_the_registered_slot() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/mismatch"),
        );
        let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
        let request = tracker
            .register(current, EnginePrivilegedRequestKind::Network)
            .expect("register request");
        let forged = EnginePrivilegedRequest {
            id: request.id,
            authority: request.authority,
            kind: EnginePrivilegedRequestKind::Clipboard,
        };

        assert_eq!(
            tracker.preflight_once(&host, forged),
            Err(EnginePrivilegedRequestError::MismatchedRequest(request.id()))
        );
        assert_eq!(tracker.pending_requests(), 0);
        assert_eq!(
            tracker.preflight_once(&host, request),
            Err(EnginePrivilegedRequestError::UnknownRequest(request.id()))
        );
    }

    #[test]
    fn tracker_enforces_nonzero_limit_and_bounded_capacity() {
        assert!(matches!(
            EnginePrivilegedRequestTracker::try_new(0),
            Err(EnginePrivilegedRequestError::InvalidLimit)
        ));

        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/capacity"),
        );
        let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
        let first = tracker
            .register(current, EnginePrivilegedRequestKind::Network)
            .expect("first request");

        assert_eq!(tracker.max_pending(), 1);
        assert_eq!(tracker.pending_requests(), 1);
        assert_eq!(
            tracker.register(current, EnginePrivilegedRequestKind::Clipboard),
            Err(EnginePrivilegedRequestError::CapacityExceeded)
        );
        assert_eq!(tracker.pending_requests(), 1);
        assert_eq!(
            tracker.preflight_once(&host, first),
            Ok(EnginePrivilegedRequestDecision::DeniedUnsupported)
        );
    }
}
