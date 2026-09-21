use crate::engine::{EngineCommittedDocumentSource, EngineHost, EngineHostError};
use rarog_url::WebUrl;

/// Fail-closed result of the browser-product Network target policy.
///
/// There is intentionally no allow/authorized variant. `DeniedUnsupported` means only that an
/// exact one-shot Network request from the current committed remote document targeted an HTTP(S)
/// URL in the same canonical Rarog Site; actual capability brokering remains unsupported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineNetworkTargetDecision {
    DeniedStaleAuthority,
    DeniedInvalidTarget,
    DeniedUnsupportedScheme,
    DeniedCrossSite,
    DeniedUnsupported,
}

/// Applies canonical target policy only after the caller has consumed and exact-matched a
/// specialized one-shot Network request.
///
/// This function is crate-private by design. Public callers cannot present an EngineHost plus a
/// source/target pair as a privileged admission primitive; the public path is
/// `EnginePrivilegedRequestTracker::preflight_network_once`.
///
/// Source authority is revalidated before target parsing. The exact live committed source,
/// including canonical Rarog Origin, must still equal the source captured in the consumed request.
/// Source Site identity comes only from that Host-cross-checked Origin and target identity comes
/// only from pinned Rarog URL semantics. This grants no capability and starts no network operation.
pub(crate) fn preflight_consumed_network_target(
    host: &EngineHost,
    source: &EngineCommittedDocumentSource,
    target: &str,
) -> Result<EngineNetworkTargetDecision, EngineHostError> {
    let authority = source.authority();
    if !host.validate_committed_document_authority(authority)? {
        return Ok(EngineNetworkTargetDecision::DeniedStaleAuthority);
    }
    let Some(current_source) = host.committed_document_source(source.tab())? else {
        return Ok(EngineNetworkTargetDecision::DeniedStaleAuthority);
    };
    if &current_source != source {
        return Ok(EngineNetworkTargetDecision::DeniedStaleAuthority);
    }
    let source_site = source.origin().site();

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
