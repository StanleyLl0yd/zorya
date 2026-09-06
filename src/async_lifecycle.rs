use crate::{BrowserWindowId, TabId};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct AsyncRequestId(u64);

impl AsyncRequestId {
    pub(crate) const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AsyncTarget {
    request: AsyncRequestId,
    window: BrowserWindowId,
    tab: TabId,
}

impl AsyncTarget {
    pub(crate) const fn request(self) -> AsyncRequestId {
        self.request
    }

    pub(crate) const fn window(self) -> BrowserWindowId {
        self.window
    }

    pub(crate) const fn tab(self) -> TabId {
        self.tab
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AsyncLifecycleError {
    RequestIdExhausted,
    RequestAlreadyPending {
        current: AsyncTarget,
        attempted: AsyncTarget,
    },
}

impl fmt::Display for AsyncLifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequestIdExhausted => {
                formatter.write_str("native async request identifier space is exhausted")
            }
            Self::RequestAlreadyPending { current, attempted } => write!(
                formatter,
                "async request {} is still pending; cannot start request {}",
                current.request().get(),
                attempted.request().get()
            ),
        }
    }
}

impl std::error::Error for AsyncLifecycleError {}

#[derive(Debug)]
pub(crate) struct AsyncRequestSequence {
    next: u64,
}

impl Default for AsyncRequestSequence {
    fn default() -> Self {
        Self::new()
    }
}

impl AsyncRequestSequence {
    pub(crate) const fn new() -> Self {
        Self { next: 1 }
    }

    pub(crate) fn allocate(
        &mut self,
        window: BrowserWindowId,
        tab: TabId,
    ) -> Result<AsyncTarget, AsyncLifecycleError> {
        let request = self.next;
        if request == 0 {
            return Err(AsyncLifecycleError::RequestIdExhausted);
        }

        self.next = request.checked_add(1).unwrap_or(0);
        Ok(AsyncTarget {
            request: AsyncRequestId(request),
            window,
            tab,
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl CancellationToken {
    pub(crate) fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

#[derive(Debug, Default)]
pub(crate) struct PendingRequest {
    current: Option<AsyncTarget>,
}

impl PendingRequest {
    pub(crate) const fn is_pending(&self) -> bool {
        self.current.is_some()
    }

    pub(crate) fn is_current(&self, target: AsyncTarget) -> bool {
        self.current == Some(target)
    }

    pub(crate) fn begin(&mut self, target: AsyncTarget) -> Result<(), AsyncLifecycleError> {
        if let Some(current) = self.current {
            return Err(AsyncLifecycleError::RequestAlreadyPending {
                current,
                attempted: target,
            });
        }

        self.current = Some(target);
        Ok(())
    }

    pub(crate) fn complete_if_current(&mut self, target: AsyncTarget) -> bool {
        if self.current == Some(target) {
            self.current = None;
            true
        } else {
            false
        }
    }

    pub(crate) fn invalidate(&mut self) {
        self.current = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BrowserApp;

    fn target_ids() -> (BrowserWindowId, TabId) {
        let app = BrowserApp::bootstrap().expect("browser bootstrap");
        let window = app.windows().next().expect("bootstrap window");
        (
            window.id(),
            window.active_tab_id().expect("bootstrap active tab"),
        )
    }

    #[test]
    fn cancellation_is_visible_to_clones() {
        let token = CancellationToken::new();
        let clone = token.clone();

        assert!(!token.is_cancelled());
        assert!(!clone.is_cancelled());

        token.cancel();

        assert!(token.is_cancelled());
        assert!(clone.is_cancelled());
    }

    #[test]
    fn request_sequence_is_monotonic() {
        let (window, tab) = target_ids();
        let mut sequence = AsyncRequestSequence::new();

        let first = sequence.allocate(window, tab).expect("first request");
        let second = sequence.allocate(window, tab).expect("second request");

        assert!(second.request().get() > first.request().get());
        assert_eq!(first.window(), window);
        assert_eq!(first.tab(), tab);
    }

    #[test]
    fn stale_completion_cannot_clear_new_pending_request() {
        let (window, tab) = target_ids();
        let mut sequence = AsyncRequestSequence::new();
        let mut pending = PendingRequest::default();
        let stale = sequence.allocate(window, tab).expect("stale request");
        let current = sequence.allocate(window, tab).expect("current request");

        pending.begin(stale).expect("begin stale request");
        assert!(pending.complete_if_current(stale));
        pending.begin(current).expect("begin current request");

        assert!(!pending.complete_if_current(stale));
        assert!(pending.is_pending());
        assert!(pending.complete_if_current(current));
        assert!(!pending.is_pending());
    }

    #[test]
    fn invalidation_rejects_late_completion() {
        let (window, tab) = target_ids();
        let mut sequence = AsyncRequestSequence::new();
        let mut pending = PendingRequest::default();
        let target = sequence.allocate(window, tab).expect("request");

        pending.begin(target).expect("begin request");
        assert!(pending.is_current(target));

        pending.invalidate();

        assert!(!pending.is_current(target));
        assert!(!pending.complete_if_current(target));
        assert!(!pending.is_pending());
    }

    #[test]
    fn overlapping_request_is_rejected() {
        let (window, tab) = target_ids();
        let mut sequence = AsyncRequestSequence::new();
        let mut pending = PendingRequest::default();
        let current = sequence.allocate(window, tab).expect("current request");
        let attempted = sequence.allocate(window, tab).expect("attempted request");

        pending.begin(current).expect("begin current request");

        assert_eq!(
            pending.begin(attempted),
            Err(AsyncLifecycleError::RequestAlreadyPending { current, attempted })
        );
    }
}
