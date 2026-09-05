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
- viewport resize, DPI redraw routing and DX12 presentation;
- deterministic local start document as the first navigation entry point;
- bounded native surface recreation only for Rarog-reported surface acquisition failure;
- clear separation between the privileged shell and Web content surface;
- Windows developer executable built and retained by CI;
- full GPU device-loss recovery is blocked on a stable Rarog recovery contract tracked in issue #6.

The existing rarog-window binary remains a small engine reference host. Product UX belongs here.

## Z2 — Navigation and Tabs

- multi-tab create/close/select/reorder UX building on the stable Z1 identities;
- address bar and navigation state;
- back/forward/reload/stop;
- page title and loading state;
- keyboard-first tab/navigation controls;
- cancellation of stale navigation work.

## Z3 — Browser Profile

- settings;
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
