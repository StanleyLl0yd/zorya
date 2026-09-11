<p align="center">
  <img src="assets/branding/zorya-icon.png" alt="Zorya" width="192" height="192">
</p>

<h1 align="center">Zorya</h1>

<p align="center">
  <strong>Windows-first browser powered by the Rarog Web Engine.</strong>
</p>

<p align="center">
  <code>#07103B</code> midnight · <code>#6F4CFF</code> violet · <code>#FF9B35</code> sunrise
</p>

Zorya is an experimental desktop browser written in Rust. Its product goals are privacy, security, resource efficiency, and a clean native experience without duplicating the Web engine inside the browser repository.

The first release-quality target is **Windows 10/11**. Zorya uses Rarog for Web-platform semantics and rendering:
https://github.com/StanleyLl0yd/rarog

The repository has implemented and verified the Z0 bootstrap, Z1 native-shell, Z2 navigation/tabs and Z3 browser-profile milestones. Z4 feature work is not part of the current post-Z3 state. Zorya is still experimental and is not yet a general-purpose or production-ready browser.

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

The current developer build has a real Windows native window, a Zorya-owned multi-tab browser model, one Rarog View per materialized tab, deterministic `about:blank` startup for a fresh profile, real HTTP(S) document navigation through Rarog's Fetch/navigation contract, off-UI rendering and DX12 presentation through Rarog's public platform/compositor boundary. Native tab activation, close and rapid-supersession paths are generation/identity guarded and covered by Windows smokes.

Z3 adds a bounded profile I/O worker and lock-owned, versioned persistence for typed settings, browsing history, bookmarks and session restore. Profile selection/replacement is stale-safe; storage writes are generation-safe and scheduler-driven; graceful shutdown coordinates dirty state without moving filesystem I/O onto the event loop. Windows consumes the persisted color-scheme setting, supports profile cycling, toggles bookmarks for the exact committed page, and restores persisted windows/tabs into fresh process-local browser identities while preserving stable persisted session identities.

    Zorya browser/profile state + native window
                       |
                       v
              bounded workers
              /             \
             v               v
      profile persistence   EngineHost
                              |
                              v
                         Rarog View(s)
                              |
                              v
                 Rarog compositor / DX12

The native shell still does not render the full browser chrome/tab-strip/address-bar UX. Browser accelerators and browser-owned navigation commands exist, but Web-content keyboard/pointer/IME input remains blocked on a supported Rarog View input contract. Page-title observation also remains blocked on a supported Rarog contract, so bookmarks currently persist an empty title. Downloads, broader bookmark/profile/settings UX, installer/updater and full GPU device-loss recovery remain future work.

Current tracked upstream boundaries include:

- issue #6 — stable Rarog GPU device-loss recovery contract;
- issue #16 — supported Rarog View input dispatch;
- issue #18 — Rarog document-title observation.

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
    cargo run --locked -- --native-http-navigation-smoke
    cargo run --locked -- --native-profile-cycle-smoke
    cargo run --locked -- --native-session-restore-persistence-smoke

The `--version` path exits before native window or GPU initialization. On Windows, the native smokes exercise the real native-window/Rarog/DX12 path plus targeted navigation, tab lifecycle and profile-persistence scenarios. Windows CI runs the complete smoke matrix before retaining the debug executable as an artifact named `zorya-windows-dev-<commit SHA>` for 14 days. Linux remains a portability compile/test target where practical, even though Windows is the first product platform.

## Technical Preview release

`Zorya 0.1.0 Technical Preview` is published as a Windows 10/11 x86-64 GitHub prerelease. It is intentionally not described as a general-purpose or production-ready browser.

Release: https://github.com/StanleyLl0yd/zorya/releases/tag/v0.1.0

The release-candidate workflow builds `target/release/zorya.exe` from the locked dependency graph, verifies the exact Cargo/binary version, runs the release-mode native smoke matrix, generates a dependency license bundle, packages the executable with preview notes and licenses, records a SHA-256 digest, extracts the resulting ZIP, and repeats packaged verification before retaining the candidate artifact.

The published Technical Preview remains unsigned and has no installer or updater. The repository has advanced substantially since the original preview release; the package notes shipped by current release-candidate builds describe the current executable rather than preserving old pre-Z3 limitations. The historical release notes under `docs/releases/0.1.0.md` remain historical provenance for the published prerelease.

See `README-TECHNICAL-PREVIEW.md` for the package-specific current scope and known limitations.

## Rarog dependency

Zorya pins every Rarog crate to the same specific Git commit in `Cargo.toml` for reproducible builds. Engine updates are intentional dependency changes: update the revision, review the upstream changes, regenerate `Cargo.lock`, and run the full Zorya verification matrix including dependency/security gates.

See `docs/DEPENDENCIES.md`.

## Project documents

- `docs/ARCHITECTURE.md` — ownership and trust boundaries;
- `docs/ROADMAP.md` — product milestones;
- `docs/RELEASING.md` — release-candidate and publication gates;
- `docs/BRANDING.md` — project icon, palette and visual-language rules;
- `AGENTS.md` — mandatory rules for automated coding agents;
- `CONTRIBUTING.md` — contribution workflow;
- `SECURITY.md` — vulnerability reporting guidance.

## License

Apache License 2.0.
