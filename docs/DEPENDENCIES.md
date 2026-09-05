# Dependencies

## Rarog Web Engine

Zorya consumes rarog-engine from:

https://github.com/StanleyLl0yd/rarog

The dependency is pinned to exact commit:

b330f94fd43b6b809ec0d784f6d0d7f2cce44989

Do not change this to a floating main or branch dependency.

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
