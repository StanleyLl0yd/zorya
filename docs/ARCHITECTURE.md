# Zorya Architecture

## Mission

Zorya is the reference desktop browser for the Rarog Web Engine.

The product is Windows-first and Rust-first. The browser repository owns user-facing browser behavior while Rarog remains an independently embeddable Web engine.

## Primary boundary

The most important architectural rule is that Zorya is not another Web engine layer.

    User
      |
      v
    Zorya browser chrome and product state
      |  windows · tabs · navigation policy · profile · permissions
      v
    Rarog embedder boundary
      |
      v
    Rarog Web engine
      |  DOM · CSS · script · layout · paint · compositor
      v
    Platform services / pixels

Zorya may coordinate Rarog views and apply browser-level policy. It must not become an alternate source of DOM, CSS, layout, paint, origin, or Web compatibility semantics.

If a browser feature requires information or control that Rarog does not expose, extend the supported Rarog embedder/platform contract first.

## Product-owned state

Zorya is authoritative for:

- browser windows and chrome;
- tab identity, ordering, selection and lifecycle;
- navigation UI state and user intent;
- profile selection and product settings;
- history and bookmarks;
- downloads and user-visible transfer state;
- permission prompts and persisted browser decisions;
- session restore;
- browser-level crash/recovery UX;
- updater, packaging and product release state.

These are not engine-derived caches. Their persistence and lifecycle must be explicit.

### Browser product model

`BrowserApp` is the authoritative in-memory owner of browser-window and tab lifecycle. `BrowserWindowId` and `TabId` are monotonically allocated product identities and are never derived from vector positions, native window handles or UI widget identities.

`BrowserWindow` owns tab ordering and active-tab selection. Closing an active tab selects a surviving neighbor when one exists; closing the last tab leaves the window with no active tab. Recreating a tab allocates a new identity, so stale work targeting a closed tab cannot silently attach to a replacement.

Tab reorder is expressed through stable identities rather than persistent indices: `move_tab_before(tab, anchor)` moves one `TabId` before another, while a missing anchor means move to the end. All target identities are validated before mutation. Reordering changes only presentation order; it does not change active-tab identity, navigation ownership or a window address-bar edit already bound to that tab.

The product model is platform-independent. Native Windows identifiers and Rarog `ViewId` values are adapter concerns and must be mapped to product identities rather than becoming product identity themselves.

### Two-phase tab activation

A future native tab switch must not change privileged chrome to tab B while the shared Web-content surface can still present a late frame from tab A. The browser product model therefore represents selection handoff explicitly with monotonically allocated `TabActivationId` values and `TabActivationIntent { from, to }`.

Beginning an activation does not mutate `active_tab` or address-bar edit state. A newer activation returns the superseded intent so platform code can cancel older worker-side preparation. Only the exact current activation may commit; commit then changes active-tab identity. Explicit cancellation returns the cancelled intent as well.

Immediate product selection and tab closure expose any activation they invalidate instead of silently deleting it. Closing the activation source or target invalidates that transition, closing an unrelated tab and reordering tabs preserve it, and stable `TabId` values remain the only identities involved.

Keyboard-style cyclic tab targeting is also identity-based. `begin_tab_cycle(Next|Previous)` follows the current tab order with wraparound but, while an activation is pending, advances from that pending target rather than from the still-committed active tab. Rapid Next input can therefore supersede A→B with A→C without prematurely changing privileged chrome. A single-tab window produces no activation and does not consume an activation identity.

The platform-independent `TabPresentationHandoff` guard models the other side of that identity boundary without pretending to provide native atomicity. It mirrors the tab represented by privileged chrome, whether Web content is still owned by that tab or is explicitly neutral, the exact pending `TabActivationId`, and a monotonically increasing presentation generation. Target Web pixels cannot receive a presentation permit until neutral content has been confirmed and the product's target chrome commit has been acknowledged. Superseding or invalidating an activation revokes older permits by identity/generation while an already-confirmed neutral state remains neutral rather than guessing that source pixels became visible again. This guard is a protocol/state validator; the native platform still has to implement and confirm the actual privileged neutral cover or reviewed neutral surface operation.

