# Zorya Architecture

## Mission

Zorya is the reference desktop browser for the Rarog Web Engine.

The product is Windows-first and Rust-first. The browser repository owns user-facing browser behavior while Rarog remains an independently embeddable Web engine.

## Primary boundary

The most important architectural rule is that Zorya is not another Web engine layer.

    User
      |
      v
    Zorya browser chrome and product state
      |  windows · tabs · navigation policy · profile · permissions
      v
    Rarog embedder boundary
      |
      v
    Rarog Web engine
      |  DOM · CSS · script · layout · paint · compositor
      v
    Platform services / pixels

Zorya may coordinate Rarog views and apply browser-level policy. It must not become an alternate source of DOM, CSS, layout, paint, origin, or Web compatibility semantics.

If a browser feature requires information or control that Rarog does not expose, extend the supported Rarog embedder/platform contract first.

## Product-owned state

Zorya is authoritative for:

- browser windows and chrome;
- tab identity, ordering, selection and lifecycle;
- navigation UI state and user intent;
- profile selection and product settings;
- history and bookmarks;
- downloads and user-visible transfer state;
- permission prompts and persisted browser decisions;
- session restore;
- browser-level crash/recovery UX;
- updater, packaging and product release state.

These are not engine-derived caches. Their persistence and lifecycle must be explicit.

### Browser product model

`BrowserApp` is the authoritative in-memory owner of browser-window and tab lifecycle. `BrowserWindowId` and `TabId` are monotonically allocated product identities and are never derived from vector positions, native window handles or UI widget identities.

`BrowserWindow` owns tab ordering and active-tab selection. Closing an active tab selects a surviving neighbor when one exists; closing the last tab leaves the window with no active tab. Recreating a tab allocates a new identity, so stale work targeting a closed tab cannot silently attach to a replacement.

The product model is platform-independent. Native Windows identifiers and Rarog `ViewId` values are adapter concerns and must be mapped to product identities rather than becoming product identity themselves.

### Rarog engine host

`engine::EngineHost` is the Zorya-owned adapter around the public Rarog embedder API. It owns the shared `rarog_engine::Engine` and maps each product `TabId` to one live Rarog `View` without exposing Rarog identifiers as browser identity.

Every hosted View receives a Zorya-owned monotonically increasing generation. Frame work is represented by `EngineFrameRequest`, which binds the product `TabId`, View generation and Rarog frame request number. A completion is accepted only while all three still identify the currently active request. Closing and recreating a View for the same tab therefore invalidates work from the previous View even when Rarog's per-View request numbering starts again from the same value.

The adapter is responsible for View creation/destruction, deterministic local HTML loading, viewport conversion and Rarog frame-request lifecycle. It does not implement Web parsing, layout, paint, navigation semantics or compositor behavior.

## Engine-owned state

Rarog is authoritative for:

- Web document and script semantics;
- URL/origin/site security primitives exposed by its public API;
- style, layout, fragments, display lists and rendering;
- engine scheduling/invalidation semantics;
- Web resource and compatibility behavior;
- compositor and platform contracts exposed to embedders.

Zorya must not infer engine truth by scraping derived pixels or reconstructing hidden engine state.

## Trust boundaries

Treat all Web-controlled input as untrusted, including titles, URLs, suggested filenames, downloads, clipboard payloads, permission requests, external-protocol targets and content-originated UI text.

Browser chrome is privileged. Web content must not be able to impersonate, overlap, mutate, or directly own privileged browser controls.

Future Rarog process/site isolation must remain visible in Zorya architecture. Do not design product state around the assumption that Web content permanently runs in the same process as browser chrome.

## Navigation boundary

Navigation is shared work with distinct ownership:

- Zorya owns user intent, browser UX, tab lifecycle and browser policy;
- Rarog owns URL/origin semantics and Web navigation execution exposed through its embedder API.

Display strings and canonical security identities are different concepts. Never use a user-facing URL string as an authorization or same-origin decision.

External protocols, local files, downloads and privileged internal pages require explicit browser policy.

## Platform boundary

Windows 10/11 is the primary product target.

Windows-only APIs should stay behind narrow Zorya platform modules when they are product-shell concerns. Engine/platform functionality that belongs to reusable Web embedding should live behind Rarog platform contracts instead.

Do not spread Win32, WinRT, shell, registry, credential-manager or installer types through browser-model code.

Linux CI exists to keep non-platform product logic portable and to expose accidental Windows coupling early. It is not a promise of a Linux release.

