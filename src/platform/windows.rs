use crate::async_lifecycle::{
    AsyncRequestSequence, AsyncTarget, CancellationToken, PendingRequest,
};
use crate::engine::{EngineFrameCause, EngineFrameRequest, EngineHost, Viewport};
use crate::{BrowserApp, BrowserWindowId, TabId};
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

const START_PAGE: &str = include_str!("../../assets/z1-start.html");

pub(crate) fn run() -> Result<(), Box<dyn Error>> {
    let browser = BrowserApp::bootstrap()?;
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
    let proxy = event_loop.create_proxy();
    let mut shell = NativeShell::new(browser, browser_window, tab, proxy);
    event_loop.run_app(&mut shell)?;
    Ok(())
}

enum WorkerEvent {
    Initialized {
        target: AsyncTarget,
        result: Result<(), String>,
    },
    FrameFinished {
        target: AsyncTarget,
        result: Result<(), String>,
    },
}

enum WorkerCommand {
    Render {
        target: AsyncTarget,
        viewport: Viewport,
    },
}

struct WorkerHandle {
    sender: SyncSender<WorkerCommand>,
    thread: JoinHandle<()>,
    cancellation: CancellationToken,
}

impl WorkerHandle {
    fn spawn(
        target: AsyncTarget,
        window: Arc<Window>,
        viewport: Viewport,
        proxy: EventLoopProxy<WorkerEvent>,
    ) -> Result<Self, String> {
        let (sender, receiver) = sync_channel(1);
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let thread = thread::Builder::new()
            .name("zorya-render".into())
            .spawn(move || {
                render_worker_main(
                    target,
                    window,
                    viewport,
                    receiver,
                    proxy,
                    worker_cancellation,
                )
            })
            .map_err(|error| format!("failed to start render worker: {error}"))?;

        Ok(Self {
            sender,
            thread,
            cancellation,
        })
    }

