use crate::engine::{
    EngineCommittedDocumentAuthority, EngineCommittedDocumentSource, EngineHost, EngineHostError,
};
use crate::network_target_policy::EngineNetworkTargetDecision;
use rarog_fetch::{
    CredentialsMode, DEFAULT_MAX_RESPONSE_BODY_BYTES, FetchMethod, HeaderList, RedirectMode,
    RequestDestination, RequestMode,
};
use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub const DEFAULT_MAX_PENDING_PRIVILEGED_REQUESTS: usize = 4096;
pub const DEFAULT_MAX_PENDING_PRIVILEGED_NETWORK_BODY_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_PRIVILEGED_NETWORK_TARGET_BYTES: usize = 4 * 1024;
pub const MAX_PRIVILEGED_NETWORK_HEADERS: usize = 64;
pub const MAX_PRIVILEGED_NETWORK_HEADER_BYTES: usize = 16 * 1024;
pub const MAX_PRIVILEGED_NETWORK_BODY_BYTES: usize = 1024 * 1024;
pub const MAX_PRIVILEGED_NETWORK_DESTINATION_BYTES: usize = 256;
pub const MAX_PRIVILEGED_NETWORK_RESPONSE_BODY_BYTES: usize = DEFAULT_MAX_RESPONSE_BODY_BYTES;

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

/// One-shot Network request bound to the exact source authority, canonical method, bounded
/// canonical headers, exact optional bounded body, canonical Fetch envelope metadata, exact response-body
/// byte limit and raw target.
///
/// Target, header/body and custom destination contents are retained only in process memory and are
/// deliberately not included in `Debug` output because they can contain sensitive data. Body bytes
/// use immutable shared storage so cloning a request handle does not duplicate the body allocation.
/// Method, headers, mode, credentials, redirect and destination use canonical Rarog Fetch types;
/// the response-body limit follows the pinned Rarog non-zero byte-limit contract. Zorya does not
/// parse or normalize them independently. Registration does not parse or authorize
/// the target; canonical target policy runs only when the request is consumed.
#[derive(Clone, PartialEq, Eq)]
pub struct EngineNetworkPrivilegedRequest {
    id: EnginePrivilegedRequestId,
    source: EngineCommittedDocumentSource,
    method: FetchMethod,
    headers: HeaderList,
    body: Option<Arc<[u8]>>,
    mode: RequestMode,
    credentials: CredentialsMode,
    redirect: RedirectMode,
    destination: RequestDestination,
    max_response_body_bytes: usize,
    target: String,
}

impl EngineNetworkPrivilegedRequest {
    pub const fn id(&self) -> EnginePrivilegedRequestId {
        self.id
    }

    pub fn source(&self) -> &EngineCommittedDocumentSource {
        &self.source
    }

    pub const fn authority(&self) -> EngineCommittedDocumentAuthority {
        self.source.authority()
    }

    pub fn source_origin(&self) -> &rarog_url::Origin {
        self.source.origin()
    }

    pub fn method(&self) -> &FetchMethod {
        &self.method
    }

    pub fn headers(&self) -> &HeaderList {
        &self.headers
    }

    pub fn body(&self) -> Option<&[u8]> {
        self.body.as_deref()
    }

    pub const fn mode(&self) -> RequestMode {
        self.mode
    }

    pub const fn credentials(&self) -> CredentialsMode {
        self.credentials
    }

    pub const fn redirect(&self) -> RedirectMode {
        self.redirect
    }

    pub fn destination(&self) -> &RequestDestination {
        &self.destination
    }

