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
- Windows native application lifecycle;
- one Rarog View behind a TabId-bound engine-host adapter;
- stale-safe frame request lifecycle;
- native event-loop integration;
- viewport resize and presentation;
- minimal navigation entry point;
- surface/device recovery;
- clear separation between browser chrome and Web content.

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
