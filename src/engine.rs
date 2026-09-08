use crate::TabId;
use crate::http_transport::HttpTransport;
use rarog_compositor::FrameCause;
use rarog_engine::{
    BaseUrl, Engine, EngineError, FrameStatus, NavigationCompletion,
    NavigationId as RarogNavigationId, NavigationRequest, NavigationStartOutcome, View, ViewOptions,
};
use rarog_fetch::{NetworkCapability, NetworkPoll};
use rarog_host::{
    HostControlErrorKind, HostControlPlane, NavigationContextCapability, NavigationContextId,
    NetworkOperationId,
};
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
pub struct EngineNavigationRequest {
    tab: TabId,
    view_generation: u64,
    navigation_id: u64,
}

impl EngineNavigationRequest {
    pub const fn tab(self) -> TabId {
        self.tab
    }

    pub const fn view_generation(self) -> u64 {
        self.view_generation
    }

    pub const fn navigation_id(self) -> u64 {
        self.navigation_id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineNavigationPoll {
    Pending,
    Committed {
        location: String,
        status: u16,
        source_bytes: usize,
    },
    Failed {
        message: String,
    },
    Stale,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineHostError {
    Engine(EngineError),
    DuplicateView(TabId),
    UnknownTab(TabId),
    ViewGenerationExhausted,
    Host(String),
    Navigation(String),
    InconsistentNavigationState {
        tab: TabId,
    },
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
            Self::Host(message) => write!(formatter, "Rarog Host authority failure: {message}"),
            Self::Navigation(message) => write!(formatter, "Rarog navigation failure: {message}"),
            Self::InconsistentNavigationState { tab } => write!(
                formatter,
                "engine/Host navigation state diverged for tab {}",
                tab.get()
            ),
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

#[derive(Clone, Copy)]
struct PendingHostedNavigation {
    navigation: RarogNavigationId,
    context: NavigationContextId,
    capability: NavigationContextCapability,
    operation: NetworkOperationId,
}

struct HostedView {
    generation: u64,
    view: View,
    committed_context: Option<NavigationContextId>,
    pending_navigation: Option<PendingHostedNavigation>,
}

pub struct EngineHost {
    engine: Engine,
    host: HostControlPlane,
    network: Option<Box<dyn NetworkCapability>>,
    views: BTreeMap<TabId, HostedView>,
    next_view_generation: u64,
}

impl EngineHost {
    pub fn new() -> Result<Self, EngineHostError> {
        Self::with_network(None)
    }

    fn with_network(
        network: Option<Box<dyn NetworkCapability>>,
    ) -> Result<Self, EngineHostError> {
        Ok(Self {
            engine: Engine::builder().build()?,
            host: HostControlPlane::with_default_limits()
                .map_err(|error| EngineHostError::Host(error.to_string()))?,
            network,
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
        self.views.insert(
            tab,
            HostedView {
                generation,
                view,
                committed_context: None,
                pending_navigation: None,
            },
        );
        Ok(())
    }

    pub fn close_view(&mut self, tab: TabId) -> Result<bool, EngineHostError> {
        let Some(hosted) = self.views.remove(&tab) else {
            return Ok(false);
        };

        if let Some(pending) = hosted.pending_navigation {
            self.cancel_pending_authority(pending)?;
        }
        if let Some(context) = hosted.committed_context {
            self.host
                .close_navigation_context(context)
                .map_err(|error| EngineHostError::Host(error.to_string()))?;
        }
        Ok(true)
    }

    pub fn has_view(&self, tab: TabId) -> bool {
        self.views.contains_key(&tab)
    }

    pub fn load_local_html(
        &mut self,
        tab: TabId,
        source: impl Into<String>,
    ) -> Result<(), EngineHostError> {
        let (pending, committed) = {
            let hosted = self.view_mut(tab)?;
            hosted.view.load_html(source, BaseUrl::about_blank())?;
            (
                hosted.pending_navigation.take(),
                hosted.committed_context.take(),
            )
        };

        if let Some(pending) = pending {
            self.cancel_pending_authority(pending)?;
        }
        if let Some(context) = committed {
            self.host
                .close_navigation_context(context)
                .map_err(|error| EngineHostError::Host(error.to_string()))?;
        }
        Ok(())
    }

    pub fn begin_navigation(
        &mut self,
        tab: TabId,
        location: impl Into<String>,
    ) -> Result<Option<EngineNavigationRequest>, EngineHostError> {
        let location = location.into();
        let (generation, outcome) = {
            let hosted = self.view_mut(tab)?;
            (
                hosted.generation,
                hosted
                    .view
                    .begin_navigation(NavigationRequest::new(BaseUrl::new(location)))
                    .map_err(|error| EngineHostError::Navigation(error.to_string()))?,
            )
        };

        let NavigationStartOutcome::Started(start) = outcome else {
            return Ok(None);
        };
        let (transport, superseded) = start.into_parts();
        let (navigation, network_request) = transport.into_parts();

        if let Some(superseded) = superseded {
            let previous = self.view_mut(tab)?.pending_navigation.take();
            if previous.map(|pending| pending.navigation) != Some(superseded) {
                let _ = self.view_mut(tab)?.view.cancel_navigation(navigation);
                return Err(EngineHostError::InconsistentNavigationState { tab });
            }
            if let Some(previous) = previous {
                self.cancel_pending_authority(previous)?;
            }
        } else if self.view_mut(tab)?.pending_navigation.is_some() {
            let _ = self.view_mut(tab)?.view.cancel_navigation(navigation);
            return Err(EngineHostError::InconsistentNavigationState { tab });
        }

        let context = match self.host.open_navigation_context(network_request.url()) {
            Ok(context) => context.context(),
            Err(error) => {
                let _ = self.view_mut(tab)?.view.cancel_navigation(navigation);
                return Err(EngineHostError::Host(error.to_string()));
            }
        };
        let capability = match self
            .host
            .grant_navigation_context_network_capability(context)
        {
            Ok(capability) => capability,
            Err(error) => {
                let _ = self.host.close_navigation_context(context);
                let _ = self.view_mut(tab)?.view.cancel_navigation(navigation);
                return Err(EngineHostError::Host(error.to_string()));
            }
        };

        if self.network.is_none() {
            match HttpTransport::new() {
                Ok(network) => self.network = Some(Box::new(network)),
                Err(error) => {
                    let _ = self.host.close_navigation_context(context);
                    let _ = self.view_mut(tab)?.view.cancel_navigation(navigation);
                    return Err(EngineHostError::Navigation(error.to_string()));
                }
            }
        }

        let operation = {
            let network = self
                .network
                .as_deref_mut()
                .expect("HTTP transport initialized before Host operation");
            match self.host.start_navigation_context_network_operation(
                capability,
                network_request,
                network,
            ) {
                Ok(operation) => operation,
                Err(error) => {
                    let _ = self.host.close_navigation_context(context);
                    let _ = self.view_mut(tab)?.view.cancel_navigation(navigation);
                    return Err(EngineHostError::Host(error.to_string()));
                }
            }
        };

        self.view_mut(tab)?.pending_navigation = Some(PendingHostedNavigation {
            navigation,
            context,
            capability,
            operation,
        });

        Ok(Some(EngineNavigationRequest {
            tab,
            view_generation: generation,
            navigation_id: navigation.get(),
        }))
    }

    pub fn poll_navigation(
        &mut self,
        request: EngineNavigationRequest,
    ) -> Result<EngineNavigationPoll, EngineHostError> {
        let Some(pending) = self.pending_navigation_for(request)? else {
            return Ok(EngineNavigationPoll::Stale);
        };
        let Some(network) = self.network.as_deref_mut() else {
            return Err(EngineHostError::InconsistentNavigationState { tab: request.tab });
        };

        let poll = self.host.poll_navigation_context_network_operation(
            pending.capability,
            pending.operation,
            network,
        );
        match poll {
            Ok(NetworkPoll::Pending) => Ok(EngineNavigationPoll::Pending),
            Ok(NetworkPoll::Complete(response)) => {
                if let Err(error) = self
                    .host
                    .revoke_navigation_context_capability(pending.capability)
                {
                    self.abort_pending_navigation(request, pending);
                    return Err(EngineHostError::Host(error.to_string()));
                }

                let completion = self
                    .view_mut(request.tab)?
                    .view
                    .complete_navigation(pending.navigation, response);
                let removed = self.take_pending_navigation(request)?;
                if removed.map(|state| state.navigation) != Some(pending.navigation) {
                    return Err(EngineHostError::InconsistentNavigationState { tab: request.tab });
                }

                match completion {
                    NavigationCompletion::Committed(commit) => {
                        let previous = {
                            let hosted = self.view_mut(request.tab)?;
                            hosted.committed_context.replace(pending.context)
                        };
                        if let Some(previous) = previous {
                            self.host
                                .close_navigation_context(previous)
                                .map_err(|error| EngineHostError::Host(error.to_string()))?;
                        }
                        Ok(EngineNavigationPoll::Committed {
                            location: commit.url().as_str().to_owned(),
                            status: commit.status(),
                            source_bytes: commit.source_bytes(),
                        })
                    }
                    NavigationCompletion::Failed(error) => {
                        self.host
                            .close_navigation_context(pending.context)
                            .map_err(|close| EngineHostError::Host(close.to_string()))?;
                        Ok(EngineNavigationPoll::Failed {
                            message: error.to_string(),
                        })
                    }
                    NavigationCompletion::Stale => {
                        self.host
                            .close_navigation_context(pending.context)
                            .map_err(|error| EngineHostError::Host(error.to_string()))?;
                        Ok(EngineNavigationPoll::Stale)
                    }
                }
            }
            Err(error) => {
                let _ = self
                    .view_mut(request.tab)?
                    .view
                    .cancel_navigation(pending.navigation);
                let removed = self.take_pending_navigation(request)?;
                if removed.map(|state| state.navigation) != Some(pending.navigation) {
                    return Err(EngineHostError::InconsistentNavigationState { tab: request.tab });
                }
                self.host
                    .close_navigation_context(pending.context)
                    .map_err(|close| EngineHostError::Host(close.to_string()))?;
                if matches!(error.kind, HostControlErrorKind::Fetch(_)) {
                    Ok(EngineNavigationPoll::Failed {
                        message: error.to_string(),
                    })
                } else {
                    Err(EngineHostError::Host(error.to_string()))
                }
            }
        }
    }

    pub fn cancel_navigation(
        &mut self,
        request: EngineNavigationRequest,
    ) -> Result<bool, EngineHostError> {
        let Some(pending) = self.pending_navigation_for(request)? else {
            return Ok(false);
        };

        let _ = self
            .view_mut(request.tab)?
            .view
            .cancel_navigation(pending.navigation);
        let removed = self.take_pending_navigation(request)?;
        if removed.map(|state| state.navigation) != Some(pending.navigation) {
            return Err(EngineHostError::InconsistentNavigationState { tab: request.tab });
        }
        self.cancel_pending_authority(pending)?;
        Ok(true)
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

    fn pending_navigation_for(
        &self,
        request: EngineNavigationRequest,
    ) -> Result<Option<PendingHostedNavigation>, EngineHostError> {
        let hosted = self
            .views
            .get(&request.tab)
            .ok_or(EngineHostError::UnknownTab(request.tab))?;
        if hosted.generation != request.view_generation {
            return Ok(None);
        }
        Ok(hosted.pending_navigation.filter(|pending| {
            pending.navigation.get() == request.navigation_id
        }))
    }

    fn take_pending_navigation(
        &mut self,
        request: EngineNavigationRequest,
    ) -> Result<Option<PendingHostedNavigation>, EngineHostError> {
        let hosted = self.view_mut(request.tab)?;
        if hosted.generation != request.view_generation
            || !hosted
                .pending_navigation
                .is_some_and(|pending| pending.navigation.get() == request.navigation_id)
        {
            return Ok(None);
        }
        Ok(hosted.pending_navigation.take())
    }

    fn abort_pending_navigation(
        &mut self,
        request: EngineNavigationRequest,
        pending: PendingHostedNavigation,
    ) {
        let _ = self
            .view_mut(request.tab)
            .map(|hosted| hosted.view.cancel_navigation(pending.navigation));
        let _ = self.take_pending_navigation(request);
        let _ = self.host.close_navigation_context(pending.context);
    }

    fn cancel_pending_authority(
        &mut self,
        pending: PendingHostedNavigation,
    ) -> Result<(), EngineHostError> {
        let cancel_error = match self.network.as_deref_mut() {
            Some(network) => self
                .host
                .cancel_navigation_context_network_operation(
                    pending.capability,
                    pending.operation,
                    network,
                )
                .err(),
            None => {
                return Err(EngineHostError::Host(
                    "pending network authority exists without a transport".into(),
                ));
            }
        };
        let close_result = self.host.close_navigation_context(pending.context);

        if let Err(error) = close_result {
            return Err(EngineHostError::Host(error.to_string()));
        }
        if let Some(error) = cancel_error {
            return Err(EngineHostError::Host(error.to_string()));
        }
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
        assert!(host.close_view(tab).expect("close view"));
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

        assert!(host.close_view(tab).expect("close view"));
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
    fn simultaneous_views_keep_frame_requests_isolated_by_tab_and_generation() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first_tab = app.create_tab(window).expect("first tab");
        let second_tab = app.create_tab(window).expect("second tab");
        let mut host = EngineHost::new().expect("engine host");

        host.create_view(first_tab).expect("first view");
        host.create_view(second_tab).expect("second view");
        host.load_local_html(first_tab, "<p>first tab</p>")
            .expect("first document");
        host.load_local_html(second_tab, "<p>second tab</p>")
            .expect("second document");

        let first = host
            .begin_frame(first_tab)
            .expect("first begin")
            .expect("first frame");
        let second = host
            .begin_frame(second_tab)
            .expect("second begin")
            .expect("second frame");

        assert_eq!(first.tab(), first_tab);
        assert_eq!(second.tab(), second_tab);
        assert_ne!(first.view_generation(), second.view_generation());

        let forged_second = EngineFrameRequest {
            tab: second_tab,
            view_generation: first.view_generation(),
            request_id: second.request_id(),
        };
        assert_eq!(
            host.render_frame(forged_second, Viewport::new(640, 480))
                .map(|_| ()),
            Err(EngineHostError::StaleFrameRequest {
                tab: second_tab,
                view_generation: first.view_generation(),
                request_id: second.request_id(),
            })
        );

        {
            let frame = host
                .render_frame(first, Viewport::new(640, 480))
                .expect("render first");
            assert_eq!(frame.status(), EngineFrameStatus::Initial);
        }
        host.complete_frame(first).expect("complete first");
        assert!(host.close_view(first_tab).expect("close first view"));

        {
            let frame = host
                .render_frame(second, Viewport::new(640, 480))
                .expect("second remains independently valid");
            assert_eq!(frame.status(), EngineFrameStatus::Initial);
        }
        host.complete_frame(second).expect("complete second");
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
        host.complete_frame(initial)
            .expect("complete initial frame");

        host.request_frame(tab, EngineFrameCause::Resize)
            .expect("schedule resize");
        assert!(host.begin_frame(tab).expect("begin resize frame").is_some());
    }

    use rarog_fetch::{
        FetchError, FetchResponse, HeaderList, NetworkRequest, NetworkTicket,
    };
    use std::num::NonZeroU64;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct FixtureNetworkStats {
        starts: usize,
        polls: usize,
        cancels: usize,
    }

    struct FixtureNetwork {
        next_ticket: u64,
        requests: BTreeMap<NetworkTicket, NetworkRequest>,
        fail_on_poll: Option<usize>,
        stats: Arc<Mutex<FixtureNetworkStats>>,
    }

    impl FixtureNetwork {
        fn new(
            fail_on_poll: Option<usize>,
            stats: Arc<Mutex<FixtureNetworkStats>>,
        ) -> Self {
            Self {
                next_ticket: 1,
                requests: BTreeMap::new(),
                fail_on_poll,
                stats,
            }
        }
    }

    impl NetworkCapability for FixtureNetwork {
        fn start(&mut self, request: NetworkRequest) -> Result<NetworkTicket, FetchError> {
            let ticket = NetworkTicket::new(NonZeroU64::new(self.next_ticket).expect("ticket"));
            self.next_ticket += 1;
            self.requests.insert(ticket, request);
            self.stats.lock().expect("stats").starts += 1;
            Ok(ticket)
        }

        fn poll(&mut self, ticket: NetworkTicket) -> Result<NetworkPoll, FetchError> {
            let mut stats = self.stats.lock().expect("stats");
            stats.polls += 1;
            let poll_number = stats.polls;
            drop(stats);

            if self.fail_on_poll == Some(poll_number) {
                self.requests.remove(&ticket);
                return Err(FetchError::network("fixture network failure"));
            }

            let request = self
                .requests
                .remove(&ticket)
                .ok_or_else(|| FetchError::network("unknown fixture ticket"))?;
            let mut headers = HeaderList::default();
            headers
                .append("content-type", "text/html; charset=utf-8")
                .expect("fixture content type");
            FetchResponse::try_new(
                Some(request.url().clone()),
                200,
                headers,
                b"<main>Zorya HTTP</main>".to_vec(),
                request.max_response_body_bytes(),
            )
            .map(NetworkPoll::Complete)
        }

        fn cancel(&mut self, ticket: NetworkTicket) -> Result<(), FetchError> {
            self.requests.remove(&ticket);
            self.stats.lock().expect("stats").cancels += 1;
            Ok(())
        }
    }

    fn engine_host_with_fixture(
        fail_on_poll: Option<usize>,
    ) -> (EngineHost, Arc<Mutex<FixtureNetworkStats>>) {
        let stats = Arc::new(Mutex::new(FixtureNetworkStats::default()));
        let network = FixtureNetwork::new(fail_on_poll, Arc::clone(&stats));
        let host = EngineHost::with_network(Some(Box::new(network))).expect("engine host");
        (host, stats)
    }

    #[test]
    fn successful_remote_navigation_promotes_target_before_retiring_committed_context() {
        let tab = initial_tab();
        let (mut host, _) = engine_host_with_fixture(None);
        host.create_view(tab).expect("view");

        let first = host
            .begin_navigation(tab, "https://first.example/")
            .expect("begin first")
            .expect("first forwarded");
        assert_eq!(host.host.active_navigation_contexts(), 1);
        assert!(matches!(
            host.poll_navigation(first).expect("poll first"),
            EngineNavigationPoll::Committed { .. }
        ));
        let first_context = host
            .views
            .get(&tab)
            .and_then(|hosted| hosted.committed_context)
            .expect("first committed context");

        let second = host
            .begin_navigation(tab, "https://second.example/")
            .expect("begin second")
            .expect("second forwarded");
        assert_eq!(host.host.active_navigation_contexts(), 2);
        assert_eq!(
            host.views
                .get(&tab)
                .and_then(|hosted| hosted.committed_context),
            Some(first_context)
        );

        assert!(matches!(
            host.poll_navigation(second).expect("poll second"),
            EngineNavigationPoll::Committed { .. }
        ));
        let second_context = host
            .views
            .get(&tab)
            .and_then(|hosted| hosted.committed_context)
            .expect("second committed context");
        assert_ne!(second_context, first_context);
        assert_eq!(host.host.active_navigation_contexts(), 1);
        assert!(host.host.navigation_context(first_context).is_err());
        assert!(host.host.navigation_context(second_context).is_ok());
    }

    #[test]
    fn network_failure_closes_only_pending_target_authority() {
        let tab = initial_tab();
        let (mut host, _) = engine_host_with_fixture(Some(2));
        host.create_view(tab).expect("view");

        let committed = host
            .begin_navigation(tab, "https://committed.example/")
            .expect("begin committed")
            .expect("forwarded");
        assert!(matches!(
            host.poll_navigation(committed).expect("commit"),
            EngineNavigationPoll::Committed { .. }
        ));
        let committed_context = host
            .views
            .get(&tab)
            .and_then(|hosted| hosted.committed_context)
            .expect("committed context");

        let failing = host
            .begin_navigation(tab, "https://failing.example/")
            .expect("begin failing")
            .expect("forwarded");
        assert_eq!(host.host.active_navigation_contexts(), 2);
        assert!(matches!(
            host.poll_navigation(failing).expect("network failure"),
            EngineNavigationPoll::Failed { .. }
        ));
        assert_eq!(host.host.active_navigation_contexts(), 1);
        assert_eq!(
            host.views
                .get(&tab)
                .and_then(|hosted| hosted.committed_context),
            Some(committed_context)
        );
        assert!(host.host.navigation_context(committed_context).is_ok());
    }

    #[test]
    fn newer_remote_navigation_cancels_and_closes_superseded_pending_authority() {
        let tab = initial_tab();
        let (mut host, stats) = engine_host_with_fixture(None);
        host.create_view(tab).expect("view");

        let stale = host
            .begin_navigation(tab, "https://stale.example/")
            .expect("begin stale")
            .expect("forwarded");
        let current = host
            .begin_navigation(tab, "https://current.example/")
            .expect("begin current")
            .expect("forwarded");

        assert!(current.navigation_id() > stale.navigation_id());
        assert_eq!(host.host.active_navigation_contexts(), 1);
        assert_eq!(stats.lock().expect("stats").cancels, 1);
        assert_eq!(
            host.poll_navigation(stale).expect("stale poll"),
            EngineNavigationPoll::Stale
        );
        assert!(matches!(
            host.poll_navigation(current).expect("current poll"),
            EngineNavigationPoll::Committed { .. }
        ));
    }

    #[test]
    fn cancelling_remote_navigation_keeps_committed_authority_alive() {
        let tab = initial_tab();
        let (mut host, stats) = engine_host_with_fixture(None);
        host.create_view(tab).expect("view");

        let committed = host
            .begin_navigation(tab, "https://committed.example/")
            .expect("begin committed")
            .expect("forwarded");
        assert!(matches!(
            host.poll_navigation(committed).expect("commit"),
            EngineNavigationPoll::Committed { .. }
        ));
        let committed_context = host
            .views
            .get(&tab)
            .and_then(|hosted| hosted.committed_context)
            .expect("committed context");

        let pending = host
            .begin_navigation(tab, "https://pending.example/")
            .expect("begin pending")
            .expect("forwarded");
        assert!(host.cancel_navigation(pending).expect("cancel"));
        assert_eq!(host.host.active_navigation_contexts(), 1);
        assert_eq!(stats.lock().expect("stats").cancels, 1);
        assert!(host.host.navigation_context(committed_context).is_ok());
        assert!(!host.cancel_navigation(pending).expect("stale cancel"));
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
