use crate::navigation::{
    HistoryEntry, HistoryEntryId, NavigationFailure, NavigationId, NavigationIntent,
    NavigationIntentKind, NavigationStart, TabNavigation,
};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BrowserWindowId(u64);

impl BrowserWindowId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TabId(u64);

impl TabId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserModelError {
    WindowIdExhausted,
    TabIdExhausted,
    NavigationIdExhausted,
    HistoryEntryIdExhausted,
    UnknownWindow(BrowserWindowId),
    UnknownTab {
        window: BrowserWindowId,
        tab: TabId,
    },
    StaleNavigation {
        window: BrowserWindowId,
        tab: TabId,
        expected: Option<NavigationId>,
        actual: NavigationId,
    },
    MissingHistoryEntry {
        window: BrowserWindowId,
        tab: TabId,
        entry: HistoryEntryId,
    },
}

impl fmt::Display for BrowserModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WindowIdExhausted => {
                formatter.write_str("browser window identifier space is exhausted")
            }
            Self::TabIdExhausted => formatter.write_str("tab identifier space is exhausted"),
            Self::NavigationIdExhausted => {
                formatter.write_str("navigation identifier space is exhausted")
            }
            Self::HistoryEntryIdExhausted => {
                formatter.write_str("history entry identifier space is exhausted")
            }
            Self::UnknownWindow(window) => {
                write!(formatter, "unknown browser window {}", window.get())
            }
            Self::UnknownTab { window, tab } => {
                write!(
                    formatter,
                    "unknown tab {} in browser window {}",
                    tab.get(),
                    window.get()
                )
            }
            Self::StaleNavigation {
                window,
                tab,
                expected,
                actual,
            } => match expected {
                Some(expected) => write!(
                    formatter,
                    "navigation {} is stale for window {} tab {}; current navigation is {}",
                    actual.get(),
                    window.get(),
                    tab.get(),
                    expected.get()
                ),
                None => write!(
                    formatter,
                    "navigation {} is stale for window {} tab {}; no navigation is pending",
                    actual.get(),
                    window.get(),
                    tab.get()
                ),
            },
            Self::MissingHistoryEntry { window, tab, entry } => write!(
                formatter,
                "history entry {} is unavailable in window {} tab {}",
                entry.get(),
                window.get(),
                tab.get()
            ),
        }
    }
}

impl std::error::Error for BrowserModelError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    id: TabId,
    navigation: TabNavigation,
}

impl Tab {
    pub const fn id(&self) -> TabId {
        self.id
    }

    pub const fn navigation(&self) -> &TabNavigation {
        &self.navigation
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserWindow {
    id: BrowserWindowId,
    tabs: Vec<Tab>,
    active_tab: Option<TabId>,
}

impl BrowserWindow {
    fn new(id: BrowserWindowId) -> Self {
        Self {
            id,
            tabs: Vec::new(),
            active_tab: None,
        }
    }

    pub const fn id(&self) -> BrowserWindowId {
        self.id
    }

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    pub const fn active_tab_id(&self) -> Option<TabId> {
        self.active_tab
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tab(self.active_tab?)
    }

    fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|tab| tab.id == id)
    }

    fn insert_tab(&mut self, tab: Tab) {
        let id = tab.id;
        self.tabs.push(tab);
        if self.active_tab.is_none() {
            self.active_tab = Some(id);
        }
    }

    fn close_tab(&mut self, tab: TabId) -> Option<Tab> {
        let position = self.tabs.iter().position(|candidate| candidate.id == tab)?;
        let removed = self.tabs.remove(position);

        if self.active_tab == Some(tab) {
            self.active_tab = self
                .tabs
                .get(position)
                .or_else(|| {
                    position
                        .checked_sub(1)
                        .and_then(|index| self.tabs.get(index))
                })
                .map(Tab::id);
        }

        Some(removed)
    }

    fn set_active_tab(&mut self, tab: TabId) -> bool {
        if self.tabs.iter().any(|candidate| candidate.id == tab) {
            self.active_tab = Some(tab);
            true
        } else {
            false
        }
    }
}

