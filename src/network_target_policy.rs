use crate::engine::{
    EngineCommittedDocumentSource, EngineHost, EngineHostError, EngineNetworkAuthorization,
};
use rarog_url::WebUrl;

/// Fail-closed denial result of the browser-product Network target policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineNetworkTargetDecision {
    DeniedStaleAuthority,
    DeniedInvalidTarget,
    DeniedUnsupportedScheme,
    DeniedCrossSite,
    #[cfg(test)]
    DeniedUnsupported,
}

/// Result of consuming one exact Network privileged request through canonical target policy.
///
/// An authorized result owns one opaque, non-cloneable Rarog context-scoped Network capability.
/// The capability starts no backend operation and must be explicitly revoked if it is not handed to
/// a later reviewed execution path.
#[derive(Debug)]
pub enum EngineNetworkAuthorizationResult {
    Authorized(EngineNetworkAuthorization),
    Denied(EngineNetworkTargetDecision),
}

fn classify_consumed_network_target(
    host: &EngineHost,
    source: &EngineCommittedDocumentSource,
    target: &str,
) -> Result<Option<EngineNetworkTargetDecision>, EngineHostError> {
    if !host.validate_committed_document_source(source)? {
        return Ok(Some(EngineNetworkTargetDecision::DeniedStaleAuthority));
    }
    let source_site = source.origin().site();

    let target = match WebUrl::parse(target) {
        Ok(target) => target,
        Err(_) => return Ok(Some(EngineNetworkTargetDecision::DeniedInvalidTarget)),
    };
    if !matches!(target.scheme(), "http" | "https") {
        return Ok(Some(EngineNetworkTargetDecision::DeniedUnsupportedScheme));
    }
    let target_site = match target.site_identity() {
        Ok(site) => site,
        Err(_) => return Ok(Some(EngineNetworkTargetDecision::DeniedInvalidTarget)),
    };
    if !source_site.same_site(&target_site) {
        return Ok(Some(EngineNetworkTargetDecision::DeniedCrossSite));
    }

    Ok(None)
}

/// Applies canonical target policy only after the caller has consumed and exact-matched a
/// specialized one-shot Network request.
///
/// This function is crate-private by design. It revalidates the exact committed source before
/// parsing/classifying the target. Only an exact current same-Site HTTP(S) target can reach the
/// Host-owned context capability grant. The source is revalidated again immediately before the
/// grant through `EngineHost::grant_network_authorization`.
pub(crate) fn authorize_consumed_network_target(
    host: &mut EngineHost,
    source: &EngineCommittedDocumentSource,
    target: &str,
) -> Result<EngineNetworkAuthorizationResult, EngineHostError> {
    if let Some(decision) = classify_consumed_network_target(host, source, target)? {
        return Ok(EngineNetworkAuthorizationResult::Denied(decision));
    }

    match host.grant_network_authorization(source)? {
        Some(authorization) => Ok(EngineNetworkAuthorizationResult::Authorized(authorization)),
        None => Ok(EngineNetworkAuthorizationResult::Denied(
            EngineNetworkTargetDecision::DeniedStaleAuthority,
        )),
    }
}

#[cfg(test)]
pub(crate) fn preflight_consumed_network_target(
    host: &EngineHost,
    source: &EngineCommittedDocumentSource,
    target: &str,
) -> Result<EngineNetworkTargetDecision, EngineHostError> {
    Ok(classify_consumed_network_target(host, source, target)?
        .unwrap_or(EngineNetworkTargetDecision::DeniedUnsupported))
}
