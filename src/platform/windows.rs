use super::RunMode;
use crate::async_lifecycle::{
    AsyncRequestSequence, AsyncTarget, CancellationToken, PendingRequest,
};
use crate::engine::{EngineFrameCause, EngineFrameRequest, EngineHost, Viewport};
use crate::{BrowserApp, BrowserWindowId, NavigationId, TabId};
use pollster::block_on;
use rarog_compositor::{
    CompositorBackend, FrameDecision, FramePlanner, FrameSubmission, SurfaceId, SurfaceSize,
};
use rarog_compositor_wgpu::WgpuCompositorBackend;
use rarog_platform_windows::{WindowsGpuDevice, WindowsGpuError, WindowsGpuSurface};
use std::error::Error;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::{self, JoinHandle};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

const START_LOCATION: &str = "about:blank";
const START_PAGE: &str = include_str!("../../assets/z1-start.html");

pub(crate) fn run(mode: RunMode) -> Result<(), Box<dyn Error>> {
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
    let initial_navigation = browser
        .begin_navigation(browser_window, tab, START_LOCATION)?
        .intent()
        .id();

    let event_loop = EventLoop::<WorkerEvent>::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    let mut shell = NativeShell::new(
        browser,
        browser_window,
        tab,
        initial_navigation,
        proxy,
        mode,
    );
    event_loop.run_app(&mut shell)?;

    if let Some(error) = shell.fatal_error.take() {
        return Err(std::io::Error::other(error).into());
    }

    Ok(())
}

enum WorkerEvent {
    GpuReady {
        target: AsyncTarget,
        result: Result<Arc<WindowsGpuDevice>, String>,
    },
    Initialized {
        target: AsyncTarget,
        result: Result<(), String>,
    },
    FrameFinished {
        target: AsyncTarget,
        result: Result<FrameOutcome, String>,
    },
    SurfaceReplaced {
        target: AsyncTarget,
        result: Result<(), String>,
    },
}

