# Dependencies

## Rarog Web Engine

Zorya consumes public Rarog crates from:

https://github.com/StanleyLl0yd/rarog

The dependency is pinned to exact commit:

b330f94fd43b6b809ec0d784f6d0d7f2cce44989

`rarog-engine`, `rarog-compositor`, and `rarog-types` are portable integration dependencies. On Windows, Zorya additionally uses `rarog-compositor-wgpu` and `rarog-platform-windows` for the public GPU/presentation boundary. Every Rarog crate uses the same exact revision.

The native Windows shell also uses exact `winit = 0.30.13` and `pollster = 0.4.0`, matching the pinned Rarog reference host. `winit` is restricted to the Windows platform adapter and does not become browser-model identity; `pollster` is used only on the render worker, never to block the UI event loop.

Do not change any Rarog dependency to a floating main or branch dependency.

### Upgrade procedure

1. Review the Rarog changes between the current and proposed revisions.
2. Confirm that the public embedder behavior used by Zorya remains compatible.
3. Update the exact Git revision in Cargo.toml.
4. Regenerate Cargo.lock.
5. Run Windows-primary, Linux-portability and Rust 1.85 checks.
6. Add focused integration coverage for any new or changed engine contract used by Zorya.

Do not copy Rarog source into this repository to work around an API limitation. Add the missing supported boundary upstream.

## GitHub Actions

Third-party actions must be pinned to immutable full commit SHAs. Do not use moving tags such as v4 in committed workflow files.

## General dependency policy

Add a dependency only for a concrete product requirement. Prefer small adapters around replaceable platform, UI and storage implementations and avoid allowing third-party types to become the browser authoritative cross-module state model.

Application lockfile changes are committed.
