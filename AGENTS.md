# Repository Agent Rules

These rules apply to all automated coding agents and repository-wide maintenance work in Zorya.

## Project identity and priorities

Zorya is a Windows-first desktop Web browser powered by the Rarog Web Engine.

Preserve these priorities, in order:

1. Security, privacy and browser/Web trust-boundary integrity.
2. Correct browser behavior and correct use of Rarog embedder contracts.
3. User-data integrity and recoverable lifecycle behavior.
4. Responsiveness and bounded resource use.
5. Accessible, predictable native Windows user experience.
6. Maintainable product architecture and practical portability of non-platform code.

Zorya is the reference browser for Rarog, but Rarog remains independently embeddable. Do not collapse the two repositories into one architecture by moving engine semantics into Zorya or browser-product state into Rarog.

The first product target is Windows 10/11. Linux CI is a portability guard for non-platform code, not a promise of a Linux release.

Do not claim production readiness, broad site compatibility, security hardening, privacy guarantees or performance leadership beyond what current evidence supports.

## Read before changing architecture

Before changing durable product architecture, inspect:

- docs/ARCHITECTURE.md;
- docs/ROADMAP.md;
- docs/DEPENDENCIES.md when dependency or Rarog integration changes;
- CONTRIBUTING.md.

Update architecture documentation in the same work when a durable ownership, trust, persistence or platform decision changes.

For full-repository audits or deep refactors, also read docs/agent/AUDIT_REFACTOR.md.

## Zorya and Rarog ownership boundary

This is the central repository rule.

Zorya owns browser-product behavior:

- browser windows and browser chrome;
- tabs and their lifecycle;
- user navigation intent and navigation UX;
- profiles and product settings;
- history and bookmarks;
- downloads;
- permission prompts and persisted browser decisions;
- session restore;
- browser crash and recovery UX;
- updater, packaging and product release behavior.

Rarog owns Web-engine behavior:

- HTML, DOM, CSS and Web-platform semantics;
- script-facing behavior;
- URL, origin and site security semantics exposed by the engine;
- style, layout, fragments, display lists and paint;
- engine invalidation, rendering and compositor semantics;
- Web compatibility behavior;
- reusable platform and embedder contracts.

Do not implement missing DOM, CSS, layout, paint, origin or Web compatibility behavior in Zorya as a workaround.

Do not copy Rarog source into Zorya.

Do not reach into undocumented Rarog internals merely because a public embedder contract is inconvenient.

If Zorya needs an engine capability that is missing, prefer this sequence:

1. define the narrow Rarog-owned contract;
2. implement and verify it in Rarog;
3. merge the Rarog change;
4. update Zorya exact Rarog revision;
5. add Zorya integration coverage.

Likewise, do not push Zorya-specific tab, profile, history, bookmark, download or chrome state into Rarog simply because the engine is shared by the application.

## Authoritative browser state

Keep authority explicit.

Browser and product source state includes, as applicable:

- stable window and tab identities;
- tab ordering and active selection;
- navigation intent and browser-level lifecycle;
- profile identity;
- settings;
- permission decisions;
- download records;
- history, bookmarks and session data.

Rendered chrome, transient menu state, cached labels, thumbnails, layout measurements and similar UI products are derived state unless explicitly documented otherwise.

Do not use vector positions, widget handles or transient UI indices as durable identities across asynchronous work.

A stale completion must never silently apply to a new tab, window or profile that happens to reuse a position or handle.

## Browser chrome and untrusted Web content

Browser chrome is privileged. Web content is untrusted.

Treat as untrusted any content-controlled:

- title or text shown in chrome;
- URL or string;
- favicon or image;
- suggested download filename;
- clipboard payload;
- external-protocol target;
- permission prompt metadata;
- file or path-like value;
- status text.

Web content must not directly own privileged browser controls or OS capabilities.

Avoid UI constructions where content can visually impersonate the address bar, permission prompt, download confirmation or other privileged chrome without a clear security boundary.

Do not convert a display string into a security decision.

## Navigation and URL security

User-visible navigation state and engine security identity are not interchangeable.

Use Rarog URL, origin and site primitives for security semantics when available. Do not implement ad hoc same-origin parsing, host canonicalization or scheme logic in product UI code.

External protocols, local files, downloads, privileged internal pages and OS launches require explicit browser policy.

Do not automatically launch an external application solely because untrusted Web content supplied a URL.

Preserve user intent across redirects and navigation without allowing stale navigation completions to overwrite a newer navigation.

## Permissions and OS capabilities

Permissions are browser decisions mediated on behalf of Web content.

A permission decision must be bound to the correct requesting context, origin and capability. Do not reuse a decision merely because UI text looks equivalent.

File pickers, clipboard, notifications, camera, microphone, external protocols, credential access and similar OS capabilities must stay behind explicit browser or engine capability boundaries.

Do not expose raw privileged handles directly to Web-controlled state.

## Privacy and user data

Do not add telemetry, analytics, background reporting, remote configuration or unrelated network calls as incidental implementation details.

Any future data collection must be an intentional, documented product decision with a clear user and privacy model.

Persistent browser data must have explicit ownership and lifecycle. Before persistence schemas become public, make them versioned and migration-aware.

