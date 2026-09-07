use crate::{TabActivationId, TabActivationIntent, TabId};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PresentationGeneration(u64);

impl PresentationGeneration {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebContentPresentation {
    Neutral,
    Tab(TabId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PresentationTransitionStart {
    intent: TabActivationIntent,
    superseded: Option<TabActivationIntent>,
    generation: PresentationGeneration,
}

impl PresentationTransitionStart {
    pub const fn intent(self) -> TabActivationIntent {
        self.intent
    }

    pub const fn superseded(self) -> Option<TabActivationIntent> {
        self.superseded
    }

    pub const fn generation(self) -> PresentationGeneration {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TargetFramePermit {
    activation: TabActivationId,
    tab: TabId,
    generation: PresentationGeneration,
}

impl TargetFramePermit {
    pub const fn activation(self) -> TabActivationId {
        self.activation
    }

    pub const fn tab(self) -> TabId {
        self.tab
    }

    pub const fn generation(self) -> PresentationGeneration {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentationHandoffError {
    GenerationExhausted,
    ActivationSourceMismatch {
        represented: TabId,
        source: TabId,
    },
    ContentOwnerMismatch {
        represented: TabId,
        content: TabId,
    },
    StaleActivation {
        expected: Option<TabActivationId>,
        actual: TabActivationId,
    },
    NeutralContentRequired {
        activation: TabActivationId,
    },
    ChromeCommitTargetMismatch {
        activation: TabActivationId,
        expected: TabId,
        actual: TabId,
    },
    ChromeNotCommitted {
        activation: TabActivationId,
        expected: TabId,
        represented: TabId,
    },
    TargetFrameMismatch {
        activation: TabActivationId,
        expected: TabId,
        actual: TabId,
    },
    StaleTargetFramePermit {
        activation: TabActivationId,
        expected: PresentationGeneration,
        actual: PresentationGeneration,
    },
}

impl fmt::Display for PresentationHandoffError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GenerationExhausted => {
                formatter.write_str("presentation generation space is exhausted")
            }
            Self::ActivationSourceMismatch {
                represented,
                source,
            } => write!(
                formatter,
                "tab activation source {} does not match represented chrome tab {}",
                source.get(),
                represented.get()
            ),
            Self::ContentOwnerMismatch {
                represented,
                content,
            } => write!(
                formatter,
                "Web content for tab {} cannot remain visible while chrome represents tab {}",
                content.get(),
                represented.get()
            ),
            Self::StaleActivation { expected, actual } => match expected {
                Some(expected) => write!(
                    formatter,
                    "tab activation {} is stale for presentation handoff; current activation is {}",
                    actual.get(),
                    expected.get()
                ),
                None => write!(
                    formatter,
                    "tab activation {} is stale for presentation handoff; no activation is pending",
                    actual.get()
                ),
            },
            Self::NeutralContentRequired { activation } => write!(
                formatter,
                "tab activation {} cannot change represented chrome until Web content is neutral",
                activation.get()
            ),
            Self::ChromeCommitTargetMismatch {
                activation,
                expected,
                actual,
            } => write!(
                formatter,
                "tab activation {} expected chrome commit to tab {}, got tab {}",
                activation.get(),
                expected.get(),
                actual.get()
            ),
            Self::ChromeNotCommitted {
                activation,
                expected,
                represented,
            } => write!(
                formatter,
                "tab activation {} cannot present tab {} while chrome still represents tab {}",
                activation.get(),
                expected.get(),
                represented.get()
            ),
            Self::TargetFrameMismatch {
                activation,
                expected,
                actual,
            } => write!(
                formatter,
                "tab activation {} expected a target frame for tab {}, got tab {}",
                activation.get(),
                expected.get(),
                actual.get()
            ),
            Self::StaleTargetFramePermit {
                activation,
                expected,
                actual,
            } => write!(
                formatter,
                "target-frame permit for activation {} belongs to presentation generation {}, current generation is {}",
                activation.get(),
                actual.get(),
                expected.get()
            ),
        }
    }
}

impl std::error::Error for PresentationHandoffError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabPresentationHandoff {
    represented_tab: TabId,
    content: WebContentPresentation,
    pending_activation: Option<TabActivationIntent>,
    generation: PresentationGeneration,
}

impl TabPresentationHandoff {
    pub const fn new(represented_tab: TabId) -> Self {
        Self {
            represented_tab,
            content: WebContentPresentation::Tab(represented_tab),
            pending_activation: None,
            generation: PresentationGeneration(0),
        }
    }

    pub const fn represented_tab(&self) -> TabId {
        self.represented_tab
    }

    pub const fn content(&self) -> WebContentPresentation {
        self.content
    }

    pub const fn pending_activation(&self) -> Option<TabActivationIntent> {
        self.pending_activation
    }

    pub const fn generation(&self) -> PresentationGeneration {
        self.generation
    }

    pub fn begin_activation(
        &mut self,
        intent: TabActivationIntent,
    ) -> Result<PresentationTransitionStart, PresentationHandoffError> {
        if intent.from() != self.represented_tab {
            return Err(PresentationHandoffError::ActivationSourceMismatch {
                represented: self.represented_tab,
                source: intent.from(),
            });
        }

        if let WebContentPresentation::Tab(content) = self.content {
            if content != self.represented_tab {
                return Err(PresentationHandoffError::ContentOwnerMismatch {
                    represented: self.represented_tab,
                    content,
                });
            }
        }

        let generation = self.next_generation()?;
        let superseded = self.pending_activation.replace(intent);
        self.generation = generation;

        Ok(PresentationTransitionStart {
            intent,
            superseded,
            generation,
        })
    }

    pub fn confirm_neutral(
        &mut self,
        activation: TabActivationId,
    ) -> Result<(), PresentationHandoffError> {
        self.current_intent(activation)?;
        self.content = WebContentPresentation::Neutral;
        Ok(())
    }

    pub fn acknowledge_chrome_commit(
        &mut self,
        activation: TabActivationId,
        committed_tab: TabId,
    ) -> Result<(), PresentationHandoffError> {
        let intent = self.current_intent(activation)?;
        if self.content != WebContentPresentation::Neutral {
            return Err(PresentationHandoffError::NeutralContentRequired { activation });
        }
        if committed_tab != intent.to() {
            return Err(PresentationHandoffError::ChromeCommitTargetMismatch {
                activation,
                expected: intent.to(),
                actual: committed_tab,
            });
        }

        self.represented_tab = committed_tab;
        Ok(())
    }

    pub fn authorize_target_frame(
        &self,
        activation: TabActivationId,
        tab: TabId,
    ) -> Result<TargetFramePermit, PresentationHandoffError> {
        let intent = self.current_intent(activation)?;
        if self.content != WebContentPresentation::Neutral {
            return Err(PresentationHandoffError::NeutralContentRequired { activation });
        }
        if self.represented_tab != intent.to() {
            return Err(PresentationHandoffError::ChromeNotCommitted {
                activation,
                expected: intent.to(),
                represented: self.represented_tab,
            });
        }
        if tab != intent.to() {
            return Err(PresentationHandoffError::TargetFrameMismatch {
                activation,
                expected: intent.to(),
                actual: tab,
            });
        }

        Ok(TargetFramePermit {
            activation,
            tab,
            generation: self.generation,
        })
    }

    pub fn present_target_frame(
        &mut self,
        permit: TargetFramePermit,
    ) -> Result<TabId, PresentationHandoffError> {
        let intent = self.current_intent(permit.activation)?;
        if permit.generation != self.generation {
            return Err(PresentationHandoffError::StaleTargetFramePermit {
                activation: permit.activation,
                expected: self.generation,
                actual: permit.generation,
            });
        }
        if self.content != WebContentPresentation::Neutral {
            return Err(PresentationHandoffError::NeutralContentRequired {
                activation: permit.activation,
            });
        }
        if self.represented_tab != intent.to() {
            return Err(PresentationHandoffError::ChromeNotCommitted {
                activation: permit.activation,
                expected: intent.to(),
                represented: self.represented_tab,
            });
        }
        if permit.tab != intent.to() {
            return Err(PresentationHandoffError::TargetFrameMismatch {
                activation: permit.activation,
                expected: intent.to(),
                actual: permit.tab,
            });
        }

        self.content = WebContentPresentation::Tab(permit.tab);
        self.pending_activation = None;
        Ok(permit.tab)
    }

    pub fn invalidate_activation(
        &mut self,
        activation: TabActivationId,
    ) -> Result<TabActivationIntent, PresentationHandoffError> {
        let intent = self.current_intent(activation)?;
        let generation = self.next_generation()?;
        self.pending_activation = None;
        self.generation = generation;
        Ok(intent)
    }

    fn current_intent(
        &self,
        activation: TabActivationId,
    ) -> Result<TabActivationIntent, PresentationHandoffError> {
        let expected = self.pending_activation.map(TabActivationIntent::id);
        match self.pending_activation {
            Some(intent) if intent.id() == activation => Ok(intent),
            _ => Err(PresentationHandoffError::StaleActivation {
                expected,
                actual: activation,
            }),
        }
    }

    fn next_generation(&self) -> Result<PresentationGeneration, PresentationHandoffError> {
        self.generation
            .0
            .checked_add(1)
            .map(PresentationGeneration)
            .ok_or(PresentationHandoffError::GenerationExhausted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BrowserApp, BrowserWindow};

    fn active_tab(app: &BrowserApp, window: crate::BrowserWindowId) -> TabId {
        app.window(window)
            .and_then(BrowserWindow::active_tab_id)
            .expect("active tab")
    }

    #[test]
    fn target_frame_requires_neutral_content_and_committed_target_chrome() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first tab");
        let second = app.create_tab(window).expect("second tab");
        let intent = app
            .begin_tab_activation(window, second)
            .expect("begin activation")
            .intent();
        let mut handoff = TabPresentationHandoff::new(first);
        handoff.begin_activation(intent).expect("begin handoff");

        assert_eq!(
            handoff.authorize_target_frame(intent.id(), second),
            Err(PresentationHandoffError::NeutralContentRequired {
                activation: intent.id(),
            })
        );

        handoff
            .confirm_neutral(intent.id())
            .expect("confirm neutral");
        assert_eq!(
            handoff.authorize_target_frame(intent.id(), second),
            Err(PresentationHandoffError::ChromeNotCommitted {
                activation: intent.id(),
                expected: second,
                represented: first,
            })
        );

        let committed = app
            .commit_tab_activation(window, intent.id())
            .expect("commit product activation");
        handoff
            .acknowledge_chrome_commit(intent.id(), committed)
            .expect("acknowledge chrome");
        let permit = handoff
            .authorize_target_frame(intent.id(), second)
            .expect("target frame permit");

        assert_eq!(
            handoff.present_target_frame(permit).expect("present target"),
            second
        );
        assert_eq!(handoff.represented_tab(), second);
        assert_eq!(handoff.content(), WebContentPresentation::Tab(second));
        assert_eq!(handoff.pending_activation(), None);
        assert_eq!(active_tab(&app, window), second);
    }

    #[test]
    fn rapid_activation_supersession_keeps_only_latest_target_committable() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first tab");
        let second = app.create_tab(window).expect("second tab");
        let third = app.create_tab(window).expect("third tab");
        let first_intent = app
            .begin_tab_activation(window, second)
            .expect("first activation")
            .intent();
        let mut handoff = TabPresentationHandoff::new(first);
        handoff
            .begin_activation(first_intent)
            .expect("first handoff");
        handoff
            .confirm_neutral(first_intent.id())
            .expect("neutral content");

        let second_start = app
            .begin_tab_activation(window, third)
            .expect("superseding activation");
        let second_intent = second_start.intent();
        assert_eq!(second_start.superseded(), Some(first_intent));
        let handoff_start = handoff
            .begin_activation(second_intent)
            .expect("superseding handoff");
        assert_eq!(handoff_start.superseded(), Some(first_intent));

        assert_eq!(
            handoff.acknowledge_chrome_commit(first_intent.id(), second),
            Err(PresentationHandoffError::StaleActivation {
                expected: Some(second_intent.id()),
                actual: first_intent.id(),
            })
        );

        let committed = app
            .commit_tab_activation(window, second_intent.id())
            .expect("commit latest activation");
        handoff
            .acknowledge_chrome_commit(second_intent.id(), committed)
            .expect("acknowledge latest chrome");
        let permit = handoff
            .authorize_target_frame(second_intent.id(), third)
            .expect("latest permit");
        handoff
            .present_target_frame(permit)
            .expect("present latest target");

        assert_eq!(handoff.represented_tab(), third);
        assert_eq!(handoff.content(), WebContentPresentation::Tab(third));
        assert_eq!(active_tab(&app, window), third);
    }

    #[test]
    fn newer_activation_invalidates_an_already_issued_target_frame_permit() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first tab");
        let second = app.create_tab(window).expect("second tab");
        let third = app.create_tab(window).expect("third tab");
        let first_intent = app
            .begin_tab_activation(window, second)
            .expect("first activation")
            .intent();
        let mut handoff = TabPresentationHandoff::new(first);
        handoff
            .begin_activation(first_intent)
            .expect("first handoff");
        handoff
            .confirm_neutral(first_intent.id())
            .expect("neutral content");
        let committed = app
            .commit_tab_activation(window, first_intent.id())
            .expect("commit second tab");
        handoff
            .acknowledge_chrome_commit(first_intent.id(), committed)
            .expect("acknowledge second chrome");
        let stale_permit = handoff
            .authorize_target_frame(first_intent.id(), second)
            .expect("permit for second");

        let second_intent = app
            .begin_tab_activation(window, third)
            .expect("next activation")
            .intent();
        handoff
            .begin_activation(second_intent)
            .expect("next handoff");

        assert_eq!(
            handoff.present_target_frame(stale_permit),
            Err(PresentationHandoffError::StaleActivation {
                expected: Some(second_intent.id()),
                actual: first_intent.id(),
            })
        );

        let committed = app
            .commit_tab_activation(window, second_intent.id())
            .expect("commit third tab");
        handoff
            .acknowledge_chrome_commit(second_intent.id(), committed)
            .expect("acknowledge third chrome");
        let permit = handoff
            .authorize_target_frame(second_intent.id(), third)
            .expect("third permit");
        handoff
            .present_target_frame(permit)
            .expect("present third tab");

        assert_eq!(handoff.content(), WebContentPresentation::Tab(third));
    }

    #[test]
    fn invalidation_after_neutral_confirmation_keeps_content_neutral() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first tab");
        let second = app.create_tab(window).expect("second tab");
        let third = app.create_tab(window).expect("third tab");
        let first_intent = app
            .begin_tab_activation(window, second)
            .expect("first activation")
            .intent();
        let mut handoff = TabPresentationHandoff::new(first);
        handoff
            .begin_activation(first_intent)
            .expect("first handoff");
        handoff
            .confirm_neutral(first_intent.id())
            .expect("neutral content");

        app.cancel_tab_activation(window, first_intent.id())
            .expect("cancel product activation");
        assert_eq!(
            handoff
                .invalidate_activation(first_intent.id())
                .expect("invalidate handoff"),
            first_intent
        );
        assert_eq!(handoff.content(), WebContentPresentation::Neutral);
        assert_eq!(handoff.represented_tab(), first);

        let second_intent = app
            .begin_tab_activation(window, third)
            .expect("replacement activation")
            .intent();
        handoff
            .begin_activation(second_intent)
            .expect("replacement handoff");
        let committed = app
            .commit_tab_activation(window, second_intent.id())
            .expect("commit replacement");
        handoff
            .acknowledge_chrome_commit(second_intent.id(), committed)
            .expect("neutral cover is still sufficient");
        let permit = handoff
            .authorize_target_frame(second_intent.id(), third)
            .expect("replacement permit");
        handoff
            .present_target_frame(permit)
            .expect("present replacement");

        assert_eq!(handoff.content(), WebContentPresentation::Tab(third));
    }