This is the browser-model half of the native presentation-safety rule tracked in issue #20. The Windows shell issues a generation-bound current-frame permit at each redraw boundary, and the render worker rejects permits whose stable tab identity or presentation generation no longer matches its current target. A newer authorized target-frame permit can advance the worker only to an already-live Rarog View; that transition discards both the compositor FramePlanner state and the WgpuCompositorBackend retained staging state, clears the remembered viewport and explicitly requests a fresh target frame.

The Windows shell now uses that contract for real keyboard-driven tabs. `Ctrl+T` allocates a product `TabId`, asynchronously creates and initializes its Rarog View on the worker, commits its initial `about:blank` navigation only after View creation succeeds, and then enters the same activation protocol. `Ctrl+Tab` and `Ctrl+Shift+Tab` use the product tab-cycle model and never select by vector index outside that model.

Until native browser chrome owns a dedicated content cover, Windows uses a deliberately conservative neutral fallback: the whole native window is hidden and `Window::is_visible() == Some(false)` must be observed before the handoff marks Web content neutral or commits target chrome identity. The first target frame is allowed to present only while that neutral state remains in force; the window is shown again, and user focus restored when appropriate, only after the current target permit is accepted. Superseded target completions are ignored only while the window is still confirmed hidden and the presentation guard remains neutral. This is less polished than a content-only native cover, but it preserves the required security ordering without synthetic Web display lists, raw wgpu access or UI-thread waiting.

Windows now applies the same ordering to active-tab close. `Ctrl+W` keeps the window hidden while the source tab is removed from the product model, presents the fallback through a generation-bound target permit, and retires the closed source Rarog View only after the fallback frame is accepted. A late source completion after product removal is tolerated only while that exact source is tracked as retired and the native window is still confirmed neutral. The final tab is removed only after current-tab neutral confirmation, then the native window exits.

Rapid A → B → C supersession is now covered by a Windows integration smoke that creates live background B and C Views, verifies B is already the exact pending native frame, then commits C while B remains in flight. C must become the only queued generation-bound target permit; late B acknowledgement is tolerated only while native presentation remains confirmed hidden and neutral, and the smoke succeeds only after C becomes the sole accepted presented target.

A later dedicated chrome cover may replace whole-window hiding for UX quality, but it must preserve the same identity/generation protocol.

### Presentation-aware tab closing

`begin_tab_close` distinguishes close operations that can mutate immediately from those that would change the identity represented by privileged chrome. Closing a background tab completes synchronously because it does not change the committed active tab. Closing the active tab while a neighbor survives starts a normal stable-ID `TabActivationId` transition to the same neighbor that direct product removal would select, but leaves the source tab present and committed until a neutral presentation state is confirmed.

`commit_active_tab_close_after_neutral` accepts only a presentation handoff that still represents the closing source tab, is explicitly neutral and carries the exact pending activation identity. It commits the fallback activation and removes the now-background source tab as one product operation. The platform must then acknowledge target chrome in the presentation handoff before any target Web frame can receive a permit. Closing the final active tab has no fallback activation; it similarly requires a neutral handoff with no pending activation, and the commit is rejected if another tab appears before removal.

`TabPresentationHandoff::confirm_current_tab_neutral` exists for this neutral-only final-tab/window transition. It advances presentation generation and refuses to run while an activation remains unresolved. It records that a browser-owned neutral state has already been made observable; it does not itself create, draw or synthesize that state.

These coordinator APIs make the product-side ordering testable; on Windows the platform now supplies the required observable neutral phase by hiding the native window and verifying `is_visible() == Some(false)` before calling the corresponding presentation confirmation. The coordinator method itself is still never treated as proof that pixels were hidden.

### Browser navigation model

