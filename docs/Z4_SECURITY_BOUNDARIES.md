# Z4 Security and Process-Integration Boundaries

This document records product-side Z4 security boundaries that sit above the pinned Rarog embedder/Host contracts. It does not redefine Web-engine URL, origin, Fetch or Site semantics.

## Remote document policy

`engine::EngineHost` installs a Zorya-owned implementation of Rarog's embedder `HostPolicy`.

Document navigation is fail-closed at this product boundary:

- the requested target must parse through `rarog_url::WebUrl`;
- only `http` and `https` document schemes are allowed to proceed;
- malformed or relative input and `file:`, `data:`, `mailto:` or custom/external schemes are blocked before Zorya opens a Rarog Host navigation context, grants a context-scoped Network capability or starts a Host network operation;
- blocking a target does not displace the authority snapshot of an already committed remote document;
- generic Rarog resource-request forwarding is denied until Zorya has a separately reviewed capability-routed subresource transport path.

Rarog's own navigation and Fetch checks remain independently authoritative after this browser-product gate. Zorya does not implement an alternate Fetch policy or infer security identity from browser display/history strings.

## Host authority boundary

Each `EngineHost` instance receives a monotonic non-zero process-local incarnation token. Allocation fails closed if that identity space is exhausted. The token is ephemeral process state: it is not persisted, not a Rarog identity and not reused to reconstruct Host authority after replacement.

For a successfully committed remote document, `EngineHost::committed_document_authority` exposes only ephemeral wrapper tokens derived from the current EngineHost incarnation, live Rarog Host navigation context and current Host-assigned Site process. The snapshot is bound to the Host incarnation, stable product `TabId` and current View generation. Pending, cancelled, failed or policy-blocked navigation cannot replace it; retired or lost Host state fails closed.

`EngineHost::validate_committed_document_authority` is the replay-safety gate for a previously captured snapshot before any future privileged brokering. Validation resolves the current committed authority again through the live Host and accepts only an exact match of EngineHost incarnation, `TabId`, View generation, navigation-context token and Site-process token. A replaced EngineHost, replaced context, local document, closed View or recreated View is stale and returns `false`; unexpected missing or divergent live Host authority remains `InconsistentNavigationState`. Possession of an old snapshot is never authorization by itself. In particular, restarting the EngineHost cannot make an old snapshot current merely because fresh per-Host context/process counters happen to reuse the same numeric values.

Revalidation is read-only: it mints no Network or Clipboard capability, starts no network operation, grants no IPC authority and does not persist any Host token.

These Host/context/process tokens are not product identities and are never persisted in profile or session state.

## Privileged request admission

`EngineHost::preflight_privileged_request` is a deny-by-default browser-product gate for future privileged requests. It first reuses committed-document authority revalidation so a replaced Host, replaced/local document, closed or recreated View is rejected as stale before any later capability policy could run. A still-current remote document reaches only `DeniedUnsupported` for both current request labels (`Network` and `Clipboard`). There is deliberately no allow/authorized result in this slice.

The request labels are Zorya product taxonomy only. They are not Rarog `CapabilityClass` values, capability IDs, grants or IPC authority. This preflight does not add a `rarog-broker` dependency, grant a capability, start a Host network operation, call a clipboard service or make possession of a current snapshot sufficient authorization. A future brokering slice must define a separately reviewed allow policy and must revalidate immediately before any actual grant.

## Canonical Network target policy

`EngineHost::preflight_network_target` adds a target-aware, still-non-authorizing policy gate for future privileged Network requests. It revalidates the committed-document authority before parsing or classifying the requested target. If the source is stale, replaced, local, closed, recreated or belongs to a retired EngineHost, target text is not allowed to upgrade that stale source into an eligible request.

For a current source, Zorya obtains the source `SiteIdentity` only from the live Rarog Host navigation context. The target is parsed only with the pinned `rarog_url::WebUrl` contract and its canonical `site_identity()` result. Browser display/history strings and home-grown registrable-domain logic are not security identities. Malformed or relative targets are denied, non-HTTP(S) targets are denied before Site comparison, and HTTP(S) targets must be schemeful-same-site according to Rarog `SiteIdentity`; ports do not split a Rarog Site, while a scheme or Site change does. Missing or divergent live Host process/context state remains `InconsistentNavigationState` rather than falling back to URL-derived source identity.

Even a current same-site HTTP(S) target reaches only `DeniedUnsupported`. There is deliberately no allow/authorized target-policy result, no `rarog-broker` dependency, no capability grant and no Host network/resource operation in this slice. The target policy does not implement Fetch/CORS/origin/credentials/redirect behavior and must not be presented as doing so. A later actual broker path must repeat the required source/policy checks immediately before any grant.

## One-shot privileged request lifecycle

`EnginePrivilegedRequestTracker` provides a bounded process-local lifecycle around future privileged request attempts. The default tracker retains at most 4096 pending attempts, rejects a zero configured limit or capacity overflow, and uses process-global monotonic non-zero `EnginePrivilegedRequestId` values. Allocation fails closed on identity-space exhaustion instead of wrapping or restarting when a tracker is rebuilt.

Generic registration captures the exact committed-document authority snapshot plus a product request kind, but it cannot create a source-only `Network` entry: `register(..., Network)` fails with `NetworkTargetRequired`. Network attempts must use `register_network`, which binds the same exact source authority to the exact raw requested target. The tracker retains at most 4096 bytes of target text per Network request (`MAX_PRIVILEGED_NETWORK_TARGET_BYTES`); oversized targets are rejected before they occupy a pending slot. Raw targets are process-local, are not persisted, and are deliberately omitted from the Network handle's `Debug` output because URLs may contain sensitive query or fragment data.

Registration remains correlation only. It does not parse a Network target, revalidate source authority or authorize an operation. Deferring target parsing until consumption preserves the fail-closed order established by `preflight_network_target`: a request staged before source replacement is rejected as stale before malformed, unsupported-scheme or cross-Site target classification can run.

Both `preflight_once` and `preflight_network_once` remove the exact registered identity before validation or policy. Unknown/already-consumed identities are rejected, while same-ID source/kind/target mismatches burn the stored slot rather than making it reusable. Generic exact requests invoke the deny-by-default source preflight. Exact target-bound Network requests invoke the canonical Network target preflight with the stored raw target. Consequently copied/cloned handles cannot replay through the same tracker or a replacement tracker, cross-EngineHost numeric counter reuse cannot revive an old request, and swapping the target on a captured request cannot redirect its policy check.

Even a current exact same-site target-bound Network request still ends at `DeniedUnsupported`. The lifecycle adds no allow result, Rarog broker dependency, capability grant, Host network operation, clipboard action or IPC authority. A later actual brokering path must accept only target-bound Network requests, preserve consume-once semantics and perform the separately reviewed policy/revalidation immediately before any grant.

## Explicitly not provided yet

This boundary does not provide or claim:

- production Site execution in a separate OS process;
- Windows Site IPC transport;
- process sandboxing or complete process isolation;
- OS external-protocol dispatch;
- local-file navigation or file chooser policy;
- download mediation;
- permission or clipboard mediation;
- privileged internal-page authorization;
- any successful Network capability policy result or grant;
- a capability-routed subresource pipeline;
- any successful Network or Clipboard capability brokering.

Those remain separate reviewed Z4/Rarog work. Browser policy, authority revalidation, target admission, privileged-request preflight and one-shot request tracking must not be presented as substitutes for missing process isolation or capability mediation.
