from pathlib import Path

p = Path("src/privileged_request.rs")
s = p.read_text()

def once(old: str, new: str) -> None:
    global s
    if old not in s:
        raise SystemExit("missing marker:\n" + old[:240])
    s = s.replace(old, new, 1)

def all_replace(old: str, new: str) -> int:
    global s
    count = s.count(old)
    s = s.replace(old, new)
    return count

once(
"""pub const MAX_PRIVILEGED_NETWORK_RESPONSE_BODY_BYTES: usize = DEFAULT_MAX_RESPONSE_BODY_BYTES;
""",
"""pub const MAX_PRIVILEGED_NETWORK_RESPONSE_BODY_BYTES: usize = DEFAULT_MAX_RESPONSE_BODY_BYTES;
pub const MAX_PRIVILEGED_CLIPBOARD_TEXT_BYTES: usize = 1024 * 1024;
pub const DEFAULT_MAX_PENDING_PRIVILEGED_CLIPBOARD_TEXT_BYTES: usize = 16 * 1024 * 1024;
"""
)

once(
"""/// This generic handle is used only for privileged request kinds that do not require an additional
/// payload. Network requests must use `EngineNetworkPrivilegedRequest` so the target cannot be
/// omitted from the one-shot lifecycle.
""",
"""/// This generic handle is retained as a fail-closed compatibility shape only. Both current
/// concrete request kinds require specialized envelopes: Network requests bind a full Fetch
/// correlation envelope and Clipboard requests bind an exact bounded read/write operation.
"""
)

marker = """impl EnginePrivilegedRequest {
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

"""
insert = marker + r'''#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineClipboardOperationKind {
    ReadText,
    WriteText,
}

#[derive(Clone, PartialEq, Eq)]
enum EngineClipboardOperation {
    ReadText { max_result_bytes: usize },
    WriteText { text: Arc<str> },
}

/// One-shot Clipboard request bound to the exact committed remote-document authority and an exact
/// bounded text operation.
///
/// This is correlation state only. Registration does not grant a Rarog Clipboard capability and
/// consumption never calls an OS/platform clipboard service in this slice. Write text is retained
/// in immutable shared storage so cloning a handle does not duplicate the allocation; Debug output
/// exposes only safe operation/size metadata.
#[derive(Clone, PartialEq, Eq)]
pub struct EngineClipboardPrivilegedRequest {
    id: EnginePrivilegedRequestId,
    authority: EngineCommittedDocumentAuthority,
    operation: EngineClipboardOperation,
}

impl EngineClipboardPrivilegedRequest {
    pub const fn id(&self) -> EnginePrivilegedRequestId {
        self.id
    }

    pub const fn authority(&self) -> EngineCommittedDocumentAuthority {
        self.authority
    }

    pub fn operation_kind(&self) -> EngineClipboardOperationKind {
        match &self.operation {
            EngineClipboardOperation::ReadText { .. } => EngineClipboardOperationKind::ReadText,
            EngineClipboardOperation::WriteText { .. } => EngineClipboardOperationKind::WriteText,
        }
    }

    pub fn max_read_text_bytes(&self) -> Option<usize> {
        match &self.operation {
            EngineClipboardOperation::ReadText { max_result_bytes } => Some(*max_result_bytes),
            EngineClipboardOperation::WriteText { .. } => None,
        }
    }

    pub fn write_text(&self) -> Option<&str> {
        match &self.operation {
            EngineClipboardOperation::ReadText { .. } => None,
            EngineClipboardOperation::WriteText { text } => Some(text),
        }
    }

    fn write_text_bytes(&self) -> usize {
        self.write_text().map_or(0, str::len)
    }
}

impl fmt::Debug for EngineClipboardPrivilegedRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("EngineClipboardPrivilegedRequest");
        debug
            .field("id", &self.id)
            .field("authority", &self.authority)
            .field("operation", &self.operation_kind());
        match &self.operation {
            EngineClipboardOperation::ReadText { max_result_bytes } => {
                debug.field("max_result_bytes", max_result_bytes);
            }
            EngineClipboardOperation::WriteText { text } => {
                debug.field("write_text_bytes", &text.len());
            }
        }
        debug.finish()
    }
}

'''
once(marker, insert)