#[derive(Debug)]
pub struct BrowserApp {
    windows: BTreeMap<BrowserWindowId, BrowserWindow>,
    next_window_id: u64,
    next_tab_id: u64,
    next_navigation_id: u64,
    next_history_entry_id: u64,
}

impl Default for BrowserApp {
    fn default() -> Self {
        Self::new()
    }
}

impl BrowserApp {
    pub const fn new() -> Self {
        Self {
            windows: BTreeMap::new(),
            next_window_id: 1,
            next_tab_id: 1,
            next_navigation_id: 1,
            next_history_entry_id: 1,
        }
    }

    pub fn bootstrap() -> Result<Self, BrowserModelError> {
        let mut app = Self::new();
        let window = app.create_window()?;
        app.create_tab(window)?;
        Ok(app)
    }

    pub fn windows(&self) -> impl ExactSizeIterator<Item = &BrowserWindow> {
        self.windows.values()
    }

    pub fn window(&self, id: BrowserWindowId) -> Option<&BrowserWindow> {
        self.windows.get(&id)
    }

    pub fn window_mut(&mut self, id: BrowserWindowId) -> Option<&mut BrowserWindow> {
        self.windows.get_mut(&id)
    }

    pub fn create_window(&mut self) -> Result<BrowserWindowId, BrowserModelError> {
        let id = BrowserWindowId(self.next_window_id);
        self.next_window_id = self
            .next_window_id
            .checked_add(1)
            .ok_or(BrowserModelError::WindowIdExhausted)?;
        self.windows.insert(id, BrowserWindow::new(id));
        Ok(id)
    }

    pub fn close_window(&mut self, id: BrowserWindowId) -> Option<BrowserWindow> {
        self.windows.remove(&id)
    }

    pub fn create_tab(&mut self, window: BrowserWindowId) -> Result<TabId, BrowserModelError> {
        if !self.windows.contains_key(&window) {
            return Err(BrowserModelError::UnknownWindow(window));
        }

        let id = TabId(self.next_tab_id);
        self.next_tab_id = self
            .next_tab_id
            .checked_add(1)
            .ok_or(BrowserModelError::TabIdExhausted)?;

        self.windows
            .get_mut(&window)
            .expect("window existence checked before tab allocation")
            .insert_tab(Tab {
                id,
                navigation: TabNavigation::default(),
            });
        Ok(id)
    }

    pub fn close_tab(
        &mut self,
        window: BrowserWindowId,
        tab: TabId,
    ) -> Result<Tab, BrowserModelError> {
        let browser_window = self
            .windows
            .get_mut(&window)
            .ok_or(BrowserModelError::UnknownWindow(window))?;

        browser_window
            .close_tab(tab)
            .ok_or(BrowserModelError::UnknownTab { window, tab })
    }

    pub fn set_active_tab(
        &mut self,
        window: BrowserWindowId,
        tab: TabId,
    ) -> Result<(), BrowserModelError> {
        let browser_window = self
            .windows
            .get_mut(&window)
            .ok_or(BrowserModelError::UnknownWindow(window))?;

        if browser_window.set_active_tab(tab) {
            Ok(())
        } else {
            Err(BrowserModelError::UnknownTab { window, tab })
        }
    }

    pub fn begin_navigation(
        &mut self,
        window: BrowserWindowId,
        tab: TabId,
        requested_location: impl Into<String>,
    ) -> Result<NavigationStart, BrowserModelError> {
        self.ensure_tab(window, tab)?;
        let id = self.allocate_navigation_id()?;
        let intent = NavigationIntent::new(
            id,
            requested_location.into(),
            NavigationIntentKind::NewDocument,
        );
        Ok(self.tab_mut(window, tab)?.navigation.start(intent))
    }

    pub fn begin_back_navigation(
        &mut self,
        window: BrowserWindowId,
        tab: TabId,
    ) -> Result<Option<NavigationStart>, BrowserModelError> {
        let target = self.tab(window, tab)?.navigation.back_target();
        self.begin_existing_navigation(window, tab, target, |entry| {
            NavigationIntentKind::TraverseHistory { entry }
        })
    }

