# Zorya Roadmap

The roadmap tracks browser-product work. Web-engine milestones remain in the Rarog repository.

## Z0 — Bootstrap

Goal: establish a clean product repository and a reproducible Rarog dependency.

- Rust application skeleton;
- exact Rarog Git revision pin;
- Windows-primary, Linux-portability and Rust 1.85 CI;
- architecture, contribution, security and agent rules;
- no duplicated Web-engine implementation.

## Z1 — Native Shell

Goal: first useful developer browser host.

- platform-independent browser model with stable window and tab identities;
- one browser window with one active tab;
- Windows native application lifecycle and top-level window;
- one Rarog View behind a TabId-bound engine-host adapter;
- stale-safe async lifecycle with monotonic targeted request IDs, explicit invalidation, tested pending ownership and cooperative worker cancellation before presentation;
- stale-safe Rarog frame request lifecycle;
- native event-loop integration with bounded off-UI render work;
- viewport resize, DPI redraw routing and DX12 presentation with event-loop-thread native surface creation;
- deterministic local `about:blank` start document tracked as a pending browser navigation and committed into history only after successful native/Rarog initialization;
- bounded native surface recreation only for Rarog-reported surface acquisition failure;
- clear separation between the privileged shell and Web content surface;
- Windows developer executable built, headless-smoked, full native-window/Rarog/DX12-smoked and retained by CI;
- full GPU device-loss recovery is blocked on a stable Rarog recovery contract tracked in issue #6.

The existing rarog-window binary remains a small engine reference host. Product UX belongs here.

## Z2 — Navigation and Tabs

- platform-independent per-tab navigation intent and browser-history model with stable navigation/history identities;
- supersession, stop and stale-completion rejection for browser-owned navigation work;
- stable-ID multi-tab create/close/reorder product model plus two-phase TabActivationId selection lifecycle, an identity/generation-bound neutral presentation handoff guard and presentation-aware active-tab close coordination; Windows redraws are generation-gated, native tab creation owns one Rarog View per stable TabId, keyboard activation and Ctrl+W close use a confirmed hidden-window neutral fallback, closed source Views are retired only after fallback presentation, and rapid A → B → C supersession is integration-tested with B already in flight and only C allowed to become the accepted target;
- window-owned address bar edit/display model with stable TabId-bound raw submissions;
- back/forward/reload/stop are wired from the Windows shell into exact tab/navigation worker targets; Stop uses an acknowledged exact Rarog cancellation before removing product loading state, and general HTTP(S) navigation uses the pinned Rarog Fetch/View contract with Host-owned context-scoped network authority and deterministic localhost commit/presentation coverage;
- derived tab-strip chrome projects stable tab order, committed selection, pending activation target, per-tab loading and current browser display location without treating address-bar edit text as tab metadata; page-title observation remains blocked on a supported Rarog View title contract tracked in issue #18;
- keyboard-first browser commands are modeled and atomically dispatched against committed active-tab identity, including new-tab creation, presentation-aware close, pending-aware tab cycling and navigation controls; Windows maps Ctrl+T, Ctrl+W, Ctrl+Tab/Ctrl+Shift+Tab, Alt+Left/Alt+Right and Ctrl+R to browser-owned commands, with Escape requesting exact Stop while loading; Web-content input dispatch remains separate and blocked on issue #16;
- loading-state chrome is projected through a derived committed-tab snapshot with pending-navigation Reload/Stop state; native chrome rendering remains separate.

## Z3 — Browser Profile

