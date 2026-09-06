# Contributing to Zorya

Zorya is an early-stage Windows-first browser built on Rarog.

## Before changing code

Read:

- AGENTS.md;
- docs/ARCHITECTURE.md;
- docs/ROADMAP.md;
- docs/DEPENDENCIES.md when changing dependencies.

The core ownership rule is simple: browser-product behavior belongs in Zorya; Web-engine semantics belong in Rarog.

## Development flow

Use a short-lived topic branch and a focused pull request. Do not use main as a working branch.

Before a change is considered complete, run the relevant checks:

    cargo fmt --all -- --check
    cargo check --locked --all-targets
    cargo clippy --locked --all-targets -- -D warnings
    cargo test --locked --all-targets

For Windows native-shell changes, also build the actual application binary:

    cargo build --locked --bin zorya
    target/debug/zorya --version

On Windows, use `target\debug\zorya.exe --version` for the headless executable smoke and `target\debug\zorya.exe --native-smoke` for the full native-window/Rarog/DX12 smoke. The native smoke must exit automatically after its first successful presentation.

Windows is the primary target. Changes to portable product code should also continue to compile and test on Linux CI.

## Rarog changes

Do not patch around a missing Rarog capability by duplicating engine code in Zorya or depending on undocumented internals.

When an engine change is required:

1. implement and verify the supported contract in Rarog;
2. merge it there;
3. update Zorya exact Rarog revision;
4. add or adjust Zorya integration coverage.

## Pull requests

Keep each PR coherent. Explain:

- what product or architecture problem it solves;
- which trust or state boundaries it touches;
- which checks actually ran;
- whether persistent data, permissions, navigation, downloads or Rarog integration changed.

Do not claim checks or platform behavior that were not actually verified.

## Source style

Keep comments minimal, necessary, current and English-only. Prefer clear ownership and names over explanatory narration.

Never commit credentials, signing keys, private user data, real browsing profiles, crash dumps containing private data, or generated secrets.