    pub fn begin_forward_navigation(
        &mut self,
        window: BrowserWindowId,
        tab: TabId,
    ) -> Result<Option<NavigationStart>, BrowserModelError> {
        let target = self.tab(window, tab)?.navigation.forward_target();
        self.begin_existing_navigation(window, tab, target, |entry| {
            NavigationIntentKind::TraverseHistory { entry }
        })
    }

    pub fn begin_reload(
        &mut self,
        window: BrowserWindowId,
        tab: TabId,
    ) -> Result<Option<NavigationStart>, BrowserModelError> {
        let target = self.tab(window, tab)?.navigation.reload_target();
        self.begin_existing_navigation(window, tab, target, |entry| {
            NavigationIntentKind::Reload { entry }
        })
    }

    pub fn stop_navigation(
        &mut self,
        window: BrowserWindowId,
        tab: TabId,
    ) -> Result<Option<NavigationIntent>, BrowserModelError> {
        Ok(self.tab_mut(window, tab)?.navigation.stop())
    }

    pub fn commit_navigation(
        &mut self,
        window: BrowserWindowId,
        tab: TabId,
        navigation: NavigationId,
        committed_location: impl Into<String>,
    ) -> Result<HistoryEntryId, BrowserModelError> {
        let kind = self.pending_kind(window, tab, navigation)?;
        let committed_location = committed_location.into();

        match kind {
            NavigationIntentKind::NewDocument => {
                let entry_id = self.allocate_history_entry_id()?;
                let entry = HistoryEntry::new(entry_id, committed_location);
                Ok(self
                    .tab_mut(window, tab)?
                    .navigation
                    .commit_new(navigation, entry)
                    .expect("pending navigation validated before commit"))
            }
            NavigationIntentKind::Reload { entry }
            | NavigationIntentKind::TraverseHistory { entry } => {
                if !self
                    .tab(window, tab)?
                    .navigation
                    .contains_history_entry(entry)
                {
                    return Err(BrowserModelError::MissingHistoryEntry { window, tab, entry });
                }

                Ok(self
                    .tab_mut(window, tab)?
                    .navigation
                    .commit_existing(navigation, entry, committed_location)
                    .expect("pending history navigation validated before commit"))
            }
        }
    }

    pub fn fail_navigation(
        &mut self,
        window: BrowserWindowId,
        tab: TabId,
        navigation: NavigationId,
        message: impl Into<String>,
    ) -> Result<NavigationFailure, BrowserModelError> {
        self.pending_kind(window, tab, navigation)?;
        Ok(self
            .tab_mut(window, tab)?
            .navigation
            .fail(navigation, message.into())
            .expect("pending navigation validated before failure"))
    }

    fn begin_existing_navigation(
        &mut self,
        window: BrowserWindowId,
        tab: TabId,
        target: Option<(HistoryEntryId, String)>,
        kind: fn(HistoryEntryId) -> NavigationIntentKind,
    ) -> Result<Option<NavigationStart>, BrowserModelError> {
        let Some((entry, location)) = target else {
            return Ok(None);
        };
        let id = self.allocate_navigation_id()?;
        let intent = NavigationIntent::new(id, location, kind(entry));
        Ok(Some(self.tab_mut(window, tab)?.navigation.start(intent)))
    }

    fn pending_kind(
        &self,
        window: BrowserWindowId,
        tab: TabId,
        navigation: NavigationId,
    ) -> Result<NavigationIntentKind, BrowserModelError> {
        let state = &self.tab(window, tab)?.navigation;
        state
            .pending_for(navigation)
            .map(NavigationIntent::kind)
            .ok_or(BrowserModelError::StaleNavigation {
                window,
                tab,
                expected: state.pending_id(),
                actual: navigation,
            })
    }