- versioned, bounded and recoverable profile-settings storage foundation with monotonic generation records, corruption fallback reporting and fail-closed newer-schema handling;
- stable runtime profile identity and two-phase stale-safe profile selection, with profile preparation kept off the UI thread; typed product settings preserve unknown same-schema entries and storage generations while rejecting malformed known values, and carry a one-in-flight stale-safe persistence lifecycle with monotonic dirty revisions, exact save identity, newer-mutation preservation and failure-safe dirty state; settings save intents execute only through the bounded `ProfileWorker`, while a clock-injected scheduler and Windows 1 s debounce / 10 s maximum dirty age / 8-mutation policy provide event-driven nonblocking persistence, exact cancellation/retry on queue failure and coordinated graceful shutdown flush with history; Windows now consumes the active profile System/Light/Dark preference through the native winit window-theme boundary and Ctrl+Shift+L cycles that typed value through the same stale-safe persistence lifecycle;
- profile preparation now acquires an exact profile-root ownership lock before persisted identity/display-metadata/settings/history reads; immutable versioned `ProfileStorageId` records are distinct from process-local `ProfileId` and paths, bounded versioned display metadata is identity-bound and generation-safe, legacy `Profiles/Default` receives missing identity/metadata only during locked worker preparation, catalog discovery rejects symlinks, malformed/unsupported/duplicate identity state and mismatched metadata, create work initializes a storage-id-derived root while unpublished and exposes it only through an atomic ready marker after explicit lock release, and inactive rename publishes only the next exact metadata generation without moving the root; all catalog discover/create/rename work runs through the same bounded worker with exact intent preservation; held or malformed locks fail closed, settings/history writers require and recheck that exact root owner before publication, lock tokens survive committed runtime and in-flight saves without filesystem work in destructors, abandoned-owner recovery is explicit and exact-owner guarded with no timeout-based stealing, graceful native shutdown releases ownership only on the bounded profile worker, and replacement cannot commit while the old profile has dirty or in-flight persistence work; the native replacement lifecycle retains the prepared target through flush, returns the clean replaced profile's exact lock to the worker, waits for exact release completion and drains interrupted replacement ownership during shutdown; `Ctrl+Shift+M` now performs bounded worker-side catalog discovery and deterministic exact-identity profile cycling with queue-full retry, hides the native window before commit, resets the browser window to a fresh monotonic tab/session, resets the background Rarog `EngineHost` without overlapping GPU workers, commits only fresh `about:blank`, and labels the window with the new display name only after reset; native navigation work is bound to its exact starting `ProfileId`, replacement waits for browser/native transition quiescence before commit, and fresh browser commands are suppressed while a profile transition is pending; profile create/rename/delete UX, multi-tab close-confirmation consumption and broader settings UX remain;
- bounded, versioned and recoverable profile-wide browsing-history generations with stable persisted visit identities, explicit corruption fallback, stale-writer rejection and bounded retention; two-phase profile preparation loads the recoverable snapshot under stable runtime profile identity, a one-in-flight stale-safe persistence lifecycle advances durable generations without dropping newer in-memory mutations, exact successful browser-navigation commits can be recorded as bounded in-memory visits against only the active profile, process-local mutation revisions expose stale-safe dirty state without redundant clean saves, a clock-injected bounded scheduler coalesces saves by debounce/max-age/mutation threshold plus explicit flush and exposes its next observed deadline for event-driven wakeups, exact pending save ownership can be cancelled without clearing dirty mutations when worker submission fails, and a dedicated one-slot background profile worker executes preparation/save I/O while returning exact one-shot completions; the Windows shell now uses an `Instant` clock with a 5 s debounce / 30 s maximum dirty age / 32-mutation policy, drives deadlines only through `about_to_wait` + `ControlFlow::WaitUntil`, submits save intents non-blockingly through `ProfileWorker::save_history`, cancels exact runtime ownership on submit failure, and performs graceful shutdown by asynchronously flushing until the exact completion leaves no newer dirty mutations; profile/history filesystem I/O remains off event-loop and navigation callbacks, while private-browsing policy and history UX remain separate;
- bounded, versioned and recoverable profile-local bookmarks storage with stable monotonic bookmark identities, bounded title/location records, exact profile-lock verification before write and publication, immutable no-clobber generations, stale/concurrent writer rejection, explicit corruption fallback, fail-closed unsupported schemas and bounded retained generations; locked worker-side profile preparation loads the exact bookmark snapshot plus recovery report into the committed `ActiveProfile`; an exact-profile one-in-flight persistence lifecycle now tracks monotonic mutation/durable revisions, captured save generations, newer-mutation preservation and failure-safe dirty state, blocks profile replacement until bookmark state is durable, and executes saves only through the bounded `ProfileWorker` with exact queue-failure ownership; scheduler/native bookmarks UX remain;
- downloads;
- session restore;
- permission decisions;
- profile storage layout and migrations;
- privacy controls.

## Z4 — Security and Process Integration

Tracks Rarog process/isolation maturity rather than replacing it.

- host/site process integration;
- crash recovery;
- capability brokering;
- download/file/external-protocol hardening;
- privileged internal-page boundary;
- permission mediation;
- Windows hardening.

## Z5 — Alpha Readiness

- accessible browser chrome;
- high-DPI and multi-monitor behavior;
- installer/update path;
- diagnostics and crash UX;
- release signing pipeline;
- privacy/security review;
- real-machine Windows 10/11 testing;
- documented known limitations.

Public readiness is determined by engine compatibility, browser security, data integrity and product reliability—not by milestone count alone.