Prefer atomic or recoverable writes for user data. A partial write or one corrupt record must not silently destroy unrelated profile data.

Do not store passwords, authentication tokens, private keys or equivalent secrets in plaintext settings or database fields. Use a reviewed OS-backed secret-storage boundary when such features are introduced.

Logs, diagnostics and crash reports must avoid unnecessary browsing data, secrets and full content payloads.

## Async work and the UI thread

Keep the native UI thread responsive.

Do not perform potentially blocking network, filesystem, database, decoding, engine waits or heavy computation directly in input or window callbacks.

Asynchronous work must have:

- an owner;
- a stable request and target identity;
- cancellation or invalidation semantics;
- a completion path;
- bounded pending work.

When a tab, window, profile or navigation is closed or superseded, stale work must be cancelled or rejected on completion.

Do not introduce polling or busy redraw loops when event-driven or completion-driven wakeups are sufficient.

## Resource bounds

Browser-controlled and Web-controlled queues must be bounded by count, bytes, lifetime or another explicit budget.

Pay particular attention to:

- tabs and windows;
- navigation histories;
- downloads;
- permission requests;
- image and icon caches;
- thumbnails;
- session journals;
- async completion queues;
- diagnostics and log buffers;
- closed or background tab work.

Avoid process-global immortal caches unless their lifetime, invalidation and bounds are explicitly justified.

## Platform boundary

Windows 10/11 is the primary target.

Keep product-specific Windows APIs behind narrow platform modules. Win32, WinRT, shell, registry, credential-manager, installer, updater and native-window types should not spread through browser state or model code.

When a capability belongs to reusable Web embedding rather than Zorya product chrome, prefer the corresponding Rarog platform or embedder boundary.

Non-Windows builds may use stubs for Windows-only product functions, but portable product logic should remain genuinely portable where practical.

## Rust and memory safety

Preserve the declared MSRV unless a concrete product or toolchain requirement justifies changing it.

Preserve unsafe_code = "forbid" for repository-owned Rust.

If native integration eventually requires unsafe code, isolate it behind the narrowest reviewed platform boundary and change the safety policy only through an explicit architectural decision.

Prefer ownership and state models that make stale or cross-profile and cross-tab mistakes difficult to represent.

Do not hide correctness failures with broad error suppression, unchecked conversions or catch-all fallback behavior.

## Rarog dependency discipline

Rarog must be pinned to an exact Git revision until a stable versioned distribution mechanism replaces it.

Do not depend on floating main, a moving branch or an unreviewed local fork in committed product configuration.

Treat a Rarog revision update as a meaningful dependency change:

- review upstream changes;
- update the exact revision;
- regenerate and commit Cargo.lock;
- run the full Zorya CI matrix;
- add integration regression coverage when a used contract changes.

Do not edit vendored or generated dependency code to simulate an upstream fix.

## Dependencies and generated code

Add a third-party dependency only for a concrete current product need.

Prefer narrow adapters around UI, storage, networking, updater and platform dependencies so vendor types do not become the browser authoritative cross-module state model.

Do not add overlapping libraries when the current stack already provides the needed capability adequately.

Keep Cargo.lock committed and synchronized.

Keep third-party GitHub Actions pinned to immutable full commit SHAs.

Do not weaken CI, MSRV, portability, lint or correctness checks simply to make a change pass.

Generated artifacts are not authoritative source when a generator or source representation exists.

## Verification

Baseline verification is:

    cargo fmt --all -- --check
    cargo check --locked --all-targets
    cargo clippy --locked --all-targets -- -D warnings
    cargo test --locked --all-targets

Changes to native Windows behavior must also be verified on Windows.

Changes to Rarog integration must exercise the affected integration path, not merely compile unrelated code.

Changes to persistence need migration and recovery coverage appropriate to the schema.

Changes to navigation, downloads, permissions or privileged OS actions need tests for stale or wrong-context requests and conservative failure behavior.

Never claim a check passed unless it actually ran successfully. State unavailable platforms, hardware, credentials, signing material or external services explicitly.

## Change discipline

Use short-lived topic branches and pull requests. Do not use main as a working branch.

Keep commits and pull requests focused on one coherent purpose. Separate behavior-preserving cleanup from unrelated product features.

Do not force-push shared history, discard unrelated user changes or weaken repository protections without explicit authorization.

Merge only after the required checks pass.

Never commit credentials, signing material, private keys, real user profiles, private browsing data, production tokens or generated secrets.

## Comments and documentation

Keep source-code comments minimal, necessary, current and English-only.

Do not add comments that narrate obvious code. Prefer names and types that express ownership and state transitions.

Keep comments that explain non-obvious security boundaries, lifecycle invariants, cancellation, persistence guarantees, resource limits or platform constraints.

Remove stale, redundant and commented-out historical code when the surrounding work proves it is obsolete.

When behavior, architecture, persistent formats, dependency requirements, supported commands or durable subsystem contracts change, update the relevant documentation in the same work.

## Repository-wide audit and deep refactoring

For a full repository audit, cleanup, optimization, simplification or deep-refactoring task, read and follow docs/agent/AUDIT_REFACTOR.md in full before editing.

The Zorya-specific ownership, trust, privacy and lifecycle invariants in this file remain mandatory and take precedence over generic simplification goals.