    pub const fn max_response_body_bytes(&self) -> usize {
        self.max_response_body_bytes
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    fn body_bytes(&self) -> usize {
        self.body.as_ref().map_or(0, |body| body.len())
    }
}

impl fmt::Debug for EngineNetworkPrivilegedRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (destination_kind, destination_other_bytes) = match &self.destination {
            RequestDestination::Empty => ("empty", 0),
            RequestDestination::Document => ("document", 0),
            RequestDestination::Script => ("script", 0),
            RequestDestination::Style => ("style", 0),
            RequestDestination::Image => ("image", 0),
            RequestDestination::Font => ("font", 0),
            RequestDestination::Other(value) => ("other", value.len()),
        };
        formatter
            .debug_struct("EngineNetworkPrivilegedRequest")
            .field("id", &self.id)
            .field("authority", &self.source.authority())
            .field("method", &self.method)
            .field("target_bytes", &self.target.len())
            .field("header_count", &self.headers.len())
            .field("header_bytes", &self.headers.byte_len())
            .field("body_present", &self.body.is_some())
            .field("body_bytes", &self.body_bytes())
            .field("mode", &self.mode)
            .field("credentials", &self.credentials)
            .field("redirect", &self.redirect)
            .field("destination_kind", &destination_kind)
            .field("destination_other_bytes", &destination_other_bytes)
            .field("max_response_body_bytes", &self.max_response_body_bytes)
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
    InvalidNetworkBodyBudget,
    CapacityExceeded,
    IdentitySpaceExhausted,
    NetworkTargetRequired,
    NetworkTargetTooLong {
        bytes: usize,
        max: usize,
    },
    NetworkHeaderCountExceeded {
        count: usize,
        max: usize,
    },
    NetworkHeaderBytesExceeded {
        bytes: usize,
        max: usize,
    },
    NetworkHeaderRebindFailed,
    NetworkBodyNotPermitted {
        method: FetchMethod,
    },
    NetworkBodyTooLong {
        bytes: usize,
        max: usize,
    },
    NetworkDestinationTooLong {
        bytes: usize,
        max: usize,
    },
    NetworkResponseBodyLimitOutOfRange {
        bytes: usize,
        max: usize,
    },
    NetworkBodyBudgetExceeded {
        pending: usize,
        requested: usize,
        max: usize,
    },
    NetworkBodyAccountingInvariant,
    UnknownRequest(EnginePrivilegedRequestId),
    MismatchedRequest(EnginePrivilegedRequestId),
    Host(EngineHostError),
}

impl fmt::Display for EnginePrivilegedRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimit => formatter.write_str("privileged request limit must be non-zero"),
            Self::InvalidNetworkBodyBudget => {
                formatter.write_str("privileged Network body budget must be non-zero")
            }
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
            Self::NetworkHeaderCountExceeded { count, max } => write!(
                formatter,
                "Network privileged request has {count} headers; maximum is {max}"
            ),
            Self::NetworkHeaderBytesExceeded { bytes, max } => write!(
                formatter,
                "Network privileged request headers require {bytes} bytes; maximum is {max} bytes"
            ),
            Self::NetworkHeaderRebindFailed => formatter
                .write_str("canonical Network headers could not be rebound to product limits"),
            Self::NetworkBodyNotPermitted { method } => write!(
                formatter,
                "Network privileged request method {method} does not permit a request body"
            ),
            Self::NetworkBodyTooLong { bytes, max } => write!(
                formatter,
                "Network privileged request body is {bytes} bytes; maximum is {max} bytes"
            ),
            Self::NetworkDestinationTooLong { bytes, max } => write!(
                formatter,
                "Network privileged request destination is {bytes} bytes; maximum is {max} bytes"
            ),
            Self::NetworkResponseBodyLimitOutOfRange { bytes, max } => write!(
                formatter,
                "Network privileged request response-body limit is {bytes} bytes; allowed range is 1..={max} bytes"
            ),
            Self::NetworkBodyBudgetExceeded {
                pending,
                requested,
                max,
            } => write!(
                formatter,
                "Network privileged request body budget exceeded: {pending} pending bytes + {requested} requested bytes; maximum is {max} bytes"
            ),
            Self::NetworkBodyAccountingInvariant => {
                formatter.write_str("privileged Network body accounting invariant was violated")
            }
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

fn bind_network_headers(headers: &HeaderList) -> Result<HeaderList, EnginePrivilegedRequestError> {
    if headers.len() > MAX_PRIVILEGED_NETWORK_HEADERS {
        return Err(EnginePrivilegedRequestError::NetworkHeaderCountExceeded {
            count: headers.len(),
            max: MAX_PRIVILEGED_NETWORK_HEADERS,
        });
    }
    if headers.byte_len() > MAX_PRIVILEGED_NETWORK_HEADER_BYTES {
        return Err(EnginePrivilegedRequestError::NetworkHeaderBytesExceeded {
            bytes: headers.byte_len(),
            max: MAX_PRIVILEGED_NETWORK_HEADER_BYTES,
        });
    }

    let mut bounded = HeaderList::try_new(
        MAX_PRIVILEGED_NETWORK_HEADERS,
        MAX_PRIVILEGED_NETWORK_HEADER_BYTES,
    )
    .map_err(|_| EnginePrivilegedRequestError::NetworkHeaderRebindFailed)?;
    for header in headers.iter() {
        bounded
            .append(header.name().to_owned(), header.value().to_owned())
            .map_err(|_| EnginePrivilegedRequestError::NetworkHeaderRebindFailed)?;
    }
    Ok(bounded)
}

