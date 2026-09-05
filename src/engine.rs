use crate::TabId;
use rarog_compositor::FrameCause;
use rarog_engine::{BaseUrl, Engine, EngineError, FrameStatus, View, ViewOptions};
use rarog_types::Size;
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport {
    width: u32,
    height: u32,
}

impl Viewport {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub const fn width(self) -> u32 {
        self.width
    }

    pub const fn height(self) -> u32 {
        self.height
    }

    pub const fn is_suspended(self) -> bool {
        self.width == 0 || self.height == 0
    }

    fn rarog_size(self) -> Size {
        Size {
            width: self.width as f32,
            height: self.height as f32,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineFrameRequest {
    tab: TabId,
    view_generation: u64,
    request_id: u64,
}

impl EngineFrameRequest {
    pub const fn tab(self) -> TabId {
        self.tab
    }

    pub const fn view_generation(self) -> u64 {
        self.view_generation
    }

    pub const fn request_id(self) -> u64 {
        self.request_id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineFrameStatus {
    Initial,
    ViewportRebuild,
    Incremental,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineFrameCause {
    Initial,
    Resize,
    SceneChange,
    Scroll,
    ResourceReady,
    Explicit,
}

impl EngineFrameCause {
    const fn rarog(self) -> FrameCause {
        match self {
            Self::Initial => FrameCause::Initial,
            Self::Resize => FrameCause::Resize,
            Self::SceneChange => FrameCause::SceneChange,
            Self::Scroll => FrameCause::Scroll,
            Self::ResourceReady => FrameCause::ResourceReady,
            Self::Explicit => FrameCause::Explicit,
        }
    }
}

pub struct EngineRenderedFrame<'a> {
    inner: rarog_engine::ViewFrame<'a>,
}

impl EngineRenderedFrame<'_> {
    pub const fn status(&self) -> EngineFrameStatus {
        match self.inner.status {
            FrameStatus::Initial => EngineFrameStatus::Initial,
            FrameStatus::ViewportRebuild => EngineFrameStatus::ViewportRebuild,
            FrameStatus::Incremental(_) => EngineFrameStatus::Incremental,
        }
    }

    #[cfg(target_os = "windows")]
    pub(crate) const fn rarog_frame(&self) -> &rarog_engine::ViewFrame<'_> {
        &self.inner
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineHostError {
    Engine(EngineError),
    DuplicateView(TabId),
    UnknownTab(TabId),
    ViewGenerationExhausted,
    StaleFrameRequest {
        tab: TabId,
        view_generation: u64,
        request_id: u64,
    },
}

impl fmt::Display for EngineHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Engine(error) => write!(formatter, "{error}"),
            Self::DuplicateView(tab) => {
                write!(formatter, "tab {} already has an engine view", tab.get())
            }
            Self::UnknownTab(tab) => {
                write!(formatter, "tab {} has no engine view", tab.get())
            }
            Self::ViewGenerationExhausted => {
                formatter.write_str("engine view generation space is exhausted")
            }
            Self::StaleFrameRequest {
                tab,
                view_generation,
                request_id,
            } => write!(
                formatter,
                "stale frame request {request_id} for tab {} view generation {view_generation}",
                tab.get()
            ),
        }
    }
}

impl std::error::Error for EngineHostError {}

impl From<EngineError> for EngineHostError {
    fn from(error: EngineError) -> Self {
        Self::Engine(error)
    }
}

struct HostedView {
    generation: u64,
    view: View,
}

pub struct EngineHost {
    engine: Engine,
    views: BTreeMap<TabId, HostedView>,
    next_view_generation: u64,
}

impl EngineHost {
    pub fn new() -> Result<Self, EngineHostError> {
        Ok(Self {
            engine: Engine::builder().build()?,
            views: BTreeMap::new(),
            next_view_generation: 1,
        })
    }

    pub fn create_view(&mut self, tab: TabId) -> Result<(), EngineHostError> {
        if self.views.contains_key(&tab) {
            return Err(EngineHostError::DuplicateView(tab));
        }

        let generation = self.allocate_view_generation()?;
        let view = self.engine.create_view(ViewOptions::default())?;
        self.views.insert(tab, HostedView { generation, view });
        Ok(())
    }

    pub fn close_view(&mut self, tab: TabId) -> bool {
        self.views.remove(&tab).is_some()
    }

    pub fn has_view(&self, tab: TabId) -> bool {
        self.views.contains_key(&tab)
    }

    pub fn load_local_html(
        &mut self,
        tab: TabId,
        source: impl Into<String>,
    ) -> Result<(), EngineHostError> {
        self.view_mut(tab)?
            .view
            .load_html(source, BaseUrl::about_blank())?;
        Ok(())
    }

    pub fn request_frame(
        &mut self,
        tab: TabId,
        cause: EngineFrameCause,
    ) -> Result<(), EngineHostError> {
        self.view_mut(tab)?.view.request_frame(cause.rarog());
        Ok(())
    }

    pub fn begin_frame(
        &mut self,
        tab: TabId,
    ) -> Result<Option<EngineFrameRequest>, EngineHostError> {
        let hosted = self.view_mut(tab)?;
        let Some(scheduled) = hosted.view.begin_frame_request()? else {
            return Ok(None);
        };

        Ok(Some(EngineFrameRequest {
            tab,
            view_generation: hosted.generation,
            request_id: scheduled.id().get(),
        }))
    }

    pub fn render_frame(
        &mut self,
        request: EngineFrameRequest,
        viewport: Viewport,
    ) -> Result<EngineRenderedFrame<'_>, EngineHostError> {
        let hosted = self.validate_frame_request(request)?;
        let frame = hosted.view.render(viewport.rarog_size())?;
        Ok(EngineRenderedFrame { inner: frame })
    }

    pub fn complete_frame(&mut self, request: EngineFrameRequest) -> Result<(), EngineHostError> {
        let hosted = self.validate_frame_request(request)?;
        let active = hosted
            .view
            .active_frame_request()
            .expect("validated frame request remains active");
        hosted.view.complete_frame_request(active)?;
        Ok(())
    }

    pub fn discard_frame(&mut self, request: EngineFrameRequest) -> Result<(), EngineHostError> {
        let hosted = self.validate_frame_request(request)?;
        let active = hosted
            .view
            .active_frame_request()
            .expect("validated frame request remains active");
        hosted.view.discard_frame_request(active)?;
        Ok(())
    }

    fn allocate_view_generation(&mut self) -> Result<u64, EngineHostError> {
        let generation = self.next_view_generation;
        if generation == 0 {
            return Err(EngineHostError::ViewGenerationExhausted);
        }

        self.next_view_generation = generation.checked_add(1).unwrap_or(0);
        Ok(generation)
    }

    fn view_mut(&mut self, tab: TabId) -> Result<&mut HostedView, EngineHostError> {
        self.views
            .get_mut(&tab)
            .ok_or(EngineHostError::UnknownTab(tab))
    }

    fn validate_frame_request(
        &mut self,
        request: EngineFrameRequest,
    ) -> Result<&mut HostedView, EngineHostError> {
        let hosted = self.view_mut(request.tab)?;
        let active_matches = hosted.generation == request.view_generation
            && hosted
                .view
                .active_frame_request()
                .is_some_and(|active| active.get() == request.request_id);

        if active_matches {
            Ok(hosted)
        } else {
            Err(EngineHostError::StaleFrameRequest {
                tab: request.tab,
                view_generation: request.view_generation,
                request_id: request.request_id,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BrowserApp, BrowserWindow};

    fn initial_tab() -> TabId {
        let app = BrowserApp::bootstrap().expect("browser bootstrap");
        app.windows()
            .next()
            .and_then(BrowserWindow::active_tab_id)
            .expect("bootstrap creates an active tab")
    }

    #[test]
    fn one_engine_view_is_owned_per_tab() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");

        host.create_view(tab).expect("first view");
        assert!(host.has_view(tab));
        assert_eq!(
            host.create_view(tab),
            Err(EngineHostError::DuplicateView(tab))
        );
        assert!(host.close_view(tab));
        assert!(!host.has_view(tab));
    }

    #[test]
    fn local_document_runs_through_view_frame_lifecycle() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        host.load_local_html(tab, "<main>Zorya</main>")
            .expect("local document");

        let request = host
            .begin_frame(tab)
            .expect("begin frame")
            .expect("document load schedules an initial frame");
        {
            let frame = host
                .render_frame(request, Viewport::new(640, 480))
                .expect("render frame");
            assert_eq!(frame.status(), EngineFrameStatus::Initial);
        }

        host.complete_frame(request).expect("complete frame");
        assert_eq!(host.begin_frame(tab).expect("next frame"), None);
    }

    #[test]
    fn stale_request_cannot_target_recreated_view_for_same_tab() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("first view");
        host.load_local_html(tab, "<p>first</p>")
            .expect("first document");
        let stale = host
            .begin_frame(tab)
            .expect("first frame")
            .expect("first request");

        assert!(host.close_view(tab));
        host.create_view(tab).expect("replacement view");
        host.load_local_html(tab, "<p>replacement</p>")
            .expect("replacement document");
        let current = host
            .begin_frame(tab)
            .expect("replacement frame")
            .expect("replacement request");

        assert_eq!(stale.request_id(), current.request_id());
        assert_ne!(stale.view_generation(), current.view_generation());
        assert_eq!(
            host.render_frame(stale, Viewport::new(640, 480))
                .map(|_| ()),
            Err(EngineHostError::StaleFrameRequest {
                tab,
                view_generation: stale.view_generation(),
                request_id: stale.request_id(),
            })
        );

        {
            let frame = host
                .render_frame(current, Viewport::new(640, 480))
                .expect("current request remains valid");
            assert_eq!(frame.status(), EngineFrameStatus::Initial);
        }
        host.complete_frame(current)
            .expect("complete current frame");
    }

    #[test]
    fn render_failure_keeps_request_available_for_explicit_discard() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        host.load_local_html(tab, "<p>bounded</p>")
            .expect("local document");

        let request = host
            .begin_frame(tab)
            .expect("begin frame")
            .expect("initial frame");
        assert!(matches!(
            host.render_frame(request, Viewport::new(u32::MAX, u32::MAX)),
            Err(EngineHostError::Engine(_))
        ));

        host.discard_frame(request)
            .expect("failed render leaves the request active");
        assert!(host.begin_frame(tab).expect("begin retry").is_some());
    }