Each `Tab` owns a platform-independent `TabNavigation` product state. Navigation work uses monotonically allocated `NavigationId` values; committed browser-history entries use separate monotonic `HistoryEntryId` values. Ordering stays in vectors, but current/back/forward targets are stable entry identities rather than array indices.

Starting a newer navigation replaces the pending intent and returns the superseded intent to the caller so future engine/transport integration can cancel old work explicitly. Commit and failure transitions accept only the currently pending `NavigationId`; stale completions fail without mutating committed history. Stop removes and returns pending work while leaving the last committed history entry intact.

A new-document commit truncates only the forward portion after the current entry and then appends a newly identified history entry. Reload preserves the current history-entry identity. Back/forward traversal targets an existing stable entry and changes the current entry only on commit, not when the user merely initiates navigation.

Locations stored in this product model are browser display/history strings. They are not canonical URL, origin, site or authorization identities and must never be used for Web security decisions. Rarog remains authoritative for those semantics. General HTTP(S) navigation now crosses the supported Rarog Fetch/navigation boundary: Zorya supplies only the transport capability, while Rarog owns Fetch policy and navigation completion. Zorya does not turn network responses into documents by calling `load_html`.

### Privileged address-bar state

Address-bar editing is window-owned privileged chrome state, not part of the Web document and not a property of a Rarog View. `AddressBarState` binds an active edit to the stable `TabId` that was active when editing began. Switching away from that tab or closing it cancels the edit so unfinished chrome input cannot later target another tab by position.

Outside edit mode, the displayed location follows the active tab's browser navigation state: the currently pending requested location takes precedence over the committed history location. While editing, user text is preserved even if the underlying tab starts or commits navigation; cancelling the edit reveals the current navigation display location.

Submitting the address bar returns an `AddressBarSubmission` containing the stable target `TabId` and the user's verbatim text, then leaves edit mode. Submission deliberately does not parse, normalize or start navigation. Search-vs-URL interpretation, canonical Web URL/origin semantics and HTTP navigation execution must cross reviewed browser/Rarog policy boundaries rather than being hidden inside chrome state.

### Derived tab-strip state

`TabStripSnapshot` is a read-only projection of authoritative `BrowserWindow` state for future privileged native tab chrome. Items preserve current product order and carry their stable `TabId`, committed-active flag, pending-activation-target flag, per-tab loading state and browser display location. A pending target is presentation intent only and is never reported as the committed active tab.

The current location is a temporary browser-owned display fallback, not a security identity or a substitute for document title. Unfinished address-bar edit text is intentionally excluded so private or mistyped privileged input cannot become another tab's label source. Document-title observation remains blocked on the supported Rarog View contract tracked in issue #18; once available, title text must still be treated as untrusted Web-controlled display data.

The snapshot does not commit activation, create Views or authorize Web presentation. Native tab-strip rendering must continue to obey the neutral presentation handoff required by issue #20.

### Rarog engine host

`engine::EngineHost` is the Zorya-owned adapter around the public Rarog embedder API. It owns the shared `rarog_engine::Engine` and maps each product `TabId` to one live Rarog `View` without exposing Rarog identifiers as browser identity.

Every hosted View receives a Zorya-owned monotonically increasing generation. Frame work is represented by `EngineFrameRequest`, which binds the product `TabId`, View generation and Rarog frame request number. A completion is accepted only while all three still identify the currently active request. Closing and recreating a View for the same tab therefore invalidates work from the previous View even when Rarog's per-View request numbering starts again from the same value.

The adapter is responsible for View creation/destruction, deterministic local HTML loading, viewport conversion, Rarog frame-request lifecycle and the embedder side of Rarog navigation transport. Each remote navigation receives a separate Host-owned pending navigation context, context-scoped Network capability and operation. The previously committed context remains authoritative while the target is pending; only a successful Rarog document commit promotes the target and retires the old context. Failure, cancellation, supersession and View retirement close pending authority. Zorya never receives authoritative site-process identity and does not implement Web parsing, layout, paint, Fetch policy, navigation semantics or compositor behavior.

