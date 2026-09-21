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

## Canonical Network source Origin

`EngineHost::committed_document_source` is the only product constructor for a Network source. It combines the exact current `EngineCommittedDocumentAuthority` with the canonical Rarog `Origin` derived from the live committed Rarog `View::document_url()`. A source is minted only for a present HTTP(S) document URL with a non-opaque Origin, and that Origin's Rarog Site must agree with the live Host navigation-context Site/process assignment. Local documents expose no Network source; missing, opaque or divergent live state fails closed instead of fabricating identity. Zorya adds no independent Origin parser, default-port table or host normalization.

The canonical Origin is shared across source/request clones and is part of exact one-shot Network request identity together with the Host authority snapshot. It is correlation state, not authorization. `Debug` output intentionally omits Origin/host/port serialization. Binding this source does not synthesize an HTTP `Origin` header and does not implement CORS or Fetch execution. Canonical mode/credentials/redirect/destination values retained by the one-shot request envelope are correlation state only and do not enact Fetch policy.

## Committed internal-page authority

Privileged internal-page provenance is no longer inferred from a URL-like browser string. `EngineInternalPage` is a finite Zorya product identity; in this slice only `Start` exists. `EngineHost::load_internal_page` chooses the exact bundled Start HTML internally and loads it with Rarog's local `about:blank` base. No caller-provided source can be marked as a privileged internal page. The existing `load_local_html` path remains available for deterministic local rendering/tests but is explicitly unprivileged and clears any committed internal-page authority, including when passed bytes identical to the Start asset.

A committed internal document is represented by `EngineCommittedInternalPageAuthority`: exact EngineHost incarnation, stable product `TabId`, current View generation, finite page kind and a monotonic non-zero per-Host internal-document token. A same-page reload consumes a new token, so an older Start snapshot cannot become valid again merely because the page kind and `about:blank` display label match. Identity-space exhaustion fails closed before replacing the current document. Closing/recreating a View or replacing EngineHost invalidates old snapshots through the existing View-generation/Host-incarnation boundaries even when per-Host document counters collide.

`committed_internal_page_authority` cross-checks the stored page state against the live Rarog View: there must be no committed remote Host context, no remote `document_url`, and the local base must still equal the page's expected `about:blank` base. `validate_committed_internal_page_authority` compares the exact current snapshot and returns false for normal stale/closed/replaced state; impossible live-state divergence remains `InconsistentNavigationState`. The browser-model history/address value `about:blank` is only a display/session label and is never consulted as security authority.

Internal and remote committed authority are mutually exclusive. Beginning remote navigation does not displace a committed internal page; failed, cancelled, superseded or Host-policy-blocked attempts preserve it. A successful remote document commit clears internal-page authority and establishes the normal remote Host authority. Loading an internal or generic local document removes committed remote Host authority. This slice introduces no custom externally navigable internal scheme, privileged page action, JavaScript/native bridge, capability grant, permission decision or broker path.

## Specialized privileged request admission

Zorya exposes no generic privileged-request handle, request-kind label, tracker registration path or public generic Host preflight. Network and Clipboard attempts can enter `EnginePrivilegedRequestTracker` only through their specialized exact envelopes: `EngineNetworkPrivilegedRequest` and `EngineClipboardPrivilegedRequest`. This prevents a future caller from bypassing reviewed per-class identity requirements by supplying only a source authority plus a broad request label.

Network consumption continues into the canonical target-aware policy after exact one-shot removal/equality. Clipboard consumption reaches a private Clipboard-specific source revalidation only after exact one-shot removal/equality; a replaced Host, replaced/local document, closed or recreated View yields stale denial, while a still-current committed remote document yields only `DeniedUnsupported`. There is no allow/authorized Clipboard result.

These specialized Zorya envelopes and deny results are not Rarog `CapabilityClass` values, capability IDs, grants or IPC authority. This boundary does not add a `rarog-broker` dependency, grant a capability, start a Host Network operation, call a clipboard service or make possession of a current snapshot sufficient authorization. A later brokering slice must define a separately reviewed allow policy and repeat required live revalidation immediately before any actual grant/backend operation.

## Canonical Network target policy

`EngineHost::preflight_network_target` adds a target-aware, still-non-authorizing policy gate for future privileged Network requests. It first revalidates the exact Host-minted committed source, including re-minting the current authority plus canonical Origin from live View/Host state and requiring exact source equality, before parsing or classifying the requested target. If the source is stale, replaced, local, closed, recreated or belongs to a retired EngineHost, target text is not allowed to upgrade that stale source into an eligible request.