    fn tab(&self, window: BrowserWindowId, tab: TabId) -> Result<&Tab, BrowserModelError> {
        let browser_window = self
            .windows
            .get(&window)
            .ok_or(BrowserModelError::UnknownWindow(window))?;
        browser_window
            .tab(tab)
            .ok_or(BrowserModelError::UnknownTab { window, tab })
    }

    fn tab_mut(
        &mut self,
        window: BrowserWindowId,
        tab: TabId,
    ) -> Result<&mut Tab, BrowserModelError> {
        let browser_window = self
            .windows
            .get_mut(&window)
            .ok_or(BrowserModelError::UnknownWindow(window))?;
        browser_window
            .tab_mut(tab)
            .ok_or(BrowserModelError::UnknownTab { window, tab })
    }

    fn ensure_tab(&self, window: BrowserWindowId, tab: TabId) -> Result<(), BrowserModelError> {
        self.tab(window, tab).map(|_| ())
    }

    fn allocate_navigation_id(&mut self) -> Result<NavigationId, BrowserModelError> {
        let id = NavigationId(self.next_navigation_id);
        self.next_navigation_id = self
            .next_navigation_id
            .checked_add(1)
            .ok_or(BrowserModelError::NavigationIdExhausted)?;
        Ok(id)
    }