once(
"""enum PendingPrivilegedRequest {
    Generic(EnginePrivilegedRequest),
    Network(EngineNetworkPrivilegedRequest),
}
""",
"""enum PendingPrivilegedRequest {
    Generic(EnginePrivilegedRequest),
    Network(EngineNetworkPrivilegedRequest),
    Clipboard(EngineClipboardPrivilegedRequest),
}
"""
)

once(
"""    InvalidNetworkBodyBudget,
    CapacityExceeded,
    IdentitySpaceExhausted,
    NetworkTargetRequired,
""",
"""    InvalidNetworkBodyBudget,
    InvalidClipboardTextBudget,
    CapacityExceeded,
    IdentitySpaceExhausted,
    NetworkTargetRequired,
    ClipboardOperationRequired,
    ClipboardReadTextLimitOutOfRange {
        bytes: usize,
        max: usize,
    },
    ClipboardWriteTextTooLong {
        bytes: usize,
        max: usize,
    },
    ClipboardTextBudgetExceeded {
        pending: usize,
        requested: usize,
        max: usize,
    },
    ClipboardTextAccountingInvariant,
"""
)

once(
"""            Self::InvalidNetworkBodyBudget => {
                formatter.write_str("privileged Network body budget must be non-zero")
            }
            Self::CapacityExceeded => formatter.write_str("privileged request limit reached"),
""",
"""            Self::InvalidNetworkBodyBudget => {
                formatter.write_str("privileged Network body budget must be non-zero")
            }
            Self::InvalidClipboardTextBudget => {
                formatter.write_str("privileged Clipboard text budget must be non-zero")
            }
            Self::CapacityExceeded => formatter.write_str("privileged request limit reached"),
"""
)

once(
"""            Self::NetworkTargetRequired => {
                formatter.write_str("Network privileged requests require a target-bound request")
            }
            Self::NetworkTargetTooLong { bytes, max } => write!(
""",
"""            Self::NetworkTargetRequired => {
                formatter.write_str("Network privileged requests require a target-bound request")
            }
            Self::ClipboardOperationRequired => formatter.write_str(
                "Clipboard privileged requests require an operation-bound request",
            ),
            Self::ClipboardReadTextLimitOutOfRange { bytes, max } => write!(
                formatter,
                "Clipboard read-text result limit is {bytes} bytes; allowed range is 1..={max} bytes"
            ),
            Self::ClipboardWriteTextTooLong { bytes, max } => write!(
                formatter,
                "Clipboard write text is {bytes} bytes; maximum is {max} bytes"
            ),
            Self::ClipboardTextBudgetExceeded {
                pending,
                requested,
                max,
            } => write!(
                formatter,
                "Clipboard privileged request text budget exceeded: {pending} pending bytes + {requested} requested bytes; maximum is {max} bytes"
            ),
            Self::ClipboardTextAccountingInvariant => {
                formatter.write_str("privileged Clipboard text accounting invariant was violated")
            }
            Self::NetworkTargetTooLong { bytes, max } => write!(
"""
)

once(
"""/// old copied handle collide with a newly registered request. Generic registration cannot create a
/// Network request: Network attempts must retain an exact bounded target, canonical Rarog method,
/// bounded canonical Rarog headers, exact optional bounded body and canonical Fetch envelope
/// metadata. Consumption removes the exact stored request and releases its tracker-accounted body
/// bytes before source/target policy.
""",
"""/// old copied handle collide with a newly registered request. Generic registration cannot create
/// either current concrete request kind: Network attempts must retain the exact bounded Fetch
/// correlation envelope, while Clipboard attempts must retain an exact bounded read/write
/// operation. Consumption removes the exact stored request and releases tracker-accounted Network
/// body / Clipboard write-text bytes before equality and source/target policy.
"""
)

once(
"""pub struct EnginePrivilegedRequestTracker {
    max_pending: usize,
    max_pending_network_body_bytes: usize,
    pending_network_body_bytes: usize,
    pending: BTreeMap<EnginePrivilegedRequestId, PendingPrivilegedRequest>,
}
""",
"""pub struct EnginePrivilegedRequestTracker {
    max_pending: usize,
    max_pending_network_body_bytes: usize,
    pending_network_body_bytes: usize,
    max_pending_clipboard_text_bytes: usize,
    pending_clipboard_text_bytes: usize,
    pending: BTreeMap<EnginePrivilegedRequestId, PendingPrivilegedRequest>,
}
"""
)

