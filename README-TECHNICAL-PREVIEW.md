<p align="center">
  <img src="assets/branding/zorya-icon.png" alt="Zorya" width="160" height="160">
</p>

<h1 align="center">Zorya 0.1.0 Technical Preview</h1>

<p align="center">
  Early Windows technical preview powered by the Rarog Web Engine.
</p>

This package is an experimental Windows build of Zorya, a desktop browser powered by the Rarog Web Engine. It is a technical preview, not a general-purpose or production-ready browser.

## What this build proves

- a real native Windows application window and Zorya-owned multi-tab browser model;
- one Rarog View per materialized tab with identity/generation-safe activation and close handoff;
- off-UI-thread Rarog rendering and native DX12 presentation through Rarog's public platform/compositor boundary;
- deterministic `about:blank` startup for a fresh profile and real HTTP(S) document navigation through Rarog's supported Fetch/navigation contract;
- locked, versioned profile persistence for settings, browsing history, bookmarks and session restore through a bounded background worker;
- stale-safe profile replacement and committed-session restoration into fresh runtime browser identities;
- a release-mode executable built from the locked dependency graph and packaged with provenance and third-party license evidence.

## Known limitations

- The full visual browser chrome, tab strip and normal address-bar UI are not rendered yet; current browser interaction is primarily through native-window behavior and keyboard commands.
- Web-content keyboard, pointer, wheel and IME input are not wired yet because the supported Rarog View input-dispatch contract is still pending.
- Page-title observation is not wired yet, so current committed-page bookmarks use an empty title.
- Full GPU device-loss recovery is not available yet pending a stable Rarog recovery contract.
- Profile/settings/history/bookmark/session persistence exists, but broader management UI, profile create/rename/delete UX, history UI and bookmark management UI are not complete.
- Downloads, installer and updater are not implemented yet.
- The executable is not code-signed, so Windows may show a reputation or SmartScreen warning.

These limitations are intentional. Zorya does not reproduce missing Web-engine behavior inside the browser repository just to make the preview appear more complete.

## Run

Windows 10/11 x86-64 is the first release target.

Extract the ZIP and run:

    zorya.exe

A fresh profile starts from the committed `about:blank` browser state. A profile with a persisted session can restore its committed window/tab order, locations and active selection using fresh process-local browser identities.

To verify the packaged version without opening a window:

    zorya.exe --version

Expected output for the current package version:

    Zorya 0.1.0

## Verification

The release candidate is built with Cargo's release profile and must pass:

- locked dependency resolution and dependency-license preflight;
- release build and exact version check;
- the real native-window/Rarog/DX12 smoke path;
- native tab activation, close and rapid-supersession smokes;
- real HTTP navigation smoke coverage;
- profile-cycle and persisted color-scheme smokes;
- two-run bookmark and session-restore persistence/reload smokes;
- committed-page bookmark-toggle persistence/reload coverage;
- package integrity hashing and third-party license bundle generation;
- provenance recording for the exact Zorya build checkout, source head and resolved Rarog commit;
- extraction and packaged-executable verification.

The package includes `BUILD-INFO.txt` with the exact build-checkout commit, source-head commit, target, Rarog revision and Rust/Cargo tool versions used for that candidate. Pull-request candidates can have different build/source commits because GitHub verifies the proposed source against the current base through a synthetic merge checkout; a release candidate built from `main` records the release commit as both.

The normal repository gates separately enforce formatting, check, Clippy, tests, Linux portability, Rust 1.85 MSRV, Dependency Review, RustSec audit, secret scanning, CodeQL and CI supply-chain policy.

## License

Zorya is licensed under Apache-2.0. The package includes Zorya's `LICENSE` and a generated `THIRD_PARTY_LICENSES` directory containing license/notice evidence for the non-development packages reachable from Zorya in Cargo's Windows x86-64 resolved dependency graph.