fn bind_network_body(
    method: &FetchMethod,
    body: Option<Vec<u8>>,
) -> Result<Option<Arc<[u8]>>, EnginePrivilegedRequestError> {
    if body.is_some() && !method.permits_body() {
        return Err(EnginePrivilegedRequestError::NetworkBodyNotPermitted {
            method: method.clone(),
        });
    }
    if let Some(body) = &body
        && body.len() > MAX_PRIVILEGED_NETWORK_BODY_BYTES
    {
        return Err(EnginePrivilegedRequestError::NetworkBodyTooLong {
            bytes: body.len(),
            max: MAX_PRIVILEGED_NETWORK_BODY_BYTES,
        });
    }
    Ok(body.map(Arc::<[u8]>::from))
}

fn bind_network_destination(
    destination: RequestDestination,
) -> Result<RequestDestination, EnginePrivilegedRequestError> {
    if let RequestDestination::Other(value) = &destination {
        if value.len() > MAX_PRIVILEGED_NETWORK_DESTINATION_BYTES {
            return Err(EnginePrivilegedRequestError::NetworkDestinationTooLong {
                bytes: value.len(),
                max: MAX_PRIVILEGED_NETWORK_DESTINATION_BYTES,
            });
        }
    }
    Ok(destination)
}

/// Bounded process-local lifecycle for future privileged request attempts.
///
/// Registration allocates only a one-shot correlation identity. It does not validate or grant
/// authority. Request IDs are process-global and monotonic so rebuilding a tracker cannot make an
/// old copied handle collide with a newly registered request. Generic registration cannot create a
/// Network request: Network attempts must retain an exact bounded target, canonical Rarog method,
/// bounded canonical Rarog headers, exact optional bounded body and canonical Fetch envelope
/// metadata. Consumption removes the exact stored request and releases its tracker-accounted body
/// bytes before source/target policy.
#[derive(Debug)]
pub struct EnginePrivilegedRequestTracker {
    max_pending: usize,
    max_pending_network_body_bytes: usize,
    pending_network_body_bytes: usize,
    pending: BTreeMap<EnginePrivilegedRequestId, PendingPrivilegedRequest>,
}