    fn render(&self, target: AsyncTarget, viewport: Viewport) -> Result<(), String> {
        if self.cancellation.is_cancelled() {
            return Err("render worker is cancelled".into());
        }

        self.sender
            .try_send(WorkerCommand::Render { target, viewport })
            .map_err(|error| match error {
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
    proxy: EventLoopProxy<WorkerEvent>,
    window: Option<Arc<Window>>,
    worker: Option<WorkerHandle>,
    requests: AsyncRequestSequence,
    pending_init: PendingRequest,
    pending_frame: PendingRequest,
    worker_ready: bool,
    needs_redraw: bool,
}

impl NativeShell {
    fn new(
        browser: BrowserApp,
        browser_window: BrowserWindowId,
        tab: TabId,
        proxy: EventLoopProxy<WorkerEvent>,
    ) -> Self {
        Self {
            browser,
            browser_window,
            tab,
            proxy,
            window: None,
            worker: None,
            requests: AsyncRequestSequence::new(),
            pending_init: PendingRequest::default(),
            pending_frame: PendingRequest::default(),
            worker_ready: false,
            needs_redraw: false,
        }
    }

    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> Result<(), String> {
        if let Some(window) = &self.window {
            if self.worker_ready {
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
        let size = window.inner_size();
        let viewport = Viewport::new(size.width, size.height);
        let target = self
            .requests
            .allocate(self.browser_window, self.tab)
            .map_err(|error| error.to_string())?;
        self.pending_init
            .begin(target)
            .map_err(|error| error.to_string())?;

        let worker =
            match WorkerHandle::spawn(target, Arc::clone(&window), viewport, self.proxy.clone()) {
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

    fn start_frame(&mut self) -> Result<(), String> {
        if !self.worker_ready {
            return Ok(());
        }
        if self.pending_frame.is_pending() {
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

    fn handle_worker_event(&mut self, event_loop: &ActiveEventLoop, event: WorkerEvent) {
        match event {
            WorkerEvent::Initialized { target, result } => {
                if !self.pending_init.complete_if_current(target) || !self.target_alive(target) {
                    return;
                }

                match result {
                    Ok(()) => {
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
                    Ok(()) => {
                        if self.needs_redraw {
                            self.request_redraw();
                        }
                    }
                    Err(error) => self.fail(event_loop, error),
                }
            }
        }
    }

    fn shutdown(&mut self, event_loop: &ActiveEventLoop) {
        self.pending_init.invalidate();
        self.pending_frame.invalidate();
        self.worker_ready = false;
        if let Some(worker) = self.worker.take() {
            worker.shutdown();
        }
        self.browser.close_window(self.browser_window);
        self.window = None;
        event_loop.exit();
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, error: impl std::fmt::Display) {
        eprintln!("zorya: {error}");
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
                if self.worker_ready && !self.pending_frame.is_pending() {
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
    }
}

fn render_worker_main(
    init_target: AsyncTarget,
    window: Arc<Window>,
    viewport: Viewport,
    receiver: Receiver<WorkerCommand>,
    proxy: EventLoopProxy<WorkerEvent>,
    cancellation: CancellationToken,
) {
    let initialized =
        RenderWorker::initialize(init_target, window, viewport, cancellation.clone());
    if cancellation.is_cancelled() {
        return;
    }

    let result = initialized.as_ref().map(|_| ()).map_err(Clone::clone);
    if proxy
        .send_event(WorkerEvent::Initialized {
            target: init_target,
            result,
        })
        .is_err()
    {
        return;
    }

    let Ok(mut worker) = initialized else {
        return;
    };

    while let Ok(command) = receiver.recv() {
        if cancellation.is_cancelled() {
            return;
        }

        match command {
            WorkerCommand::Render { target, viewport } => {
                let result = worker.render(target, viewport);
                if proxy
                    .send_event(WorkerEvent::FrameFinished { target, result })
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
    content: WebContentSurface,
    cancellation: CancellationToken,
    last_request_id: u64,
    last_viewport: Option<Viewport>,
}

impl RenderWorker {
    fn initialize(
        init_target: AsyncTarget,
        window: Arc<Window>,
        viewport: Viewport,
        cancellation: CancellationToken,
    ) -> Result<Self, String> {
        let window_id = init_target.window();
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

        let gpu = block_on(WindowsGpuDevice::request())
            .map_err(|error| format!("failed to initialize DX12 device: {error}"))?;
        if cancellation.is_cancelled() {
            return Err("render worker initialization was cancelled".into());
        }

        let content = WebContentSurface::new(window, gpu, viewport)?;

        Ok(Self {
            window: window_id,
            tab,
            engine,
            content,
            cancellation,
            last_request_id: init_target.request().get(),
            last_viewport: None,
        })
    }

    fn render(&mut self, target: AsyncTarget, viewport: Viewport) -> Result<(), String> {
        if self.cancellation.is_cancelled() {
            return Err("render worker is cancelled".into());
        }

        if target.window() != self.window
            || target.tab() != self.tab
            || target.request().get() <= self.last_request_id
        {
            return Err(format!(
                "stale render request {} targeted window {} tab {}",
                target.request().get(),
                target.window().get(),
                target.tab().get()
            ));
        }
        self.last_request_id = target.request().get();

        if viewport.is_suspended() {
            return Ok(());
        }

        self.content.ensure_size(viewport)?;
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

        let result = match request {
            Some(request) => self.render_engine_frame(request, viewport),
            None if self.cancellation.is_cancelled() => Err("render worker is cancelled".into()),
            None => self.content.present_retained(viewport),
        };

        if result.is_ok() {
            self.last_viewport = Some(viewport);
        }
        result
    }

    fn render_engine_frame(
        &mut self,
        request: EngineFrameRequest,
        viewport: Viewport,
    ) -> Result<(), String> {
        let presentation = {
            let engine = &mut self.engine;
            let content = &mut self.content;
            match engine.render_frame(request, viewport) {
                Ok(_) if self.cancellation.is_cancelled() => {
                    Err("render worker was cancelled before presentation".into())
                }
                Ok(frame) => content.present_frame(frame.rarog_frame(), viewport),
                Err(error) => Err(format!("Rarog frame render failed: {error}")),
            }
        };

        match presentation {
            Ok(()) => self
                .engine
                .complete_frame(request)
                .map_err(|error| format!("failed to complete Rarog frame request: {error}")),
            Err(error) => match self.engine.discard_frame(request) {
                Ok(()) => Err(error),
                Err(discard) => Err(format!(
                    "{error}; failed to discard Rarog frame request: {discard}"
                )),
            },
        }
    }
}

struct WebContentSurface {
    window: Arc<Window>,
    gpu: WindowsGpuDevice,
    surface: WindowsGpuSurface,
    backend: WgpuCompositorBackend,
    planner: FramePlanner,
}

impl WebContentSurface {
    fn new(window: Arc<Window>, gpu: WindowsGpuDevice, viewport: Viewport) -> Result<Self, String> {
        let surface = gpu
            .create_surface(Arc::clone(&window), viewport.width(), viewport.height())
            .map_err(|error| format!("failed to create Web content surface: {error}"))?;
        let backend = gpu.compositor_backend();
        let surface_id = SurfaceId::new(1).expect("Z1 Web content surface id is non-zero");

        Ok(Self {
            window,
            gpu,
            surface,
            backend,
            planner: FramePlanner::new(surface_id),
        })
    }

    fn ensure_size(&mut self, viewport: Viewport) -> Result<(), String> {
        if self.surface.width() == viewport.width() && self.surface.height() == viewport.height() {
            return Ok(());
        }

        self.surface
            .resize(&self.gpu, viewport.width(), viewport.height())
            .map_err(|error| format!("Web content surface resize failed: {error}"))
    }

    fn present_frame(
        &mut self,
        frame: &rarog_engine::ViewFrame<'_>,
        viewport: Viewport,
    ) -> Result<(), String> {
        let decision = frame
            .plan_compositor_frame(
                &mut self.planner,
                SurfaceSize::new(viewport.width(), viewport.height()),
            )
            .map_err(|error| format!("failed to plan compositor frame: {error}"))?;

        match decision {
            FrameDecision::Noop => self.present_with_recovery(viewport),
            FrameDecision::Suspended { .. } => Ok(()),
            FrameDecision::Submit(plan) => {
                let id = plan.id();
                if let Err(error) = self.backend.submit(FrameSubmission {
                    plan: &plan,
                    display_list: frame.display_list,
                    clear_color: frame.clear_color,
                }) {
                    let _ = self.planner.discard(id);
                    return Err(format!("failed to submit compositor frame: {error}"));
                }

                if let Err(error) = self.present_with_recovery(viewport) {
                    let _ = self.planner.discard(id);
                    return Err(error);
                }

                if let Err(error) = self.planner.complete(id) {
                    let _ = self.planner.discard(id);
                    return Err(format!("failed to complete compositor frame: {error}"));
                }
                Ok(())
            }
        }
    }

    fn present_retained(&mut self, viewport: Viewport) -> Result<(), String> {
        self.present_with_recovery(viewport)
    }

    fn present_with_recovery(&mut self, viewport: Viewport) -> Result<(), String> {
        match self.surface.present(&mut self.backend) {
            Ok(()) => Ok(()),
            Err(first @ WindowsGpuError::Surface(_)) => {
                self.recreate_surface(viewport).map_err(|recovery| {
                    format!(
                        "Web content surface acquisition failed ({first}); surface recreation failed ({recovery})"
                    )
                })?;
                self.surface.present(&mut self.backend).map_err(|second| {
                    format!(
                        "Web content surface acquisition failed ({first}); retry after surface recreation failed ({second})"
                    )
                })
            }
            Err(error) => Err(format!("Web content presentation failed: {error}")),
        }
    }

    fn recreate_surface(&mut self, viewport: Viewport) -> Result<(), String> {
        self.surface = self
            .gpu
            .create_surface(
                Arc::clone(&self.window),
                viewport.width(),
                viewport.height(),
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}
