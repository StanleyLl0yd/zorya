use super::RunMode;
use crate::async_lifecycle::{
    AsyncRequestSequence, AsyncTarget, CancellationToken, PendingRequest,
};
use crate::branding::application_icon;
use crate::engine::{
    EngineFrameCause, EngineFrameRequest, EngineHost, EngineNavigationPoll,
    EngineNavigationRequest, Viewport,
};
use crate::{
    BrowserApp, BrowserCommand, BrowserCommandEffect, BrowserNavigationCommit, BrowserWindowId,
    NavigationId, NavigationStart, PresentationFramePermit, PresentationGeneration,
    PresentationHandoffError, ProfileHistorySavePolicy, ProfileHistorySaveScheduler,
    ProfileHistorySaveUrgency, ProfileLockOwner, ProfileRuntime, ProfileSelectionIntent,
    ProfileSettingsSavePolicy, ProfileSettingsSaveScheduler, ProfileSettingsSaveUrgency,
    ProfileWorker, ProfileWorkerCompletion, TabActivationStart, TabCloseStart, TabCycleDirection,
    TabId, TabPresentationHandoff, TargetFramePermit, WebContentPresentation,
};
use pollster::block_on;
use rarog_compositor::{
    CompositorBackend, FrameDecision, FramePlanner, FrameSubmission, SurfaceId, SurfaceSize,
};
use rarog_compositor_wgpu::WgpuCompositorBackend;
use rarog_platform_windows::{WindowsGpuDevice, WindowsGpuError, WindowsGpuSurface};
use std::collections::BTreeMap;
use std::error::Error;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Icon, Window, WindowId};

const START_LOCATION: &str = "about:blank";
const START_PAGE: &str = include_str!("../../assets/z1-start.html");
const HTTP_SMOKE_BODY: &str = "<main>Zorya real HTTP navigation</main>";
const NAVIGATION_POLL_INTERVAL: Duration = Duration::from_millis(5);
const PRODUCT_DATA_DIRECTORY: &str = "Zorya";
const PROFILES_DIRECTORY: &str = "Profiles";
const DEFAULT_PROFILE_DIRECTORY: &str = "Default";
const SETTINGS_SAVE_DEBOUNCE_MILLIS: u64 = 1_000;
const SETTINGS_SAVE_MAX_DIRTY_MILLIS: u64 = 10_000;
const SETTINGS_SAVE_MUTATION_THRESHOLD: u64 = 8;
const HISTORY_SAVE_DEBOUNCE_MILLIS: u64 = 5_000;
const HISTORY_SAVE_MAX_DIRTY_MILLIS: u64 = 30_000;
const HISTORY_SAVE_MUTATION_THRESHOLD: u64 = 32;

#[derive(Debug)]
struct NativeProfileClock {
    origin: Instant,
}

impl NativeProfileClock {
    fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }

    fn now_millis(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn deadline(&self, millis: u64) -> Option<Instant> {
        self.origin.checked_add(Duration::from_millis(millis))
    }
}

fn native_settings_save_policy() -> ProfileSettingsSavePolicy {
    ProfileSettingsSavePolicy::new(
        SETTINGS_SAVE_DEBOUNCE_MILLIS,
        SETTINGS_SAVE_MAX_DIRTY_MILLIS,
        SETTINGS_SAVE_MUTATION_THRESHOLD,
    )
    .expect("native profile-settings save policy is valid")
}

fn native_history_save_policy() -> ProfileHistorySavePolicy {
    ProfileHistorySavePolicy::new(
        HISTORY_SAVE_DEBOUNCE_MILLIS,
        HISTORY_SAVE_MAX_DIRTY_MILLIS,
        HISTORY_SAVE_MUTATION_THRESHOLD,
    )
    .expect("native browsing-history save policy is valid")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WorkerNavigationTarget {
    window: BrowserWindowId,
    tab: TabId,
    navigation: NavigationId,
}

impl WorkerNavigationTarget {
    const fn new(window: BrowserWindowId, tab: TabId, navigation: NavigationId) -> Self {
        Self {
            window,
            tab,
            navigation,
        }
    }
}

enum WorkerNavigationOutcome {
    Committed {
        location: String,
        status: u16,
        source_bytes: usize,
    },
    Failed {
        message: String,
    },
    InternalFailure {
        message: String,
    },
    Stale,
}

#[derive(Clone, Copy)]
struct PendingWorkerNavigation {
    target: WorkerNavigationTarget,
    engine: EngineNavigationRequest,
}

fn spawn_http_smoke_server() -> Result<String, std::io::Error> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    thread::Builder::new()
        .name("zorya-http-smoke".into())
        .spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                HTTP_SMOKE_BODY.len(),
                HTTP_SMOKE_BODY
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        })?;
    Ok(format!("http://{address}/zorya-http-smoke"))
}

fn profile_root_from_local_app_data(local_app_data: Option<OsString>) -> io::Result<PathBuf> {
    let local_app_data = local_app_data
        .filter(|value| !value.as_os_str().is_empty())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "LOCALAPPDATA is unavailable for the default Zorya profile",
            )
        })?;
    Ok(PathBuf::from(local_app_data)
        .join(PRODUCT_DATA_DIRECTORY)
        .join(PROFILES_DIRECTORY)
        .join(DEFAULT_PROFILE_DIRECTORY))
}

fn default_profile_root() -> io::Result<PathBuf> {
    profile_root_from_local_app_data(std::env::var_os("LOCALAPPDATA"))
}

fn current_unix_millis() -> Result<u64, String> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock precedes Unix epoch: {error}"))?;
    u64::try_from(elapsed.as_millis())
        .map_err(|_| "system time exceeds browsing-history timestamp range".to_string())
}

fn record_profile_navigation_at(
    runtime: &mut ProfileRuntime,
    visited_unix_millis: u64,
    commit: BrowserNavigationCommit,
) -> Result<(), String> {
    if commit.location() == START_LOCATION {
        return Ok(());
    }
    let profile = runtime
        .active_profile()
        .ok_or_else(|| "no active profile for committed navigation history".to_string())?
        .id();
    runtime
        .record_committed_navigation(profile, visited_unix_millis, commit)
        .map(|_| ())
        .map_err(|error| {
            format!("failed to record committed navigation in active profile: {error}")
        })
}

pub(crate) fn run(mode: RunMode) -> Result<(), Box<dyn Error>> {
    let http_smoke_location = if mode == RunMode::ExitAfterRealHttpNavigation {
        Some(spawn_http_smoke_server()?)
    } else {
        None
    };
    let mut browser = BrowserApp::bootstrap()?;
    let browser_window = browser
        .windows()
        .next()
        .expect("browser bootstrap creates one window")
        .id();
    let tab = browser
        .window(browser_window)
        .and_then(|window| window.active_tab_id())
        .expect("browser bootstrap creates one active tab");
    let event_loop = EventLoop::<WorkerEvent>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let initial_navigation = browser
        .begin_navigation(browser_window, tab, START_LOCATION)?
        .intent()
        .id();
    let proxy = event_loop.create_proxy();
    let mut profile_runtime = ProfileRuntime::new();
    let initial_profile_selection = profile_runtime
        .begin_selection(default_profile_root()?)?
        .into_intent();
    let profile_worker = ProfileWorker::spawn({
        let proxy = proxy.clone();
        move |completion| {
            let _ = proxy.send_event(WorkerEvent::Profile(completion));
        }
    })?;
    let startup = NativeShellStartup {
        browser,
        browser_window,
        tab,
        initial_navigation,
        profile_runtime,
        profile_worker,
        initial_profile_selection,
        http_smoke_location,
    };
    let mut shell = NativeShell::new(startup, proxy, mode);
    event_loop.run_app(&mut shell)?;

    if let Some(error) = shell.fatal_error.take() {
        return Err(std::io::Error::other(error).into());
    }

    Ok(())
}

enum WorkerEvent {
    Profile(ProfileWorkerCompletion),
    GpuReady {
        target: AsyncTarget,
        result: Result<Arc<WindowsGpuDevice>, String>,
    },
    Initialized {
        target: AsyncTarget,
        result: Result<(), String>,
    },
    ViewCreated {
        target: AsyncTarget,
        result: Result<(), String>,
    },
    ViewClosed {
        tab: TabId,
        result: Result<(), String>,
    },
    FrameFinished {
        target: AsyncTarget,
        permit: PresentationFramePermit,
        result: Result<FrameOutcome, String>,
    },
    SurfaceReplaced {
        target: AsyncTarget,
        result: Result<(), String>,
    },
    NavigationFinished {
        target: WorkerNavigationTarget,
        outcome: WorkerNavigationOutcome,
    },
    NavigationCancelFinished {
        target: WorkerNavigationTarget,
        result: Result<bool, String>,
    },
}

enum WorkerCommand {
    AttachInitialSurface {
        target: AsyncTarget,
        surface: WindowsGpuSurface,
    },
    CreateView {
        target: AsyncTarget,
    },
    CloseView {
        tab: TabId,
    },
    Render {
        target: AsyncTarget,
        permit: PresentationFramePermit,
        viewport: Viewport,
    },
    ReplaceSurface {
        target: AsyncTarget,
        surface: WindowsGpuSurface,
    },
    BeginNavigation {
        target: WorkerNavigationTarget,
        location: String,
    },
    CancelNavigation {
        target: WorkerNavigationTarget,
    },
}

enum FrameOutcome {
    Presented,
    SurfaceRecoveryNeeded(String),
}

struct WorkerHandle {
    sender: SyncSender<WorkerCommand>,
    thread: JoinHandle<()>,
    cancellation: CancellationToken,
}

impl WorkerHandle {
    fn spawn(
        target: AsyncTarget,
        generation: PresentationGeneration,
        proxy: EventLoopProxy<WorkerEvent>,
    ) -> Result<Self, String> {
        let (sender, receiver) = sync_channel(1);
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let thread = thread::Builder::new()
            .name("zorya-render".into())
            .spawn(move || {
                render_worker_main(target, generation, receiver, proxy, worker_cancellation)
            })
            .map_err(|error| format!("failed to start render worker: {error}"))?;

        Ok(Self {
            sender,
            thread,
            cancellation,
        })
    }

    fn attach_initial_surface(
        &self,
        target: AsyncTarget,
        surface: WindowsGpuSurface,
    ) -> Result<(), String> {
        self.send(WorkerCommand::AttachInitialSurface { target, surface })
    }

    fn create_view(&self, target: AsyncTarget) -> Result<(), String> {
        self.send(WorkerCommand::CreateView { target })
    }

    fn close_view(&self, tab: TabId) -> Result<(), String> {
        self.send(WorkerCommand::CloseView { tab })
    }

    fn render(
        &self,
        target: AsyncTarget,
        permit: PresentationFramePermit,
        viewport: Viewport,
    ) -> Result<(), String> {
        self.send(WorkerCommand::Render {
            target,
            permit,
            viewport,
        })
    }

    fn replace_surface(
        &self,
        target: AsyncTarget,
        surface: WindowsGpuSurface,
    ) -> Result<(), String> {
        self.send(WorkerCommand::ReplaceSurface { target, surface })
    }

    fn begin_navigation(
        &self,
        target: WorkerNavigationTarget,
        location: String,
    ) -> Result<(), String> {
        self.send(WorkerCommand::BeginNavigation { target, location })
    }

    fn cancel_navigation(&self, target: WorkerNavigationTarget) -> Result<(), String> {
        self.send(WorkerCommand::CancelNavigation { target })
    }

    fn send(&self, command: WorkerCommand) -> Result<(), String> {
        if self.cancellation.is_cancelled() {
            return Err("render worker is cancelled".into());
        }

        self.sender.try_send(command).map_err(|error| match error {
            TrySendError::Full(_) => "render worker command queue is full".into(),
            TrySendError::Disconnected(_) => "render worker is unavailable".into(),
        })
    }