The Z1 start fixture is loaded by Rarog with `BaseUrl::about_blank()`. Zorya therefore represents the startup document with the browser-history display location `about:blank` rather than inventing a privileged or origin-bearing internal URL. The browser model starts that navigation before native initialization, commits it only after the Rarog View has loaded and the initial Web-content surface has attached successfully, records initialization failure against the pending navigation, and stops the pending navigation if the shell closes first. This product-history transition does not create or override Rarog URL/origin semantics.

## Release packaging boundary

Release packaging is Zorya-owned product behavior. A distributable candidate must be derived from the committed Cargo version and locked dependency graph; the packaged executable version must match that source version exactly.

The Windows Technical Preview candidate is a ZIP, not an installer. Candidate generation runs on Windows x86-64, builds Cargo's release profile, executes the release native-window/Rarog/DX12 smoke, generates third-party license material from the resolved Windows dependency graph, records the exact build-checkout commit, source-head commit, Rarog revision and toolchain provenance plus a SHA-256 digest, then re-extracts and re-smokes the packaged executable. Pull-request candidates distinguish GitHub's synthetic merge checkout from the real source head; main-branch candidates record the release commit as both. Candidate creation fails closed on missing version, package, provenance, hash or dependency-license evidence.

Candidate generation is distinct from public publication. Producing a green CI artifact does not itself make a GitHub Release, imply code signing, or change browser-readiness claims. Publication/tagging is a separate release action after the candidate has passed its gates.

## Engine-owned state

Rarog is authoritative for:

- Web document and script semantics;
- URL/origin/site security primitives exposed by its public API;
- style, layout, fragments, display lists and rendering;
- engine scheduling/invalidation semantics;
- Web resource and compatibility behavior;
- compositor and platform contracts exposed to embedders.

Zorya must not infer engine truth by scraping derived pixels or reconstructing hidden engine state.

## Trust boundaries

Treat all Web-controlled input as untrusted, including titles, URLs, suggested filenames, downloads, clipboard payloads, permission requests, external-protocol targets and content-originated UI text.

Browser chrome is privileged. Web content must not be able to impersonate, overlap, mutate, or directly own privileged browser controls.

Future Rarog process/site isolation must remain visible in Zorya architecture. Do not design product state around the assumption that Web content permanently runs in the same process as browser chrome.

## Navigation boundary

Navigation is shared work with distinct ownership:

- Zorya owns user intent, browser UX, tab lifecycle and browser policy;
- Rarog owns URL/origin semantics and Web navigation execution exposed through its embedder API.

Display strings and canonical security identities are different concepts. Never use a user-facing URL string as an authorization or same-origin decision.

External protocols, local files, downloads and privileged internal pages require explicit browser policy.

## Platform boundary

Windows 10/11 is the primary product target.

Windows-only APIs should stay behind narrow Zorya platform modules when they are product-shell concerns. Engine/platform functionality that belongs to reusable Web embedding should live behind Rarog platform contracts instead.

Do not spread Win32, WinRT, shell, registry, credential-manager or installer types through browser-model code.

Linux CI exists to keep non-platform product logic portable and to expose accidental Windows coupling early. It is not a promise of a Linux release.

### Z1 Windows native shell

The Z1 developer shell uses `winit` only as a narrow native-window/event-loop adapter. Winit `WindowId` values stay inside the Windows platform module; browser identity remains `BrowserWindowId`/`TabId`.

The UI thread owns the native event loop, top-level window lifecycle, resize/redraw routing and browser product state. It does not wait for DX12 device initialization or synchronous Rarog rendering. A dedicated bounded render worker owns `EngineHost`, the Rarog View, GPU device lifetime and compositor work. UI-to-worker render requests and worker completions carry a monotonically increasing Zorya request ID together with their target browser window and tab. Completions are ignored unless they still match the currently pending request and live product identities.

