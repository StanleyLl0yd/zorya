#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NavigationId(pub(crate) u64);

impl NavigationId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HistoryEntryId(pub(crate) u64);

impl HistoryEntryId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationIntentKind {
    NewDocument,
    Reload { entry: HistoryEntryId },
    TraverseHistory { entry: HistoryEntryId },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationIntent {
    id: NavigationId,
    requested_location: String,
    kind: NavigationIntentKind,
}

impl NavigationIntent {
    pub(crate) fn new(
        id: NavigationId,
        requested_location: String,
        kind: NavigationIntentKind,
    ) -> Self {
        Self {
            id,
            requested_location,
            kind,
        }
    }

    pub const fn id(&self) -> NavigationId {
        self.id
    }

    pub fn requested_location(&self) -> &str {
        &self.requested_location
    }

    pub const fn kind(&self) -> NavigationIntentKind {
        self.kind
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationStart {
    intent: NavigationIntent,
    superseded: Option<NavigationIntent>,
}

impl NavigationStart {
    pub(crate) fn new(intent: NavigationIntent, superseded: Option<NavigationIntent>) -> Self {
        Self { intent, superseded }
    }

    pub fn intent(&self) -> &NavigationIntent {
        &self.intent
    }

    pub fn superseded(&self) -> Option<&NavigationIntent> {
        self.superseded.as_ref()
    }

    pub fn into_parts(self) -> (NavigationIntent, Option<NavigationIntent>) {
        (self.intent, self.superseded)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    id: HistoryEntryId,
    location: String,
}

impl HistoryEntry {
    pub(crate) fn new(id: HistoryEntryId, location: String) -> Self {
        Self { id, location }
    }

    pub const fn id(&self) -> HistoryEntryId {
        self.id
    }

    pub fn location(&self) -> &str {
        &self.location
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavigationFailure {
    navigation: NavigationId,
    requested_location: String,
    message: String,
}

impl NavigationFailure {
    pub const fn navigation(&self) -> NavigationId {
        self.navigation
    }

    pub fn requested_location(&self) -> &str {
        &self.requested_location
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReloadControl {
    Unavailable,
    Reload,
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NavigationControls {
    can_go_back: bool,
    can_go_forward: bool,
    reload: ReloadControl,
}

impl NavigationControls {
    pub const fn can_go_back(self) -> bool {
        self.can_go_back
    }

    pub const fn can_go_forward(self) -> bool {
        self.can_go_forward
    }

    pub const fn reload(self) -> ReloadControl {
        self.reload
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabNavigation {
    history: Vec<HistoryEntry>,
    current: Option<HistoryEntryId>,
    pending: Option<NavigationIntent>,
    last_failure: Option<NavigationFailure>,
}

impl TabNavigation {
    pub fn history(&self) -> &[HistoryEntry] {
        &self.history
    }

    pub const fn current_entry_id(&self) -> Option<HistoryEntryId> {
        self.current
    }

    pub fn current_entry(&self) -> Option<&HistoryEntry> {
        let current = self.current?;
        self.history.iter().find(|entry| entry.id == current)
    }

    pub fn pending(&self) -> Option<&NavigationIntent> {
        self.pending.as_ref()
    }

    pub fn last_failure(&self) -> Option<&NavigationFailure> {
        self.last_failure.as_ref()
    }

    pub fn display_location(&self) -> Option<&str> {
        self.pending
            .as_ref()
            .map(NavigationIntent::requested_location)
            .or_else(|| self.current_entry().map(HistoryEntry::location))
    }

    pub const fn is_loading(&self) -> bool {
        self.pending.is_some()
    }

    pub fn controls(&self) -> NavigationControls {
        NavigationControls {
            can_go_back: self.can_go_back(),
            can_go_forward: self.can_go_forward(),
            reload: if self.pending.is_some() {
                ReloadControl::Stop
            } else if self.current.is_some() {
                ReloadControl::Reload
            } else {
                ReloadControl::Unavailable
            },
        }
    }

    pub fn can_go_back(&self) -> bool {
        self.current_index().is_some_and(|index| index > 0)
    }

    pub fn can_go_forward(&self) -> bool {
        self.current_index()
            .is_some_and(|index| index + 1 < self.history.len())
    }

    pub(crate) fn pending_id(&self) -> Option<NavigationId> {
        self.pending.as_ref().map(NavigationIntent::id)
    }

    pub(crate) fn pending_for(&self, navigation: NavigationId) -> Option<&NavigationIntent> {
        self.pending
            .as_ref()
            .filter(|intent| intent.id == navigation)
    }

    pub(crate) fn start(&mut self, intent: NavigationIntent) -> NavigationStart {
        self.last_failure = None;
        let superseded = self.pending.replace(intent.clone());
        NavigationStart::new(intent, superseded)
    }

    pub(crate) fn stop(&mut self) -> Option<NavigationIntent> {
        self.pending.take()
    }

    pub(crate) fn back_target(&self) -> Option<(HistoryEntryId, String)> {
        let index = self.current_index()?.checked_sub(1)?;
        let entry = self.history.get(index)?;
        Some((entry.id, entry.location.clone()))
    }

    pub(crate) fn forward_target(&self) -> Option<(HistoryEntryId, String)> {
        let index = self.current_index()?.checked_add(1)?;
        let entry = self.history.get(index)?;
        Some((entry.id, entry.location.clone()))
    }

    pub(crate) fn reload_target(&self) -> Option<(HistoryEntryId, String)> {
        let entry = self.current_entry()?;
        Some((entry.id, entry.location.clone()))
    }

    pub(crate) fn contains_history_entry(&self, id: HistoryEntryId) -> bool {
        self.history.iter().any(|entry| entry.id == id)
    }

    pub(crate) fn commit_new(
        &mut self,
        navigation: NavigationId,
        entry: HistoryEntry,
    ) -> Option<HistoryEntryId> {
        self.take_pending(navigation)?;
        if let Some(index) = self.current_index() {
            self.history.truncate(index + 1);
        } else {
            self.history.clear();
        }

        let id = entry.id;
        self.history.push(entry);
        self.current = Some(id);
        self.last_failure = None;
        Some(id)
    }

    pub(crate) fn commit_existing(
        &mut self,
        navigation: NavigationId,
        entry: HistoryEntryId,
        committed_location: String,
    ) -> Option<HistoryEntryId> {
        self.take_pending(navigation)?;
        let target = self
            .history
            .iter_mut()
            .find(|candidate| candidate.id == entry)?;
        target.location = committed_location;
        self.current = Some(entry);
        self.last_failure = None;
        Some(entry)
    }

    pub(crate) fn fail(
        &mut self,
        navigation: NavigationId,
        message: String,
    ) -> Option<NavigationFailure> {
        let intent = self.take_pending(navigation)?;
        let failure = NavigationFailure {
            navigation,
            requested_location: intent.requested_location,
            message,
        };
        self.last_failure = Some(failure.clone());
        Some(failure)
    }

    fn take_pending(&mut self, navigation: NavigationId) -> Option<NavigationIntent> {
        if self.pending_id() == Some(navigation) {
            self.pending.take()
        } else {
            None
        }
    }

    fn current_index(&self) -> Option<usize> {
        let current = self.current?;
        self.history.iter().position(|entry| entry.id == current)
    }
}

#[cfg(test)]
mod control_tests {
    use super::*;

    fn entry(id: u64, location: &str) -> HistoryEntry {
        HistoryEntry::new(HistoryEntryId(id), location.into())
    }

    fn intent(id: u64, location: &str) -> NavigationIntent {
        NavigationIntent::new(
            NavigationId(id),
            location.into(),
            NavigationIntentKind::NewDocument,
        )
    }

    #[test]
    fn empty_tab_has_no_history_controls() {
        let state = TabNavigation::default();

        assert!(!state.is_loading());
        assert_eq!(
            state.controls(),
            NavigationControls {
                can_go_back: false,
                can_go_forward: false,
                reload: ReloadControl::Unavailable,
            }
        );
    }

    #[test]
    fn committed_document_enables_reload_and_history_direction() {
        let mut state = TabNavigation::default();
        state.start(intent(1, "https://a.example/"));
        state
            .commit_new(NavigationId(1), entry(1, "https://a.example/"))
            .expect("commit first");
        state.start(intent(2, "https://b.example/"));
        state
            .commit_new(NavigationId(2), entry(2, "https://b.example/"))
            .expect("commit second");

        assert_eq!(
            state.controls(),
            NavigationControls {
                can_go_back: true,
                can_go_forward: false,
                reload: ReloadControl::Reload,
            }
        );
    }

    #[test]
    fn pending_navigation_turns_reload_control_into_stop_without_hiding_history() {
        let mut state = TabNavigation::default();
        state.start(intent(1, "https://a.example/"));
        state
            .commit_new(NavigationId(1), entry(1, "https://a.example/"))
            .expect("commit first");
        state.start(intent(2, "https://b.example/"));
        state
            .commit_new(NavigationId(2), entry(2, "https://b.example/"))
            .expect("commit second");
        state.start(intent(3, "https://pending.example/"));

        assert!(state.is_loading());
        assert_eq!(
            state.controls(),
            NavigationControls {
                can_go_back: true,
                can_go_forward: false,
                reload: ReloadControl::Stop,
            }
        );

        state.stop().expect("stop pending navigation");
        assert!(!state.is_loading());
        assert_eq!(
            state.controls(),
            NavigationControls {
                can_go_back: true,
                can_go_forward: false,
                reload: ReloadControl::Reload,
            }
        );
    }

    #[test]
    fn pending_initial_navigation_can_be_stopped_before_any_commit() {
        let mut state = TabNavigation::default();
        state.start(intent(1, "about:blank"));

        let controls = state.controls();
        assert_eq!(controls.reload(), ReloadControl::Stop);
        assert!(!controls.can_go_back());
        assert!(!controls.can_go_forward());
    }
}