impl EnginePrivilegedRequestTracker {
    pub fn try_new(max_pending: usize) -> Result<Self, EnginePrivilegedRequestError> {
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

    pub fn with_default_limit() -> Result<Self, EnginePrivilegedRequestError> {
        Self::try_new(DEFAULT_MAX_PENDING_PRIVILEGED_REQUESTS)
    }

    pub const fn max_pending(&self) -> usize {
        self.max_pending
    }

    pub const fn max_pending_network_body_bytes(&self) -> usize {
        self.max_pending_network_body_bytes
    }

    pub const fn pending_network_body_bytes(&self) -> usize {
        self.pending_network_body_bytes
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

    /// Registers a bounded raw Network target with canonical GET semantics, no headers and no body.
    ///
    /// Target parsing, source revalidation and authorization remain deferred to consumption.
    pub fn register_network(
        &mut self,
        source: &EngineCommittedDocumentSource,
        target: impl Into<String>,
    ) -> Result<EngineNetworkPrivilegedRequest, EnginePrivilegedRequestError> {
        self.register_network_with_method_headers_and_body(
            source,
            FetchMethod::get(),
            target,
            HeaderList::default(),
            None,
        )
    }

    /// Registers a bounded raw Network target plus canonical GET headers and no body.
    pub fn register_network_with_headers(
        &mut self,
        source: &EngineCommittedDocumentSource,
        target: impl Into<String>,
        headers: HeaderList,
    ) -> Result<EngineNetworkPrivilegedRequest, EnginePrivilegedRequestError> {
        self.register_network_with_method_headers_and_body(
            source,
            FetchMethod::get(),
            target,
            headers,
            None,
        )
    }

    /// Registers a bounded raw Network target plus canonical Rarog Fetch method and no headers/body.
    ///
    /// HTTP method syntax, canonicalization and forbidden-method policy belong to
    /// `rarog_fetch::FetchMethod`; this function accepts that already-validated type rather than
    /// implementing another parser.
    pub fn register_network_with_method(
        &mut self,
        source: &EngineCommittedDocumentSource,
        method: FetchMethod,
        target: impl Into<String>,
    ) -> Result<EngineNetworkPrivilegedRequest, EnginePrivilegedRequestError> {
        self.register_network_with_method_headers_and_body(
            source,
            method,
            target,
            HeaderList::default(),
            None,
        )
    }

    /// Registers a bounded raw Network target plus canonical method/headers and no body.
    pub fn register_network_with_method_and_headers(
        &mut self,
        source: &EngineCommittedDocumentSource,
        method: FetchMethod,
        target: impl Into<String>,
        headers: HeaderList,
    ) -> Result<EngineNetworkPrivilegedRequest, EnginePrivilegedRequestError> {
        self.register_network_with_method_headers_and_body(source, method, target, headers, None)
    }

    /// Registers a bounded raw Network target, canonical method/headers and exact optional body
    /// with the pinned Rarog Fetch envelope defaults.
    ///
    /// This compatibility path remains exact: it binds `Cors`, `SameOrigin`, `Follow` and
    /// `Empty`, matching `rarog_fetch::FetchRequest::try_new` at the pinned Rarog revision.
    pub fn register_network_with_method_headers_and_body(
        &mut self,
        source: &EngineCommittedDocumentSource,
        method: FetchMethod,
        target: impl Into<String>,
        headers: HeaderList,
        body: Option<Vec<u8>>,
    ) -> Result<EngineNetworkPrivilegedRequest, EnginePrivilegedRequestError> {
        self.register_network_with_request_parts(
            source,
            method,
            target,
            headers,
            body,
            RequestMode::Cors,
            CredentialsMode::SameOrigin,
            RedirectMode::Follow,
            RequestDestination::Empty,
        )
    }

    /// Registers the exact bounded current Network request envelope with the pinned Rarog
    /// default response-body limit, without target parsing, source revalidation or authorization.
    ///
    /// This compatibility path binds `DEFAULT_MAX_RESPONSE_BODY_BYTES` exactly; callers that need
    /// an explicit canonical response limit use
    /// `register_network_with_request_parts_and_response_limit`.
    #[allow(clippy::too_many_arguments)]
    pub fn register_network_with_request_parts(
        &mut self,
        source: &EngineCommittedDocumentSource,
        method: FetchMethod,
        target: impl Into<String>,
        headers: HeaderList,
        body: Option<Vec<u8>>,
        mode: RequestMode,
        credentials: CredentialsMode,
        redirect: RedirectMode,
        destination: RequestDestination,
    ) -> Result<EngineNetworkPrivilegedRequest, EnginePrivilegedRequestError> {
        self.register_network_with_request_parts_and_response_limit(
            source,
            method,
            target,
            headers,
            body,
            mode,
            credentials,
            redirect,
            destination,
            DEFAULT_MAX_RESPONSE_BODY_BYTES,
        )
    }

    /// Registers the exact bounded current Network request envelope and response-body byte limit
    /// without target parsing, source revalidation or authorization.
    ///
    /// Incoming headers are rebound through Rarog `HeaderList::append` into fixed product limits.
    /// Body/method compatibility reuses Rarog `FetchMethod::permits_body`; mode, credentials,
    /// redirect and destination remain canonical pinned-Rarog Fetch values. The response-body limit
    /// is retained exactly under Rarog's non-zero `FetchLimits` contract and Zorya caps the value at
    /// the pinned Rarog default so a future execution path cannot inherit an effectively unbounded
    /// response budget from correlation state.
    /// `RequestDestination::Other` is retained byte-for-byte but capped before request-ID,
    /// pending-slot or tracker body-budget consumption. Deferring target parsing until consumption
    /// preserves stale-source-before-target classification.
    #[allow(clippy::too_many_arguments)]
    pub fn register_network_with_request_parts_and_response_limit(
        &mut self,
        source: &EngineCommittedDocumentSource,
        method: FetchMethod,
        target: impl Into<String>,
        headers: HeaderList,
        body: Option<Vec<u8>>,
        mode: RequestMode,
        credentials: CredentialsMode,
        redirect: RedirectMode,
        destination: RequestDestination,
        max_response_body_bytes: usize,
    ) -> Result<EngineNetworkPrivilegedRequest, EnginePrivilegedRequestError> {
        let target = target.into();
        if target.len() > MAX_PRIVILEGED_NETWORK_TARGET_BYTES {
            return Err(EnginePrivilegedRequestError::NetworkTargetTooLong {
                bytes: target.len(),
                max: MAX_PRIVILEGED_NETWORK_TARGET_BYTES,
            });
        }
        if max_response_body_bytes == 0
            || max_response_body_bytes > MAX_PRIVILEGED_NETWORK_RESPONSE_BODY_BYTES
        {
            return Err(
                EnginePrivilegedRequestError::NetworkResponseBodyLimitOutOfRange {
                    bytes: max_response_body_bytes,
                    max: MAX_PRIVILEGED_NETWORK_RESPONSE_BODY_BYTES,
                },
            );
        }
        let destination = bind_network_destination(destination)?;
        let headers = bind_network_headers(&headers)?;
        let body = bind_network_body(&method, body)?;
        let body_bytes = body.as_ref().map_or(0, |body| body.len());
        let next_body_bytes = self
            .pending_network_body_bytes
            .checked_add(body_bytes)
            .ok_or(EnginePrivilegedRequestError::NetworkBodyBudgetExceeded {
                pending: self.pending_network_body_bytes,
                requested: body_bytes,
                max: self.max_pending_network_body_bytes,
            })?;
        if next_body_bytes > self.max_pending_network_body_bytes {
            return Err(EnginePrivilegedRequestError::NetworkBodyBudgetExceeded {
                pending: self.pending_network_body_bytes,
                requested: body_bytes,
                max: self.max_pending_network_body_bytes,
            });
        }
        self.ensure_capacity()?;

        let id = allocate_privileged_request_id()?;
        let request = EngineNetworkPrivilegedRequest {
            id,
            source: source.clone(),
            method,
            headers,
            body,
            mode,
            credentials,
            redirect,
            destination,
            max_response_body_bytes,
            target,
        };
        self.insert_pending(id, PendingPrivilegedRequest::Network(request.clone()));
        self.pending_network_body_bytes = next_body_bytes;
        Ok(request)
    }

    /// Consumes an exact generic request before checking its source authority.
    ///
    /// Replays are therefore rejected even when the source document is still current. A handle
    /// with the right ID but mismatched source/kind also burns that slot and fails closed. If the
    /// ID actually belongs to a Network request, its tracker-accounted body bytes are released
    /// before returning the mismatch.
    pub fn preflight_once(
        &mut self,
        host: &EngineHost,
        request: EnginePrivilegedRequest,
    ) -> Result<EnginePrivilegedRequestDecision, EnginePrivilegedRequestError> {
        let stored = self.remove_pending(request.id)?;
        let PendingPrivilegedRequest::Generic(stored) = stored else {
            return Err(EnginePrivilegedRequestError::MismatchedRequest(request.id));
        };
        if stored != request {
            return Err(EnginePrivilegedRequestError::MismatchedRequest(request.id));
        }

        host.preflight_privileged_request(stored.authority, stored.kind)
            .map_err(EnginePrivilegedRequestError::from)
    }

    /// Consumes the exact source/target/method/header/body/envelope-bound Network request before
    /// applying canonical target policy.
    ///
    /// Same-ID source, method, headers, body, mode, credentials, redirect, destination, response limit or target
    /// mismatches burn the stored slot. Stored body
    /// bytes are released from tracker accounting before equality or policy. For an exact handle,
    /// `preflight_network_target` revalidates source authority before parsing/classifying the raw
    /// target. The current policy remains non-authorizing; retained method/headers/body/envelope
    /// metadata and response limit are correlation data for a later reviewed Fetch/broker path and are
    /// not executed or enforced here.
    pub fn preflight_network_once(
        &mut self,
        host: &EngineHost,
        request: EngineNetworkPrivilegedRequest,
    ) -> Result<EngineNetworkTargetDecision, EnginePrivilegedRequestError> {
        let id = request.id;
        let stored = self.remove_pending(id)?;
        let PendingPrivilegedRequest::Network(stored) = stored else {
            return Err(EnginePrivilegedRequestError::MismatchedRequest(id));
        };
        if stored != request {
            return Err(EnginePrivilegedRequestError::MismatchedRequest(id));
        }

        host.preflight_network_target(&stored.source, stored.target())
            .map_err(EnginePrivilegedRequestError::from)
    }

    fn ensure_capacity(&self) -> Result<(), EnginePrivilegedRequestError> {
        if self.pending.len() >= self.max_pending {
            Err(EnginePrivilegedRequestError::CapacityExceeded)
        } else {
            Ok(())
        }
    }

    fn remove_pending(
        &mut self,
        id: EnginePrivilegedRequestId,
    ) -> Result<PendingPrivilegedRequest, EnginePrivilegedRequestError> {
        let request = self
            .pending
            .remove(&id)
            .ok_or(EnginePrivilegedRequestError::UnknownRequest(id))?;
        let body_bytes = match &request {
            PendingPrivilegedRequest::Generic(_) => 0,
            PendingPrivilegedRequest::Network(request) => request.body_bytes(),
        };
        self.pending_network_body_bytes =
            self.pending_network_body_bytes
                .checked_sub(body_bytes)
                .ok_or(EnginePrivilegedRequestError::NetworkBodyAccountingInvariant)?;
        Ok(request)
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
#[path = "privileged_request_network_body_tests.rs"]
mod network_body_tests;

#[cfg(test)]
#[path = "privileged_request_network_envelope_tests.rs"]
mod network_envelope_tests;

#[cfg(test)]
#[path = "privileged_request_network_origin_tests.rs"]
mod network_origin_tests;

#[cfg(test)]
#[path = "privileged_request_network_response_limit_tests.rs"]
mod network_response_limit_tests;

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
                EngineNavigationPoll::Pending => panic!("privileged fixture timed out"),
                EngineNavigationPoll::Committed { .. } => break,
                EngineNavigationPoll::Failed { message } => {
                    panic!("privileged fixture navigation failed: {message}")
                }
                EngineNavigationPoll::Stale => panic!("privileged fixture navigation went stale"),
            }
        }

        host.committed_document_source(tab)
            .expect("committed authority query")
            .expect("committed remote authority")
    }

