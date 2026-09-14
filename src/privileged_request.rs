use crate::engine::{EngineCommittedDocumentAuthority, EngineHost, EngineHostError};
use crate::network_target_policy::EngineNetworkTargetDecision;
use rarog_fetch::FetchMethod;
use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

pub const DEFAULT_MAX_PENDING_PRIVILEGED_REQUESTS: usize = 4096;
pub const MAX_PRIVILEGED_NETWORK_TARGET_BYTES: usize = 4 * 1024;

static NEXT_ENGINE_PRIVILEGED_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

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
/// This generic handle is used only for privileged request kinds that do not require an additional
/// payload. Network requests must use `EngineNetworkPrivilegedRequest` so the target cannot be
/// omitted from the one-shot lifecycle.
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

/// One-shot Network request bound to the exact source authority, canonical method and raw target.
///
/// The target is retained only in process memory and is deliberately not included in `Debug`
/// output because URLs can contain sensitive query or fragment data. The HTTP method is the
/// canonical Rarog `FetchMethod`; Zorya does not parse or normalize HTTP methods independently.
/// Registration does not parse or authorize the target; canonical target policy runs only when the
/// request is consumed.
#[derive(Clone, PartialEq, Eq)]
pub struct EngineNetworkPrivilegedRequest {
    id: EnginePrivilegedRequestId,
    authority: EngineCommittedDocumentAuthority,
    method: FetchMethod,
    target: String,
}

impl EngineNetworkPrivilegedRequest {
    pub const fn id(&self) -> EnginePrivilegedRequestId {
        self.id
    }

    pub const fn authority(&self) -> EngineCommittedDocumentAuthority {
        self.authority
    }

    pub fn method(&self) -> &FetchMethod {
        &self.method
    }

    pub fn target(&self) -> &str {
        &self.target
    }
}