Windows native surface creation is intentionally split from GPU-device initialization. In winit 0.30, safe raw-window-handle access is available only on the event-loop thread. The worker therefore requests `WindowsGpuDevice` asynchronously and sends a shared device handle to the event loop; the event-loop thread performs only the thread-affine `WindowsGpuDevice::create_surface` call and transfers the resulting `WindowsGpuSurface` back to the worker. Surface replacement follows the same handshake. Zorya does not use winit's unsafe any-thread window-handle escape hatch and does not weaken `unsafe_code = "forbid"`.

Rendering is event driven. Resize/DPI changes mark the window dirty and request a redraw; only one render request may be in flight. A redraw that arrives while rendering is coalesced into one later redraw rather than starting a busy loop.

Remote navigation uses a separate identity path from presentation requests. Browser-owned `NavigationId` values are carried with the exact window and tab target, while the render worker privately maps them to `EngineNavigationRequest` values. Network I/O runs on the bounded HTTP runtime thread; the render worker performs only nonblocking navigation polls between bounded command waits. Worker completions are accepted by the UI only if the exact product window/tab/navigation is still pending. Stop is two-phase on Windows: the worker first confirms cancellation of that exact engine/Host navigation, and only then is the matching browser-product pending intent removed, preventing a late engine commit from being mistaken for a successful stop.

The developer binary has an explicit native-runtime smoke mode. It follows the same window, worker, GPU, surface, Rarog render and presentation path as the interactive shell, then exits immediately after the first successfully presented frame. Fatal shell failures are retained in `NativeShell` and propagated back through `run` after the event loop exits, so automation receives a non-zero process result instead of mistaking an internally failed event loop for success.

`WebContentSurface` is the platform-owned presentation boundary for untrusted Web pixels. In the first Z1 vertical it occupies the full client area because privileged browser chrome is not rendered yet, but Web content does not own the top-level window or future chrome state. When Rarog reports its public `WindowsGpuError::Surface` category, the worker discards the affected compositor/Rarog frame and requests one replacement native surface from the event-loop thread; after replacement it requests a fresh explicit Rarog frame. Resize/configuration and compositor failures are not treated as surface-loss recovery candidates. The nested backend-specific surface error is intentionally not inspected in Zorya; finer recovery classification and GPU device-loss handling remain blocked on the stable Rarog contract tracked in issue #6. Repeated or non-recoverable failure terminates the developer shell conservatively.

## Async and UI lifecycle

The native UI thread must remain responsive.

Do not perform unbounded file I/O, networking, decoding, database maintenance, engine waits or other potentially blocking work directly in event handlers.

Long-running work needs explicit ownership, cancellation and completion routing. A closed tab/window/profile must not receive stale completions as though it were still current.

Stable product identities should be used for asynchronous work instead of borrowing array positions or transient UI indices.

`async_lifecycle` centralizes Zorya-owned asynchronous identity and pending-request validation. `AsyncRequestSequence` allocates monotonically increasing request identities bound to a specific `BrowserWindowId` and `TabId`; `PendingRequest` permits only one owner for a request slot, rejects overlapping starts, accepts only the exact current completion and supports explicit invalidation on lifecycle teardown.

Platform adapters use this mechanism rather than implementing their own request counters or stale-completion checks. A shared `CancellationToken` separately provides cooperative cancellation for accepted worker work. Closing a window first invalidates pending initialization/render/surface slots, then marks the worker cancelled before closing the bounded command channel. New commands are rejected after cancellation; queued commands are dropped when the worker observes the token. GPU initialization that is already inside the platform request cannot be force-interrupted, but cancellation is checked before the device is published to the event loop. Rarog rendering is checked again before Web-content presentation, so completed engine work is discarded rather than presented after teardown whenever cancellation is observed before presentation begins. The UI thread never joins or waits for the worker.

## Persistent data