    fn one_header_list(name: &str, value: &str) -> HeaderList {
        let mut headers = HeaderList::try_new(128, 32 * 1024).expect("caller header limits");
        headers
            .append(name.to_owned(), value.to_owned())
            .expect("canonical header");
        headers
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
            host.preflight_privileged_request(
                current.authority(),
                EnginePrivilegedRequestKind::Network
            ),
            Ok(EnginePrivilegedRequestDecision::DeniedUnsupported)
        );
        assert_eq!(
            host.preflight_privileged_request(
                current.authority(),
                EnginePrivilegedRequestKind::Clipboard
            ),
            Ok(EnginePrivilegedRequestDecision::DeniedUnsupported)
        );

        assert_eq!(
            host.begin_navigation(tab, "mailto:zorya@example.invalid")
                .expect("blocked external protocol"),
            None
        );
        assert_eq!(
            host.preflight_privileged_request(
                current.authority(),
                EnginePrivilegedRequestKind::Clipboard
            ),
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
            host.preflight_privileged_request(
                first.authority(),
                EnginePrivilegedRequestKind::Network
            ),
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
            host.preflight_privileged_request(
                same_site.authority(),
                EnginePrivilegedRequestKind::Clipboard
            ),
            Ok(EnginePrivilegedRequestDecision::DeniedStaleAuthority)
        );