once(
"""    pub fn try_new(max_pending: usize) -> Result<Self, EnginePrivilegedRequestError> {
        Self::try_new_with_body_budget(
            max_pending,
            DEFAULT_MAX_PENDING_PRIVILEGED_NETWORK_BODY_BYTES,
        )
    }

    pub fn try_new_with_body_budget(
        max_pending: usize,
        max_pending_network_body_bytes: usize,
    ) -> Result<Self, EnginePrivilegedRequestError> {
        if max_pending == 0 {
            return Err(EnginePrivilegedRequestError::InvalidLimit);
        }
        if max_pending_network_body_bytes == 0 {
            return Err(EnginePrivilegedRequestError::InvalidNetworkBodyBudget);
        }

        Ok(Self {
            max_pending,
            max_pending_network_body_bytes,
            pending_network_body_bytes: 0,
            pending: BTreeMap::new(),
        })
    }
""",
"""    pub fn try_new(max_pending: usize) -> Result<Self, EnginePrivilegedRequestError> {
        Self::try_new_with_budgets(
            max_pending,
            DEFAULT_MAX_PENDING_PRIVILEGED_NETWORK_BODY_BYTES,
            DEFAULT_MAX_PENDING_PRIVILEGED_CLIPBOARD_TEXT_BYTES,
        )
    }

    pub fn try_new_with_body_budget(
        max_pending: usize,
        max_pending_network_body_bytes: usize,
    ) -> Result<Self, EnginePrivilegedRequestError> {
        Self::try_new_with_budgets(
            max_pending,
            max_pending_network_body_bytes,
            DEFAULT_MAX_PENDING_PRIVILEGED_CLIPBOARD_TEXT_BYTES,
        )
    }

    pub fn try_new_with_budgets(
        max_pending: usize,
        max_pending_network_body_bytes: usize,
        max_pending_clipboard_text_bytes: usize,
    ) -> Result<Self, EnginePrivilegedRequestError> {
        if max_pending == 0 {
            return Err(EnginePrivilegedRequestError::InvalidLimit);
        }
        if max_pending_network_body_bytes == 0 {
            return Err(EnginePrivilegedRequestError::InvalidNetworkBodyBudget);
        }
        if max_pending_clipboard_text_bytes == 0 {
            return Err(EnginePrivilegedRequestError::InvalidClipboardTextBudget);
        }

        Ok(Self {
            max_pending,
            max_pending_network_body_bytes,
            pending_network_body_bytes: 0,
            max_pending_clipboard_text_bytes,
            pending_clipboard_text_bytes: 0,
            pending: BTreeMap::new(),
        })
    }
"""
)

once(
"""    pub const fn pending_network_body_bytes(&self) -> usize {
        self.pending_network_body_bytes
    }

    pub fn pending_requests(&self) -> usize {
""",
"""    pub const fn pending_network_body_bytes(&self) -> usize {
        self.pending_network_body_bytes
    }

    pub const fn max_pending_clipboard_text_bytes(&self) -> usize {
        self.max_pending_clipboard_text_bytes
    }

    pub const fn pending_clipboard_text_bytes(&self) -> usize {
        self.pending_clipboard_text_bytes
    }

    pub fn pending_requests(&self) -> usize {
"""
)

