# Z4 Clipboard Permission Integration - Contract Proposal

**Status:** Design only. No native prompt, Web Clipboard permission event, successful grant, or OS Clipboard route is implemented by this document.

**Tracking:** [Zorya #202](https://github.com/StanleyLl0yd/zorya/issues/202) depends on [Rarog #352](https://github.com/StanleyLl0yd/rarog/issues/352). Baseline Zorya `main` at design start: `64f3892d54bb743c566c30880182501cf67b3e01`; exact Rarog pin: `331689e7dff02a556d0b96530178def821056e20`. This proposal does not change either dependency revision or the Rarog R4/R5 milestone state.

## Missing trust boundary

At the pinned Rarog revision, `rarog-engine/src/embedder.rs` publishes navigation, resource, frame, and document events, but no authenticated Web Clipboard permission-request event. The separate `rarog-host` context-scoped Clipboard capability checks and `PlatformClipboardService` backend do **not** prove that an actual Web operation requested permission. Nor do Zorya's exact Host-minted committed source, one-shot Clipboard tracker, trusted Origin display, or deny-only `resolve_once` prove Web provenance.

**Fail closed until the reviewed Rarog contract exists.** Do not construct an `AllowOnce` grant from a browser button, a tab URL, a renderer-supplied Origin string, JavaScript/native bridge, generic resource event, or a caller-selected `EngineCommittedDocumentSource`. Browser-product tests can simulate lifecycle outcomes, but simulated events do not turn into real Web requests.

## Ownership and message sequence

1. **Rarog Web implementation** validates the actual Clipboard API operation under its own Web semantics, including initiator/frame Origin and any necessary user activation. Unsupported APIs do not emit a successful permission request. An event can originate only from this reviewed path.
2. **Rarog embedder/Host contract** binds the operation to exact live engine View and navigation context, an engine-owned one-shot non-reused request identity, canonical initiator Origin and immutable bounded operation envelope. A document in a subframe cannot borrow the top-level Origin; where frame/initiator binding cannot be proved, deny.
3. **Zorya engine worker** correlates the authenticated event with the live Host context and its stable product tab/View generation and current profile/window identity. It revalidates the exact Host-minted source including canonical Origin, not a UI string. An event lacking exact correlation is denied before UI enqueue.
4. **Bounded asynchronous handoff** transfers *only* a request correlation token and safe UI metadata to privileged Windows chrome. The engine worker retains the non-cloneable exact request, source and operation behind an owner-controlled slot; it never forwards Clipboard payloads, Host capabilities or engine handles to native UI.
5. **Native chrome** displays only `trusted_origin_for_prompt(&EngineHost)` output delivered after a fresh worker-side validation, the operation kind and safe byte-limit metadata. Chrome is privileged and outside Web-painted pixels. It is not a Web DOM dialog; text is rendered as data and never interpreted as markup or an authorization token.
6. **User response** is returned as a typed choice associated with the exact prompt identity and still-current window/tab/generation. The engine worker atomically consumes the pending request, repeats exact Host/source/initiator/lifecycle validation, and issues one terminal Rarog acknowledgment. A stale or duplicate response cannot grant.
7. **Separate, later reviewed capability path** may interpret a valid AllowOnce through the Rarog context-scoped `Clipboard` capability, existing backend limits and revoke/cleanup. Until that path is reviewed and merged, Zorya's `AllowOnce` response remains `Unsupported` and no backend call occurs.

Rarog must define event types, initiator Origin semantics, operation identity, cancellation/acknowledgment and the Web API continuation. Zorya must not invent those semantics in a local adapter. Neither Network/Fetch capabilities nor `EngineNavigationContextToken::get()` may substitute for the exact retained Rarog `NavigationContextId`.

## Proposed bounded native lifecycle (not current behavior)

| Rule | Proposed Zorya implementation |
| --- | --- |
| Visible prompts | At most **one active** native Clipboard prompt per browser window |
| Pending UI capacity | At most **eight** queued/visible prompts for one window, also constrained by the existing tracker request/write budgets |
| Prompt lifetime | **30 seconds** maximum from authenticated event receipt; if the engine has an earlier deadline, use that |
| Default disposition | Deny on deadline, queue full, unsupported operation, event loss, worker error, missing Host, invalid Origin, UI failure or transport disconnect |
| Visibility | Only the requesting active tab may show a prompt. Backgrounding, tab switch or profile/window replacement denies rather than rebinding the prompt |
| Ownership | The engine worker retains the unique non-cloneable permission handoff; chrome has only a bounded opaque correlation ID |
| Budget | Existing Zorya Clipboard 1 MiB per operation, default 16 MiB shared pending+handoff write budget and tracker slot accounting remain authoritative |
| Privacy | Never log or display Clipboard text, canonical Origin except directly in trusted UI, raw capability IDs, sensitive errors or request payloads |

Timeout is a **real monotonic deadline** with cancellation and release; it is not a polling loop or a best-effort UI timer that leaves an engine promise suspended. The engine worker must own terminal disposal even when the UI process or window disappears. A dropped handoff already releases the tracker lease; no stored request may be deliberately forgotten. All cancel/denial paths must notify Rarog exactly once if Rarog has a pending API operation.

Any shell/worker queue must be bounded independently from the existing tracker limit. Backpressure denies an incoming request rather than holding an unaccounted payload. Prompt supersession, activation changes, navigation (including same-tab document replacement), View close/recreation, Host restart, Site-process loss, profile replacement and shutdown invalidate both display and eventual response. A late UI click is rejected even if the prompt text still looks identical.

## Capability execution: independent security review

A successful grant is **not part of the bridge proposal**. Before enabling it, require:

- An exact engine-originated, context/initiator-bound one-shot Web operation and fresh source revalidation **immediately before** Host grant; stale response must fail before any privileged backend effect.
- Rarog `grant_navigation_context_clipboard_capability` using the original `NavigationContextId`, never a reconstructed numeric token, with correct `CapabilityClass::Clipboard`; no raw `CapabilityId` or process ID exported.
- A single prepared operation whose type, read maximum or immutable write text is bound to the consumed handoff; reject replacement or replay.
- Concrete `ClipboardLimits` and `ClipboardText::try_new` read/write enforcement, including the stricter Zorya 1 MiB UTF-8 limit. No backend calls on invalid limits or denied request.
- Revoke and release on success, failure, timeout, cancellation and process loss, including backend errors; no persistent Allow and no cross-navigation reusable grant.
- Async OS backend integration that cannot block the native UI event handler; no bypass of Rarog Fetch, Origin or Site isolation boundaries.

## Required verification matrix

| Case | Required result |
| --- | --- |
| Genuine supported Web Clipboard request | Exactly one engine-authenticated operation-bound event; no direct OS action |
| Renderer-selected Origin/title/URL or spoofed event | No accepted event or privileged prompt |
| Wrong Host, View, TabId, View generation, context, initiator/frame, Site process or profile | Denial, no backend calls |
| Navigation, cross-origin/same-origin replacement, tab close/recreate, profile reset, Host restart | Pending prompt canceled, old response stale, quotas released |
| Duplicate click, replay, same-ID operation substitution | At most one consumed terminal acknowledgment; no backend call on rejected response |
| Prompt queue full, timeout, UI/worker disconnect and absent native window | Denial and bounded cleanup without indefinitely pending Web promise |
| User Deny / AllowOnce before separate capability review | `UserDenied` / `Unsupported` (or `StaleAuthority`); **zero backend calls** |
| After separately reviewed capability integration: wrong class, revoked or foreign capability | Host rejects before any Clipboard backend read/write |
| Read/write limits including UTF-8 multibyte content and invalid backend limits | No operation outside Rarog and Zorya byte bounds |
| Windows native event/response integration | Verify trusted chrome provenance, tab-switch/close race, UI focus and no Web overlay impersonation |

Use deterministic portable fixture tests for engine event identity, terminal acknowledgment and queue/accounting. Add Windows native fixture/smoke tests for shell rendering, input dispatch and worker handoff. After implementation: exact-head `cargo fmt`, check, clippy and tests (including MSRV and Windows); full CI/Verify, Security/RustSec, Dependency Review, CodeQL and Release Candidate; guarded squash merge; repeat exact-main verification.

## Staged implementation and stop conditions

1. Review and implement **Rarog #352** as a narrow Rarog embedder contract, with engine/Host-originated tests. This is not permission to start R5 or silently move the pin.
2. Review upstream code and tests, update Zorya's exact Rarog revision and lockfile in its **own** focused change, and run the full required verification.
3. Implement Zorya worker-to-native prompt dispatch with fail-closed bounded lifecycle, using the real event. Do not connect AllowOnce to Clipboard yet; test user denial, stale response and cancellation.
4. Independently review context-scoped grant/backend/revocation and enable successful AllowOnce only after all negative tests and Windows integration security gates pass.

**Current terminal:** `EngineClipboardPermissionRequest::resolve_once` always denies or reports unsupported/stale; no user-facing Clipboard prompt is wired and no permission is granted. [Zorya #202](https://github.com/StanleyLl0yd/zorya/issues/202) stays open until the real native path is integrated and verified.