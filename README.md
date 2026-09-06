# Zorya

**A Windows-first browser powered by the Rarog Web Engine.**

Zorya is an experimental desktop browser written in Rust. Its product goals are privacy, security, resource efficiency, and a clean native experience without duplicating the Web engine inside the browser repository.

The first release-quality target is **Windows 10/11**. Zorya uses Rarog for Web-platform semantics and rendering:
https://github.com/StanleyLl0yd/rarog

Zorya has a verified Z1 native-shell vertical and is now building the Z2 browser-product model. It is not yet a general-purpose or production-ready browser.

## Repository responsibility

Zorya owns browser-product behavior:

- native browser windows and browser chrome;
- tabs, navigation UX and session lifecycle;
- history, bookmarks, settings and profiles;
- downloads and user-facing permission decisions;
- browser-level security policy and OS integration;
- crash/recovery UX, updates and packaging.

Rarog owns Web-engine behavior:

- HTML, DOM, CSS and layout semantics;
- script/Web-platform behavior;
- display-list, paint, compositor and rendering semantics;
- origin/site identity and engine security primitives;
- portable engine and embedder contracts.

If Zorya needs an engine capability that is not available through a supported Rarog boundary, extend the Rarog embedder API rather than reproduce or reach around engine internals here.

## Current development state

The current developer build has a real Windows native window, Zorya-owned browser state, one natively presented Rarog View, deterministic local HTML loading, off-UI rendering and DX12 presentation through Rarog's public platform/compositor boundary. Windows CI exercises this full vertical with a real native-window/GPU/Rarog presentation smoke test.

The platform-independent Z2 product model already includes stable window/tab identities, tab create/close/select/reorder state, monotonic navigation and history identities, stale-navigation rejection, committed `about:blank` startup history, back/forward/reload/stop state, and privileged address-bar edit/display state bound to stable `TabId` values.

    Zorya browser state + native window
                    |
                    v
          bounded render worker
                    |
                    v
           Zorya EngineHost
                    |
                    v
             Rarog View
                    |
                    v
      Rarog compositor / DX12 surface

The current Windows presentation shell still presents only one active Web View and does not yet render browser chrome or general multi-tab UX. General HTTP(S) navigation is also intentionally not implemented until Rarog exposes the supported navigation-completion/Fetch integration. Native Web input, page-title observation and presentation-safe native tab activation likewise remain explicit integration/design work. Web content does not own the top-level window or privileged browser state.

Current tracked boundaries include:

- issue #6 — stable Rarog GPU device-loss recovery contract;
- issue #13 — Rarog navigation completion / Fetch integration;
- issue #16 — supported Rarog View input dispatch;
- issue #18 — Rarog document-title observation;
- issue #20 — presentation-safe native tab activation in Zorya.

## Build

Requirements:

- Rust stable;
- Rust 1.85 or newer;
- Windows 10/11 is the primary target.

    cargo check --locked
    cargo test --locked
    cargo build --locked --bin zorya
    cargo run --locked
    cargo run --locked -- --version
    cargo run --locked -- --native-smoke

The `--version` path exits before native window or GPU initialization. On Windows, `--native-smoke` runs the real native-window, Rarog render and DX12 presentation path and exits after the first successful presentation. Windows CI executes both smoke paths before retaining the debug executable as an artifact named `zorya-windows-dev-<commit SHA>` for 14 days. Linux is kept as a portability compile/test target where practical, even though Windows is the first product platform.

## Rarog dependency

Zorya pins Rarog to a specific Git commit in Cargo.toml for reproducible builds. Engine updates are intentional dependency changes: update the revision, review the upstream changes, regenerate Cargo.lock, and run the full Zorya verification matrix.

See docs/DEPENDENCIES.md.

## Project documents

- docs/ARCHITECTURE.md — ownership and trust boundaries;
- docs/ROADMAP.md — product milestones;
- AGENTS.md — mandatory rules for automated coding agents;
- CONTRIBUTING.md — contribution workflow;
- SECURITY.md — vulnerability reporting guidance.

## License

Apache License 2.0.