Persistent browser data must be versioned and migration-aware before schemas become public.

Writes that affect user data should be atomic or recoverable after interruption. Corruption must fail visibly and conservatively rather than silently discarding unrelated user state.

The first Z3 storage primitive is `ProfileStore`, which is deliberately not wired into the native UI event loop. It owns a caller-selected profile root and a bounded settings-generation directory. Settings records use an explicit schema version, stable generation number, strict bounded key/value/count limits and a checksum for corruption detection only. The checksum is not an authenticity or secret-protection mechanism.

Settings are never updated by truncating the current generation. A save first verifies that the caller still owns the current valid generation and reserves bounded directory capacity. It writes the complete record to a unique create-new pending file, synchronizes that file, then publishes the immutable final generation with a no-overwrite hard link. A second writer racing for the same generation therefore cannot replace the winner, and a partially written pending file is never considered a committed generation. An interrupted write leaves the previous valid final record intact. Loading scans a bounded number of total directory entries and final generation files newest-first; unpublished pending files are ignored, while corrupt final generations may be skipped only with an explicit `SettingsRecovery` result. A valid record with a newer unsupported schema fails closed before current-format checksum/size decoding, so an older browser cannot fall back and overwrite newer settings. Successful saves retain a bounded generation window and surface post-publication cleanup failures separately from durability of the new record.

This is a synchronous storage primitive for background/profile workers and tests, not permission to perform filesystem I/O from window/input callbacks.

`ProfileRuntime` is the portable product-side coordinator for runtime profile selection. A selection receives a monotonic `ProfileSelectionId`; beginning a newer selection supersedes only the pending intent and leaves the committed profile authoritative. `PreparedProfile::load` opens and decodes the profile synchronously and therefore belongs on a background/profile worker, not the native UI thread. Only a prepared result carrying the exact current selection identity and target may commit. Successful commit allocates a fresh process-local `ProfileId`; filesystem paths remain profile locators and are never used as browser profile identity. Stale selections and mutations addressed to a replaced `ProfileId` are rejected.

`ProductSettings` is a typed view over the committed raw `SettingsSnapshot`. Typed mutations preserve the snapshot generation so `ProfileStore` stale-writer protection remains authoritative. Defaults do not materialize new raw keys merely by loading a profile, unknown keys within the supported storage schema are preserved across typed edits, and malformed values for known typed keys fail visibly instead of silently resetting unrelated settings. The first typed keys are browser-chrome color-scheme preference and confirmation before closing multiple tabs; consuming those preferences in native UX is separate from persistence semantics.

Persistent browsing history is a separate profile-wide visit log, not the per-tab back/forward model in `TabNavigation`. `BrowsingHistoryVisitId` is persisted and monotonic across generations; it is unrelated to session-scoped `HistoryEntryId`. Each visit stores only bounded browser display-location text plus caller-supplied visit-time metadata. Neither field is an engine origin/site identity or a security decision. The history store follows the same immutable-generation durability model as settings: bounded discovery, strict schema decoding, create-new pending writes, file synchronization, no-overwrite publication, stale/concurrent writer rejection, explicit corruption fallback, fail-closed newer schemas and bounded retained generations. The in-memory visit count is bounded; recording beyond the bound evicts the oldest visit and returns that eviction explicitly to the caller. History I/O is synchronous and belongs on a background/profile worker; runtime recording, batching, search/UI and private-browsing policy are separate product work.

`PreparedProfile::load` opens both recoverable settings and browsing-history state on the background/profile-worker side. A successful exact profile-selection commit transfers the history snapshot and any recovery report into `ActiveProfile`; runtime history access is gated by the same process-local stable `ProfileId` used for settings so a replaced profile cannot be mutated through a stale identity.