For a current source, target admission uses `source.origin().site()` only after source minting has already cross-checked that canonical Origin Site against the live Rarog Host navigation context and Site-process assignment. The target is parsed only with the pinned `rarog_url::WebUrl` contract and its canonical `site_identity()` result. Browser display/history strings and home-grown registrable-domain logic are not security identities. Malformed or relative targets are denied, non-HTTP(S) targets are denied before Site comparison, and HTTP(S) targets must be schemeful-same-site according to Rarog `SiteIdentity`; ports do not split a Rarog Site, while a scheme or Site change does. Missing or divergent live Host process/context state remains `InconsistentNavigationState` rather than falling back to URL-derived source identity.

Even a current same-site HTTP(S) target reaches only `DeniedUnsupported`. There is deliberately no allow/authorized target-policy result, no `rarog-broker` dependency, no capability grant and no Host network/resource operation in this slice. The target policy does not synthesize an HTTP `Origin` header and does not implement CORS, Fetch execution or mode/credentials/redirect/destination behavior; retaining canonical values in the one-shot envelope must not be presented as enacting those semantics. A later actual broker path must repeat the required source/policy checks immediately before any grant.

## One-shot privileged request lifecycle

`EnginePrivilegedRequestTracker` provides a bounded process-local lifecycle around future privileged request attempts. The default tracker retains at most 4096 pending attempts, at most 16 MiB of tracker-accounted pending Network request-body bytes (`DEFAULT_MAX_PENDING_PRIVILEGED_NETWORK_BODY_BYTES`) and independently at most 16 MiB of pending Clipboard write-text bytes (`DEFAULT_MAX_PENDING_PRIVILEGED_CLIPBOARD_TEXT_BYTES`). Zero configured request/body/Clipboard budgets are rejected, and process-global monotonic non-zero `EnginePrivilegedRequestId` values fail closed on exhaustion instead of wrapping or restarting when a tracker is rebuilt.

There is no generic registration path or generic pending request representation. Network attempts require and clone the Host-minted exact committed source (authority plus canonical Origin), binding that exact source to the raw requested target, a canonical pinned-Rarog `FetchMethod`, a bounded canonical pinned-Rarog `HeaderList`, an exact optional bounded request body, canonical pinned-Rarog `RequestMode`, `CredentialsMode`, `RedirectMode` and `RequestDestination`, and an exact bounded response-body byte limit. The existing `register_network`, `register_network_with_headers`, `register_network_with_method`, and `register_network_with_method_and_headers` paths remain explicit no-body conveniences; `register_network_with_method_headers_and_body` preserves the pinned Rarog Fetch envelope defaults (`Cors`, `SameOrigin`, `Follow`, `Empty`), while `register_network_with_request_parts` also binds the pinned `DEFAULT_MAX_RESPONSE_BODY_BYTES`. `register_network_with_request_parts_and_response_limit` is the explicit full path and accepts only response limits in `1..=MAX_PRIVILEGED_NETWORK_RESPONSE_BODY_BYTES`. Zorya does not add another HTTP-method or header parser or normalization table: method canonicalization/token validation/CONNECT-TRACE-TRACK rejection plus header-name/value validation and normalization remain Rarog Fetch semantics.

Clipboard attempts use a separate `EngineClipboardPrivilegedRequest` bound only to an `EngineCommittedDocumentAuthority` minted for a committed remote document. `ReadText` retains an exact non-zero maximum result size and defaults to `MAX_PRIVILEGED_CLIPBOARD_TEXT_BYTES`; `WriteText` retains the exact UTF-8 text in shared immutable `Arc<str>` storage. Zorya caps either direction at 1 MiB, deliberately stricter than the pinned Rarog platform Clipboard default, and a future backend path must still construct/revalidate Rarog Clipboard state against its live `ClipboardLimits`. Zero/over-maximum read limits, oversized writes, aggregate Clipboard write-budget overflow and arithmetic overflow fail before request-ID allocation or pending-slot use. Internal-page authority is a distinct type and is not accepted by these remote-document Clipboard registration APIs.

The tracker retains at most 4096 bytes of target text per Network request (`MAX_PRIVILEGED_NETWORK_TARGET_BYTES`), at most 64 canonical header entries (`MAX_PRIVILEGED_NETWORK_HEADERS`), at most 16 KiB of aggregate canonical Rarog header bytes (`MAX_PRIVILEGED_NETWORK_HEADER_BYTES`), and at most 1 MiB of body bytes in any one pending Network request (`MAX_PRIVILEGED_NETWORK_BODY_BYTES`). Incoming canonical headers are re-bound through `HeaderList::iter()` plus `HeaderList::append()` into fixed product limits, so caller-selected `HeaderList` capacities are not part of request identity. Request bodies are retained as exact immutable shared bytes so cloning a request handle does not duplicate the body allocation; absence and a present-but-empty body remain distinct request states. Body compatibility reuses the pinned Rarog `FetchMethod::permits_body()` contract, so present bodies on GET/HEAD are rejected without a second Zorya method table. `RequestDestination::Other(String)` is retained byte-for-byte but capped at 256 bytes (`MAX_PRIVILEGED_NETWORK_DESTINATION_BYTES`) before request-ID allocation, pending-slot use or body-budget consumption. The correlated response limit is a fixed-size scalar but is still bounded: `MAX_PRIVILEGED_NETWORK_RESPONSE_BODY_BYTES` equals the pinned Rarog `DEFAULT_MAX_RESPONSE_BODY_BYTES` (64 MiB at this revision), and zero or over-maximum explicit values fail closed before request-ID allocation, pending-slot use or request-body-budget consumption. Header/body/destination/response-limit overflow, aggregate body-budget overflow and body-accounting arithmetic overflow therefore fail before request-ID allocation or pending-slot consumption.

