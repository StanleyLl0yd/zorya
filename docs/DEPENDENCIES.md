# Dependencies

## Rarog Web Engine

Zorya consumes public Rarog crates from:

https://github.com/StanleyLl0yd/rarog

The dependency is pinned to exact commit:

b330f94fd43b6b809ec0d784f6d0d7f2cce44989

`rarog-engine`, `rarog-compositor`, and `rarog-types` are portable integration dependencies. On Windows, Zorya additionally uses `rarog-compositor-wgpu` and `rarog-platform-windows` for the public GPU/presentation boundary. Every Rarog crate uses the same exact revision.

The native Windows shell also uses exact `winit = 0.30.13` and `pollster = 0.4.0`, matching the pinned Rarog reference host. `winit` is restricted to the Windows platform adapter and does not become browser-model identity. Safe winit window-handle access is thread-affine on Windows, so Rarog native-surface creation/replacement is performed on the event-loop thread while GPU-device initialization stays on the worker. `pollster` is used only on the render worker, never to block the UI event loop.

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

Windows CI uses GitHub's `actions/upload-artifact` at an immutable commit to retain the developer `zorya.exe` after the full Windows verification sequence and explicit binary build. The artifact is CI output only and is not a release, installer or signed distribution package.

The release-candidate workflow reuses the same immutable checkout/toolchain/upload actions and adds no packaging dependency. It follows the non-development packages reachable from Cargo's Windows x86-64 filtered `resolve` graph after the locked fetch, rather than treating every entry in the unfiltered metadata `packages` array as a Windows dependency. It then collects declared license/notice files from the resolved package sources. Candidate packaging fails if a dependency lacks both license metadata and discoverable license text. When a registry archive declares a license but intentionally omits its license file, an exact source-controlled copy may be stored under `third_party/licenses/<name>-<version>/` only with `ORIGIN.txt` recording the upstream package/version/source revision; the packager uses such a copy only when normal package-source discovery found no license text.

## General dependency policy

Add a dependency only for a concrete product requirement. Prefer small adapters around replaceable platform, UI and storage implementations and avoid allowing third-party types to become the browser authoritative cross-module state model.

Application lockfile changes are committed.