once(
"""    pub fn register(
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

""",
"""    pub fn register(
        &mut self,
        _authority: EngineCommittedDocumentAuthority,
        kind: EnginePrivilegedRequestKind,
    ) -> Result<EnginePrivilegedRequest, EnginePrivilegedRequestError> {
        match kind {
            EnginePrivilegedRequestKind::Network => {
                Err(EnginePrivilegedRequestError::NetworkTargetRequired)
            }
            EnginePrivilegedRequestKind::Clipboard => {
                Err(EnginePrivilegedRequestError::ClipboardOperationRequired)
            }
        }
    }

    /// Registers a Clipboard text read request with the exact product maximum result bound.
    pub fn register_clipboard_read(
        &mut self,
        authority: EngineCommittedDocumentAuthority,
    ) -> Result<EngineClipboardPrivilegedRequest, EnginePrivilegedRequestError> {
        self.register_clipboard_read_with_limit(authority, MAX_PRIVILEGED_CLIPBOARD_TEXT_BYTES)
    }

    /// Registers a Clipboard text read request with an exact non-zero bounded result limit.
    pub fn register_clipboard_read_with_limit(
        &mut self,
        authority: EngineCommittedDocumentAuthority,
        max_result_bytes: usize,
    ) -> Result<EngineClipboardPrivilegedRequest, EnginePrivilegedRequestError> {
        if max_result_bytes == 0 || max_result_bytes > MAX_PRIVILEGED_CLIPBOARD_TEXT_BYTES {
            return Err(
                EnginePrivilegedRequestError::ClipboardReadTextLimitOutOfRange {
                    bytes: max_result_bytes,
                    max: MAX_PRIVILEGED_CLIPBOARD_TEXT_BYTES,
                },
            );
        }
        self.ensure_capacity()?;

        let id = allocate_privileged_request_id()?;
        let request = EngineClipboardPrivilegedRequest {
            id,
            authority,
            operation: EngineClipboardOperation::ReadText { max_result_bytes },
        };
        self.insert_pending(id, PendingPrivilegedRequest::Clipboard(request.clone()));
        Ok(request)
    }

    /// Registers an exact bounded Clipboard text write request.
    pub fn register_clipboard_write(
        &mut self,
        authority: EngineCommittedDocumentAuthority,
        text: impl Into<String>,
    ) -> Result<EngineClipboardPrivilegedRequest, EnginePrivilegedRequestError> {
        let text = text.into();
        let text_bytes = text.len();
        if text_bytes > MAX_PRIVILEGED_CLIPBOARD_TEXT_BYTES {
            return Err(EnginePrivilegedRequestError::ClipboardWriteTextTooLong {
                bytes: text_bytes,
                max: MAX_PRIVILEGED_CLIPBOARD_TEXT_BYTES,
            });
        }
        let next_text_bytes = self
            .pending_clipboard_text_bytes
            .checked_add(text_bytes)
            .ok_or(EnginePrivilegedRequestError::ClipboardTextBudgetExceeded {
                pending: self.pending_clipboard_text_bytes,
                requested: text_bytes,
                max: self.max_pending_clipboard_text_bytes,
            })?;
        if next_text_bytes > self.max_pending_clipboard_text_bytes {
            return Err(EnginePrivilegedRequestError::ClipboardTextBudgetExceeded {
                pending: self.pending_clipboard_text_bytes,
                requested: text_bytes,
                max: self.max_pending_clipboard_text_bytes,
            });
        }
        self.ensure_capacity()?;

        let id = allocate_privileged_request_id()?;
        let request = EngineClipboardPrivilegedRequest {
            id,
            authority,
            operation: EngineClipboardOperation::WriteText {
                text: Arc::<str>::from(text),
            },
        };
        self.insert_pending(id, PendingPrivilegedRequest::Clipboard(request.clone()));
        self.pending_clipboard_text_bytes = next_text_bytes;
        Ok(request)
    }

"""
)

once(
"""    /// Consumes the exact source/target/method/header/body/envelope-bound Network request before
""",
"""    /// Consumes an exact operation-bound Clipboard request before source policy.
    pub fn preflight_clipboard_once(
        &mut self,
        host: &EngineHost,
        request: EngineClipboardPrivilegedRequest,
    ) -> Result<EnginePrivilegedRequestDecision, EnginePrivilegedRequestError> {
        let id = request.id;
        let stored = self.remove_pending(id)?;
        let PendingPrivilegedRequest::Clipboard(stored) = stored else {
            return Err(EnginePrivilegedRequestError::MismatchedRequest(id));
        };
        if stored != request {
            return Err(EnginePrivilegedRequestError::MismatchedRequest(id));
        }

        host.preflight_privileged_request(stored.authority, EnginePrivilegedRequestKind::Clipboard)
            .map_err(EnginePrivilegedRequestError::from)
    }

    /// Consumes the exact source/target/method/header/body/envelope-bound Network request before
"""
)

