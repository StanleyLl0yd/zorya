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

These context/process tokens are not product identities and are never persisted in profile or session state.

## Explicitly not provided yet

This boundary does not provide or claim:

- production Site execution in a separate OS process;
- Windows Site IPC transport;
- process sandboxing or complete process isolation;
- OS external-protocol dispatch;
- local-file navigation or file chooser policy;
- download mediation;
- permission mediation;
- privileged internal-page authorization;
- a capability-routed subresource pipeline.

Those remain separate reviewed Z4/Rarog work. Browser policy must not be presented as a substitute for missing process isolation.
