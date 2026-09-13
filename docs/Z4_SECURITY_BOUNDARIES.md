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

For a successfully committed remote document, `EngineHost::committed_document_authority` exposes only ephemeral wrapper tokens derived from the live Rarog Host navigation context and current Host-assigned Site process. The snapshot is bound to stable product `TabId` plus the current View generation. Pending, cancelled, failed or policy-blocked navigation cannot replace it; retired or lost Host state fails closed.

`EngineHost::validate_committed_document_authority` is the replay-safety gate for a previously captured snapshot before any future privileged brokering. Validation resolves the current committed authority again through the live Host and accepts only an exact match of `TabId`, View generation, navigation-context token and Site-process token. A replaced context, local document, closed View or recreated View is stale and returns `false`; unexpected missing or divergent live Host authority remains `InconsistentNavigationState`. Possession of an old snapshot is never authorization by itself.

Revalidation is read-only: it mints no Network or Clipboard capability, starts no network operation, grants no IPC authority and does not persist any Host token.

These context/process tokens are not product identities and are never persisted in profile or session state.

## Privileged request admission

`EngineHost::preflight_privileged_request` is a deny-by-default browser-product gate for future privileged requests. It first reuses committed-document authority revalidation so a replaced, local, closed or recreated document is rejected as stale before any later capability policy could run. A still-current remote document reaches only `DeniedUnsupported` for both current request labels (`Network` and `Clipboard`). There is deliberately no allow/authorized result in this slice.

The request labels are Zorya product taxonomy only. They are not Rarog `CapabilityClass` values, capability IDs, grants or IPC authority. This preflight does not add a `rarog-broker` dependency, grant a capability, start a Host network operation, call a clipboard service or make possession of a current snapshot sufficient authorization. A future brokering slice must define a separately reviewed allow policy and must revalidate immediately before any actual grant.

## One-shot privileged request lifecycle

`EnginePrivilegedRequestTracker` adds a bounded process-local lifecycle around future privileged request attempts. Registration captures the exact committed-document authority snapshot plus the product request kind under a monotonic non-zero `EnginePrivilegedRequestId`. The default tracker retains at most 4096 pending attempts and rejects a zero configured limit, capacity overflow and identity-space exhaustion.

A request identity is a correlation token only. It is not a Rarog capability ID, grant, Site identity or persisted browser identity, and registration performs no authorization. `preflight_once` removes the exact registered request before checking it, rejects unknown/already-consumed identities, rejects a handle whose authority or kind no longer matches the registered tuple, and leaves a mismatched slot consumed rather than reusable. For an exact request it then invokes the deny-by-default Host preflight, which revalidates live committed-document authority at consumption time. Consequently a copied handle cannot be replayed, and a request staged before a replacement commit becomes `DeniedStaleAuthority` when consumed.

Even a current exact one-shot request still ends at `DeniedUnsupported`. The lifecycle does not add an allow result, Rarog broker dependency, capability grant, Host network operation, clipboard action or IPC authority. A later actual brokering path must preserve consume-once semantics and perform the separately reviewed policy/revalidation immediately before any grant.

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
- target-aware Network capability policy;
- a capability-routed subresource pipeline;
- any successful Network or Clipboard capability brokering.

Those remain separate reviewed Z4/Rarog work. Browser policy, authority revalidation, privileged-request preflight and one-shot request tracking must not be presented as substitutes for missing process isolation or capability mediation.