once(
"""        let body_bytes = match &request {
            PendingPrivilegedRequest::Generic(_) => 0,
            PendingPrivilegedRequest::Network(request) => request.body_bytes(),
        };
        self.pending_network_body_bytes =
            self.pending_network_body_bytes
                .checked_sub(body_bytes)
                .ok_or(EnginePrivilegedRequestError::NetworkBodyAccountingInvariant)?;
        Ok(request)
""",
"""        let (body_bytes, clipboard_text_bytes) = match &request {
            PendingPrivilegedRequest::Generic(_) => (0, 0),
            PendingPrivilegedRequest::Network(request) => (request.body_bytes(), 0),
            PendingPrivilegedRequest::Clipboard(request) => (0, request.write_text_bytes()),
        };
        let next_network_body_bytes = self
            .pending_network_body_bytes
            .checked_sub(body_bytes)
            .ok_or(EnginePrivilegedRequestError::NetworkBodyAccountingInvariant)?;
        let next_clipboard_text_bytes = self
            .pending_clipboard_text_bytes
            .checked_sub(clipboard_text_bytes)
            .ok_or(EnginePrivilegedRequestError::ClipboardTextAccountingInvariant)?;
        self.pending_network_body_bytes = next_network_body_bytes;
        self.pending_clipboard_text_bytes = next_clipboard_text_bytes;
        Ok(request)
"""
)

once(
"""#[cfg(test)]
#[path = "privileged_request_network_body_tests.rs"]
mod network_body_tests;
""",
"""#[cfg(test)]
#[path = "privileged_request_clipboard_tests.rs"]
mod clipboard_tests;

#[cfg(test)]
#[path = "privileged_request_network_body_tests.rs"]
mod network_body_tests;
"""
)

# Migrate existing inline Clipboard lifecycle tests to the specialized envelope.
s = s.replace(
    ".register(current.authority(), EnginePrivilegedRequestKind::Clipboard)",
    ".register_clipboard_read(current.authority())",
)
s = s.replace(
    ".register(first.authority(), EnginePrivilegedRequestKind::Clipboard)",
    ".register_clipboard_read(first.authority())",
)
s = s.replace("tracker.preflight_once(&host, request)", "tracker.preflight_clipboard_once(&host, request)")
s = s.replace("tracker.preflight_once(&host, first)", "tracker.preflight_clipboard_once(&host, first)")
s = s.replace("assert_eq!(request.kind(), EnginePrivilegedRequestKind::Clipboard);",
              "assert_eq!(request.operation_kind(), EngineClipboardOperationKind::ReadText);")

# Repair the forged generic cross-kind test after request type migration.
s = s.replace("id: request.id,\n            authority: request.authority,\n            kind: EnginePrivilegedRequestKind::Network,",
              "id: request.id,\n            authority: request.authority,\n            kind: EnginePrivilegedRequestKind::Network,")
s = s.replace(
"""            tracker.preflight_clipboard_once(&host, forged),
            Err(EnginePrivilegedRequestError::MismatchedRequest(
                request.id()
            ))
""",
"""            tracker.preflight_once(&host, forged),
            Err(EnginePrivilegedRequestError::MismatchedRequest(
                request.id()
            ))
""",
1,
)
s = s.replace(
"""            tracker.preflight_clipboard_once(&host, request),
            Err(EnginePrivilegedRequestError::UnknownRequest(request.id()))
""",
"""            tracker.preflight_clipboard_once(&host, request.clone()),
            Err(EnginePrivilegedRequestError::UnknownRequest(request.id()))
""",
1,
)

p.write_text(s)