Raw targets, canonical source-Origin/host/port serialization, header names/values, request-body contents and `RequestDestination::Other` contents are process-local, are not persisted, and are deliberately omitted from the Network handle's `Debug` output. Only target byte count, header count/aggregate header bytes, body-present/body-byte metadata, finite mode/credentials/redirect values, destination kind/custom-destination byte count and the numeric response-body limit may be shown. The canonical HTTP method and response limit are non-secret correlation/budget metadata. The aggregate request-body budget accounts bytes while requests are pending in the tracker; cloned request handles share the same immutable body allocation rather than multiplying retained body bytes per clone.

Clipboard write text is likewise process-local, not persisted and omitted from `Debug`; only `ReadText`/`WriteText` kind, a read-result byte limit or write-text byte count may be shown. The Clipboard aggregate budget accounts only pending write bytes; read requests consume no Clipboard text budget, and cloned write handles share the same immutable text allocation rather than multiplying retained bytes per clone.

Network registration remains correlation only. It accepts but cannot fabricate the Host-minted source; it does not parse a Network target, revalidate source state, synthesize an HTTP `Origin` header, create a Rarog `FetchRequest` or authorize an operation. Deferring target parsing until consumption preserves the fail-closed order established by `preflight_network_target`: a request staged before source replacement is rejected as stale before malformed, unsupported-scheme or cross-Site target classification can run. Binding a canonical method, headers, body, mode, credentials, redirect, destination and bounded response-body limit does not execute the method, interpret header/body semantics beyond Rarog's method/body compatibility contract, enact Fetch mode/credential/redirect/destination behavior, allocate or buffer response data, enforce a response stream, or start a network operation.

`preflight_network_once` and `preflight_clipboard_once` remove the exact stored specialized identity before equality or policy. Removal computes and releases tracker-accounted Network body and Clipboard write-text bytes before variant/equality checks, so exact consumption, same-ID substitution and specialized cross-kind mismatch cannot strand either budget; both accounting subtractions are checked before counters are mutated. Unknown/already-consumed identities are rejected without changing accounting, and internal accounting underflow fails closed rather than saturating silently. Network source/method/header/body/mode/credentials/redirect/destination/response-limit/target substitutions and Clipboard authority/operation/read-limit/write-text substitutions burn the stored slot rather than making it reusable. Exact Network handles then invoke canonical target preflight; exact Clipboard handles invoke only the private Clipboard-specific committed remote-source check and still return stale/unsupported denial. No generic `preflight_once` or source-only privileged handle remains. Consequently copied/cloned handles cannot replay through the same tracker or a replacement tracker, and captured metadata cannot be swapped to redirect later policy identity.

Even a current exact same-site fully envelope-bound Network request and an exact current operation-bound Clipboard request still end at `DeniedUnsupported`. The lifecycle adds no allow result, Rarog broker dependency, capability grant, Host network operation, OS clipboard action, permission decision or IPC authority. Network envelope fields and Clipboard operation/payload bounds are correlation/budget metadata only. Response allocation/buffering/enforcement, Origin-header synthesis, CORS/Fetch execution, Clipboard backend access and streaming/file uploads are not introduced. A later actual brokering path must accept only fully reviewed specialized request envelopes, preserve consume-once semantics and repeat the required live policy/revalidation immediately before any grant/backend operation.

## Explicitly not provided yet

This boundary does not provide or claim:

- production Site execution in a separate OS process;
- Windows Site IPC transport;
- process sandboxing or complete process isolation;
- OS external-protocol dispatch;
- local-file navigation or file chooser policy;
- download mediation;
- permission or clipboard mediation;
- any privileged internal-page action, JavaScript/native bridge, permission decision or capability grant;
- any successful Network capability policy result or grant;
- Network response allocation, buffering or enforcement and any execution/policy semantics for the bound request mode, credentials, redirect, destination or response-body limit;
- HTTP `Origin` header synthesis, CORS enforcement or Fetch execution;
- streaming or file-backed request-body transport;
- a capability-routed subresource pipeline;
- any successful Network or Clipboard capability brokering.

Those remain separate reviewed Z4/Rarog work. Browser policy, authority revalidation, specialized target/source denial checks and one-shot request tracking must not be presented as substitutes for missing process isolation or capability mediation.