    #[test]
    fn activation_source_must_match_the_tab_represented_by_privileged_chrome() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first tab");
        let second = app.create_tab(window).expect("second tab");
        let third = app.create_tab(window).expect("third tab");
        let intent = app
            .begin_tab_activation(window, second)
            .expect("activation")
            .intent();
        let mut handoff = TabPresentationHandoff::new(third);

        assert_eq!(
            handoff.begin_activation(intent),
            Err(PresentationHandoffError::ActivationSourceMismatch {
                represented: third,
                source: first,
            })
        );
    }

    #[test]
    fn rebeginning_the_same_activation_revokes_an_older_permit_by_generation() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first tab");
        let second = app.create_tab(window).expect("second tab");
        let intent = app
            .begin_tab_activation(window, second)
            .expect("activation")
            .intent();
        let mut handoff = TabPresentationHandoff::new(first);
        handoff.begin_activation(intent).expect("handoff");
        handoff.confirm_neutral(intent.id()).expect("neutral");
        let committed = app
            .commit_tab_activation(window, intent.id())
            .expect("commit product activation");
        handoff
            .acknowledge_chrome_commit(intent.id(), committed)
            .expect("acknowledge chrome");
        let stale = handoff
            .authorize_target_frame(intent.id(), second)
            .expect("first permit");

        let repeated = TabActivationIntent::new(intent.id(), second, second);
        handoff
            .begin_activation(repeated)
            .expect("repeat activation generation");
        let current = handoff
            .authorize_target_frame(intent.id(), second)
            .expect("current permit");

        assert_ne!(stale.generation(), current.generation());
        assert_eq!(
            handoff.present_target_frame(stale),
            Err(PresentationHandoffError::StaleTargetFramePermit {
                activation: intent.id(),
                expected: current.generation(),
                actual: stale.generation(),
            })
        );
    }
}