enum WorkerCommand {
    AttachInitialSurface {
        target: AsyncTarget,
        surface: WindowsGpuSurface,
    },
    Render {
        target: AsyncTarget,
        viewport: Viewport,
    },
    ReplaceSurface {
        target: AsyncTarget,
        surface: WindowsGpuSurface,
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
    fn spawn(target: AsyncTarget, proxy: EventLoopProxy<WorkerEvent>) -> Result<Self, String> {
        let (sender, receiver) = sync_channel(1);
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let thread = thread::Builder::new()
            .name("zorya-render".into())
            .spawn(move || render_worker_main(target, receiver, proxy, worker_cancellation))
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

    fn render(&self, target: AsyncTarget, viewport: Viewport) -> Result<(), String> {
        self.send(WorkerCommand::Render { target, viewport })
    }

    fn replace_surface(
        &self,
        target: AsyncTarget,
        surface: WindowsGpuSurface,
    ) -> Result<(), String> {
        self.send(WorkerCommand::ReplaceSurface { target, surface })
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

struct NativeShell {
    browser: BrowserApp,
    browser_window: BrowserWindowId,
    tab: TabId,
    initial_navigation: Option<NavigationId>,
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
    fatal_error: Option<String>,
}

impl NativeShell {
    fn new(
        browser: BrowserApp,
        browser_window: BrowserWindowId,
        tab: TabId,
        initial_navigation: NavigationId,
        proxy: EventLoopProxy<WorkerEvent>,
        run_mode: RunMode,
    ) -> Self {
        Self {
            browser,
            browser_window,
            tab,
            initial_navigation: Some(initial_navigation),
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
            fatal_error: None,
        }
    }

    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        if let Some(window) = &self.window {
            if self.worker_ready && !self.pending_surface.is_pending() {
                window.request_redraw();
            }
            return Ok(());
        }

        let attributes = Window::default_attributes()
            .with_title("Zorya Developer Build")
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

        let worker = match WorkerHandle::spawn(target, self.proxy.clone()) {
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
        if self.pending_frame.is_pending() || self.pending_surface.is_pending() {
            self.needs_redraw = true;
            return Ok(());
        }

        let Some(window) = &self.window else {
            return Ok(());
        };
        let size = window.inner_size();
        let viewport = Viewport::new(size.width, size.height);
        if viewport.is_suspended() {
            return Ok(());
        }

        let target = self
            .requests
            .allocate(self.browser_window, self.tab)
            .map_err(|error| error.to_string())?;
        self.pending_frame
            .begin(target)
            .map_err(|error| error.to_string())?;

        let render_result = self
            .worker
            .as_ref()
            .ok_or_else(|| "render worker is unavailable".to_string())
            .and_then(|worker| worker.render(target, viewport));
        if let Err(error) = render_result {
            self.pending_frame.complete_if_current(target);
            return Err(error);
        }

        self.needs_redraw = false;
        Ok(())
    }

    fn start_surface_recovery(&mut self) -> Result<(), String> {
        let target = self
            .requests
            .allocate(self.browser_window, self.tab)
            .map_err(|error| error.to_string())?;
        self.pending_surface
            .begin(target)
            .map_err(|error| error.to_string())?;

        let surface = match self.create_surface() {
            Ok(surface) => surface,
            Err(error) => {
                self.pending_surface.complete_if_current(target);
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
            return Err(error);
        }

        Ok(())
    }

    fn handle_worker_event(&mut self, event_loop: &ActiveEventLoop, event: WorkerEvent) {
        match event {
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
            WorkerEvent::FrameFinished { target, result } => {
                if !self.pending_frame.complete_if_current(target) || !self.target_alive(target) {
                    return;
                }

                match result {
                    Ok(FrameOutcome::Presented) => {
                        if self.run_mode == RunMode::ExitAfterFirstPresentation {
                            self.shutdown(event_loop);
                        } else if self.needs_redraw {
                            self.request_redraw();
                        }
                    }
                    Ok(FrameOutcome::SurfaceRecoveryNeeded(error)) => {
                        self.needs_redraw = true;
                        if let Err(recovery) = self.start_surface_recovery() {
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
                        self.request_redraw();
                    }
                    Err(error) => self.fail(event_loop, error),
                }
            }
        }
    }

    fn commit_initial_navigation(&mut self) -> Result<(), String> {
        let navigation = self
            .initial_navigation
            .take()
            .ok_or_else(|| "initial browser navigation is already resolved".to_string())?;
        self.browser
            .commit_navigation(self.browser_window, self.tab, navigation, START_LOCATION)
            .map(|_| ())
            .map_err(|error| format!("failed to commit initial browser navigation: {error}"))
    }

    fn fail_initial_navigation(&mut self, message: &str) -> Result<(), String> {
        let Some(navigation) = self.initial_navigation.take() else {
            return Ok(());
        };
        self.browser
            .fail_navigation(self.browser_window, self.tab, navigation, message)
            .map(|_| ())
            .map_err(|error| format!("failed to fail initial browser navigation: {error}"))
    }

    fn stop_initial_navigation(&mut self) -> Result<(), String> {
        let Some(navigation) = self.initial_navigation.take() else {
            return Ok(());
        };
        let stopped = self
            .browser
            .stop_navigation(self.browser_window, self.tab)
            .map_err(|error| format!("failed to stop initial browser navigation: {error}"))?;
        match stopped {
            Some(intent) if intent.id() == navigation => Ok(()),
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

    fn shutdown(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.stop_initial_navigation()
            && self.fatal_error.is_none()
        {
            self.fatal_error = Some(error);
        }
        self.pending_init.invalidate();
        self.pending_frame.invalidate();
        self.pending_surface.invalidate();
        self.worker_ready = false;
        if let Some(worker) = self.worker.take() {
            worker.shutdown();
        }
        self.gpu = None;
        self.browser.close_window(self.browser_window);
        self.window = None;
        event_loop.exit();
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
        if let Err(error) = self.initialize(event_loop) {
            self.fail(event_loop, error);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: WorkerEvent) {
        self.handle_worker_event(event_loop, event);
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
        if let Some(worker) = self.worker.take() {
            worker.shutdown();
        }
        self.gpu = None;
    }
}

fn render_worker_main(
    init_target: AsyncTarget,
    receiver: Receiver<WorkerCommand>,
    proxy: EventLoopProxy<WorkerEvent>,
    cancellation: CancellationToken,
) {
    let mut worker = match RenderWorker::initialize(init_target, cancellation.clone()) {
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

    while let Ok(command) = receiver.recv() {
        if cancellation.is_cancelled() {
            return;
        }

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
            WorkerCommand::Render { target, viewport } => {
                let result = worker.render(target, viewport);
                if proxy
                    .send_event(WorkerEvent::FrameFinished { target, result })
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
        }
    }
}

struct RenderWorker {
    window: BrowserWindowId,
    tab: TabId,
    engine: EngineHost,
    gpu: Arc<WindowsGpuDevice>,
    content: Option<WebContentSurface>,
    cancellation: CancellationToken,
    last_request_id: u64,
    last_viewport: Option<Viewport>,
}

impl RenderWorker {
    fn initialize(
        init_target: AsyncTarget,
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
            engine,
            gpu,
            content: None,
            cancellation,
            last_request_id: init_target.request().get(),
            last_viewport: None,
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

    fn replace_surface(
        &mut self,
        target: AsyncTarget,
        surface: WindowsGpuSurface,
    ) -> Result<(), String> {
        self.ensure_active()?;
        self.validate_new_target(target)?;
        let content = self
            .content
            .as_mut()
            .ok_or_else(|| "Web content surface is not initialized".to_string())?;
        content.replace_surface(surface);
        self.last_viewport = None;
        self.engine
            .request_frame(self.tab, EngineFrameCause::Explicit)
            .map_err(|error| format!("failed to schedule recovery frame: {error}"))
    }

    fn render(&mut self, target: AsyncTarget, viewport: Viewport) -> Result<FrameOutcome, String> {
        self.ensure_active()?;
        self.validate_new_target(target)?;

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

    fn validate_new_target(&mut self, target: AsyncTarget) -> Result<(), String> {
        if target.window() != self.window
            || target.tab() != self.tab
            || target.request().get() <= self.last_request_id
        {
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

    fn replace_surface(&mut self, surface: WindowsGpuSurface) {
        self.surface = surface;
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