    #[test]
    fn host_can_schedule_a_resize_frame_after_initial_presentation() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        host.load_local_html(tab, "<p>resize</p>")
            .expect("local document");

        let initial = host
            .begin_frame(tab)
            .expect("begin initial frame")
            .expect("initial frame");
        {
            let frame = host
                .render_frame(initial, Viewport::new(640, 480))
                .expect("render initial frame");
            assert_eq!(frame.status(), EngineFrameStatus::Initial);
        }
        host.complete_frame(initial).expect("complete initial frame");

        host.request_frame(tab, EngineFrameCause::Resize)
            .expect("schedule resize");
        assert!(host.begin_frame(tab).expect("begin resize frame").is_some());
    }

    #[test]
    fn discarded_request_is_requeued_by_rarog_scheduler() {
        let tab = initial_tab();
        let mut host = EngineHost::new().expect("engine host");
        host.create_view(tab).expect("view");
        host.load_local_html(tab, "<p>retry</p>")
            .expect("local document");

        let first = host
            .begin_frame(tab)
            .expect("begin frame")
            .expect("initial frame");
        host.discard_frame(first).expect("discard frame");
        let retry = host
            .begin_frame(tab)
            .expect("begin retry")
            .expect("discard requeues work");

        assert!(retry.request_id() > first.request_id());
        assert_eq!(retry.view_generation(), first.view_generation());
    }
}