        host.load_local_html(tab, "<main>local</main>")
            .expect("replace with local document");
        assert_eq!(
            host.preflight_privileged_request(
                cross_site.authority(),
                EnginePrivilegedRequestKind::Network
            ),
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
            host.preflight_privileged_request(
                stale.authority(),
                EnginePrivilegedRequestKind::Clipboard
            ),
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
            host.preflight_privileged_request(
                stale.authority(),
                EnginePrivilegedRequestKind::Network
            ),
            Ok(EnginePrivilegedRequestDecision::DeniedStaleAuthority)
        );
        assert_eq!(
            host.preflight_privileged_request(
                current.authority(),
                EnginePrivilegedRequestKind::Network
            ),
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
            tracker.register(current.authority(), EnginePrivilegedRequestKind::Network),
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
            .register(current.authority(), EnginePrivilegedRequestKind::Clipboard)
            .expect("register request");

        assert!(request.id().get() > 0);
        assert_eq!(request.authority(), current.authority());
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
            .register(first.authority(), EnginePrivilegedRequestKind::Clipboard)
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
            .register_network(&current, "http://127.0.0.1:1/resource")
            .expect("register Network request");

        assert!(request.id().get() > 0);
        assert_eq!(request.authority(), current.authority());
        assert_eq!(request.method().as_str(), "GET");
        assert!(request.headers().is_empty());
        assert_eq!(request.body(), None);
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
            .register_network_with_method(&current, method, "http://127.0.0.1:1/method-bound")
            .expect("register method-bound request");
        assert_eq!(request.method().as_str(), "POST");
        assert!(request.headers().is_empty());
        assert_eq!(request.body(), None);
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
    fn network_headers_are_rebound_to_fixed_product_limits() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/header-binding"),
        );
        let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
        let headers = one_header_list("X-Zorya-Request", "bound");
        let mut expected = HeaderList::try_new(
            MAX_PRIVILEGED_NETWORK_HEADERS,
            MAX_PRIVILEGED_NETWORK_HEADER_BYTES,
        )
        .expect("product header limits");
        expected
            .append("X-Zorya-Request", "bound")
            .expect("expected canonical header");

