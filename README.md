# Zorya

**A Windows-first browser powered by the Rarog Web Engine.**

Zorya is an experimental desktop browser written in Rust. Its product goals are privacy, security, resource efficiency, and a clean native experience without duplicating the Web engine inside the browser repository.

The first release-quality target is **Windows 10/11**. Zorya uses Rarog for Web-platform semantics and rendering:
https://github.com/StanleyLl0yd/rarog

Zorya is in early bootstrap development. It is not yet a general-purpose or production-ready browser.

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

## Current bootstrap

The repository currently proves one intentionally small integration point: Zorya can initialize the pinned Rarog engine revision. Native browser-window work is the next product slice.

    Zorya product state / browser chrome
                   |
                   v
            Rarog embedder API
                   |
                   v
              Rarog engine
                   |
                   v
         platform / compositor

## Build

Requirements:

- Rust stable;
- Rust 1.85 or newer;
- Windows 10/11 is the primary target.

    cargo check --locked
    cargo test --locked
    cargo run --locked

Linux is kept as a portability compile/test target where practical, even though Windows is the first product platform.

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
