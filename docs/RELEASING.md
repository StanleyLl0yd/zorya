# Releasing Zorya

Zorya release work is browser-product work and follows the same repository trust, dependency and verification rules as the application itself.

## Release stages

A release candidate and a public release are deliberately separate stages.

A **release candidate** is a verified CI artifact. It proves that a particular source revision can produce a package that passes the required release checks.

A **public release** is a tagged, user-facing GitHub Release created only after the corresponding candidate has passed on `main`.

Do not describe a pull-request artifact or an untagged Actions artifact as a released version.

## Version source

The Cargo package version in `Cargo.toml` is authoritative for the application version.

The executable must report exactly:

    Zorya <Cargo version>

through `zorya.exe --version`.

A release process must fail rather than publish when the package version, executable version or intended release tag disagree.

## Dependency source

Release candidates are built from `Cargo.lock` and the exact Rarog Git revision declared in `Cargo.toml`.

The candidate workflow performs the dependency fetch first and then runs the release build/package metadata path offline. It must not change dependency versions as part of packaging.

The generated package records the resolved Rarog commit in `BUILD-INFO.txt`.

## Windows Technical Preview candidate

The first release target is Windows 10/11 x86-64.

The candidate workflow must:

1. validate the packaging script before the expensive release build;
2. resolve the Cargo version;
3. fetch the locked Windows x86-64 dependency graph;
4. run the same packager logic in license-preflight mode and reject missing license evidence before the expensive build;
5. build `zorya.exe` with the Cargo release profile;
6. verify the executable version;
7. run the real native-window/Rarog/DX12 `--native-smoke`;
8. package the executable, Zorya license, Technical Preview notes, build provenance and third-party license material;
9. create and verify the SHA-256 digest;
10. extract the ZIP into a new directory;
11. repeat the version and native smoke checks against the packaged executable;
12. verify packaged provenance and third-party license index;
13. upload the ZIP and digest only after every previous step succeeds.

The normal CI matrix remains independently required. A release candidate passing does not replace format/check/Clippy/tests/Linux portability/MSRV verification.

## Provenance

`BUILD-INFO.txt` records at least:

- Zorya version;
- exact checkout commit whose tree was built;
- source-head commit represented by that checkout;
- build target;
- release profile;
- exact resolved Rarog commit;
- Rust compiler version;
- Cargo version.

For a pull-request candidate, GitHub Actions normally builds a synthetic merge checkout, so the build checkout and source-head commits are intentionally recorded separately. For a candidate built from `main`, they must identify the same release commit. The public release must be traceable to the source/build commit recorded in its package.

## Licensing

Zorya's Apache-2.0 `LICENSE` is included at the package root.

The packager traverses non-development edges reachable from Zorya in Cargo's Windows x86-64 filtered resolve graph and copies discoverable `LICENSE`, `LICENCE`, `COPYING` and `NOTICE` material into `THIRD_PARTY_LICENSES`. Packaging fails closed when a dependency has neither declared license metadata nor discoverable license/notice text.

Some registry packages deliberately omit license files from the published crate even though their Cargo metadata declares a license. For those cases only, `third_party/licenses/<name>-<version>/` may contain an exact upstream license copy plus mandatory `ORIGIN.txt` provenance. The override is used only after normal package-source discovery found no text and is itself shipped in the package. Do not replace a real missing license obligation with a manually invented attribution.

The Windows release inventory is also guarded against silent drift. The packager derives a canonical target-specific manifest containing dependency identity/source/license metadata plus the filename and SHA-256 of every packaged license, notice and override-origin evidence file. A compact reviewed baseline lives under `third_party/licenses/inventory/`. License preflight fails when the dependency count, evidence-file count or complete canonical-manifest digest changes, so dependency/license changes require an explicit baseline review rather than silently altering the release bundle.

The complete canonical manifest is packaged as `THIRD_PARTY_LICENSES/INVENTORY.tsv` for inspection. The baseline is a review gate, not a substitute for fail-closed evidence discovery: a dependency with missing or conflicting evidence still fails before baseline comparison.

## Publication gate

Before creating a public GitHub Release:

- the release candidate workflow must be green on the exact `main` commit to publish;
- normal CI must be green on that commit;
- the aggregate Security workflow and Rust CodeQL workflow must be green on that commit;
- package hash, source provenance and GitHub artifact attestation must be available;
- release notes must accurately describe current limitations;
- signing status must be stated accurately;
- no open issue may be silently represented as implemented.

For Technical Preview releases, use prerelease status while the product remains intentionally incomplete.

## Publication procedure

Technical Preview publication uses an explicit short-lived `release-publish/v<version>` branch.

The publication branch must be created from the exact current `main` commit after both normal CI and the release-candidate workflow have succeeded for that same commit. The publication workflow rejects a branch whose commit differs from current `main`, whose version differs from Cargo metadata, whose release notes are missing, or whose tag/release already exists.

After those guards pass, the publication workflow:

1. verifies successful normal CI, release-candidate, Security and CodeQL runs for the exact `main` SHA;
2. fetches the locked Windows x86-64 dependency graph;
3. runs the same fail-closed license preflight used by the candidate;
4. builds the release executable offline from the locked graph;
5. verifies `--version` and runs the native-window/Rarog/DX12 smoke;
6. packages with the same source-controlled packager and verifies the ZIP digest, extracted executable, native smoke, provenance and third-party license index;
7. uploads only the verified ZIP and SHA-256 file as an internal workflow artifact;
8. in a separate least-privilege job with OIDC attestation permission, downloads and re-verifies those exact files and creates GitHub artifact attestations;
9. in a separate publication job with `contents: write`, downloads the already-built artifacts, verifies their checksum and attestations, re-checks that the source commit is still current `main`, and refuses an existing tag/release;
10. creates the `v<version>` Git tag and GitHub prerelease without rebuilding product code under a write-capable token;
11. verifies that the published prerelease contains both required assets.

The build job never receives `contents: write`. The attestation job does not check out or execute repository code and is the only release job with `id-token: write`/`attestations: write`.

Do not create the public version tag manually before this workflow runs. The tag is a publication result, not an input that bypasses the publication gate.

## Signing and installer status

The 0.1.0 Technical Preview is an unsigned ZIP package. It is not an installer and does not contain an updater.

Code signing, installer/update design and release-key handling remain later Z5 work. Never commit signing private keys or credentials to the repository.

## 0.1.0 Technical Preview scope

The first Technical Preview demonstrates the verified native Windows/Rarog/DX12 vertical in a release-mode package.

It does not imply:

- general HTTP(S) browsing;
- Web-content input;
- complete browser chrome;
- native multi-tab presentation;
- page-title integration;
- full GPU device-loss recovery;
- browser-profile persistence;
- installer/update support;
- production readiness.
