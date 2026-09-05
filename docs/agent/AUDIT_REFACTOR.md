# Repository Audit and Refactor Protocol

Use this protocol for a full Zorya repository audit, cleanup, simplification, optimization or deep refactor.

The mandatory architecture and security rules in the root AGENTS.md remain in force.

## 1. Establish the baseline

Before editing:

- inspect the repository tree and current roadmap;
- identify the current Rarog revision;
- inspect open work relevant to the requested scope;
- run or inspect the available baseline CI;
- record existing failures instead of attributing them to later changes.

Do not begin a broad rewrite from assumptions about code that has not been inspected.

## 2. Map ownership and trust boundaries

For every subsystem, identify:

- authoritative source state;
- derived or cached state;
- stable identities;
- async work and cancellation ownership;
- privileged browser capabilities;
- Web-controlled or untrusted input;
- persistent user data;
- Rarog-facing contracts;
- Windows and platform-specific code.

Flag reverse ownership, duplicated truth, hidden global mutation and product code that reconstructs engine semantics.

## 3. Correctness and lifecycle audit

Look for:

- stale tab, window or profile identities;
- work delivered after the owner was closed or replaced;
- index-based identity used across async boundaries;
- incomplete cancellation;
- inconsistent navigation state;
- state transitions that can become wedged after error or recovery;
- duplicated checks with divergent behavior;
- silent fallbacks that hide corruption or security failure.

Prefer one explicit state machine or ownership path over multiple loosely synchronized booleans.

## 4. Security and privacy audit

Inspect privileged boundaries for:

- unsanitized Web-controlled strings entering chrome or OS APIs;
- URL display and security-identity confusion;
- unsafe download paths or filenames;
- unexpected external protocol launches;
- permission decisions without origin or context binding;
- plaintext secrets;
- accidental telemetry or networking;
- sensitive data in logs or crash output;
- unbounded or attacker-controlled allocation.

Do not weaken a boundary to simplify code.

## 5. Rarog integration audit

Verify that Zorya uses supported public or embedder contracts and does not duplicate:

- DOM, CSS or layout behavior;
- origin and site security semantics;
- paint or compositor semantics;
- Web compatibility logic.

When Zorya is compensating for a missing engine API, prefer a focused upstream Rarog change and then update the pinned revision.

## 6. Performance and resource audit

Measure or identify concrete cost before optimizing.

Pay attention to:

- UI-thread blocking;
- repeated engine or view creation;
- unnecessary full browser-model cloning;
- unbounded histories, queues or caches;
- redundant disk writes;
- work continuing for background or closed tabs;
- expensive work triggered by every paint or input event.

Do not trade correctness or security for unmeasured speed.

## 7. Simplification rules

Remove code when it is proven redundant, unreachable, obsolete or replaced by a stronger single path.

Do not delete:

- conservative error or recovery paths;
- security validation;
- ownership checks;
- migration code still needed for supported persisted data;
- portability boundaries;
- regression coverage;

merely to reduce line count.

Separate behavior-preserving cleanup from feature changes when practical.

## 8. Dependencies

For each dependency, ask whether it has a concrete current responsibility and whether its types leak beyond a narrow adapter.

Keep the Rarog revision exact. Keep GitHub Actions pinned to immutable SHAs. Keep Cargo.lock synchronized.

## 9. Verification

At minimum:

    cargo fmt --all -- --check
    cargo check --locked --all-targets
    cargo clippy --locked --all-targets -- -D warnings
    cargo test --locked --all-targets

Also verify the affected Windows behavior for changes to native UI, platform integration, permissions, downloads, persistence or presentation.

Never describe a check as passing unless it actually ran.

## 10. Finish with evidence

A full audit should end with:

- concrete issues found and fixed;
- remaining risks or deferrals;
- architecture or documentation changes;
- tests added;
- checks run and their results.

Avoid cleanup churn that cannot be tied to correctness, security, maintainability, resource use or a documented simplification.