### Z1 Windows native shell

The Z1 developer shell uses `winit` only as a narrow native-window/event-loop adapter. Winit `WindowId` values stay inside the Windows platform module; browser identity remains `BrowserWindowId`/`TabId`.

The UI thread owns the native event loop, top-level window lifecycle, resize/redraw routing and browser product state. It does not wait for DX12 device initialization or synchronous Rarog rendering. A dedicated bounded render worker owns `EngineHost`, the Rarog View, GPU device lifetime and compositor work. UI-to-worker render requests and worker completions carry a monotonically increasing Zorya request ID together with their target browser window and tab. Completions are ignored unless they still match the currently pending request and live product identities.

Windows native surface creation is intentionally split from GPU-device initialization. In winit 0.30, safe raw-window-handle access is available only on the event-loop thread. The worker therefore requests `WindowsGpuDevice` asynchronously and sends a shared device handle to the event loop; the event-loop thread performs only the thread-affine `WindowsGpuDevice::create_surface` call and transfers the resulting `WindowsGpuSurface` back to the worker. Surface replacement follows the same handshake. Zorya does not use winit's unsafe any-thread window-handle escape hatch and does not weaken `unsafe_code = "forbid"`.

Rendering is event driven. Resize/DPI changes mark the window dirty and request a redraw; only one render request may be in flight. A redraw that arrives while rendering is coalesced into one later redraw rather than starting a busy loop.

`WebContentSurface` is the platform-owned presentation boundary for untrusted Web pixels. In the first Z1 vertical it occupies the full client area because privileged browser chrome is not rendered yet, but Web content does not own the top-level window or future chrome state. When Rarog reports its public `WindowsGpuError::Surface` category, the worker discards the affected compositor/Rarog frame and requests one replacement native surface from the event-loop thread; after replacement it requests a fresh explicit Rarog frame. Resize/configuration and compositor failures are not treated as surface-loss recovery candidates. The nested backend-specific surface error is intentionally not inspected in Zorya; finer recovery classification and GPU device-loss handling remain blocked on the stable Rarog contract tracked in issue #6. Repeated or non-recoverable failure terminates the developer shell conservatively.

## Async and UI lifecycle

The native UI thread must remain responsive.

Do not perform unbounded file I/O, networking, decoding, database maintenance, engine waits or other potentially blocking work directly in event handlers.

Long-running work needs explicit ownership, cancellation and completion routing. A closed tab/window/profile must not receive stale completions as though it were still current.

Stable product identities should be used for asynchronous work instead of borrowing array positions or transient UI indices.

`async_lifecycle` centralizes Zorya-owned asynchronous identity and pending-request validation. `AsyncRequestSequence` allocates monotonically increasing request identities bound to a specific `BrowserWindowId` and `TabId`; `PendingRequest` permits only one owner for a request slot, rejects overlapping starts, accepts only the exact current completion and supports explicit invalidation on lifecycle teardown.

Platform adapters use this mechanism rather than implementing their own request counters or stale-completion checks. A shared `CancellationToken` separately provides cooperative cancellation for accepted worker work. Closing a window first invalidates pending initialization/render/surface slots, then marks the worker cancelled before closing the bounded command channel. New commands are rejected after cancellation; queued commands are dropped when the worker observes the token. GPU initialization that is already inside the platform request cannot be force-interrupted, but cancellation is checked before the device is published to the event loop. Rarog rendering is checked again before Web-content presentation, so completed engine work is discarded rather than presented after teardown whenever cancellation is observed before presentation begins. The UI thread never joins or waits for the worker.

## Persistent data

Persistent browser data must be versioned and migration-aware before schemas become public.

Writes that affect user data should be atomic or recoverable after interruption. Corruption must fail visibly and conservatively rather than silently discarding unrelated user state.

Secrets and authentication material must not be stored in plaintext configuration files. Windows credential storage or another reviewed secret-storage boundary should be used when such features are introduced.

## Dependency on Rarog

Rarog is currently consumed as a Git dependency pinned to an exact commit.

The pin is intentional:

- builds are reproducible;
- Zorya does not silently inherit breaking engine changes;
- an engine upgrade is reviewable as its own change.

When Rarog exposes a stable published/versioned embedder package, the dependency strategy may be revisited through an architectural change.

## Safety

Repository-owned Rust forbids unsafe by default.

If native integration eventually requires unsafe code, isolate it behind the narrowest reviewed platform boundary. Do not weaken the repository-wide safety posture simply to make a dependency or convenience API compile.