History persistence uses a separate one-in-flight runtime lifecycle. `ProfileHistorySaveId` binds a cloned save snapshot to the exact active `ProfileId` and base generation before synchronous storage work is handed to a background/profile worker. Visits may continue to mutate the active in-memory snapshot while that save runs. An exact successful completion advances only the active snapshot's durable generation, preserving those newer mutations for the next save; a failed or concurrent save leaves the in-memory generation unchanged. Profile replacement invalidates pending save ownership, and a late completion cannot reattach to the replacement profile. The storage call remains inside explicit worker-side `ProfileHistorySaveIntent::execute`; begin/completion coordination itself performs no filesystem I/O.

A successful top-level `BrowserApp` navigation commit returns a `BrowserNavigationCommit` carrying the exact product window, tab and navigation identity together with the committed per-tab history entry, intent kind and final browser display location. Stale, failed or stopped navigation work cannot produce this value. `ProfileRuntime::record_committed_navigation` accepts that committed value only for the exact active `ProfileId` and appends one bounded in-memory visit with caller-supplied visit-time metadata. This recording path performs no filesystem I/O; save timing, batching and worker dispatch remain separate so a native navigation-completion callback never needs to synchronously persist history.

The active profile also tracks a process-local monotonic browsing-history mutation revision separately from the durable storage generation. Loaded history starts clean. Successful typed recording advances the revision only after mutation succeeds, while direct mutable history access conservatively advances it before exposing the mutable snapshot so untracked writes cannot appear clean. A save intent captures the exact revision represented by its cloned snapshot; only an exact successful completion marks that captured revision durable. Mutations made while the save is in flight therefore remain dirty, and failed saves leave all unsaved mutations dirty. Dirty-state observation and `begin_browsing_history_save_if_dirty` perform no filesystem work.

`ProfileHistorySaveScheduler` adds deterministic coalescing over that dirty state without owning a clock, timer, thread or storage call. Callers provide monotonic milliseconds and a validated policy containing a debounce, maximum dirty age and mutation threshold. Normal polling saves after quiescence, the mutation threshold or the maximum dirty age; explicit flush urgency saves dirty history immediately. While a save is in flight newer mutations are observed but no overlapping intent is created. A successful captured save therefore leaves newer mutations eligible for the next scheduling window. Clock regression is treated conservatively as an immediate dirty save, and exact `ProfileId` validation occurs before scheduler ownership changes so stale profile events cannot retarget replacement state. The scheduler only returns `ProfileHistorySaveIntent`.

`ProfileWorker` is the dedicated bounded storage-I/O boundary for profile work. Caller submission uses a one-slot synchronous queue through non-blocking `try_send`; queue-full and disconnected errors return the exact unsubmitted intent to preserve one-shot ownership. The named `zorya-profile` thread consumes selection intents with `PreparedProfile::load` and history-save intents with `ProfileHistorySaveIntent::execute`, then invokes a caller-provided completion handler with the exact typed selection/save identity. This lets the Windows layer later forward completions through its event-loop proxy without polling or performing storage work on the event-loop thread. Dropping the worker drops its sender and `JoinHandle`; Rust's detached-handle drop semantics mean it does not synchronously join an in-progress filesystem operation.

Native profile-selection UX, profile locking, default native save timings, Windows profile/history event-loop wiring, private-browsing history policy, bookmarks/session stores, broader settings wiring and migrations beyond schema v1 remain separate Z3 slices.

Secrets and authentication material must not be stored in plaintext configuration files. Windows credential storage or another reviewed secret-storage boundary should be used when such features are introduced.

## Dependency on Rarog

Rarog is currently consumed as a Git dependency pinned to an exact commit.

The pin is intentional:

- builds are reproducible;
- Zorya does not silently inherit breaking engine changes;
- an engine upgrade is reviewable as its own change.

When Rarog exposes a stable published/versioned embedder package, the dependency strategy may be revisited through an architectural change.

## Safety

Repository-owned Rust forbids unsafe by default.

If native integration eventually requires unsafe code, isolate it behind the narrowest reviewed platform boundary. Do not weaken the repository-wide safety posture simply to make a dependency or convenience API compile.