p = Path("src/lib.rs")
s = p.read_text()
old = """pub use privileged_request::{
    DEFAULT_MAX_PENDING_PRIVILEGED_NETWORK_BODY_BYTES, DEFAULT_MAX_PENDING_PRIVILEGED_REQUESTS,
    EngineNetworkPrivilegedRequest, EnginePrivilegedRequest, EnginePrivilegedRequestDecision,
    EnginePrivilegedRequestError, EnginePrivilegedRequestId, EnginePrivilegedRequestKind,
    EnginePrivilegedRequestTracker, MAX_PRIVILEGED_NETWORK_BODY_BYTES,
"""
new = """pub use privileged_request::{
    DEFAULT_MAX_PENDING_PRIVILEGED_CLIPBOARD_TEXT_BYTES,
    DEFAULT_MAX_PENDING_PRIVILEGED_NETWORK_BODY_BYTES, DEFAULT_MAX_PENDING_PRIVILEGED_REQUESTS,
    EngineClipboardOperationKind, EngineClipboardPrivilegedRequest, EngineNetworkPrivilegedRequest,
    EnginePrivilegedRequest, EnginePrivilegedRequestDecision, EnginePrivilegedRequestError,
    EnginePrivilegedRequestId, EnginePrivilegedRequestKind, EnginePrivilegedRequestTracker,
    MAX_PRIVILEGED_CLIPBOARD_TEXT_BYTES, MAX_PRIVILEGED_NETWORK_BODY_BYTES,
"""
if old not in s:
    raise SystemExit("missing lib export marker")
p.write_text(s.replace(old, new, 1))