    fn shutdown(self) {
        self.cancellation.cancel();
        drop(self.sender);
        drop(self.thread);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PendingNativeTabCreate {
    target: AsyncTarget,
    navigation: NavigationId,
    activate_after_create: bool,
}

struct NativeShellStartup {
    browser: BrowserApp,
    browser_window: BrowserWindowId,
    tab: TabId,
    initial_navigation: NavigationId,
    profile_runtime: ProfileRuntime,
    profile_worker: ProfileWorker,
    initial_profile_selection: ProfileSelectionIntent,
    http_smoke_location: Option<String>,
}

struct NativeShell {
    browser: BrowserApp,
    browser_window: BrowserWindowId,
    profile_runtime: ProfileRuntime,
    profile_worker: Option<ProfileWorker>,
    settings_scheduler: ProfileSettingsSaveScheduler,
    history_scheduler: ProfileHistorySaveScheduler,
    profile_clock: NativeProfileClock,
    initial_profile_selection: Option<ProfileSelectionIntent>,
    tab: TabId,
    presentation: TabPresentationHandoff,
    pending_target_permit: Option<TargetFramePermit>,
    surface_recovery_permit: Option<PresentationFramePermit>,
    pending_tab_create: Option<PendingNativeTabCreate>,
    pending_view_close: Option<TabId>,
    rapid_smoke_tabs: Vec<TabId>,
    restore_focus_after_activation: bool,
    modifiers: ModifiersState,
    initial_navigation: Option<NavigationId>,
    pending_navigation_cancel: Option<WorkerNavigationTarget>,
    http_smoke_location: Option<String>,
    http_smoke_navigation: Option<NavigationId>,
    http_smoke_committed: bool,
    http_smoke_frame: Option<AsyncTarget>,
    proxy: EventLoopProxy<WorkerEvent>,
    window: Option<Arc<Window>>,
    gpu: Option<Arc<WindowsGpuDevice>>,
    worker: Option<WorkerHandle>,
    requests: AsyncRequestSequence,
    pending_init: PendingRequest,
    pending_frame: PendingRequest,
    pending_surface: PendingRequest,
    worker_ready: bool,
    needs_redraw: bool,
    run_mode: RunMode,
    shutdown_requested: bool,
    pending_profile_lock_release: Option<ProfileLockOwner>,
    profile_lock_release_completed: bool,
    fatal_error: Option<String>,
}

impl NativeShell {
    fn new(
        startup: NativeShellStartup,
        proxy: EventLoopProxy<WorkerEvent>,
        run_mode: RunMode,
    ) -> Self {
        let NativeShellStartup {
            browser,
            browser_window,
            tab,
            initial_navigation,
            profile_runtime,
            profile_worker,
            initial_profile_selection,
            http_smoke_location,
        } = startup;
        Self {
            browser,
            browser_window,
            profile_runtime,
            profile_worker: Some(profile_worker),
            settings_scheduler: ProfileSettingsSaveScheduler::new(native_settings_save_policy()),
            history_scheduler: ProfileHistorySaveScheduler::new(native_history_save_policy()),
            profile_clock: NativeProfileClock::new(),
            initial_profile_selection: Some(initial_profile_selection),
            tab,
            presentation: TabPresentationHandoff::new(tab),
            pending_target_permit: None,
            surface_recovery_permit: None,
            pending_tab_create: None,
            pending_view_close: None,
            rapid_smoke_tabs: Vec::new(),
            restore_focus_after_activation: false,
            modifiers: ModifiersState::empty(),
            initial_navigation: Some(initial_navigation),
            pending_navigation_cancel: None,
            http_smoke_location,
            http_smoke_navigation: None,
            http_smoke_committed: false,
            http_smoke_frame: None,
            proxy,
            window: None,
            gpu: None,
            worker: None,
            requests: AsyncRequestSequence::new(),
            pending_init: PendingRequest::default(),
            pending_frame: PendingRequest::default(),
            pending_surface: PendingRequest::default(),
            worker_ready: false,
            needs_redraw: false,
            run_mode,
            shutdown_requested: false,
            pending_profile_lock_release: None,
            profile_lock_release_completed: false,
            fatal_error: None,
        }
    }

    fn record_navigation_commit(&mut self, commit: BrowserNavigationCommit) -> Result<(), String> {
        let visited_unix_millis = current_unix_millis()?;
        record_profile_navigation_at(&mut self.profile_runtime, visited_unix_millis, commit)?;
        self.drive_history_save(ProfileHistorySaveUrgency::Normal)
    }

    fn drive_settings_save(&mut self, urgency: ProfileSettingsSaveUrgency) -> Result<(), String> {
        let Some(profile) = self
            .profile_runtime
            .active_profile()
            .map(|profile| profile.id())
        else {
            return Ok(());
        };
        let now_millis = self.profile_clock.now_millis();
        let Some(intent) = self
            .settings_scheduler
            .poll(&mut self.profile_runtime, profile, now_millis, urgency)
            .map_err(|error| format!("failed to schedule profile-settings save: {error}"))?
        else {
            return Ok(());
        };
        let save = intent.id();
        let worker = self
            .profile_worker
            .as_ref()
            .ok_or_else(|| "profile worker is unavailable for profile-settings save".to_string())?;

        match worker.save_settings(intent) {
            Ok(()) => Ok(()),
            Err(error) => {
                let queue_full = error.is_full();
                let message = error.to_string();
                let intent = error.into_work();
                debug_assert_eq!(intent.id(), save);
                self.profile_runtime
                    .cancel_settings_save(save)
                    .map_err(|cancel| {
                        format!(
                            "failed to submit profile-settings save {}: {message}; failed to cancel pending save: {cancel}",
                            save.get()
                        )
                    })?;
                if queue_full {
                    Ok(())
                } else {
                    Err(format!(
                        "failed to submit profile-settings save {}: {message}",
                        save.get()
                    ))
                }
            }
        }
    }

    fn drive_history_save(&mut self, urgency: ProfileHistorySaveUrgency) -> Result<(), String> {
        let Some(profile) = self
            .profile_runtime
            .active_profile()
            .map(|profile| profile.id())
        else {
            return Ok(());
        };
        let now_millis = self.profile_clock.now_millis();
        let Some(intent) = self
            .history_scheduler
            .poll(&mut self.profile_runtime, profile, now_millis, urgency)
            .map_err(|error| format!("failed to schedule browsing-history save: {error}"))?
        else {
            return Ok(());
        };
        let save = intent.id();
        let worker = self
            .profile_worker
            .as_ref()
            .ok_or_else(|| "profile worker is unavailable for browsing-history save".to_string())?;

        match worker.save_history(intent) {
            Ok(()) => Ok(()),
            Err(error) => {
                let queue_full = error.is_full();
                let message = error.to_string();
                let intent = error.into_work();
                debug_assert_eq!(intent.id(), save);
                self.profile_runtime
                    .cancel_browsing_history_save(save)
                    .map_err(|cancel| {
                        format!(
                            "failed to submit browsing-history save {}: {message}; failed to cancel pending save: {cancel}",
                            save.get()
                        )
                    })?;
                if queue_full {
                    Ok(())
                } else {
                    Err(format!(
                        "failed to submit browsing-history save {}: {message}",
                        save.get()
                    ))
                }
            }
        }
    }

    fn settings_flush_complete(&self) -> Result<bool, String> {
        let Some(profile) = self
            .profile_runtime
            .active_profile()
            .map(|profile| profile.id())
        else {
            return Ok(true);
        };
        if self.profile_runtime.pending_settings_save().is_some() {
            return Ok(false);
        }
        self.profile_runtime
            .settings_is_dirty(profile)
            .map(|dirty| !dirty)
            .map_err(|error| format!("failed to inspect profile-settings dirty state: {error}"))
    }

    fn history_flush_complete(&self) -> Result<bool, String> {
        let Some(profile) = self
            .profile_runtime
            .active_profile()
            .map(|profile| profile.id())
        else {
            return Ok(true);
        };
        if self
            .profile_runtime
            .pending_browsing_history_save()
            .is_some()
        {
            return Ok(false);
        }
        self.profile_runtime
            .browsing_history_is_dirty(profile)
            .map(|dirty| !dirty)
            .map_err(|error| format!("failed to inspect browsing-history dirty state: {error}"))
    }

    fn profile_flush_complete(&self) -> Result<bool, String> {
        Ok(self.settings_flush_complete()? && self.history_flush_complete()?)
    }

    fn drive_profile_saves(&mut self, flush: bool) -> Result<(), String> {
        let settings_urgency = if flush {
            ProfileSettingsSaveUrgency::Flush
        } else {
            ProfileSettingsSaveUrgency::Normal
        };
        let history_urgency = if flush {
            ProfileHistorySaveUrgency::Flush
        } else {
            ProfileHistorySaveUrgency::Normal
        };
        self.drive_settings_save(settings_urgency)?;
        self.drive_history_save(history_urgency)
    }

    fn continue_shutdown_after_profile_flush(&mut self, event_loop: &ActiveEventLoop) {
        if self.profile_lock_release_completed {
            self.finish_shutdown(event_loop);
            return;
        }
        if self.pending_profile_lock_release.is_some() {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }

        let Some(lock) = self
            .profile_runtime
            .active_profile()
            .map(|profile| profile.profile_lock().clone())
        else {
            self.finish_shutdown(event_loop);
            return;
        };
        let owner = lock.owner();
        let Some(worker) = self.profile_worker.as_ref() else {
            if self.fatal_error.is_none() {
                self.fatal_error =
                    Some("profile worker is unavailable for profile-lock release".to_string());
            }
            self.finish_shutdown(event_loop);
            return;
        };

        match worker.release_lock(lock) {
            Ok(()) => {
                self.pending_profile_lock_release = Some(owner);
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            Err(error) if error.is_full() => {
                let returned = error.into_work();
                debug_assert_eq!(returned.owner(), owner);
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            Err(error) => {
                let message = error.to_string();
                let returned = error.into_work();
                debug_assert_eq!(returned.owner(), owner);
                if self.fatal_error.is_none() {
                    self.fatal_error = Some(format!(
                        "failed to submit profile-lock release for owner {}:{}: {message}",
                        owner.process_id(),
                        owner.owner_id()
                    ));
                }
                self.finish_shutdown(event_loop);
            }
        }
    }

    fn submit_initial_profile_selection(&mut self) -> Result<(), String> {
        if self.initial_profile_selection.is_none() {
            return Ok(());
        }
        let worker = self.profile_worker.as_ref().ok_or_else(|| {
            "profile worker is unavailable before initial profile selection".to_string()
        })?;
        let intent = self
            .initial_profile_selection
            .take()
            .expect("initial profile selection was checked");
        let selection = intent.id();

        match worker.prepare(intent) {
            Ok(()) => Ok(()),
            Err(error) => {
                let message = error.to_string();
                let intent = error.into_work();
                debug_assert_eq!(intent.id(), selection);
                self.profile_runtime
                    .cancel_selection(selection)
                    .map_err(|cancel| {
                        format!(
                            "failed to submit initial profile preparation: {message}; failed to cancel selection: {cancel}"
                        )
                    })?;
                Err(format!(
                    "failed to submit initial profile preparation: {message}"
                ))
            }
        }
    }

    fn handle_profile_worker_completion(
        &mut self,
        event_loop: &ActiveEventLoop,
        completion: ProfileWorkerCompletion,
    ) {
        match completion {
            ProfileWorkerCompletion::Prepared { selection, result } => {
                let prepared = match result {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        let cancellation = self.profile_runtime.cancel_selection(selection);
                        let message = match cancellation {
                            Ok(_) => format!(
                                "failed to prepare profile selection {}: {error}",
                                selection.get()
                            ),
                            Err(cancel) => format!(
                                "failed to prepare profile selection {}: {error}; failed to cancel selection: {cancel}",
                                selection.get()
                            ),
                        };
                        self.fail(event_loop, message);
                        return;
                    }
                };

                if prepared.selection() != selection {
                    self.fail(
                        event_loop,
                        format!(
                            "profile worker completion {} returned prepared selection {}",
                            selection.get(),
                            prepared.selection().get()
                        ),
                    );
                    return;
                }

                if let Err(error) = self.profile_runtime.commit_selection(prepared) {
                    self.fail(
                        event_loop,
                        format!(
                            "failed to commit prepared profile selection {}: {error}",
                            selection.get()
                        ),
                    );
                    return;
                }

                if let Err(error) = self.initialize(event_loop) {
                    self.fail(event_loop, error);
                }
            }
            ProfileWorkerCompletion::LockReleased { owner, result } => {
                let expected = self.pending_profile_lock_release;
                if expected != Some(owner) {
                    if self.fatal_error.is_none() {
                        self.fatal_error = Some(format!(
                            "profile-lock release {}:{} is stale; expected {:?}",
                            owner.process_id(),
                            owner.owner_id(),
                            expected
                        ));
                    }
                    self.finish_shutdown(event_loop);
                    return;
                }
                self.pending_profile_lock_release = None;
                self.profile_lock_release_completed = true;
                if let Err(error) = result {
                    if self.fatal_error.is_none() {
                        self.fatal_error = Some(format!(
                            "profile-lock release {}:{} failed on profile worker: {error}",
                            owner.process_id(),
                            owner.owner_id()
                        ));
                    }
                }
                self.finish_shutdown(event_loop);
            }
            ProfileWorkerCompletion::SettingsSaved(completion) => {
                let save = completion.id();
                let storage_error = completion.result().as_ref().err().map(ToString::to_string);
                match self.profile_runtime.complete_settings_save(completion) {
                    Ok(_) => {
                        if let Some(error) = storage_error {
                            if self.fatal_error.is_none() {
                                self.fatal_error = Some(format!(
                                    "profile-settings save {} failed on profile worker: {error}",
                                    save.get()
                                ));
                            }
                            self.begin_shutdown();
                            self.finish_shutdown(event_loop);
                            return;
                        }

                        if let Err(error) = self.drive_profile_saves(self.shutdown_requested) {
                            self.fail(event_loop, error);
                            return;
                        }
                        if self.shutdown_requested {
                            match self.profile_flush_complete() {
                                Ok(true) => self.continue_shutdown_after_profile_flush(event_loop),
                                Ok(false) => {}
                                Err(error) => self.fail(event_loop, error),
                            }
                        }
                    }
                    Err(error) => self.fail(
                        event_loop,
                        format!(
                            "failed to reconcile profile-settings save {}: {error}",
                            save.get()
                        ),
                    ),
                }
            }
            ProfileWorkerCompletion::HistorySaved(completion) => {
                let save = completion.id();
                let storage_error = completion.result().as_ref().err().map(ToString::to_string);
                match self
                    .profile_runtime
                    .complete_browsing_history_save(completion)
                {
                    Ok(_) => {
                        if let Some(error) = storage_error {
                            if self.fatal_error.is_none() {
                                self.fatal_error = Some(format!(
                                    "browsing-history save {} failed on profile worker: {error}",
                                    save.get()
                                ));
                            }
                            self.begin_shutdown();
                            self.finish_shutdown(event_loop);
                            return;
                        }

                        if let Err(error) = self.drive_profile_saves(self.shutdown_requested) {
                            self.fail(event_loop, error);
                            return;
                        }
                        if self.shutdown_requested {
                            match self.profile_flush_complete() {
                                Ok(true) => self.continue_shutdown_after_profile_flush(event_loop),
                                Ok(false) => {}
                                Err(error) => self.fail(event_loop, error),
                            }
                        }
                    }
                    Err(error) => self.fail(
                        event_loop,
                        format!(
                            "failed to reconcile browsing-history save {}: {error}",
                            save.get()
                        ),
                    ),
                }
            }
        }
    }

    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        if let Some(window) = &self.window {
            if self.worker_ready && !self.pending_surface.is_pending() {
                window.request_redraw();
            }
            return Ok(());
        }

        let application_icon = application_icon()?;
        let icon = Icon::from_rgba(
            application_icon.rgba,
            application_icon.width,
            application_icon.height,
        )
        .map_err(|error| format!("failed to build Zorya application icon: {error}"))?;
        let attributes = Window::default_attributes()
            .with_title("Zorya")
            .with_window_icon(Some(icon))
            .with_inner_size(LogicalSize::new(1100.0, 760.0));
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(|error| format!("failed to create native window: {error}"))?,
        );
        let target = self
            .requests
            .allocate(self.browser_window, self.tab)
            .map_err(|error| error.to_string())?;
        self.pending_init
            .begin(target)
            .map_err(|error| error.to_string())?;

        let worker =
            match WorkerHandle::spawn(target, self.presentation.generation(), self.proxy.clone()) {
                Ok(worker) => worker,
                Err(error) => {
                    self.pending_init.complete_if_current(target);
                    return Err(error);
                }
            };

        self.window = Some(window);
        self.worker = Some(worker);
        Ok(())
    }

    fn target_alive(&self, target: AsyncTarget) -> bool {
        self.browser.window(target.window()).is_some_and(|window| {
            window
                .tabs()
                .iter()
                .any(|candidate| candidate.id() == target.tab())
        })
    }

    fn navigation_target_is_current(&self, target: WorkerNavigationTarget) -> bool {
        self.browser
            .window(target.window)
            .and_then(|window| window.tab(target.tab))
            .and_then(|tab| tab.navigation().pending())
            .is_some_and(|pending| pending.id() == target.navigation)
    }

    fn dispatch_navigation_start(
        &mut self,
        tab: TabId,
        start: NavigationStart,
    ) -> Result<NavigationId, String> {
        let navigation = start.intent().id();
        let target = WorkerNavigationTarget::new(self.browser_window, tab, navigation);
        let location = start.intent().requested_location().to_owned();
        let result = self
            .worker
            .as_ref()
            .ok_or_else(|| "render worker is unavailable".to_string())
            .and_then(|worker| worker.begin_navigation(target, location));
        if let Err(error) = result {
            let _ = self.browser.fail_navigation(
                self.browser_window,
                tab,
                navigation,
                format!("native navigation dispatch failed: {error}"),
            );
            return Err(error);
        }
        Ok(navigation)
    }

    fn dispatch_http_smoke_frame(&mut self) -> Result<(), String> {
        if !self.http_smoke_committed || self.http_smoke_frame.is_some() {
            return Ok(());
        }
        if self.pending_frame.is_pending() {
            self.needs_redraw = true;
            return Ok(());
        }

        self.needs_redraw = true;
        self.start_frame()?;
        let target = self
            .pending_frame
            .current()
            .ok_or_else(|| "HTTP smoke post-commit frame was not dispatched".to_string())?;
        if target.tab() != self.tab {
            return Err(format!(
                "HTTP smoke post-commit frame targeted tab {} instead of {}",
                target.tab().get(),
                self.tab.get()
            ));
        }
        self.http_smoke_frame = Some(target);
        Ok(())
    }

    fn start_http_smoke_navigation(&mut self) -> Result<(), String> {
        if self.run_mode != RunMode::ExitAfterRealHttpNavigation {
            return Err("HTTP navigation smoke started outside its run mode".into());
        }
        if self.http_smoke_navigation.is_some() {
            return Err("HTTP navigation smoke is already running".into());
        }
        let location = self
            .http_smoke_location
            .clone()
            .ok_or_else(|| "HTTP navigation smoke server is unavailable".to_string())?;
        let start = self
            .browser
            .begin_navigation(self.browser_window, self.tab, location)
            .map_err(|error| format!("failed to begin HTTP smoke navigation: {error}"))?;
        let navigation = self.dispatch_navigation_start(self.tab, start)?;
        self.http_smoke_navigation = Some(navigation);
        Ok(())
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn create_surface(&self) -> Result<WindowsGpuSurface, String> {
        let window = self
            .window
            .as_ref()
            .ok_or_else(|| "native window is unavailable".to_string())?;
        let gpu = self
            .gpu
            .as_ref()
            .ok_or_else(|| "Windows GPU device is unavailable".to_string())?;
        let size = window.inner_size();

        gpu.create_surface(Arc::clone(window), size.width, size.height)
            .map_err(|error| format!("failed to create Web content surface: {error}"))
    }

    fn attach_initial_surface(&mut self, target: AsyncTarget) -> Result<(), String> {
        let surface = self.create_surface()?;
        self.worker
            .as_ref()
            .ok_or_else(|| "render worker is unavailable".to_string())?
            .attach_initial_surface(target, surface)
    }

    fn start_frame(&mut self) -> Result<(), String> {
        if !self.worker_ready {
            return Ok(());
        }
        if self.pending_frame.is_pending()
            || self.pending_surface.is_pending()
            || self.pending_tab_create.is_some()
        {
            self.needs_redraw = true;
            return Ok(());
        }
        if self.pending_target_permit.is_some() {
            return self.dispatch_pending_target_frame();
        }

        let permit = match self.presentation.authorize_current_frame(self.tab) {
            Ok(permit) => PresentationFramePermit::from(permit),
            Err(
                PresentationHandoffError::PendingActivationBlocksCurrentFrame { .. }
                | PresentationHandoffError::CurrentFrameContentNeutral { .. },
            ) => {
                self.needs_redraw = true;
                return Ok(());
            }
            Err(error) => return Err(error.to_string()),
        };
        self.start_frame_with_permit(permit)
    }

    fn start_frame_with_permit(&mut self, permit: PresentationFramePermit) -> Result<(), String> {
        if !self.worker_ready {
            return Err("render worker is not ready".into());
        }
        if self.pending_frame.is_pending() || self.pending_surface.is_pending() {
            return Err("cannot dispatch a frame while presentation work is pending".into());
        }

        let window = self
            .window
            .as_ref()
            .ok_or_else(|| "native window is unavailable".to_string())?;
        let size = window.inner_size();
        let viewport = Viewport::new(size.width, size.height);
        if viewport.is_suspended() {
            if let PresentationFramePermit::Target(target) = permit {
                self.pending_target_permit = Some(target);
            }
            self.needs_redraw = true;
            return Ok(());
        }

        let target = self
            .requests
            .allocate(self.browser_window, permit.tab())
            .map_err(|error| error.to_string())?;
        self.pending_frame
            .begin(target)
            .map_err(|error| error.to_string())?;

        let render_result = self
            .worker
            .as_ref()
            .ok_or_else(|| "render worker is unavailable".to_string())
            .and_then(|worker| worker.render(target, permit, viewport));
        if let Err(error) = render_result {
            self.pending_frame.complete_if_current(target);
            return Err(error);
        }

        self.needs_redraw = false;
        Ok(())
    }

    fn dispatch_pending_target_frame(&mut self) -> Result<(), String> {
        if self.pending_frame.is_pending() || self.pending_surface.is_pending() {
            return Ok(());
        }
        let Some(permit) = self.pending_target_permit.take() else {
            return Ok(());
        };
        self.start_frame_with_permit(PresentationFramePermit::from(permit))
    }

    fn enter_native_neutral(&mut self) -> Result<(), String> {
        let window = self
            .window
            .as_ref()
            .ok_or_else(|| "native window is unavailable".to_string())?;

        if window.is_visible() != Some(false) {
            self.restore_focus_after_activation |= window.has_focus();
            window.set_visible(false);
        }
        if window.is_visible() != Some(false) {
            return Err("native window did not confirm the browser-owned neutral state".into());
        }
        Ok(())
    }

    fn leave_native_neutral(&mut self) -> Result<(), String> {
        let window = self
            .window
            .as_ref()
            .ok_or_else(|| "native window is unavailable".to_string())?;
        window.set_visible(true);
        if window.is_visible() != Some(true) {
            return Err("native window did not confirm the target presentation state".into());
        }

        if self.restore_focus_after_activation {
            window.focus_window();
        }
        self.restore_focus_after_activation = false;
        Ok(())
    }

    fn native_neutral_confirmed(&self) -> bool {
        self.presentation.content() == WebContentPresentation::Neutral
            && self.window.as_ref().and_then(|window| window.is_visible()) == Some(false)
    }

    fn begin_native_tab_activation(&mut self, start: TabActivationStart) -> Result<(), String> {
        let intent = start.intent();
        if intent.from() == intent.to() {
            self.browser
                .cancel_tab_activation(self.browser_window, intent.id())
                .map_err(|error| format!("failed to cancel no-op tab activation: {error}"))?;
            return Ok(());
        }

        self.presentation
            .begin_activation(intent)
            .map_err(|error| format!("failed to begin presentation handoff: {error}"))?;
        self.enter_native_neutral()?;
        self.presentation
            .confirm_neutral(intent.id())
            .map_err(|error| format!("failed to confirm neutral presentation state: {error}"))?;

        let committed = self
            .browser
            .commit_tab_activation(self.browser_window, intent.id())
            .map_err(|error| format!("failed to commit product tab activation: {error}"))?;
        if committed != intent.to() {
            return Err(format!(
                "product activation committed tab {} instead of target {}",
                committed.get(),
                intent.to().get()
            ));
        }

        self.presentation
            .acknowledge_chrome_commit(intent.id(), committed)
            .map_err(|error| format!("failed to acknowledge target chrome commit: {error}"))?;
        let permit = self
            .presentation
            .authorize_target_frame(intent.id(), committed)
            .map_err(|error| format!("failed to authorize target frame: {error}"))?;

        self.tab = committed;
        self.pending_target_permit = Some(permit);
        self.dispatch_pending_target_frame()
    }

    fn start_new_tab(&mut self) -> Result<(), String> {
        self.start_new_tab_with_activation(true)
    }

    fn start_new_tab_with_activation(&mut self, activate_after_create: bool) -> Result<(), String> {
        if !self.worker_ready
            || self.pending_tab_create.is_some()
            || self.pending_frame.is_pending()
            || self.pending_surface.is_pending()
            || self.presentation.pending_activation().is_some()
        {
            return Ok(());
        }

        let effect = self
            .browser
            .dispatch_browser_command(self.browser_window, BrowserCommand::NewTab)
            .map_err(|error| format!("failed to create browser tab: {error}"))?;
        let BrowserCommandEffect::TabCreated(tab) = effect else {
            return Err("new-tab command returned an unexpected effect".into());
        };
        let navigation =
            match self
                .browser
                .begin_navigation(self.browser_window, tab, START_LOCATION)
            {
                Ok(start) => start.intent().id(),
                Err(error) => {
                    let _ = self.browser.close_tab(self.browser_window, tab);
                    return Err(format!("failed to begin new-tab navigation: {error}"));
                }
            };
        let target = match self.requests.allocate(self.browser_window, tab) {
            Ok(target) => target,
            Err(error) => {
                let _ = self.browser.fail_navigation(
                    self.browser_window,
                    tab,
                    navigation,
                    "native tab creation request allocation failed",
                );
                let _ = self.browser.close_tab(self.browser_window, tab);
                return Err(error.to_string());
            }
        };

        let create_result = self
            .worker
            .as_ref()
            .ok_or_else(|| "render worker is unavailable".to_string())
            .and_then(|worker| worker.create_view(target));
        if let Err(error) = create_result {
            let _ = self.browser.fail_navigation(
                self.browser_window,
                tab,
                navigation,
                "native Rarog View creation could not be dispatched",
            );
            let _ = self.browser.close_tab(self.browser_window, tab);
            return Err(error);
        }

        self.pending_tab_create = Some(PendingNativeTabCreate {
            target,
            navigation,
            activate_after_create,
        });
        Ok(())
    }

    fn start_rapid_tab_activation_smoke(&mut self) -> Result<(), String> {
        if self.run_mode != RunMode::ExitAfterRapidTabActivation {
            return Err("rapid activation smoke started outside its run mode".into());
        }
        if self.rapid_smoke_tabs.len() != 2 {
            return Err(format!(
                "rapid activation smoke requires two background tabs; found {}",
                self.rapid_smoke_tabs.len()
            ));
        }

        let source = self.tab;
        let first = self.rapid_smoke_tabs[0];
        let second = self.rapid_smoke_tabs[1];
        if source == first || source == second || first == second {
            return Err("rapid activation smoke tab identities are not distinct".into());
        }

        let first_start = self
            .browser
            .begin_tab_activation(self.browser_window, first)
            .map_err(|error| format!("failed to begin first rapid activation: {error}"))?;
        self.begin_native_tab_activation(first_start)?;

        let first_frame = self
            .pending_frame
            .current()
            .ok_or_else(|| "first rapid target frame was not dispatched".to_string())?;
        if first_frame.tab() != first {
            return Err(format!(
                "first rapid target frame belongs to tab {} instead of {}",
                first_frame.tab().get(),
                first.get()
            ));
        }
        if self.pending_target_permit.is_some() {
            return Err("first rapid activation unexpectedly remained queued".into());
        }

        let second_start = self
            .browser
            .begin_tab_activation(self.browser_window, second)
            .map_err(|error| format!("failed to begin second rapid activation: {error}"))?;
        self.begin_native_tab_activation(second_start)?;

        if self.pending_frame.current() != Some(first_frame) {
            return Err("second rapid activation replaced the already in-flight B frame".into());
        }
        let queued = self
            .pending_target_permit
            .ok_or_else(|| "second rapid target permit was not queued".to_string())?;
        if queued.tab() != second {
            return Err(format!(
                "queued rapid target permit belongs to tab {} instead of {}",
                queued.tab().get(),
                second.get()
            ));
        }

        let active = self
            .browser
            .window(self.browser_window)
            .and_then(|window| window.active_tab_id());
        if active != Some(second)
            || self.tab != second
            || self.presentation.represented_tab() != second
            || self.presentation.content() != WebContentPresentation::Neutral
            || !self.native_neutral_confirmed()
        {
            return Err(
                "rapid supersession did not leave only C committed over neutral content".into(),
            );
        }

        Ok(())
    }

    fn start_close_tab(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        if !self.worker_ready
            || self.pending_tab_create.is_some()
            || self.pending_surface.is_pending()
            || self.pending_view_close.is_some()
            || self.pending_target_permit.is_some()
            || self.presentation.pending_activation().is_some()
        {
            return Ok(());
        }

        let effect = self
            .browser
            .dispatch_browser_command(self.browser_window, BrowserCommand::CloseTab)
            .map_err(|error| format!("failed to begin browser tab close: {error}"))?;
        let BrowserCommandEffect::TabCloseStarted(start) = effect else {
            return Err("close-tab command returned an unexpected effect".into());
        };

        match start {
            TabCloseStart::Closed(result) => {
                let closed = result.tab().id();
                if closed == self.tab {
                    return Err("active native tab closed without a presentation handoff".into());
                }
                self.worker
                    .as_ref()
                    .ok_or_else(|| "render worker is unavailable".to_string())?
                    .close_view(closed)
            }
            TabCloseStart::ActiveWithFallback(close) => {
                let intent = close.activation().intent();
                self.presentation
                    .begin_activation(intent)
                    .map_err(|error| {
                        format!("failed to begin close presentation handoff: {error}")
                    })?;
                self.enter_native_neutral()?;
                self.presentation
                    .confirm_neutral(intent.id())
                    .map_err(|error| format!("failed to confirm close neutral state: {error}"))?;

                let result = self
                    .browser
                    .commit_active_tab_close_after_neutral(
                        self.browser_window,
                        close,
                        &self.presentation,
                    )
                    .map_err(|error| format!("failed to commit active tab close: {error}"))?;
                let committed = result
                    .active_tab()
                    .ok_or_else(|| "active close lost its fallback tab".to_string())?;
                if committed != close.fallback_tab() {
                    return Err(format!(
                        "active close committed fallback tab {} instead of {}",
                        committed.get(),
                        close.fallback_tab().get()
                    ));
                }

                self.presentation
                    .acknowledge_chrome_commit(intent.id(), committed)
                    .map_err(|error| format!("failed to acknowledge fallback chrome: {error}"))?;
                let permit = self
                    .presentation
                    .authorize_target_frame(intent.id(), committed)
                    .map_err(|error| format!("failed to authorize fallback frame: {error}"))?;

                self.tab = committed;
                self.pending_view_close = Some(close.closing_tab());
                self.pending_target_permit = Some(permit);
                self.dispatch_pending_target_frame()
            }
            TabCloseStart::ActiveLast(close) => {
                self.enter_native_neutral()?;
                self.presentation
                    .confirm_current_tab_neutral(close.closing_tab())
                    .map_err(|error| {
                        format!("failed to confirm last-tab neutral state: {error}")
                    })?;
                self.browser
                    .commit_last_tab_close_after_neutral(
                        self.browser_window,
                        close,
                        &self.presentation,
                    )
                    .map_err(|error| format!("failed to commit last-tab close: {error}"))?;
                self.shutdown(event_loop);
                Ok(())
            }
        }
    }

    fn handle_browser_command(&mut self, command: BrowserCommand) -> Result<(), String> {
        match command {
            BrowserCommand::NewTab => self.start_new_tab(),
            BrowserCommand::Back | BrowserCommand::Forward => {
                if self.pending_navigation_cancel.is_some() {
                    return Ok(());
                }
                let tab = self
                    .browser
                    .window(self.browser_window)
                    .and_then(|window| window.active_tab_id())
                    .ok_or_else(|| "browser window has no active tab".to_string())?;
                let effect = self
                    .browser
                    .dispatch_browser_command(self.browser_window, command)
                    .map_err(|error| format!("failed to dispatch navigation command: {error}"))?;
                match effect {
                    BrowserCommandEffect::NavigationStarted(start) => {
                        self.dispatch_navigation_start(tab, start).map(|_| ())
                    }
                    BrowserCommandEffect::Unavailable => Ok(()),
                    _ => Err("navigation command returned an unexpected effect".into()),
                }
            }
            BrowserCommand::ReloadOrStop => self.handle_reload_or_stop(),
            BrowserCommand::CycleTab(direction) => {
                if self.pending_tab_create.is_some() {
                    return Ok(());
                }
                let effect = self
                    .browser
                    .dispatch_browser_command(
                        self.browser_window,
                        BrowserCommand::CycleTab(direction),
                    )
                    .map_err(|error| format!("failed to cycle browser tab: {error}"))?;
                match effect {
                    BrowserCommandEffect::TabActivationStarted(start) => {
                        self.begin_native_tab_activation(start)
                    }
                    BrowserCommandEffect::Unavailable => Ok(()),
                    _ => Err("tab-cycle command returned an unexpected effect".into()),
                }
            }
            BrowserCommand::CloseTab | BrowserCommand::FocusAddressBar => Ok(()),
        }
    }

    fn handle_reload_or_stop(&mut self) -> Result<(), String> {
        if self.pending_navigation_cancel.is_some() {
            return Ok(());
        }
        let tab = self
            .browser
            .window(self.browser_window)
            .and_then(|window| window.active_tab_id())
            .ok_or_else(|| "browser window has no active tab".to_string())?;
        let pending = self
            .browser
            .window(self.browser_window)
            .and_then(|window| window.tab(tab))
            .and_then(|tab| tab.navigation().pending())
            .map(|intent| intent.id());

        if let Some(navigation) = pending {
            let target = WorkerNavigationTarget::new(self.browser_window, tab, navigation);
            self.worker
                .as_ref()
                .ok_or_else(|| "render worker is unavailable".to_string())?
                .cancel_navigation(target)?;
            self.pending_navigation_cancel = Some(target);
            return Ok(());
        }

        let effect = self
            .browser
            .dispatch_browser_command(self.browser_window, BrowserCommand::ReloadOrStop)
            .map_err(|error| format!("failed to dispatch reload command: {error}"))?;
        match effect {
            BrowserCommandEffect::NavigationStarted(start) => {
                self.dispatch_navigation_start(tab, start).map(|_| ())
            }
            BrowserCommandEffect::Unavailable => Ok(()),
            _ => Err("reload command returned an unexpected effect".into()),
        }
    }

    fn handle_keyboard_input(
        &mut self,
        event_loop: &ActiveEventLoop,
        event: winit::event::KeyEvent,
        is_synthetic: bool,
    ) -> Result<(), String> {
        if is_synthetic || event.state != ElementState::Pressed || event.repeat {
            return Ok(());
        }
        if self.modifiers.alt_key()
            && !self.modifiers.control_key()
            && !self.modifiers.super_key()
            && !self.modifiers.shift_key()
        {
            return match event.logical_key {
                Key::Named(NamedKey::ArrowLeft) => {
                    self.handle_browser_command(BrowserCommand::Back)
                }
                Key::Named(NamedKey::ArrowRight) => {
                    self.handle_browser_command(BrowserCommand::Forward)
                }
                _ => Ok(()),
            };
        }

        if !self.modifiers.control_key() || self.modifiers.alt_key() || self.modifiers.super_key() {
            if !self.modifiers.control_key()
                && !self.modifiers.alt_key()
                && !self.modifiers.super_key()
                && !self.modifiers.shift_key()
                && event.logical_key == Key::Named(NamedKey::Escape)
            {
                let is_loading = self
                    .browser
                    .window(self.browser_window)
                    .and_then(|window| window.active_tab())
                    .is_some_and(|tab| tab.navigation().is_loading());
                if is_loading {
                    return self.handle_browser_command(BrowserCommand::ReloadOrStop);
                }
            }
            return Ok(());
        }

        match event.logical_key {
            Key::Character(character)
                if !self.modifiers.shift_key() && character.as_str().eq_ignore_ascii_case("t") =>
            {
                self.handle_browser_command(BrowserCommand::NewTab)
            }
            Key::Character(character)
                if !self.modifiers.shift_key() && character.as_str().eq_ignore_ascii_case("w") =>
            {
                self.start_close_tab(event_loop)
            }
            Key::Character(character)
                if !self.modifiers.shift_key() && character.as_str().eq_ignore_ascii_case("r") =>
            {
                self.handle_browser_command(BrowserCommand::ReloadOrStop)
            }
            Key::Named(NamedKey::Tab) => {
                let direction = if self.modifiers.shift_key() {
                    TabCycleDirection::Previous
                } else {
                    TabCycleDirection::Next
                };
                self.handle_browser_command(BrowserCommand::CycleTab(direction))
            }
            _ => Ok(()),
        }
    }

    fn start_surface_recovery(&mut self, permit: PresentationFramePermit) -> Result<(), String> {
        let target = self
            .requests
            .allocate(self.browser_window, permit.tab())
            .map_err(|error| error.to_string())?;
        self.pending_surface
            .begin(target)
            .map_err(|error| error.to_string())?;
        self.surface_recovery_permit = Some(permit);

        let surface = match self.create_surface() {
            Ok(surface) => surface,
            Err(error) => {
                self.pending_surface.complete_if_current(target);
                self.surface_recovery_permit = None;
                return Err(error);
            }
        };

        let replacement_result = self
            .worker
            .as_ref()
            .ok_or_else(|| "render worker is unavailable".to_string())
            .and_then(|worker| worker.replace_surface(target, surface));
        if let Err(error) = replacement_result {
            self.pending_surface.complete_if_current(target);
            self.surface_recovery_permit = None;
            return Err(error);
        }

        Ok(())
    }

    fn complete_native_tab_create(
        &mut self,
        target: AsyncTarget,
        result: Result<(), String>,
    ) -> Result<(), String> {
        let Some(pending) = self.pending_tab_create else {
            return Ok(());
        };
        if pending.target != target {
            return Ok(());
        }
        self.pending_tab_create = None;

        if !self.target_alive(target) {
            return Ok(());
        }

        if let Err(error) = result {
            let _ = self.browser.fail_navigation(
                self.browser_window,
                target.tab(),
                pending.navigation,
                &error,
            );
            let _ = self.browser.close_tab(self.browser_window, target.tab());
            return Err(error);
        }

        let commit = self
            .browser
            .commit_navigation(
                self.browser_window,
                target.tab(),
                pending.navigation,
                START_LOCATION,
            )
            .map_err(|error| format!("failed to commit new-tab navigation: {error}"))?;
        self.record_navigation_commit(commit)?;

        if pending.activate_after_create {
            let start = self
                .browser
                .begin_tab_activation(self.browser_window, target.tab())
                .map_err(|error| format!("failed to activate newly created tab: {error}"))?;
            return self.begin_native_tab_activation(start);
        }

        self.rapid_smoke_tabs.push(target.tab());
        match self.rapid_smoke_tabs.len() {
            1 => self.start_new_tab_with_activation(false),
            2 => self.start_rapid_tab_activation_smoke(),
            count => Err(format!(
                "rapid tab activation smoke created unexpected tab count {count}"
            )),
        }
    }

    fn handle_worker_event(&mut self, event_loop: &ActiveEventLoop, event: WorkerEvent) {
        match event {
            WorkerEvent::Profile(completion) => {
                self.handle_profile_worker_completion(event_loop, completion);
            }
            WorkerEvent::GpuReady { target, result } => {
                if !self.pending_init.is_current(target) || !self.target_alive(target) {
                    return;
                }

                match result {
                    Ok(gpu) => {
                        self.gpu = Some(gpu);
                        if let Err(error) = self.attach_initial_surface(target) {
                            self.fail(event_loop, error);
                        }
                    }
                    Err(error) => {
                        self.pending_init.complete_if_current(target);
                        self.fail(event_loop, error);
                    }
                }
            }
            WorkerEvent::Initialized { target, result } => {
                if !self.pending_init.complete_if_current(target) || !self.target_alive(target) {
                    return;
                }

                match result {
                    Ok(()) => {
                        if let Err(error) = self.commit_initial_navigation() {
                            self.fail(event_loop, error);
                            return;
                        }
                        self.worker_ready = true;
                        self.needs_redraw = true;
                        self.request_redraw();
                    }
                    Err(error) => self.fail(event_loop, error),
                }
            }
            WorkerEvent::ViewCreated { target, result } => {
                if let Err(error) = self.complete_native_tab_create(target, result) {
                    self.fail(event_loop, error);
                }
            }
            WorkerEvent::ViewClosed { tab, result } => {
                if self.pending_view_close != Some(tab) {
                    return;
                }
                match result {
                    Ok(()) => {
                        self.pending_view_close = None;
                        if self.run_mode == RunMode::ExitAfterTabClose {
                            self.shutdown(event_loop);
                            return;
                        }
                        if self.presentation.pending_activation().is_none()
                            && self.presentation.content() == WebContentPresentation::Tab(self.tab)
                        {
                            if let Err(error) = self.leave_native_neutral() {
                                self.fail(event_loop, error);
                            }
                        }
                    }
                    Err(error) => self.fail(event_loop, error),
                }
            }
            WorkerEvent::NavigationFinished { target, outcome } => {
                if let WorkerNavigationOutcome::InternalFailure { message } = &outcome {
                    self.fail(
                        event_loop,
                        format!("internal navigation lifecycle failure: {message}"),
                    );
                    return;
                }
                if !self.navigation_target_is_current(target) {
                    return;
                }

                match outcome {
                    WorkerNavigationOutcome::Committed {
                        location,
                        status,
                        source_bytes,
                    } => {
                        if self.http_smoke_navigation == Some(target.navigation) {
                            let expected = self.http_smoke_location.as_deref();
                            if status != 200
                                || source_bytes == 0
                                || expected != Some(location.as_str())
                            {
                                self.fail(
                                    event_loop,
                                    format!(
                                        "HTTP smoke committed unexpected response: status {status}, bytes {source_bytes}, location {location}"
                                    ),
                                );
                                return;
                            }
                        }
                        let commit = match self.browser.commit_navigation(
                            target.window,
                            target.tab,
                            target.navigation,
                            &location,
                        ) {
                            Ok(commit) => commit,
                            Err(error) => {
                                self.fail(
                                    event_loop,
                                    format!("failed to commit browser navigation: {error}"),
                                );
                                return;
                            }
                        };
                        if let Err(error) = self.record_navigation_commit(commit) {
                            self.fail(event_loop, error);
                            return;
                        }
                        if self.http_smoke_navigation == Some(target.navigation) {
                            self.http_smoke_committed = true;
                            if let Err(error) = self.dispatch_http_smoke_frame() {
                                self.fail(event_loop, error);
                            }
                        } else {
                            self.needs_redraw = true;
                            self.request_redraw();
                        }
                    }
                    WorkerNavigationOutcome::Failed { message } => {
                        if let Err(error) = self.browser.fail_navigation(
                            target.window,
                            target.tab,
                            target.navigation,
                            &message,
                        ) {
                            self.fail(
                                event_loop,
                                format!("failed to record browser navigation failure: {error}"),
                            );
                            return;
                        }
                        if self.http_smoke_navigation == Some(target.navigation) {
                            self.fail(
                                event_loop,
                                format!("HTTP smoke navigation failed: {message}"),
                            );
                            return;
                        }
                        self.needs_redraw = true;
                        self.request_redraw();
                    }
                    WorkerNavigationOutcome::InternalFailure { .. } => {
                        unreachable!(
                            "internal navigation failures are handled before stale filtering"
                        )
                    }
                    WorkerNavigationOutcome::Stale => {
                        self.fail(
                            event_loop,
                            "current Rarog navigation became stale before browser completion",
                        );
                    }
                }
            }
            WorkerEvent::NavigationCancelFinished { target, result } => {
                if self.pending_navigation_cancel != Some(target) {
                    return;
                }
                self.pending_navigation_cancel = None;
                match result {
                    Ok(true) => {
                        if !self.navigation_target_is_current(target) {
                            return;
                        }
                        let effect = self.browser.dispatch_browser_command(
                            self.browser_window,
                            BrowserCommand::ReloadOrStop,
                        );
                        match effect {
                            Ok(BrowserCommandEffect::NavigationStopped(intent))
                                if intent.id() == target.navigation => {}
                            Ok(_) => self.fail(
                                event_loop,
                                "navigation cancellation acknowledged for a different product state",
                            ),
                            Err(error) => self.fail(
                                event_loop,
                                format!("failed to stop cancelled browser navigation: {error}"),
                            ),
                        }
                    }
                    Ok(false) => {}
                    Err(error) => self.fail(
                        event_loop,
                        format!("failed to cancel exact Rarog navigation: {error}"),
                    ),
                }
            }
            WorkerEvent::FrameFinished {
                target,
                permit,
                result,
            } => {
                if permit.tab() != target.tab() || !self.pending_frame.complete_if_current(target) {
                    return;
                }

                if !self.target_alive(target) {
                    let retired_source = self.pending_view_close == Some(target.tab())
                        && self.native_neutral_confirmed();
                    if !retired_source {
                        self.fail(
                            event_loop,
                            format!(
                                "frame for closed tab {} completed outside a confirmed neutral close handoff",
                                target.tab().get()
                            ),
                        );
                        return;
                    }
                    if self.pending_target_permit.is_some() {
                        if let Err(error) = self.dispatch_pending_target_frame() {
                            self.fail(event_loop, error);
                        }
                    }
                    return;
                }

                match result {
                    Ok(FrameOutcome::Presented) => {
                        let mut completed_target = false;
                        if let PresentationFramePermit::Target(target_permit) = permit {
                            match self.presentation.present_target_frame(target_permit) {
                                Ok(_) => completed_target = true,
                                Err(
                                    PresentationHandoffError::StaleActivation { .. }
                                    | PresentationHandoffError::StaleTargetFramePermit { .. },
                                ) => {
                                    let hidden =
                                        self.window.as_ref().and_then(|window| window.is_visible())
                                            == Some(false);
                                    if self.presentation.content()
                                        != WebContentPresentation::Neutral
                                        || !hidden
                                    {
                                        self.fail(
                                            event_loop,
                                            "stale target frame completed outside a confirmed neutral native state",
                                        );
                                        return;
                                    }
                                }
                                Err(error) => {
                                    self.fail(event_loop, error);
                                    return;
                                }
                            }
                        }

                        if self.pending_target_permit.is_some() {
                            if let Err(error) = self.dispatch_pending_target_frame() {
                                self.fail(event_loop, error);
                            }
                            return;
                        }

                        if completed_target {
                            if self.run_mode == RunMode::ExitAfterTabActivation {
                                self.shutdown(event_loop);
                                return;
                            }
                            if self.run_mode == RunMode::ExitAfterRapidTabActivation {
                                let expected = self.rapid_smoke_tabs.last().copied();
                                let active = self
                                    .browser
                                    .window(self.browser_window)
                                    .and_then(|window| window.active_tab_id());
                                if expected != Some(self.tab)
                                    || active != Some(self.tab)
                                    || self.presentation.represented_tab() != self.tab
                                    || self.presentation.content()
                                        != WebContentPresentation::Tab(self.tab)
                                    || self.pending_target_permit.is_some()
                                {
                                    self.fail(
                                        event_loop,
                                        "rapid supersession did not finish with C as the sole presented target",
                                    );
                                    return;
                                }
                                self.shutdown(event_loop);
                                return;
                            }
                            if let Some(retired) = self.pending_view_close {
                                let close_result = self
                                    .worker
                                    .as_ref()
                                    .ok_or_else(|| "render worker is unavailable".to_string())
                                    .and_then(|worker| worker.close_view(retired));
                                if let Err(error) = close_result {
                                    self.fail(event_loop, error);
                                }
                                return;
                            }
                            if self.presentation.pending_activation().is_none()
                                && self.presentation.content()
                                    == WebContentPresentation::Tab(self.tab)
                            {
                                if let Err(error) = self.leave_native_neutral() {
                                    self.fail(event_loop, error);
                                    return;
                                }
                            }
                        }

                        if self.run_mode == RunMode::ExitAfterRealHttpNavigation {
                            if self.http_smoke_frame == Some(target) {
                                self.shutdown(event_loop);
                            } else if self.http_smoke_committed {
                                if let Err(error) = self.dispatch_http_smoke_frame() {
                                    self.fail(event_loop, error);
                                }
                            } else if self.http_smoke_navigation.is_none() {
                                if let Err(error) = self.start_http_smoke_navigation() {
                                    self.fail(event_loop, error);
                                }
                            } else if self.needs_redraw {
                                self.request_redraw();
                            }
                        } else if self.run_mode == RunMode::ExitAfterFirstPresentation {
                            self.shutdown(event_loop);
                        } else if self.run_mode == RunMode::ExitAfterTabActivation {
                            if let Err(error) = self.start_new_tab() {
                                self.fail(event_loop, error);
                            }
                        } else if self.run_mode == RunMode::ExitAfterTabClose {
                            let tab_count = self
                                .browser
                                .window(self.browser_window)
                                .map(|window| window.tabs().len())
                                .unwrap_or_default();
                            let result = if tab_count == 1 {
                                self.start_new_tab()
                            } else {
                                self.start_close_tab(event_loop)
                            };
                            if let Err(error) = result {
                                self.fail(event_loop, error);
                            }
                        } else if self.run_mode == RunMode::ExitAfterRapidTabActivation
                            && self.rapid_smoke_tabs.is_empty()
                        {
                            if let Err(error) = self.start_new_tab_with_activation(false) {
                                self.fail(event_loop, error);
                            }
                        } else if self.needs_redraw {
                            self.request_redraw();
                        }
                    }
                    Ok(FrameOutcome::SurfaceRecoveryNeeded(error)) => {
                        self.needs_redraw = true;
                        if let Err(recovery) = self.start_surface_recovery(permit) {
                            self.fail(
                                event_loop,
                                format!("{error}; surface recovery failed: {recovery}"),
                            );
                        }
                    }
                    Err(error) => self.fail(event_loop, error),
                }
            }
            WorkerEvent::SurfaceReplaced { target, result } => {
                if !self.pending_surface.complete_if_current(target) || !self.target_alive(target) {
                    return;
                }

                match result {
                    Ok(()) => {
                        self.needs_redraw = true;
                        let recovered = self.surface_recovery_permit.take();
                        if self.pending_target_permit.is_some() {
                            if let Err(error) = self.dispatch_pending_target_frame() {
                                self.fail(event_loop, error);
                            }
                        } else if let Some(permit) = recovered {
                            if let Err(error) = self.start_frame_with_permit(permit) {
                                self.fail(event_loop, error);
                            }
                        } else {
                            self.request_redraw();
                        }
                    }
                    Err(error) => {
                        self.surface_recovery_permit = None;
                        self.fail(event_loop, error);
                    }
                }
            }
        }
    }

    fn commit_initial_navigation(&mut self) -> Result<(), String> {
        let navigation = self
            .initial_navigation
            .ok_or_else(|| "initial browser navigation is already resolved".to_string())?;
        let commit = self
            .browser
            .commit_navigation(self.browser_window, self.tab, navigation, START_LOCATION)
            .map_err(|error| format!("failed to commit initial browser navigation: {error}"))?;
        self.record_navigation_commit(commit)?;
        self.initial_navigation = None;
        Ok(())
    }

    fn fail_initial_navigation(&mut self, message: &str) -> Result<(), String> {
        let Some(navigation) = self.initial_navigation else {
            return Ok(());
        };
        self.browser
            .fail_navigation(self.browser_window, self.tab, navigation, message)
            .map_err(|error| format!("failed to fail initial browser navigation: {error}"))?;
        self.initial_navigation = None;
        Ok(())
    }

    fn stop_initial_navigation(&mut self) -> Result<(), String> {
        let Some(navigation) = self.initial_navigation else {
            return Ok(());
        };
        let stopped = self
            .browser
            .stop_navigation(self.browser_window, self.tab)
            .map_err(|error| format!("failed to stop initial browser navigation: {error}"))?;
        match stopped {
            Some(intent) if intent.id() == navigation => {
                self.initial_navigation = None;
                Ok(())
            }
            Some(intent) => Err(format!(
                "stopped navigation {} instead of initial navigation {}",
                intent.id().get(),
                navigation.get()
            )),
            None => Err(format!(
                "initial navigation {} disappeared before shutdown",
                navigation.get()
            )),
        }
    }

    fn begin_shutdown(&mut self) {
        if self.shutdown_requested {
            return;
        }
        self.shutdown_requested = true;
        if let Err(error) = self.stop_initial_navigation()
            && self.fatal_error.is_none()
        {
            self.fatal_error = Some(error);
        }
        self.pending_init.invalidate();
        self.pending_frame.invalidate();
        self.pending_surface.invalidate();
        self.pending_navigation_cancel = None;
        self.http_smoke_frame = None;
        self.pending_target_permit = None;
        self.surface_recovery_permit = None;
        self.pending_tab_create = None;
        self.pending_view_close = None;
        self.rapid_smoke_tabs.clear();
        self.worker_ready = false;
        if let Some(worker) = self.worker.take() {
            worker.shutdown();
        }
        self.gpu = None;
        self.browser.close_window(self.browser_window);
        self.window = None;
    }

    fn finish_shutdown(&mut self, event_loop: &ActiveEventLoop) {
        drop(self.profile_worker.take());
        event_loop.exit();
    }

    fn shutdown(&mut self, event_loop: &ActiveEventLoop) {
        self.begin_shutdown();
        if let Err(error) = self.drive_profile_saves(true) {
            if self.fatal_error.is_none() {
                self.fatal_error = Some(error);
            }
            self.finish_shutdown(event_loop);
            return;
        }
        match self.profile_flush_complete() {
            Ok(true) => self.continue_shutdown_after_profile_flush(event_loop),
            Ok(false) => event_loop.set_control_flow(ControlFlow::Wait),
            Err(error) => {
                if self.fatal_error.is_none() {
                    self.fatal_error = Some(error);
                }
                self.finish_shutdown(event_loop);
            }
        }
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: impl std::fmt::Display) {
        let message = error.to_string();
        let failure = self.fail_initial_navigation(&message).err();
        if self.fatal_error.is_none() {
            self.fatal_error = Some(match failure {
                Some(failure) => format!("{message}; {failure}"),
                None => message,
            });
        }
        self.shutdown(event_loop);
    }
}

impl ApplicationHandler<WorkerEvent> for NativeShell {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.profile_runtime.active_profile().is_none() {
            if let Err(error) = self.submit_initial_profile_selection() {
                self.fail(event_loop, error);
            }
            return;
        }

        if let Err(error) = self.initialize(event_loop) {
            self.fail(event_loop, error);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: WorkerEvent) {
        self.handle_worker_event(event_loop, event);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.drive_profile_saves(self.shutdown_requested) {
            self.fail(event_loop, error);
            return;
        }

        if self.shutdown_requested {
            match self.profile_flush_complete() {
                Ok(true) => self.continue_shutdown_after_profile_flush(event_loop),
                Ok(false) => event_loop.set_control_flow(ControlFlow::Wait),
                Err(error) => self.fail(event_loop, error),
            }
            return;
        }

        let save_pending = self.profile_runtime.pending_settings_save().is_some()
            || self
                .profile_runtime
                .pending_browsing_history_save()
                .is_some();
        let next_save_due = match (
            self.settings_scheduler.next_save_due_millis(),
            self.history_scheduler.next_save_due_millis(),
        ) {
            (Some(settings), Some(history)) => Some(settings.min(history)),
            (Some(settings), None) => Some(settings),
            (None, Some(history)) => Some(history),
            (None, None) => None,
        };
        let control_flow = if save_pending {
            ControlFlow::Wait
        } else {
            next_save_due
                .and_then(|deadline| self.profile_clock.deadline(deadline))
                .map_or(ControlFlow::Wait, ControlFlow::WaitUntil)
        };
        event_loop.set_control_flow(control_flow);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        if self.window.as_ref().map(|window| window.id()) != Some(window_id) {
            return;
        }

        match event {
            WindowEvent::CloseRequested => self.shutdown(event_loop),
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers.state();
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                if let Err(error) = self.handle_keyboard_input(event_loop, event, is_synthetic) {
                    self.fail(event_loop, error);
                }
            }
            WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                self.needs_redraw = true;
                if self.worker_ready
                    && !self.pending_frame.is_pending()
                    && !self.pending_surface.is_pending()
                {
                    self.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(error) = self.start_frame() {
                    self.fail(event_loop, error);
                }
            }
            _ => {}
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        drop(self.profile_worker.take());
        if let Some(worker) = self.worker.take() {
            worker.shutdown();
        }
        self.gpu = None;
    }
}

fn render_worker_main(
    init_target: AsyncTarget,
    initial_generation: PresentationGeneration,
    receiver: Receiver<WorkerCommand>,
    proxy: EventLoopProxy<WorkerEvent>,
    cancellation: CancellationToken,
) {
    let mut worker =
        match RenderWorker::initialize(init_target, initial_generation, cancellation.clone()) {
            Ok(worker) => worker,
            Err(error) => {
                let _ = proxy.send_event(WorkerEvent::GpuReady {
                    target: init_target,
                    result: Err(error),
                });
                return;
            }
        };

    if cancellation.is_cancelled() {
        return;
    }

    if proxy
        .send_event(WorkerEvent::GpuReady {
            target: init_target,
            result: Ok(Arc::clone(&worker.gpu)),
        })
        .is_err()
    {
        return;
    }

    loop {
        let command = if worker.has_pending_navigation() {
            match receiver.recv_timeout(NAVIGATION_POLL_INTERVAL) {
                Ok(command) => Some(command),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match receiver.recv() {
                Ok(command) => Some(command),
                Err(_) => return,
            }
        };

        if cancellation.is_cancelled() {
            return;
        }

        if let Some(command) = command {
            match command {
                WorkerCommand::AttachInitialSurface { target, surface } => {
                    let result = worker.attach_initial_surface(target, surface);
                    if proxy
                        .send_event(WorkerEvent::Initialized { target, result })
                        .is_err()
                    {
                        return;
                    }
                }
                WorkerCommand::CreateView { target } => {
                    let result = worker.create_view(target);
                    if proxy
                        .send_event(WorkerEvent::ViewCreated { target, result })
                        .is_err()
                    {
                        return;
                    }
                }
                WorkerCommand::CloseView { tab } => {
                    let result = worker.close_view(tab);
                    if proxy
                        .send_event(WorkerEvent::ViewClosed { tab, result })
                        .is_err()
                    {
                        return;
                    }
                }
                WorkerCommand::Render {
                    target,
                    permit,
                    viewport,
                } => {
                    let result = worker.render(target, permit, viewport);
                    if proxy
                        .send_event(WorkerEvent::FrameFinished {
                            target,
                            permit,
                            result,
                        })
                        .is_err()
                    {
                        return;
                    }
                }
                WorkerCommand::ReplaceSurface { target, surface } => {
                    let result = worker.replace_surface(target, surface);
                    if proxy
                        .send_event(WorkerEvent::SurfaceReplaced { target, result })
                        .is_err()
                    {
                        return;
                    }
                }
                WorkerCommand::BeginNavigation { target, location } => {
                    if let Err(message) = worker.begin_navigation(target, location) {
                        if proxy
                            .send_event(WorkerEvent::NavigationFinished {
                                target,
                                outcome: WorkerNavigationOutcome::InternalFailure { message },
                            })
                            .is_err()
                        {
                            return;
                        }
                    }
                }
                WorkerCommand::CancelNavigation { target } => {
                    let result = worker.cancel_navigation(target);
                    if proxy
                        .send_event(WorkerEvent::NavigationCancelFinished { target, result })
                        .is_err()
                    {
                        return;
                    }
                }
            }
        }

        if cancellation.is_cancelled() {
            return;
        }
        for (target, outcome) in worker.poll_navigations() {
            if proxy
                .send_event(WorkerEvent::NavigationFinished { target, outcome })
                .is_err()
            {
                return;
            }
        }
    }
}

struct RenderWorker {
    window: BrowserWindowId,
    tab: TabId,
    presentation_generation: PresentationGeneration,
    engine: EngineHost,
    gpu: Arc<WindowsGpuDevice>,
    content: Option<WebContentSurface>,
    cancellation: CancellationToken,
    last_request_id: u64,
    last_viewport: Option<Viewport>,
    pending_navigations: BTreeMap<TabId, PendingWorkerNavigation>,
}

impl RenderWorker {
    fn initialize(
        init_target: AsyncTarget,
        initial_generation: PresentationGeneration,
        cancellation: CancellationToken,
    ) -> Result<Self, String> {
        let window = init_target.window();
        let tab = init_target.tab();
        let mut engine =
            EngineHost::new().map_err(|error| format!("failed to initialize Rarog: {error}"))?;
        engine
            .create_view(tab)
            .map_err(|error| format!("failed to create Rarog View: {error}"))?;
        engine
            .load_local_html(tab, START_PAGE)
            .map_err(|error| format!("failed to load Z1 start fixture: {error}"))?;

        if cancellation.is_cancelled() {
            return Err("render worker initialization was cancelled".into());
        }

        let gpu = Arc::new(
            block_on(WindowsGpuDevice::request())
                .map_err(|error| format!("failed to initialize DX12 device: {error}"))?,
        );
        if cancellation.is_cancelled() {
            return Err("render worker initialization was cancelled".into());
        }

        Ok(Self {
            window,
            tab,
            presentation_generation: initial_generation,
            engine,
            gpu,
            content: None,
            cancellation,
            last_request_id: init_target.request().get(),
            last_viewport: None,
            pending_navigations: BTreeMap::new(),
        })
    }

    fn attach_initial_surface(
        &mut self,
        target: AsyncTarget,
        surface: WindowsGpuSurface,
    ) -> Result<(), String> {
        self.ensure_active()?;

        if target.window() != self.window
            || target.tab() != self.tab
            || target.request().get() != self.last_request_id
            || self.content.is_some()
        {
            return Err(format!(
                "invalid initial surface attachment for request {} window {} tab {}",
                target.request().get(),
                target.window().get(),
                target.tab().get()
            ));
        }

        self.content = Some(WebContentSurface::new(self.gpu.as_ref(), surface));
        Ok(())
    }

    fn create_view(&mut self, target: AsyncTarget) -> Result<(), String> {
        self.ensure_active()?;
        self.validate_new_request(target)?;
        if self.engine.has_view(target.tab()) {
            return Err(format!(
                "tab {} already has a live Rarog View",
                target.tab().get()
            ));
        }

        self.engine.create_view(target.tab()).map_err(|error| {
            format!(
                "failed to create Rarog View for tab {}: {error}",
                target.tab().get()
            )
        })?;
        if let Err(error) = self.engine.load_local_html(target.tab(), START_PAGE) {
            let _ = self.engine.close_view(target.tab());
            return Err(format!(
                "failed to load start document for tab {}: {error}",
                target.tab().get()
            ));
        }
        Ok(())
    }

    fn close_view(&mut self, tab: TabId) -> Result<(), String> {
        self.ensure_active()?;
        if tab == self.tab {
            return Err(format!(
                "cannot retire currently presented Rarog View for tab {}",
                tab.get()
            ));
        }
        self.pending_navigations.remove(&tab);
        if !self.engine.close_view(tab).map_err(|error| {
            format!("failed to retire Rarog View for tab {}: {error}", tab.get())
        })? {
            return Err(format!(
                "cannot retire missing Rarog View for tab {}",
                tab.get()
            ));
        }
        Ok(())
    }

    fn begin_navigation(
        &mut self,
        target: WorkerNavigationTarget,
        location: String,
    ) -> Result<(), String> {
        self.ensure_active()?;
        if target.window != self.window || !self.engine.has_view(target.tab) {
            return Err(format!(
                "navigation {} targeted unavailable window {} tab {}",
                target.navigation.get(),
                target.window.get(),
                target.tab.get()
            ));
        }

        let previous = self.pending_navigations.get(&target.tab).copied();
        let request = match self.engine.begin_navigation(target.tab, location) {
            Ok(Some(request)) => request,
            Ok(None) => {
                self.pending_navigations.remove(&target.tab);
                if let Some(previous) = previous {
                    let _ = self.engine.cancel_navigation(previous.engine);
                }
                return Err("Rarog declined the browser navigation".into());
            }
            Err(error) => {
                self.pending_navigations.remove(&target.tab);
                if let Some(previous) = previous {
                    let _ = self.engine.cancel_navigation(previous.engine);
                }
                return Err(format!("failed to begin Rarog navigation: {error}"));
            }
        };
        self.pending_navigations.insert(
            target.tab,
            PendingWorkerNavigation {
                target,
                engine: request,
            },
        );
        Ok(())
    }

    fn cancel_navigation(&mut self, target: WorkerNavigationTarget) -> Result<bool, String> {
        self.ensure_active()?;
        let Some(pending) = self.pending_navigations.get(&target.tab).copied() else {
            return Ok(false);
        };
        if pending.target != target {
            return Ok(false);
        }

        self.pending_navigations.remove(&target.tab);
        self.engine
            .cancel_navigation(pending.engine)
            .map_err(|error| format!("failed to cancel Rarog navigation: {error}"))
    }

    fn has_pending_navigation(&self) -> bool {
        !self.pending_navigations.is_empty()
    }

    fn poll_navigations(&mut self) -> Vec<(WorkerNavigationTarget, WorkerNavigationOutcome)> {
        let tabs: Vec<_> = self.pending_navigations.keys().copied().collect();
        let mut completed = Vec::new();

        for tab in tabs {
            let Some(pending) = self.pending_navigations.get(&tab).copied() else {
                continue;
            };
            let outcome = match self.engine.poll_navigation(pending.engine) {
                Ok(EngineNavigationPoll::Pending) => continue,
                Ok(EngineNavigationPoll::Committed {
                    location,
                    status,
                    source_bytes,
                }) => WorkerNavigationOutcome::Committed {
                    location,
                    status,
                    source_bytes,
                },
                Ok(EngineNavigationPoll::Failed { message }) => {
                    WorkerNavigationOutcome::Failed { message }
                }
                Ok(EngineNavigationPoll::Stale) => WorkerNavigationOutcome::Stale,
                Err(error) => WorkerNavigationOutcome::InternalFailure {
                    message: format!("Rarog navigation polling failed: {error}"),
                },
            };
            if self
                .pending_navigations
                .get(&tab)
                .is_some_and(|current| current.target == pending.target)
            {
                self.pending_navigations.remove(&tab);
                completed.push((pending.target, outcome));
            }
        }

        completed
    }

    fn replace_surface(
        &mut self,
        target: AsyncTarget,
        surface: WindowsGpuSurface,
    ) -> Result<(), String> {
        self.ensure_active()?;
        self.validate_new_request(target)?;
        if target.tab() != self.tab {
            return Err(format!(
                "surface recovery targeted tab {} while presentation belongs to tab {}",
                target.tab().get(),
                self.tab.get()
            ));
        }
        let content = self
            .content
            .as_mut()
            .ok_or_else(|| "Web content surface is not initialized".to_string())?;
        content.replace_surface(self.gpu.as_ref(), surface);
        self.last_viewport = None;
        self.engine
            .request_frame(self.tab, EngineFrameCause::Explicit)
            .map_err(|error| format!("failed to schedule recovery frame: {error}"))
    }

    fn render(
        &mut self,
        target: AsyncTarget,
        permit: PresentationFramePermit,
        viewport: Viewport,
    ) -> Result<FrameOutcome, String> {
        self.ensure_active()?;
        self.validate_new_request(target)?;
        self.prepare_presentation(target, permit)?;

        if viewport.is_suspended() {
            return Ok(FrameOutcome::Presented);
        }

        let content = self
            .content
            .as_mut()
            .ok_or_else(|| "Web content surface is not initialized".to_string())?;
        content.ensure_size(self.gpu.as_ref(), viewport)?;

        if self
            .last_viewport
            .is_some_and(|previous| previous != viewport)
        {
            self.engine
                .request_frame(self.tab, EngineFrameCause::Resize)
                .map_err(|error| format!("failed to schedule resize frame: {error}"))?;
        }

        let request = self
            .engine
            .begin_frame(self.tab)
            .map_err(|error| format!("failed to begin Rarog frame: {error}"))?;

        let outcome = match request {
            Some(request) => self.render_engine_frame(request, viewport)?,
            None if self.cancellation.is_cancelled() => {
                return Err("render worker was cancelled before presentation".into());
            }
            None => match self
                .content
                .as_mut()
                .expect("content checked before frame scheduling")
                .present_retained()
            {
                Ok(()) => FrameOutcome::Presented,
                Err(PresentationError::Surface(error)) => {
                    FrameOutcome::SurfaceRecoveryNeeded(error)
                }
                Err(PresentationError::Fatal(error)) => return Err(error),
            },
        };

        if matches!(&outcome, FrameOutcome::Presented) {
            self.last_viewport = Some(viewport);
        }
        Ok(outcome)
    }

    fn render_engine_frame(
        &mut self,
        request: EngineFrameRequest,
        viewport: Viewport,
    ) -> Result<FrameOutcome, String> {
        let cancellation = self.cancellation.clone();
        let presentation = {
            let engine = &mut self.engine;
            let content = self
                .content
                .as_mut()
                .ok_or_else(|| "Web content surface is not initialized".to_string())?;

            match engine.render_frame(request, viewport) {
                Ok(_) if cancellation.is_cancelled() => Err(PresentationError::Fatal(
                    "render worker was cancelled before presentation".into(),
                )),
                Ok(frame) => content.present_frame(frame.rarog_frame(), viewport),
                Err(error) => Err(PresentationError::Fatal(format!(
                    "Rarog frame render failed: {error}"
                ))),
            }
        };

        match presentation {
            Ok(()) => {
                self.engine
                    .complete_frame(request)
                    .map_err(|error| format!("failed to complete Rarog frame request: {error}"))?;
                Ok(FrameOutcome::Presented)
            }
            Err(error) => {
                if let Err(discard) = self.engine.discard_frame(request) {
                    return Err(format!(
                        "{}; failed to discard Rarog frame request: {discard}",
                        error.message()
                    ));
                }

                match error {
                    PresentationError::Surface(error) => {
                        Ok(FrameOutcome::SurfaceRecoveryNeeded(error))
                    }
                    PresentationError::Fatal(error) => Err(error),
                }
            }
        }
    }

    fn ensure_active(&self) -> Result<(), String> {
        if self.cancellation.is_cancelled() {
            Err("render worker is cancelled".into())
        } else {
            Ok(())
        }
    }

    fn validate_new_request(&mut self, target: AsyncTarget) -> Result<(), String> {
        if target.window() != self.window || target.request().get() <= self.last_request_id {
            return Err(format!(
                "stale worker request {} targeted window {} tab {}",
                target.request().get(),
                target.window().get(),
                target.tab().get()
            ));
        }

        self.last_request_id = target.request().get();
        Ok(())
    }

    fn prepare_presentation(
        &mut self,
        target: AsyncTarget,
        permit: PresentationFramePermit,
    ) -> Result<(), String> {
        if permit.tab() != target.tab() {
            return Err(format!(
                "presentation permit for tab {} was used with worker target tab {}",
                permit.tab().get(),
                target.tab().get()
            ));
        }

        match permit {
            PresentationFramePermit::Current(current) => {
                if current.tab() != self.tab || current.generation() != self.presentation_generation
                {
                    return Err(format!(
                        "stale current-frame permit for tab {} generation {} while worker presents tab {} generation {}",
                        current.tab().get(),
                        current.generation().get(),
                        self.tab.get(),
                        self.presentation_generation.get()
                    ));
                }
            }
            PresentationFramePermit::Target(target_permit) => {
                if target_permit.generation() < self.presentation_generation {
                    return Err(format!(
                        "stale target-frame permit for tab {} generation {}; worker generation is {}",
                        target_permit.tab().get(),
                        target_permit.generation().get(),
                        self.presentation_generation.get()
                    ));
                }

                if target_permit.generation() == self.presentation_generation {
                    if target_permit.tab() != self.tab {
                        return Err(format!(
                            "target-frame permit generation {} names tab {} while worker presents tab {}",
                            target_permit.generation().get(),
                            target_permit.tab().get(),
                            self.tab.get()
                        ));
                    }
                    return Ok(());
                }

                if !self.engine.has_view(target_permit.tab()) {
                    return Err(format!(
                        "target-frame permit names tab {} without a live Rarog View",
                        target_permit.tab().get()
                    ));
                }

                self.engine
                    .request_frame(target_permit.tab(), EngineFrameCause::Explicit)
                    .map_err(|error| {
                        format!(
                            "failed to schedule first presentation frame for tab {}: {error}",
                            target_permit.tab().get()
                        )
                    })?;
                self.content
                    .as_mut()
                    .ok_or_else(|| "Web content surface is not initialized".to_string())?
                    .reset_for_view_switch(self.gpu.as_ref());
                self.tab = target_permit.tab();
                self.presentation_generation = target_permit.generation();
                self.last_viewport = None;
            }
        }

        Ok(())
    }
}

enum PresentationError {
    Surface(String),
    Fatal(String),
}

impl PresentationError {
    fn message(&self) -> &str {
        match self {
            Self::Surface(message) | Self::Fatal(message) => message,
        }
    }
}

struct WebContentSurface {
    surface: WindowsGpuSurface,
    backend: WgpuCompositorBackend,
    planner: FramePlanner,
}

impl WebContentSurface {
    fn new(gpu: &WindowsGpuDevice, surface: WindowsGpuSurface) -> Self {
        Self {
            surface,
            backend: gpu.compositor_backend(),
            planner: Self::new_planner(),
        }
    }

    fn replace_surface(&mut self, gpu: &WindowsGpuDevice, surface: WindowsGpuSurface) {
        self.surface = surface;
        self.reset_for_view_switch(gpu);
    }

    fn reset_for_view_switch(&mut self, gpu: &WindowsGpuDevice) {
        self.backend = gpu.compositor_backend();
        self.planner = Self::new_planner();
    }

    fn new_planner() -> FramePlanner {
        let surface_id = SurfaceId::new(1).expect("Z1 Web content surface id is non-zero");
        FramePlanner::new(surface_id)
    }

    fn ensure_size(&mut self, gpu: &WindowsGpuDevice, viewport: Viewport) -> Result<(), String> {
        if self.surface.width() == viewport.width() && self.surface.height() == viewport.height() {
            return Ok(());
        }

        self.surface
            .resize(gpu, viewport.width(), viewport.height())
            .map_err(|error| format!("Web content surface resize failed: {error}"))
    }

    fn present_frame(
        &mut self,
        frame: &rarog_engine::ViewFrame<'_>,
        viewport: Viewport,
    ) -> Result<(), PresentationError> {
        let decision = frame
            .plan_compositor_frame(
                &mut self.planner,
                SurfaceSize::new(viewport.width(), viewport.height()),
            )
            .map_err(|error| {
                PresentationError::Fatal(format!("failed to plan compositor frame: {error}"))
            })?;

        match decision {
            FrameDecision::Noop => self.present_retained(),
            FrameDecision::Suspended { .. } => Ok(()),
            FrameDecision::Submit(plan) => {
                let id = plan.id();
                if let Err(error) = self.backend.submit(FrameSubmission {
                    plan: &plan,
                    display_list: frame.display_list,
                    image_resources: Some(frame.image_resources),
                    viewport_translation: frame.viewport_translation,
                    clear_color: frame.clear_color,
                }) {
                    return Err(self.discard_plan_or_fatal(
                        id,
                        PresentationError::Fatal(format!(
                            "failed to submit compositor frame: {error}"
                        )),
                    ));
                }

                if let Err(error) = self.present_surface() {
                    return Err(self.discard_plan_or_fatal(id, error));
                }

                if let Err(error) = self.planner.complete(id) {
                    let completion = PresentationError::Fatal(format!(
                        "failed to complete compositor frame: {error}"
                    ));
                    return Err(self.discard_plan_or_fatal(id, completion));
                }
                Ok(())
            }
        }
    }

    fn present_retained(&mut self) -> Result<(), PresentationError> {
        self.present_surface()
    }

    fn present_surface(&mut self) -> Result<(), PresentationError> {
        match self.surface.present(&mut self.backend) {
            Ok(()) => Ok(()),
            Err(error @ WindowsGpuError::Surface(_)) => Err(PresentationError::Surface(format!(
                "Web content surface acquisition failed: {error}"
            ))),
            Err(error) => Err(PresentationError::Fatal(format!(
                "Web content presentation failed: {error}"
            ))),
        }
    }

    fn discard_plan_or_fatal(
        &mut self,
        id: rarog_compositor::FrameId,
        error: PresentationError,
    ) -> PresentationError {
        match self.planner.discard(id) {
            Ok(()) => error,
            Err(discard) => PresentationError::Fatal(format!(
                "{}; failed to discard compositor frame: {discard}",
                error.message()
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PreparedProfile;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_PROFILE_ROOT: AtomicU64 = AtomicU64::new(1);

    struct TestProfileRoot(PathBuf);

    impl TestProfileRoot {
        fn new() -> Self {
            let id = NEXT_PROFILE_ROOT.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "zorya-windows-profile-history-{}-{id}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            Self(root)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TestProfileRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn active_profile_runtime() -> (TestProfileRoot, ProfileRuntime) {
        let root = TestProfileRoot::new();
        let mut runtime = ProfileRuntime::new();
        let intent = runtime.begin_selection(root.path()).unwrap().into_intent();
        let prepared = PreparedProfile::load(&intent).unwrap();
        runtime.commit_selection(prepared).unwrap();
        (root, runtime)
    }

    fn committed_navigation(location: &str) -> BrowserNavigationCommit {
        let mut browser = BrowserApp::bootstrap().unwrap();
        let window = browser.windows().next().unwrap().id();
        let tab = browser.window(window).unwrap().active_tab_id().unwrap();
        let navigation = browser
            .begin_navigation(window, tab, location)
            .unwrap()
            .intent()
            .id();
        browser
            .commit_navigation(window, tab, navigation, location)
            .unwrap()
    }

    #[test]
    fn default_profile_root_is_stable_under_local_app_data() {
        let local_app_data = OsString::from(r"C:\Users\Zorya\AppData\Local");
        let root = profile_root_from_local_app_data(Some(local_app_data)).unwrap();

        assert_eq!(
            root,
            PathBuf::from(r"C:\Users\Zorya\AppData\Local")
                .join(PRODUCT_DATA_DIRECTORY)
                .join(PROFILES_DIRECTORY)
                .join(DEFAULT_PROFILE_DIRECTORY)
        );
    }

    #[test]
    fn missing_local_app_data_fails_closed() {
        let missing = profile_root_from_local_app_data(None).unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);

        let empty = profile_root_from_local_app_data(Some(OsString::new())).unwrap_err();
        assert_eq!(empty.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn native_history_save_policy_matches_product_bounds() {
        let policy = native_history_save_policy();

        assert_eq!(policy.debounce_millis(), 5_000);
        assert_eq!(policy.max_dirty_millis(), 30_000);
        assert_eq!(policy.mutation_threshold(), 32);
    }

    #[test]
    fn committed_native_navigation_records_exact_active_profile_visit() {
        let (_root, mut runtime) = active_profile_runtime();
        let profile = runtime.active_profile().unwrap().id();
        let commit = committed_navigation("https://example.test/final");

        record_profile_navigation_at(&mut runtime, 1_234, commit).unwrap();

        let history = runtime.active_browsing_history(profile).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history.visits()[0].visited_unix_millis(), 1_234);
        assert_eq!(history.visits()[0].location(), "https://example.test/final");
        assert!(runtime.browsing_history_is_dirty(profile).unwrap());
    }

    #[test]
    fn service_about_blank_commit_is_not_persisted() {
        let (_root, mut runtime) = active_profile_runtime();
        let profile = runtime.active_profile().unwrap().id();
        let commit = committed_navigation(START_LOCATION);

        record_profile_navigation_at(&mut runtime, 1_234, commit).unwrap();

        assert!(runtime.active_browsing_history(profile).unwrap().is_empty());
        assert!(!runtime.browsing_history_is_dirty(profile).unwrap());
    }
}