    fn allocate_history_entry_id(&mut self) -> Result<HistoryEntryId, BrowserModelError> {
        let id = HistoryEntryId(self.next_history_entry_id);
        self.next_history_entry_id = self
            .next_history_entry_id
            .checked_add(1)
            .ok_or(BrowserModelError::HistoryEntryIdExhausted)?;
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bootstrap_ids(app: &BrowserApp) -> (BrowserWindowId, TabId) {
        let window = app.windows().next().expect("bootstrap window");
        (
            window.id(),
            window.active_tab_id().expect("bootstrap active tab"),
        )
    }

    #[test]
    fn bootstrap_creates_one_window_with_one_active_tab() {
        let app = BrowserApp::bootstrap().expect("bootstrap should allocate initial identities");
        let windows = app.windows().collect::<Vec<_>>();

        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].tabs().len(), 1);
        assert_eq!(windows[0].active_tab_id(), Some(windows[0].tabs()[0].id()));
        assert!(windows[0].tabs()[0].navigation().history().is_empty());
    }

    #[test]
    fn closed_window_identity_is_not_reused() {
        let mut app = BrowserApp::new();
        let first = app.create_window().expect("first window id");
        app.close_window(first);
        let second = app.create_window().expect("second window id");

        assert!(second.get() > first.get());
    }

    #[test]
    fn closed_tab_identity_is_not_reused() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window id");
        let first = app.create_tab(window).expect("first tab id");
        app.close_tab(window, first).expect("close first tab");
        let second = app.create_tab(window).expect("second tab id");

        assert!(second.get() > first.get());
        assert_eq!(
            app.window(window).and_then(BrowserWindow::active_tab_id),
            Some(second)
        );
    }

    #[test]
    fn closing_active_tab_selects_a_surviving_neighbor() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window id");
        let first = app.create_tab(window).expect("first tab id");
        let second = app.create_tab(window).expect("second tab id");
        app.set_active_tab(window, first)
            .expect("activate first tab");

        app.close_tab(window, first).expect("close active tab");

        assert_eq!(
            app.window(window).and_then(BrowserWindow::active_tab_id),
            Some(second)
        );
    }

    #[test]
    fn stale_tab_identity_cannot_target_a_replacement_tab() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window id");
        let stale = app.create_tab(window).expect("initial tab id");
        app.close_tab(window, stale).expect("close initial tab");
        let replacement = app.create_tab(window).expect("replacement tab id");

        assert_ne!(stale, replacement);
        assert_eq!(
            app.set_active_tab(window, stale),
            Err(BrowserModelError::UnknownTab { window, tab: stale })
        );
        assert_eq!(
            app.window(window).and_then(BrowserWindow::active_tab_id),
            Some(replacement)
        );
    }

    #[test]
    fn newer_navigation_supersedes_pending_work_with_monotonic_identity() {
        let mut app = BrowserApp::bootstrap().expect("bootstrap");
        let (window, tab) = bootstrap_ids(&app);
        let first = app
            .begin_navigation(window, tab, "https://first.example/")
            .expect("first navigation");
        let first_id = first.intent().id();
        let second = app
            .begin_navigation(window, tab, "https://second.example/")
            .expect("second navigation");

        assert!(second.intent().id().get() > first_id.get());
        assert_eq!(
            second.superseded().map(NavigationIntent::id),
            Some(first_id)
        );
        assert_eq!(
            app.window(window)
                .and_then(|window| window.tab(tab))
                .and_then(|tab| tab.navigation().pending())
                .map(NavigationIntent::id),
            Some(second.intent().id())
        );
    }

    #[test]
    fn stale_navigation_cannot_commit_after_supersession() {
        let mut app = BrowserApp::bootstrap().expect("bootstrap");
        let (window, tab) = bootstrap_ids(&app);
        let stale = app
            .begin_navigation(window, tab, "https://stale.example/")
            .expect("stale navigation")
            .intent()
            .id();
        let current = app
            .begin_navigation(window, tab, "https://current.example/")
            .expect("current navigation")
            .intent()
            .id();

        assert_eq!(
            app.commit_navigation(window, tab, stale, "https://stale.example/"),
            Err(BrowserModelError::StaleNavigation {
                window,
                tab,
                expected: Some(current),
                actual: stale,
            })
        );
        assert!(
            app.window(window)
                .and_then(|window| window.tab(tab))
                .expect("tab")
                .navigation()
                .history()
                .is_empty()
        );
    }

    #[test]
    fn new_commit_after_back_truncates_forward_history_without_reusing_identity() {
        let mut app = BrowserApp::bootstrap().expect("bootstrap");
        let (window, tab) = bootstrap_ids(&app);

        let first_navigation = app
            .begin_navigation(window, tab, "https://a.example/")
            .expect("navigate a")
            .intent()
            .id();
        let first_entry = app
            .commit_navigation(window, tab, first_navigation, "https://a.example/")
            .expect("commit a");

        let second_navigation = app
            .begin_navigation(window, tab, "https://b.example/")
            .expect("navigate b")
            .intent()
            .id();
        let second_entry = app
            .commit_navigation(window, tab, second_navigation, "https://b.example/")
            .expect("commit b");

        let back = app
            .begin_back_navigation(window, tab)
            .expect("begin back")
            .expect("back available");
        assert_eq!(
            back.intent().kind(),
            NavigationIntentKind::TraverseHistory { entry: first_entry }
        );
        app.commit_navigation(window, tab, back.intent().id(), "https://a.example/")
            .expect("commit back");

        let navigation = app
            .begin_navigation(window, tab, "https://c.example/")
            .expect("navigate c")
            .intent()
            .id();
        let third_entry = app
            .commit_navigation(window, tab, navigation, "https://c.example/")
            .expect("commit c");

        let state = app
            .window(window)
            .and_then(|window| window.tab(tab))
            .expect("tab")
            .navigation();
        assert_eq!(state.history().len(), 2);
        assert_eq!(state.history()[0].id(), first_entry);
        assert_eq!(state.history()[1].id(), third_entry);
        assert!(third_entry.get() > second_entry.get());
        assert!(!state.can_go_forward());
        assert!(state.can_go_back());
    }

    #[test]
    fn reload_keeps_history_identity_and_updates_committed_location() {
        let mut app = BrowserApp::bootstrap().expect("bootstrap");
        let (window, tab) = bootstrap_ids(&app);
        let navigation = app
            .begin_navigation(window, tab, "https://example.com/")
            .expect("navigation")
            .intent()
            .id();
        let entry = app
            .commit_navigation(window, tab, navigation, "https://example.com/")
            .expect("commit");

        let reload = app
            .begin_reload(window, tab)
            .expect("begin reload")
            .expect("reload available");
        assert_eq!(
            reload.intent().kind(),
            NavigationIntentKind::Reload { entry }
        );
        let committed = app
            .commit_navigation(
                window,
                tab,
                reload.intent().id(),
                "https://example.com/final",
            )
            .expect("commit reload");

        let state = app
            .window(window)
            .and_then(|window| window.tab(tab))
            .expect("tab")
            .navigation();
        assert_eq!(committed, entry);
        assert_eq!(state.history().len(), 1);
        assert_eq!(state.current_entry_id(), Some(entry));
        assert_eq!(
            state.current_entry().map(HistoryEntry::location),
            Some("https://example.com/final")
        );
    }

    #[test]
    fn closed_tab_cannot_receive_navigation_completion_from_its_old_identity() {
        let mut app = BrowserApp::bootstrap().expect("bootstrap");
        let (window, stale_tab) = bootstrap_ids(&app);
        let stale_navigation = app
            .begin_navigation(window, stale_tab, "https://stale.example/")
            .expect("stale navigation")
            .intent()
            .id();

        app.close_tab(window, stale_tab).expect("close stale tab");
        let replacement = app.create_tab(window).expect("replacement tab");

        assert_ne!(stale_tab, replacement);
        assert_eq!(
            app.commit_navigation(
                window,
                stale_tab,
                stale_navigation,
                "https://stale.example/"
            ),
            Err(BrowserModelError::UnknownTab {
                window,
                tab: stale_tab,
            })
        );
        assert!(
            app.window(window)
                .and_then(|browser_window| browser_window.tab(replacement))
                .expect("replacement tab")
                .navigation()
                .history()
                .is_empty()
        );
    }

    #[test]
    fn navigation_and_history_identity_are_not_reused_across_tabs() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first_tab = app.create_tab(window).expect("first tab");
        let second_tab = app.create_tab(window).expect("second tab");

        let first_navigation = app
            .begin_navigation(window, first_tab, "https://first.example/")
            .expect("first navigation")
            .intent()
            .id();
        let first_entry = app
            .commit_navigation(
                window,
                first_tab,
                first_navigation,
                "https://first.example/",
            )
            .expect("first commit");

        let second_navigation = app
            .begin_navigation(window, second_tab, "https://second.example/")
            .expect("second navigation")
            .intent()
            .id();
        let second_entry = app
            .commit_navigation(
                window,
                second_tab,
                second_navigation,
                "https://second.example/",
            )
            .expect("second commit");

        assert!(second_navigation.get() > first_navigation.get());
        assert!(second_entry.get() > first_entry.get());
    }

    #[test]
    fn failure_and_stop_leave_committed_history_unchanged() {
        let mut app = BrowserApp::bootstrap().expect("bootstrap");
        let (window, tab) = bootstrap_ids(&app);
        let initial = app
            .begin_navigation(window, tab, "https://committed.example/")
            .expect("initial navigation")
            .intent()
            .id();
        let entry = app
            .commit_navigation(window, tab, initial, "https://committed.example/")
            .expect("initial commit");

        let failing = app
            .begin_navigation(window, tab, "https://fail.example/")
            .expect("failing navigation")
            .intent()
            .id();
        let failure = app
            .fail_navigation(window, tab, failing, "network unavailable")
            .expect("record failure");
        assert_eq!(failure.navigation(), failing);

        let stopped = app
            .begin_navigation(window, tab, "https://stop.example/")
            .expect("stopped navigation")
            .intent()
            .id();
        let cancelled = app
            .stop_navigation(window, tab)
            .expect("stop")
            .expect("pending navigation");
        assert_eq!(cancelled.id(), stopped);

        let state = app
            .window(window)
            .and_then(|window| window.tab(tab))
            .expect("tab")
            .navigation();
        assert_eq!(state.current_entry_id(), Some(entry));
        assert_eq!(state.history().len(), 1);
        assert!(state.pending().is_none());
        assert_eq!(
            app.commit_navigation(window, tab, stopped, "https://stop.example/"),
            Err(BrowserModelError::StaleNavigation {
                window,
                tab,
                expected: None,
                actual: stopped,
            })
        );
    }
}