        let request = tracker
            .register_network_with_method_and_headers(
                &current,
                FetchMethod::post(),
                "http://127.0.0.1:1/header-bound",
                headers,
            )
            .expect("register header-bound request");
        assert_eq!(request.method().as_str(), "POST");
        assert_eq!(request.headers(), &expected);
        assert_eq!(request.headers().len(), 1);
        assert_eq!(request.body(), None);
        assert_eq!(
            tracker.preflight_network_once(&host, request),
            Ok(EngineNetworkTargetDecision::DeniedUnsupported)
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
                &current,
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
            .register_network_with_headers(
                &first,
                "../relative",
                one_header_list("X-Zorya-Stale", "still-bound"),
            )
            .expect("register raw target and headers");

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
                .register_network(&current, target)
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
            .register_network(&current, "http://127.0.0.1:1/original")
            .expect("register request");
        let forged = EngineNetworkPrivilegedRequest {
            id: request.id,
            source: request.source.clone(),
            method: request.method.clone(),
            headers: request.headers.clone(),
            body: request.body.clone(),
            mode: RequestMode::Cors,
            credentials: CredentialsMode::SameOrigin,
            redirect: RedirectMode::Follow,
            destination: RequestDestination::Empty,
            max_response_body_bytes: request.max_response_body_bytes,
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
            .register_network(&current, "http://127.0.0.1:1/original")
            .expect("register request");
        let forged = EngineNetworkPrivilegedRequest {
            id: request.id,
            source: request.source.clone(),
            method: FetchMethod::post(),
            headers: request.headers.clone(),
            body: request.body.clone(),
            mode: RequestMode::Cors,
            credentials: CredentialsMode::SameOrigin,
            redirect: RedirectMode::Follow,
            destination: RequestDestination::Empty,
            max_response_body_bytes: request.max_response_body_bytes,
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
    fn mismatched_network_headers_burn_the_registered_slot() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/header-mismatch"),
        );
        let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
        let request = tracker
            .register_network_with_headers(
                &current,
                "http://127.0.0.1:1/original",
                one_header_list("X-Zorya", "original"),
            )
            .expect("register request");
        let forged = EngineNetworkPrivilegedRequest {
            id: request.id,
            source: request.source.clone(),
            method: request.method.clone(),
            headers: bind_network_headers(&one_header_list("X-Zorya", "forged"))
                .expect("forged bounded headers"),
            body: request.body.clone(),
            mode: RequestMode::Cors,
            credentials: CredentialsMode::SameOrigin,
            redirect: RedirectMode::Follow,
            destination: RequestDestination::Empty,
            max_response_body_bytes: request.max_response_body_bytes,
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
            tracker.register_network(&current, oversized),
            Err(EnginePrivilegedRequestError::NetworkTargetTooLong {
                bytes: MAX_PRIVILEGED_NETWORK_TARGET_BYTES + 1,
                max: MAX_PRIVILEGED_NETWORK_TARGET_BYTES,
            })
        );
        assert_eq!(tracker.pending_requests(), 0);

        tracker
            .register(current.authority(), EnginePrivilegedRequestKind::Clipboard)
            .expect("capacity remains available");
        assert_eq!(tracker.pending_requests(), 1);
    }

    #[test]
    fn oversized_network_header_count_is_rejected_without_consuming_capacity() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/header-count-bound"),
        );
        let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
        let mut headers = HeaderList::try_new(MAX_PRIVILEGED_NETWORK_HEADERS + 1, 32 * 1024)
            .expect("caller header limits");
        for _ in 0..=MAX_PRIVILEGED_NETWORK_HEADERS {
            headers.append("x", "v").expect("caller header");
        }

