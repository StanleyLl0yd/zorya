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
- stable runtime profile identity and two-phase stale-safe profile selection, with profile preparation kept off the UI thread; typed product settings preserve unknown same-schema entries and storage generations while rejecting malformed known values;
- native profile selection UX, profile locking and broader settings wiring;
- history;
- bookmarks;
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