impl fmt::Debug for EngineNetworkPrivilegedRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EngineNetworkPrivilegedRequest")
            .field("id", &self.id)
            .field("authority", &self.authority)
            .field("method", &self.method)
            .field("target_bytes", &self.target.len())
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PendingPrivilegedRequest {
    Generic(EnginePrivilegedRequest),
    Network(EngineNetworkPrivilegedRequest),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnginePrivilegedRequestError {
    InvalidLimit,
    CapacityExceeded,
    IdentitySpaceExhausted,
    NetworkTargetRequired,
    NetworkTargetTooLong { bytes: usize, max: usize },
    UnknownRequest(EnginePrivilegedRequestId),
    MismatchedRequest(EnginePrivilegedRequestId),
    Host(EngineHostError),
}

impl fmt::Display for EnginePrivilegedRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimit => formatter.write_str("privileged request limit must be non-zero"),
            Self::CapacityExceeded => formatter.write_str("privileged request limit reached"),
            Self::IdentitySpaceExhausted => {
                formatter.write_str("privileged request identity space is exhausted")
            }
            Self::NetworkTargetRequired => {
                formatter.write_str("Network privileged requests require a target-bound request")
            }
            Self::NetworkTargetTooLong { bytes, max } => write!(
                formatter,
                "Network privileged request target is {bytes} bytes; maximum is {max} bytes"
            ),
            Self::UnknownRequest(id) => write!(
                formatter,
                "unknown or already consumed privileged request {}",
                id.get()
            ),
            Self::MismatchedRequest(id) => write!(
                formatter,
                "privileged request {} does not match its registered source or payload",
                id.get()
            ),
            Self::Host(error) => write!(
                formatter,
                "privileged request source preflight failed: {error}"
            ),
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

fn allocate_privileged_request_id()
-> Result<EnginePrivilegedRequestId, EnginePrivilegedRequestError> {
    let raw = NEXT_ENGINE_PRIVILEGED_REQUEST_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .map_err(|_| EnginePrivilegedRequestError::IdentitySpaceExhausted)?;
    let raw = NonZeroU64::new(raw).ok_or(EnginePrivilegedRequestError::IdentitySpaceExhausted)?;
    Ok(EnginePrivilegedRequestId(raw))
}

/// Bounded process-local lifecycle for future privileged request attempts.
///
/// Registration allocates only a one-shot correlation identity. It does not validate or grant
/// authority. Request IDs are process-global and monotonic so rebuilding a tracker cannot make an
/// old copied handle collide with a newly registered request. Generic registration cannot create a
/// Network request: Network attempts must retain an exact bounded target and canonical Rarog method
/// through `register_network` / `register_network_with_method`. Consumption removes the exact
/// stored request before source/target policy.
#[derive(Debug)]
pub struct EnginePrivilegedRequestTracker {
    max_pending: usize,
    pending: BTreeMap<EnginePrivilegedRequestId, PendingPrivilegedRequest>,
}

impl EnginePrivilegedRequestTracker {
    pub fn try_new(max_pending: usize) -> Result<Self, EnginePrivilegedRequestError> {
        if max_pending == 0 {
            return Err(EnginePrivilegedRequestError::InvalidLimit);
        }

        Ok(Self {
            max_pending,
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
        if matches!(kind, EnginePrivilegedRequestKind::Network) {
            return Err(EnginePrivilegedRequestError::NetworkTargetRequired);
        }
        self.ensure_capacity()?;

        let id = allocate_privileged_request_id()?;
        let request = EnginePrivilegedRequest {
            id,
            authority,
            kind,
        };
        self.insert_pending(id, PendingPrivilegedRequest::Generic(request));
        Ok(request)
    }

    /// Registers a bounded raw Network target with canonical GET semantics.
    ///
    /// Target parsing, source revalidation and authorization remain deferred to consumption.
    pub fn register_network(
        &mut self,
        authority: EngineCommittedDocumentAuthority,
        target: impl Into<String>,
    ) -> Result<EngineNetworkPrivilegedRequest, EnginePrivilegedRequestError> {
        self.register_network_with_method(authority, FetchMethod::get(), target)
    }

    /// Registers a bounded raw Network target plus canonical Rarog Fetch method without target
    /// parsing, source revalidation or authorization.
    ///
    /// Deferring target parsing until consumption preserves the fail-closed order where stale
    /// source authority is rejected before target classification. HTTP method syntax,
    /// canonicalization and forbidden-method policy belong to `rarog_fetch::FetchMethod`; this
    /// function accepts that already-validated type rather than implementing another parser.
    pub fn register_network_with_method(
        &mut self,
        authority: EngineCommittedDocumentAuthority,
        method: FetchMethod,
        target: impl Into<String>,
    ) -> Result<EngineNetworkPrivilegedRequest, EnginePrivilegedRequestError> {
        let target = target.into();
        if target.len() > MAX_PRIVILEGED_NETWORK_TARGET_BYTES {
            return Err(EnginePrivilegedRequestError::NetworkTargetTooLong {
                bytes: target.len(),
                max: MAX_PRIVILEGED_NETWORK_TARGET_BYTES,
            });
        }
        self.ensure_capacity()?;

        let id = allocate_privileged_request_id()?;
        let request = EngineNetworkPrivilegedRequest {
            id,
            authority,
            method,
            target,
        };
        self.insert_pending(id, PendingPrivilegedRequest::Network(request.clone()));
        Ok(request)
    }

    /// Consumes an exact generic request before checking its source authority.
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
        let PendingPrivilegedRequest::Generic(stored) = stored else {
            return Err(EnginePrivilegedRequestError::MismatchedRequest(request.id));
        };
        if stored != request {
            return Err(EnginePrivilegedRequestError::MismatchedRequest(request.id));
        }

        host.preflight_privileged_request(stored.authority, stored.kind)
            .map_err(EnginePrivilegedRequestError::from)
    }

    /// Consumes the exact target/method-bound Network request before applying canonical target
    /// policy.
    ///
    /// Same-ID source, method or target mismatches burn the stored slot. For an exact handle,
    /// `preflight_network_target` revalidates source authority before parsing/classifying the raw
    /// target. The current policy remains non-authorizing; the retained method is correlation data
    /// for a later reviewed Fetch/broker path and is not executed here.
    pub fn preflight_network_once(
        &mut self,
        host: &EngineHost,
        request: EngineNetworkPrivilegedRequest,
    ) -> Result<EngineNetworkTargetDecision, EnginePrivilegedRequestError> {
        let id = request.id;
        let stored = self
            .pending
            .remove(&id)
            .ok_or(EnginePrivilegedRequestError::UnknownRequest(id))?;
        let PendingPrivilegedRequest::Network(stored) = stored else {
            return Err(EnginePrivilegedRequestError::MismatchedRequest(id));
        };
        if stored != request {
            return Err(EnginePrivilegedRequestError::MismatchedRequest(id));
        }

        host.preflight_network_target(stored.authority, stored.target())
            .map_err(EnginePrivilegedRequestError::from)
    }

    fn ensure_capacity(&self) -> Result<(), EnginePrivilegedRequestError> {
        if self.pending.len() >= self.max_pending {
            Err(EnginePrivilegedRequestError::CapacityExceeded)
        } else {
            Ok(())
        }
    }

    fn insert_pending(&mut self, id: EnginePrivilegedRequestId, request: PendingPrivilegedRequest) {
        let previous = self.pending.insert(id, request);
        debug_assert!(
            previous.is_none(),
            "process-global request IDs cannot collide"
        );
    }
}

impl EngineHost {
    /// Revalidates the committed remote-document source of a future privileged request and then
    /// denies the request because capability brokering is not implemented yet.
    ///
    /// Replayed or replaced authority is distinguished from a current-but-unsupported request.
    /// Missing or divergent live Host state continues to fail closed through
    /// `validate_committed_document_authority`. Tracker-created Network requests use the separate
    /// target-bound path; this source-only primitive remains non-authorizing.
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

        assert_eq!(same_site.host_instance(), first.host_instance());
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
        assert_eq!(cross_site.host_instance(), first.host_instance());
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
        assert_eq!(current.host_instance(), stale.host_instance());
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
    fn generic_registration_requires_network_target() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/target-required"),
        );
        let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");

        assert_eq!(
            tracker.register(current, EnginePrivilegedRequestKind::Network),
            Err(EnginePrivilegedRequestError::NetworkTargetRequired)
        );
        assert_eq!(tracker.pending_requests(), 0);
    }

    #[test]
    fn registered_clipboard_request_is_consumed_once_and_remains_unsupported() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/one-shot-clipboard"),
        );
        let mut tracker = EnginePrivilegedRequestTracker::try_new(2).expect("tracker");
        let request = tracker
            .register(current, EnginePrivilegedRequestKind::Clipboard)
            .expect("register request");

        assert!(request.id().get() > 0);
        assert_eq!(request.authority(), current);
        assert_eq!(request.kind(), EnginePrivilegedRequestKind::Clipboard);
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
    fn registered_clipboard_request_revalidates_authority_at_consume_time() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let first = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/clipboard-first"),
        );
        let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
        let request = tracker
            .register(first, EnginePrivilegedRequestKind::Clipboard)
            .expect("register request");

        let replacement = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/clipboard-replacement"),
        );
        assert_eq!(replacement.host_instance(), first.host_instance());
        assert_eq!(replacement.site_process(), first.site_process());
        assert_ne!(replacement.navigation_context(), first.navigation_context());
        assert_eq!(
            tracker.preflight_once(&host, request),
            Ok(EnginePrivilegedRequestDecision::DeniedStaleAuthority)
        );
        assert_eq!(tracker.pending_requests(), 0);
    }

    #[test]
    fn current_same_site_network_request_is_consumed_once_and_remains_unsupported() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/one-shot-network"),
        );
        let mut tracker = EnginePrivilegedRequestTracker::try_new(2).expect("tracker");
        let request = tracker
            .register_network(current, "http://127.0.0.1:1/resource")
            .expect("register Network request");

        assert!(request.id().get() > 0);
        assert_eq!(request.authority(), current);
        assert_eq!(request.method().as_str(), "GET");
        assert_eq!(request.target(), "http://127.0.0.1:1/resource");
        assert_eq!(tracker.pending_requests(), 1);
        assert_eq!(
            tracker.preflight_network_once(&host, request.clone()),
            Ok(EngineNetworkTargetDecision::DeniedUnsupported)
        );
        assert_eq!(tracker.pending_requests(), 0);
        assert_eq!(
            tracker.preflight_network_once(&host, request.clone()),
            Err(EnginePrivilegedRequestError::UnknownRequest(request.id()))
        );
    }

    #[test]
    fn explicit_network_method_uses_canonical_rarog_fetch_method() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/method-binding"),
        );
        let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
        let method = FetchMethod::try_new("post").expect("Rarog canonical method");
        assert_eq!(method.as_str(), "POST");

        let request = tracker
            .register_network_with_method(
                current,
                method,
                "http://127.0.0.1:1/method-bound",
            )
            .expect("register method-bound request");
        assert_eq!(request.method().as_str(), "POST");
        assert_eq!(
            tracker.preflight_network_once(&host, request.clone()),
            Ok(EngineNetworkTargetDecision::DeniedUnsupported)
        );
        assert_eq!(
            tracker.preflight_network_once(&host, request.clone()),
            Err(EnginePrivilegedRequestError::UnknownRequest(request.id()))
        );
    }

    #[test]
    fn rarog_rejects_invalid_or_forbidden_methods_before_tracker_registration() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/method-validation"),
        );
        let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");

        assert!(FetchMethod::try_new("BAD METHOD").is_err());
        assert!(FetchMethod::try_new("CONNECT").is_err());
        assert!(FetchMethod::try_new("TRACE").is_err());
        assert!(FetchMethod::try_new("TRACK").is_err());
        assert_eq!(tracker.pending_requests(), 0);

        tracker
            .register_network_with_method(
                current,
                FetchMethod::head(),
                "http://127.0.0.1:1/capacity-remains",
            )
            .expect("failed method construction consumes no tracker capacity");
        assert_eq!(tracker.pending_requests(), 1);
    }

    #[test]
    fn network_source_revalidation_precedes_target_classification() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let first = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/network-first"),
        );
        let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
        let request = tracker
            .register_network(first, "../relative")
            .expect("register raw target");

        let replacement = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/network-replacement"),
        );
        assert_ne!(replacement.navigation_context(), first.navigation_context());
        assert_eq!(
            tracker.preflight_network_once(&host, request),
            Ok(EngineNetworkTargetDecision::DeniedStaleAuthority)
        );
        assert_eq!(tracker.pending_requests(), 0);
    }

    #[test]
    fn network_target_policy_results_are_one_shot() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/network-policy"),
        );
        let mut tracker = EnginePrivilegedRequestTracker::try_new(4).expect("tracker");

        for (target, expected) in [
            (
                "../relative",
                EngineNetworkTargetDecision::DeniedInvalidTarget,
            ),
            (
                "file:///tmp/zorya",
                EngineNetworkTargetDecision::DeniedUnsupportedScheme,
            ),
            (
                "https://127.0.0.1/resource",
                EngineNetworkTargetDecision::DeniedCrossSite,
            ),
        ] {
            let request = tracker
                .register_network(current, target)
                .expect("register target-bound request");
            assert_eq!(
                tracker.preflight_network_once(&host, request.clone()),
                Ok(expected),
                "{target}"
            );
            assert_eq!(
                tracker.preflight_network_once(&host, request.clone()),
                Err(EnginePrivilegedRequestError::UnknownRequest(request.id())),
                "{target} replay"
            );
        }
    }

    #[test]
    fn mismatched_network_target_burns_the_registered_slot() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/network-mismatch"),
        );
        let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
        let request = tracker
            .register_network(current, "http://127.0.0.1:1/original")
            .expect("register request");
        let forged = EngineNetworkPrivilegedRequest {
            id: request.id,
            authority: request.authority,
            method: request.method.clone(),
            target: "http://127.0.0.1:1/forged".into(),
        };

        assert_eq!(
            tracker.preflight_network_once(&host, forged),
            Err(EnginePrivilegedRequestError::MismatchedRequest(
                request.id()
            ))
        );
        assert_eq!(tracker.pending_requests(), 0);
        assert_eq!(
            tracker.preflight_network_once(&host, request.clone()),
            Err(EnginePrivilegedRequestError::UnknownRequest(request.id()))
        );
    }

    #[test]
    fn mismatched_network_method_burns_the_registered_slot() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/method-mismatch"),
        );
        let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
        let request = tracker
            .register_network(current, "http://127.0.0.1:1/original")
            .expect("register request");
        let forged = EngineNetworkPrivilegedRequest {
            id: request.id,
            authority: request.authority,
            method: FetchMethod::post(),
            target: request.target.clone(),
        };

        assert_eq!(
            tracker.preflight_network_once(&host, forged),
            Err(EnginePrivilegedRequestError::MismatchedRequest(
                request.id()
            ))
        );
        assert_eq!(tracker.pending_requests(), 0);
        assert_eq!(
            tracker.preflight_network_once(&host, request.clone()),
            Err(EnginePrivilegedRequestError::UnknownRequest(request.id()))
        );
    }

    #[test]
    fn oversized_network_target_is_rejected_without_consuming_capacity() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/network-bound"),
        );
        let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
        let oversized = "x".repeat(MAX_PRIVILEGED_NETWORK_TARGET_BYTES + 1);

        assert_eq!(
            tracker.register_network(current, oversized),
            Err(EnginePrivilegedRequestError::NetworkTargetTooLong {
                bytes: MAX_PRIVILEGED_NETWORK_TARGET_BYTES + 1,
                max: MAX_PRIVILEGED_NETWORK_TARGET_BYTES,
            })
        );
        assert_eq!(tracker.pending_requests(), 0);

        tracker
            .register(current, EnginePrivilegedRequestKind::Clipboard)
            .expect("capacity remains available");
        assert_eq!(tracker.pending_requests(), 1);
    }

    #[test]
    fn network_request_debug_redacts_raw_target_and_retains_method() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/network-debug"),
        );
        let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
        let secret = "http://127.0.0.1:1/resource?token=do-not-log";
        let request = tracker
            .register_network_with_method(current, FetchMethod::post(), secret)
            .expect("register request");
        let debug = format!("{request:?}");

        assert!(!debug.contains(secret));
        assert!(!debug.contains("do-not-log"));
        assert!(debug.contains("POST"));
        assert!(debug.contains("target_bytes"));
    }

    #[test]
    fn target_bound_request_cannot_cross_engine_host_replacement() {
        let tab = initial_tab();
        let mut first_host = EngineHost::new().expect("first engine host");
        first_host.create_view(tab).expect("first view");
        let first = commit_remote(
            &mut first_host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/tracker-first-host"),
        );
        let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
        let request = tracker
            .register_network(first, "http://127.0.0.1:1/resource")
            .expect("register request");

        let mut replacement_host = EngineHost::new().expect("replacement engine host");
        replacement_host.create_view(tab).expect("replacement view");
        let replacement = commit_remote(
            &mut replacement_host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/tracker-replacement-host"),
        );
        assert_eq!(replacement.tab(), first.tab());
        assert_eq!(replacement.view_generation(), first.view_generation());
        assert_eq!(replacement.navigation_context(), first.navigation_context());
        assert_eq!(replacement.site_process(), first.site_process());
        assert_ne!(replacement.host_instance(), first.host_instance());

        assert_eq!(
            tracker.preflight_network_once(&replacement_host, request),
            Ok(EngineNetworkTargetDecision::DeniedStaleAuthority)
        );
        assert_eq!(tracker.pending_requests(), 0);
    }

    #[test]
    fn cloned_target_handle_cannot_replay_across_tracker_replacement() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/tracker-replacement"),
        );

        let mut first_tracker = EnginePrivilegedRequestTracker::try_new(1).expect("first tracker");
        let stale = first_tracker
            .register_network(current, "http://127.0.0.1:1/stale")
            .expect("first request");
        assert_eq!(
            first_tracker.preflight_network_once(&host, stale.clone()),
            Ok(EngineNetworkTargetDecision::DeniedUnsupported)
        );

        let mut replacement_tracker =
            EnginePrivilegedRequestTracker::try_new(1).expect("replacement tracker");
        let current_request = replacement_tracker
            .register_network(current, "http://127.0.0.1:1/current")
            .expect("replacement request");
        assert_ne!(current_request.id(), stale.id());
        assert_eq!(
            replacement_tracker.preflight_network_once(&host, stale.clone()),
            Err(EnginePrivilegedRequestError::UnknownRequest(stale.id()))
        );
        assert_eq!(replacement_tracker.pending_requests(), 1);
        assert_eq!(
            replacement_tracker.preflight_network_once(&host, current_request),
            Ok(EngineNetworkTargetDecision::DeniedUnsupported)
        );
    }

    #[test]
    fn mismatched_generic_request_burns_the_registered_slot() {
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
            .register(current, EnginePrivilegedRequestKind::Clipboard)
            .expect("register request");
        let forged = EnginePrivilegedRequest {
            id: request.id,
            authority: request.authority,
            kind: EnginePrivilegedRequestKind::Network,
        };

        assert_eq!(
            tracker.preflight_once(&host, forged),
            Err(EnginePrivilegedRequestError::MismatchedRequest(
                request.id()
            ))
        );
        assert_eq!(tracker.pending_requests(), 0);
        assert_eq!(
            tracker.preflight_once(&host, request),
            Err(EnginePrivilegedRequestError::UnknownRequest(request.id()))
        );
    }

    #[test]
    fn tracker_enforces_nonzero_limit_and_shared_bounded_capacity() {
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
            .register(current, EnginePrivilegedRequestKind::Clipboard)
            .expect("first request");

        assert_eq!(tracker.max_pending(), 1);
        assert_eq!(tracker.pending_requests(), 1);
        assert_eq!(
            tracker.register_network(current, "http://127.0.0.1:1/resource"),
            Err(EnginePrivilegedRequestError::CapacityExceeded)
        );
        assert_eq!(tracker.pending_requests(), 1);
        assert_eq!(
            tracker.preflight_once(&host, first),
            Ok(EnginePrivilegedRequestDecision::DeniedUnsupported)
        );
    }
}