        assert_eq!(
            tracker
                .register_network_with_headers(&current, "http://127.0.0.1:1/resource", headers,),
            Err(EnginePrivilegedRequestError::NetworkHeaderCountExceeded {
                count: MAX_PRIVILEGED_NETWORK_HEADERS + 1,
                max: MAX_PRIVILEGED_NETWORK_HEADERS,
            })
        );
        assert_eq!(tracker.pending_requests(), 0);
        tracker
            .register(current.authority(), EnginePrivilegedRequestKind::Clipboard)
            .expect("capacity remains available");
    }

    #[test]
    fn oversized_network_header_bytes_are_rejected_without_consuming_capacity() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        let current = commit_remote(
            &mut host,
            tab,
            serve_once("127.0.0.1", "127.0.0.1", "/header-byte-bound"),
        );
        let mut tracker = EnginePrivilegedRequestTracker::try_new(1).expect("tracker");
        let value = "v".repeat(MAX_PRIVILEGED_NETWORK_HEADER_BYTES);
        let headers = one_header_list("x", &value);

        assert_eq!(
            tracker
                .register_network_with_headers(&current, "http://127.0.0.1:1/resource", headers,),
            Err(EnginePrivilegedRequestError::NetworkHeaderBytesExceeded {
                bytes: MAX_PRIVILEGED_NETWORK_HEADER_BYTES + 1,
                max: MAX_PRIVILEGED_NETWORK_HEADER_BYTES,
            })
        );
        assert_eq!(tracker.pending_requests(), 0);
        tracker
            .register(current.authority(), EnginePrivilegedRequestKind::Clipboard)
            .expect("capacity remains available");
    }

    #[test]
    fn network_request_debug_redacts_raw_target_and_headers() {
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
        let secret_header_name = "x-super-secret-header";
        let secret_header_value = "do-not-log-header-value";
        let request = tracker
            .register_network_with_method_and_headers(
                &current,
                FetchMethod::post(),
                secret,
                one_header_list(secret_header_name, secret_header_value),
            )
            .expect("register request");
        let debug = format!("{request:?}");

        assert!(!debug.contains(secret));
        assert!(!debug.contains("do-not-log"));
        assert!(!debug.contains(secret_header_name));
        assert!(!debug.contains(secret_header_value));
        assert!(debug.contains("POST"));
        assert!(debug.contains("target_bytes"));
        assert!(debug.contains("header_count"));
        assert!(debug.contains("header_bytes"));
        assert!(debug.contains("body_present"));
        assert!(debug.contains("body_bytes"));
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
            .register_network(&first, "http://127.0.0.1:1/resource")
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
            .register_network(&current, "http://127.0.0.1:1/stale")
            .expect("first request");
        assert_eq!(
            first_tracker.preflight_network_once(&host, stale.clone()),
            Ok(EngineNetworkTargetDecision::DeniedUnsupported)
        );

        let mut replacement_tracker =
            EnginePrivilegedRequestTracker::try_new(1).expect("replacement tracker");
        let current_request = replacement_tracker
            .register_network(&current, "http://127.0.0.1:1/current")
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
            .register(current.authority(), EnginePrivilegedRequestKind::Clipboard)
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
            .register(current.authority(), EnginePrivilegedRequestKind::Clipboard)
            .expect("first request");

        assert_eq!(tracker.max_pending(), 1);
        assert_eq!(
            tracker.max_pending_network_body_bytes(),
            DEFAULT_MAX_PENDING_PRIVILEGED_NETWORK_BODY_BYTES
        );
        assert_eq!(tracker.pending_network_body_bytes(), 0);
        assert_eq!(tracker.pending_requests(), 1);
        assert_eq!(
            tracker.register_network_with_headers(
                &current,
                "http://127.0.0.1:1/resource",
                one_header_list("X-Zorya", "capacity"),
            ),
            Err(EnginePrivilegedRequestError::CapacityExceeded)
        );
        assert_eq!(tracker.pending_requests(), 1);
        assert_eq!(
            tracker.preflight_once(&host, first),
            Ok(EnginePrivilegedRequestDecision::DeniedUnsupported)
        );
    }
}
