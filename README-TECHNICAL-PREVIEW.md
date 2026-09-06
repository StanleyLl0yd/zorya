# Zorya 0.1.0 Technical Preview

This package is an early Windows technical preview of Zorya, a desktop browser powered by the Rarog Web Engine.

## What this build proves

- a real native Windows application window;
- a Zorya-owned browser product model;
- one live Rarog View;
- off-UI-thread Rarog rendering;
- native DX12 presentation through Rarog's public platform/compositor boundary;
- deterministic startup at `about:blank`;
- a release-mode executable built from the locked dependency graph.

## Known limitations

This is not yet a general-purpose browser.

- General HTTP(S) navigation is not implemented yet.
- Browser chrome and the normal address-bar UI are not rendered yet.
- Web-content keyboard, pointer and IME input are not wired yet.
- Native multi-tab presentation is not enabled yet.
- Page-title observation is not wired yet.
- Full GPU device-loss recovery is not available yet.
- There is no profile persistence, history UI, bookmarks, downloads, installer or updater yet.
- The executable is not code-signed, so Windows may show a reputation or SmartScreen warning.

These limitations are intentional. Zorya does not reproduce missing Web-engine behavior inside the browser repository just to make the preview appear more complete.

## Run

Windows 10/11 x86-64 is the first release target.

Extract the ZIP and run:

    zorya.exe

The window displays the current deterministic local Rarog-backed startup document.

To verify the packaged version without opening a window:

    zorya.exe --version

Expected output for this preview:

    Zorya 0.1.0

## Verification

The release candidate is built with Cargo's release profile and must pass:

- locked dependency resolution;
- release build;
- exact version check;
- the real native-window/Rarog/DX12 `--native-smoke` path;
- package integrity hashing;
- third-party license bundle generation;
- provenance recording for the exact Zorya source commit and resolved Rarog commit.

The package includes `BUILD-INFO.txt` with the exact source commit, target, Rarog revision and Rust/Cargo tool versions used for that candidate.

The normal repository CI separately continues to enforce formatting, check, Clippy, tests, Linux portability and Rust 1.85 MSRV.

## License

Zorya is licensed under Apache-2.0. The package includes Zorya's `LICENSE` and a generated `THIRD_PARTY_LICENSES` directory containing license/notice evidence for the non-development packages reachable from Zorya in Cargo's Windows x86-64 resolved dependency graph.