Path("src/privileged_request_clipboard_tests.rs").write_text(r'''use super::*;
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
        match host.poll_navigation(request).expect("poll remote navigation") {
            EngineNavigationPoll::Pending if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(5));
            }
            EngineNavigationPoll::Pending => panic!("Clipboard fixture timed out"),
            EngineNavigationPoll::Committed { .. } => break,
            EngineNavigationPoll::Failed { message } => panic!("Clipboard navigation failed: {message}"),
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
fn generic_clipboard_registration_is_rejected_without_consuming_capacity() {
    let (_tab, _host, current) = current_authority("/clipboard-operation-required");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    assert_eq!(
        tracker.register(current.authority(), EnginePrivilegedRequestKind::Clipboard),
        Err(EnginePrivilegedRequestError::ClipboardOperationRequired)
    );
    assert_eq!(tracker.pending_requests(), 0);
    assert_eq!(tracker.pending_clipboard_text_bytes(), 0);
}

#[test]
fn clipboard_read_binds_exact_default_and_explicit_limits() {
    let (_tab, host, current) = current_authority("/clipboard-read");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(3).expect("tracker");
    let default = tracker.register_clipboard_read(current.authority()).expect("default read");
    assert_eq!(default.operation_kind(), EngineClipboardOperationKind::ReadText);
    assert_eq!(default.max_read_text_bytes(), Some(MAX_PRIVILEGED_CLIPBOARD_TEXT_BYTES));
    assert_eq!(default.write_text(), None);
    let minimum = tracker
        .register_clipboard_read_with_limit(current.authority(), 1)
        .expect("minimum read");
    let middle = tracker
        .register_clipboard_read_with_limit(current.authority(), 4096)
        .expect("explicit read");
    assert_eq!(minimum.max_read_text_bytes(), Some(1));
    assert_eq!(middle.max_read_text_bytes(), Some(4096));
    for request in [default, minimum, middle] {
        assert_eq!(
            tracker.preflight_clipboard_once(&host, request),
            Ok(EnginePrivilegedRequestDecision::DeniedUnsupported)
        );
    }
}

#[test]
fn clipboard_read_rejects_zero_and_over_max_without_slot_use() {
    let (_tab, _host, current) = current_authority("/clipboard-read-bounds");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    for bytes in [0, MAX_PRIVILEGED_CLIPBOARD_TEXT_BYTES + 1] {
        assert_eq!(
            tracker.register_clipboard_read_with_limit(current.authority(), bytes),
            Err(EnginePrivilegedRequestError::ClipboardReadTextLimitOutOfRange {
                bytes,
                max: MAX_PRIVILEGED_CLIPBOARD_TEXT_BYTES,
            })
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
        .register_clipboard_write(current.authority(), secret)
        .expect("write request");
    let clone = request.clone();
    assert_eq!(request.operation_kind(), EngineClipboardOperationKind::WriteText);
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
    assert!(debug.contains("WriteText"));
    assert!(debug.contains("write_text_bytes"));
    assert_eq!(
        tracker.preflight_clipboard_once(&host, clone),
        Ok(EnginePrivilegedRequestDecision::DeniedUnsupported)
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
        per_request.register_clipboard_write(current.authority(), oversized),
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
        .register_clipboard_write(current.authority(), "12345")
        .expect("first write");
    assert_eq!(
        aggregate.register_clipboard_write(current.authority(), "x"),
        Err(EnginePrivilegedRequestError::ClipboardTextBudgetExceeded {
            pending: 5,
            requested: 1,
            max: 5,
        })
    );
    assert_eq!(aggregate.pending_clipboard_text_bytes(), 5);
    assert_eq!(
        aggregate.preflight_clipboard_once(&host, first),
        Ok(EnginePrivilegedRequestDecision::DeniedUnsupported)
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
        .register_clipboard_read_with_limit(current.authority(), 64)
        .expect("read request");
    let forged_limit = EngineClipboardPrivilegedRequest {
        id: read.id,
        authority: read.authority,
        operation: EngineClipboardOperation::ReadText { max_result_bytes: 63 },
    };
    assert_eq!(
        read_tracker.preflight_clipboard_once(&host, forged_limit),
        Err(EnginePrivilegedRequestError::MismatchedRequest(read.id()))
    );

    let mut write_tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let write = write_tracker
        .register_clipboard_write(current.authority(), "original")
        .expect("write request");
    let forged_text = EngineClipboardPrivilegedRequest {
        id: write.id,
        authority: write.authority,
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
        .register_clipboard_write(current.authority(), "operation")
        .expect("write request");
    let forged_operation = EngineClipboardPrivilegedRequest {
        id: write.id,
        authority: write.authority,
        operation: EngineClipboardOperation::ReadText { max_result_bytes: 64 },
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
        .register_clipboard_write(current.authority(), "cross-kind")
        .expect("Clipboard write");
    let forged_generic = EnginePrivilegedRequest {
        id: clipboard.id,
        authority: clipboard.authority,
        kind: EnginePrivilegedRequestKind::Network,
    };
    assert_eq!(
        clipboard_tracker.preflight_once(&host, forged_generic),
        Err(EnginePrivilegedRequestError::MismatchedRequest(clipboard.id()))
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
        authority: network.authority(),
        operation: EngineClipboardOperation::ReadText { max_result_bytes: 1 },
    };
    assert_eq!(
        network_tracker.preflight_clipboard_once(&host, forged_clipboard),
        Err(EnginePrivilegedRequestError::MismatchedRequest(network.id()))
    );
    assert_eq!(network_tracker.pending_network_body_bytes(), 0);
}

#[test]
fn clipboard_source_is_revalidated_only_at_consume_time() {
    let (tab, mut host, first) = current_authority("/clipboard-stale-first");
    let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
    let request = tracker
        .register_clipboard_write(first.authority(), "stale")
        .expect("Clipboard request");
    let replacement = commit_remote(&mut host, tab, serve_once("/clipboard-stale-replacement"));
    assert_eq!(replacement.host_instance(), first.authority().host_instance());
    assert_ne!(replacement.navigation_context(), first.authority().navigation_context());
    assert_eq!(
        tracker.preflight_clipboard_once(&host, request),
        Ok(EnginePrivilegedRequestDecision::DeniedStaleAuthority)
    );
    assert_eq!(tracker.pending_clipboard_text_bytes(), 0);
}

#[test]
fn clipboard_budget_is_independent_from_network_body_budget() {
    let (_tab, host, current) = current_authority("/clipboard-independent-budget");
    let mut tracker =
        EnginePrivilegedRequestTracker::try_new_with_budgets(2, 4, 4).expect("bounded tracker");
    let clipboard = tracker
        .register_clipboard_write(current.authority(), "1234")
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
        Ok(EnginePrivilegedRequestDecision::DeniedUnsupported)
    );
    assert_eq!(tracker.pending_clipboard_text_bytes(), 0);
    assert_eq!(tracker.pending_network_body_bytes(), 4);
    assert_eq!(
        tracker.preflight_network_once(&host, network),
        Ok(EngineNetworkTargetDecision::DeniedUnsupported)
    );
    assert_eq!(tracker.pending_network_body_bytes(), 0);
}
''')
