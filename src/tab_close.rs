use crate::{
    BrowserApp, BrowserModelError, BrowserWindowId, Tab, TabActivationId, TabActivationIntent,
    TabActivationStart, TabCloseResult, TabId, TabPresentationHandoff, WebContentPresentation,
};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveTabCloseHandoff {
    closing: TabId,
    activation: TabActivationStart,
}

impl ActiveTabCloseHandoff {
    pub const fn closing_tab(self) -> TabId {
        self.closing
    }

    pub const fn activation(self) -> TabActivationStart {
        self.activation
    }

    pub const fn fallback_tab(self) -> TabId {
        self.activation.intent().to()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LastTabCloseHandoff {
    closing: TabId,
    invalidated_activation: Option<TabActivationIntent>,
}

impl LastTabCloseHandoff {
    pub const fn closing_tab(self) -> TabId {
        self.closing
    }

    pub const fn invalidated_activation(self) -> Option<TabActivationIntent> {
        self.invalidated_activation
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabCloseStart {
    Closed(TabCloseResult),
    ActiveWithFallback(ActiveTabCloseHandoff),
    ActiveLast(LastTabCloseHandoff),
}

impl TabCloseStart {
    pub fn closing_tab(&self) -> TabId {
        match self {
            Self::Closed(result) => result.tab().id(),
            Self::ActiveWithFallback(handoff) => handoff.closing_tab(),
            Self::ActiveLast(handoff) => handoff.closing_tab(),
        }
    }

    pub const fn requires_neutral(&self) -> bool {
        !matches!(self, Self::Closed(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabCloseCommitError {
    Browser(BrowserModelError),
    PresentedTabMismatch {
        expected: TabId,
        actual: TabId,
    },
    NeutralContentRequired {
        tab: TabId,
    },
    PresentationActivationMismatch {
        expected: Option<TabActivationId>,
        actual: Option<TabActivationId>,
    },
    ActiveTabChanged {
        expected: TabId,
        actual: Option<TabId>,
    },
    LastTabShapeChanged {
        closing: TabId,
        tab_count: usize,
    },
}

impl fmt::Display for TabCloseCommitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Browser(error) => error.fmt(formatter),
            Self::PresentedTabMismatch { expected, actual } => write!(
                formatter,
                "tab close expected privileged chrome for tab {}, but presentation represents tab {}",
                expected.get(),
                actual.get()
            ),
            Self::NeutralContentRequired { tab } => write!(
                formatter,
                "active tab {} cannot close before Web content is confirmed neutral",
                tab.get()
            ),
            Self::PresentationActivationMismatch { expected, actual } => match (expected, actual) {
                (Some(expected), Some(actual)) => write!(
                    formatter,
                    "tab close expected presentation activation {}, current activation is {}",
                    expected.get(),
                    actual.get()
                ),
                (Some(expected), None) => write!(
                    formatter,
                    "tab close expected presentation activation {}, but no activation is pending",
                    expected.get()
                ),
                (None, Some(actual)) => write!(
                    formatter,
                    "last-tab close requires no pending presentation activation; found {}",
                    actual.get()
                ),
                (None, None) => {
                    formatter.write_str("tab close presentation activation state is unchanged")
                }
            },
            Self::ActiveTabChanged { expected, actual } => match actual {
                Some(actual) => write!(
                    formatter,
                    "tab close expected active tab {}, current active tab is {}",
                    expected.get(),
                    actual.get()
                ),
                None => write!(
                    formatter,
                    "tab close expected active tab {}, but the window has no active tab",
                    expected.get()
                ),
            },
            Self::LastTabShapeChanged { closing, tab_count } => write!(
                formatter,
                "last-tab close for tab {} became stale because the window now contains {} tabs",
                closing.get(),
                tab_count
            ),
        }
    }
}

impl std::error::Error for TabCloseCommitError {}

impl From<BrowserModelError> for TabCloseCommitError {
    fn from(error: BrowserModelError) -> Self {
        Self::Browser(error)
    }
}

impl BrowserApp {
    pub fn begin_tab_close(
        &mut self,
        window: BrowserWindowId,
        tab: TabId,
    ) -> Result<TabCloseStart, BrowserModelError> {
        let (active, fallback) = {
            let browser_window = self
                .window(window)
                .ok_or(BrowserModelError::UnknownWindow(window))?;
            let position = browser_window
                .tabs()
                .iter()
                .position(|candidate| candidate.id() == tab)
                .ok_or(BrowserModelError::UnknownTab { window, tab })?;
            let active = browser_window.active_tab_id();
            let fallback = if active == Some(tab) {
                position
                    .checked_add(1)
                    .and_then(|index| browser_window.tabs().get(index))
                    .or_else(|| {
                        position
                            .checked_sub(1)
                            .and_then(|index| browser_window.tabs().get(index))
                    })
                    .map(Tab::id)
            } else {
                None
            };
            (active, fallback)
        };

        if active != Some(tab) {
            return self.close_tab(window, tab).map(TabCloseStart::Closed);
        }

        if let Some(fallback) = fallback {
            let activation = self.begin_tab_activation(window, fallback)?;
            return Ok(TabCloseStart::ActiveWithFallback(ActiveTabCloseHandoff {
                closing: tab,
                activation,
            }));
        }

        let invalidated_activation = self.set_active_tab(window, tab)?;
        Ok(TabCloseStart::ActiveLast(LastTabCloseHandoff {
            closing: tab,
            invalidated_activation,
        }))
    }

    pub fn commit_active_tab_close_after_neutral(
        &mut self,
        window: BrowserWindowId,
        close: ActiveTabCloseHandoff,
        presentation: &TabPresentationHandoff,
    ) -> Result<TabCloseResult, TabCloseCommitError> {
        let intent = close.activation().intent();
        validate_neutral_presentation(
            presentation,
            close.closing_tab(),
            Some(intent.id()),
        )?;

        let active = self
            .window(window)
            .ok_or(BrowserModelError::UnknownWindow(window))?
            .active_tab_id();
        if active != Some(close.closing_tab()) {
            return Err(TabCloseCommitError::ActiveTabChanged {
                expected: close.closing_tab(),
                actual: active,
            });
        }

        let committed = self.commit_tab_activation(window, intent.id())?;
        debug_assert_eq!(committed, close.fallback_tab());

        let result = self.close_tab(window, close.closing_tab())?;
        debug_assert_eq!(result.active_tab(), Some(committed));
        Ok(result)
    }

    pub fn commit_last_tab_close_after_neutral(
        &mut self,
        window: BrowserWindowId,
        close: LastTabCloseHandoff,
        presentation: &TabPresentationHandoff,
    ) -> Result<TabCloseResult, TabCloseCommitError> {
        validate_neutral_presentation(presentation, close.closing_tab(), None)?;

        let browser_window = self
            .window(window)
            .ok_or(BrowserModelError::UnknownWindow(window))?;
        let active = browser_window.active_tab_id();
        if active != Some(close.closing_tab()) {
            return Err(TabCloseCommitError::ActiveTabChanged {
                expected: close.closing_tab(),
                actual: active,
            });
        }
        if browser_window.tabs().len() != 1 {
            return Err(TabCloseCommitError::LastTabShapeChanged {
                closing: close.closing_tab(),
                tab_count: browser_window.tabs().len(),
            });
        }

        Ok(self.close_tab(window, close.closing_tab())?)
    }
}

fn validate_neutral_presentation(
    presentation: &TabPresentationHandoff,
    closing: TabId,
    activation: Option<TabActivationId>,
) -> Result<(), TabCloseCommitError> {
    if presentation.represented_tab() != closing {
        return Err(TabCloseCommitError::PresentedTabMismatch {
            expected: closing,
            actual: presentation.represented_tab(),
        });
    }
    if presentation.content() != WebContentPresentation::Neutral {
        return Err(TabCloseCommitError::NeutralContentRequired { tab: closing });
    }

    let actual_activation = presentation
        .pending_activation()
        .map(TabActivationIntent::id);
    if actual_activation != activation {
        return Err(TabCloseCommitError::PresentationActivationMismatch {
            expected: activation,
            actual: actual_activation,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PresentationHandoffError;

    #[test]
    fn background_tab_closes_immediately_without_changing_active_tab() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let second = app.create_tab(window).expect("second");

        let start = app
            .begin_tab_close(window, second)
            .expect("begin background close");

        let TabCloseStart::Closed(result) = start else {
            panic!("background close should complete immediately");
        };
        assert_eq!(result.tab().id(), second);
        assert_eq!(result.active_tab(), Some(first));
        assert!(app.window(window).expect("window").tab(second).is_none());
    }

    #[test]
    fn active_close_with_neighbor_starts_activation_without_removing_source() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let second = app.create_tab(window).expect("second");

        let start = app
            .begin_tab_close(window, first)
            .expect("begin active close");
        let TabCloseStart::ActiveWithFallback(close) = start else {
            panic!("active close should require fallback activation");
        };

        assert_eq!(close.closing_tab(), first);
        assert_eq!(close.fallback_tab(), second);
        assert_eq!(close.activation().intent().from(), first);
        assert_eq!(close.activation().intent().to(), second);

        let browser_window = app.window(window).expect("window");
        assert_eq!(browser_window.active_tab_id(), Some(first));
        assert!(browser_window.tab(first).is_some());
        assert_eq!(
            browser_window.pending_tab_activation(),
            Some(close.activation().intent())
        );
    }

    #[test]
    fn active_close_cannot_commit_before_neutral_content() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let _second = app.create_tab(window).expect("second");
        let start = app
            .begin_tab_close(window, first)
            .expect("begin active close");
        let TabCloseStart::ActiveWithFallback(close) = start else {
            panic!("active close should require handoff");
        };
        let mut presentation = TabPresentationHandoff::new(first);
        presentation
            .begin_activation(close.activation().intent())
            .expect("begin presentation activation");

        assert_eq!(
            app.commit_active_tab_close_after_neutral(window, close, &presentation),
            Err(TabCloseCommitError::NeutralContentRequired { tab: first })
        );
        let browser_window = app.window(window).expect("window");
        assert_eq!(browser_window.active_tab_id(), Some(first));
        assert!(browser_window.tab(first).is_some());
    }

    #[test]
    fn active_close_commits_fallback_only_after_neutral_handoff() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let second = app.create_tab(window).expect("second");
        let start = app
            .begin_tab_close(window, first)
            .expect("begin active close");
        let TabCloseStart::ActiveWithFallback(close) = start else {
            panic!("active close should require handoff");
        };

        let intent = close.activation().intent();
        let mut presentation = TabPresentationHandoff::new(first);
        presentation
            .begin_activation(intent)
            .expect("begin presentation activation");
        presentation
            .confirm_neutral(intent.id())
            .expect("confirm neutral");

        let result = app
            .commit_active_tab_close_after_neutral(window, close, &presentation)
            .expect("commit active close");

        assert_eq!(result.tab().id(), first);
        assert_eq!(result.active_tab(), Some(second));
        let browser_window = app.window(window).expect("window");
        assert!(browser_window.tab(first).is_none());
        assert_eq!(browser_window.active_tab_id(), Some(second));
        assert_eq!(presentation.represented_tab(), first);
        assert_eq!(presentation.content(), WebContentPresentation::Neutral);

        presentation
            .acknowledge_chrome_commit(intent.id(), second)
            .expect("acknowledge target chrome");
        let permit = presentation
            .authorize_target_frame(intent.id(), second)
            .expect("authorize target frame");
        presentation
            .present_target_frame(permit)
            .expect("present target frame");

        assert_eq!(presentation.represented_tab(), second);
        assert_eq!(presentation.content(), WebContentPresentation::Tab(second));
    }

    #[test]
    fn closing_fallback_invalidates_active_close_activation() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let second = app.create_tab(window).expect("second");
        let start = app
            .begin_tab_close(window, first)
            .expect("begin active close");
        let TabCloseStart::ActiveWithFallback(close) = start else {
            panic!("active close should require handoff");
        };
        let intent = close.activation().intent();
        let mut presentation = TabPresentationHandoff::new(first);
        presentation
            .begin_activation(intent)
            .expect("begin presentation activation");
        presentation
            .confirm_neutral(intent.id())
            .expect("confirm neutral");

        let fallback_close = app.close_tab(window, second).expect("close fallback");
        assert_eq!(fallback_close.invalidated_activation(), Some(intent));
        presentation
            .invalidate_activation(intent.id())
            .expect("invalidate presentation activation");

        assert_eq!(
            app.commit_active_tab_close_after_neutral(window, close, &presentation),
            Err(TabCloseCommitError::PresentationActivationMismatch {
                expected: Some(intent.id()),
                actual: None,
            })
        );
        assert_eq!(
            app.window(window).expect("window").active_tab_id(),
            Some(first)
        );
    }

    #[test]
    fn last_active_tab_requires_neutral_content_before_removal() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let tab = app.create_tab(window).expect("tab");
        let start = app.begin_tab_close(window, tab).expect("begin close");
        let TabCloseStart::ActiveLast(close) = start else {
            panic!("last active tab should require neutral close");
        };
        let mut presentation = TabPresentationHandoff::new(tab);

        assert_eq!(
            app.commit_last_tab_close_after_neutral(window, close, &presentation),
            Err(TabCloseCommitError::NeutralContentRequired { tab })
        );

        presentation
            .confirm_current_tab_neutral(tab)
            .expect("confirm current tab neutral");
        let result = app
            .commit_last_tab_close_after_neutral(window, close, &presentation)
            .expect("commit last close");

        assert_eq!(result.tab().id(), tab);
        assert_eq!(result.active_tab(), None);
        let browser_window = app.window(window).expect("window");
        assert!(browser_window.tabs().is_empty());
        assert_eq!(browser_window.active_tab_id(), None);
        assert_eq!(presentation.content(), WebContentPresentation::Neutral);
    }

    #[test]
    fn last_tab_close_becomes_stale_if_window_shape_changes() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let start = app.begin_tab_close(window, first).expect("begin close");
        let TabCloseStart::ActiveLast(close) = start else {
            panic!("last active tab should require neutral close");
        };
        let mut presentation = TabPresentationHandoff::new(first);
        presentation
            .confirm_current_tab_neutral(first)
            .expect("confirm neutral");
        let second = app.create_tab(window).expect("second");

        assert_eq!(
            app.commit_last_tab_close_after_neutral(window, close, &presentation),
            Err(TabCloseCommitError::LastTabShapeChanged {
                closing: first,
                tab_count: 2,
            })
        );

        let browser_window = app.window(window).expect("window");
        assert!(browser_window.tab(first).is_some());
        assert!(browser_window.tab(second).is_some());
        assert_eq!(browser_window.active_tab_id(), Some(first));
    }

    #[test]
    fn last_tab_close_surfaces_invalidated_same_tab_activation() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let tab = app.create_tab(window).expect("tab");
        let activation = app
            .begin_tab_activation(window, tab)
            .expect("same-tab activation")
            .intent();

        let start = app.begin_tab_close(window, tab).expect("begin close");
        let TabCloseStart::ActiveLast(close) = start else {
            panic!("last active tab should require neutral close");
        };

        assert_eq!(close.invalidated_activation(), Some(activation));
        assert!(
            app.window(window)
                .expect("window")
                .pending_tab_activation()
                .is_none()
        );
    }

    #[test]
    fn current_tab_neutral_confirmation_rejects_unresolved_activation() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let second = app.create_tab(window).expect("second");
        let intent = app
            .begin_tab_activation(window, second)
            .expect("activation")
            .intent();
        let mut presentation = TabPresentationHandoff::new(first);
        presentation
            .begin_activation(intent)
            .expect("begin presentation activation");

        assert_eq!(
            presentation.confirm_current_tab_neutral(first),
            Err(PresentationHandoffError::PendingActivationBlocksCurrentNeutral {
                activation: intent.id(),
            })
        );
    }
}
