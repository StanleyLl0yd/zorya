use crate::chrome::{AddressBarState, AddressBarSubmission};
use crate::navigation::{
    HistoryEntry, HistoryEntryId, NavigationFailure, NavigationId, NavigationIntent,
    NavigationIntentKind, NavigationStart, TabNavigation,
};
use crate::tab_activation::{TabActivationId, TabActivationIntent, TabActivationStart};
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
    TabActivationIdExhausted,
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
    AddressBarNotEditing {
        window: BrowserWindowId,
    },
    NoActiveTab {
        window: BrowserWindowId,
    },
    StaleTabActivation {
        window: BrowserWindowId,
        expected: Option<TabActivationId>,
        actual: TabActivationId,
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
            Self::TabActivationIdExhausted => {
                formatter.write_str("tab activation identifier space is exhausted")
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
            Self::AddressBarNotEditing { window } => write!(
                formatter,
                "address bar is not being edited in browser window {}",
                window.get()
            ),
            Self::NoActiveTab { window } => write!(
                formatter,
                "browser window {} has no active tab",
                window.get()
            ),
            Self::StaleTabActivation {
                window,
                expected,
                actual,
            } => match expected {
                Some(expected) => write!(
                    formatter,
                    "tab activation {} is stale for browser window {}; current activation is {}",
                    actual.get(),
                    window.get(),
                    expected.get()
                ),
                None => write!(
                    formatter,
                    "tab activation {} is stale for browser window {}; no activation is pending",
                    actual.get(),
                    window.get()
                ),
            },
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
pub struct TabCloseResult {
    tab: Tab,
    invalidated_activation: Option<TabActivationIntent>,
    active_tab: Option<TabId>,
}

impl TabCloseResult {
    pub const fn tab(&self) -> &Tab {
        &self.tab
    }

    pub const fn invalidated_activation(&self) -> Option<TabActivationIntent> {
        self.invalidated_activation
    }

    pub const fn active_tab(&self) -> Option<TabId> {
        self.active_tab
    }

    pub fn into_tab(self) -> Tab {
        self.tab
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserWindow {
    id: BrowserWindowId,
    tabs: Vec<Tab>,
    active_tab: Option<TabId>,
    pending_activation: Option<TabActivationIntent>,
    address_bar: AddressBarState,
}

impl BrowserWindow {
    fn new(id: BrowserWindowId) -> Self {
        Self {
            id,
            tabs: Vec::new(),
            active_tab: None,
            pending_activation: None,
            address_bar: AddressBarState::default(),
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

    pub const fn pending_tab_activation(&self) -> Option<TabActivationIntent> {
        self.pending_activation
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tab(self.active_tab?)
    }

    pub const fn address_bar(&self) -> &AddressBarState {
        &self.address_bar
    }

    pub fn address_bar_text(&self) -> &str {
        if let Some(edit) = self.address_bar.edit()
            && self.active_tab == Some(edit.tab())
        {
            return edit.text();
        }

        self.active_tab()
            .and_then(|tab| tab.navigation().display_location())
            .unwrap_or("")
    }

    fn begin_address_bar_edit(&mut self) -> Option<TabId> {
        let tab = self.active_tab()?;
        let tab_id = tab.id();
        let text = tab.navigation().display_location().unwrap_or("").to_owned();
        self.address_bar.begin(tab_id, text);
        Some(tab_id)
    }

    fn set_address_bar_text(&mut self, text: String) -> bool {
        self.address_bar.set_text(text)
    }

    fn cancel_address_bar_edit(&mut self) -> bool {
        self.address_bar.cancel()
    }

    fn submit_address_bar_edit(&mut self) -> Option<AddressBarSubmission> {
        self.address_bar.submit()
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

    fn close_tab(&mut self, tab: TabId) -> Option<TabCloseResult> {
        let position = self.tabs.iter().position(|candidate| candidate.id == tab)?;
        let invalidated_activation = if self
            .pending_activation
            .is_some_and(|activation| activation.from() == tab || activation.to() == tab)
        {
            self.pending_activation.take()
        } else {
            None
        };
        self.address_bar.cancel_for_tab(tab);
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

        Some(TabCloseResult {
            tab: removed,
            invalidated_activation,
            active_tab: self.active_tab,
        })
    }

    fn set_active_tab(&mut self, tab: TabId) -> Option<TabActivationIntent> {
        debug_assert!(
            self.tabs.iter().any(|candidate| candidate.id == tab),
            "active tab target must be validated before mutation"
        );
        let invalidated_activation = self.pending_activation.take();
        if self.active_tab != Some(tab) {
            self.address_bar.cancel();
        }
        self.active_tab = Some(tab);
        invalidated_activation
    }

    fn begin_tab_activation(&mut self, intent: TabActivationIntent) -> TabActivationStart {
        let superseded = self.pending_activation.replace(intent);
        TabActivationStart::new(intent, superseded)
    }

    fn take_tab_activation(&mut self, activation: TabActivationId) -> Option<TabActivationIntent> {
        if self.pending_activation.map(TabActivationIntent::id) == Some(activation) {
            self.pending_activation.take()
        } else {
            None
        }
    }

    fn move_tab_before(&mut self, tab: TabId, before: Option<TabId>) {
        if before == Some(tab) {
            return;
        }

        let from = self
            .tabs
            .iter()
            .position(|candidate| candidate.id == tab)
            .expect("tab validated before reorder");
        let moved = self.tabs.remove(from);
        let destination = match before {
            Some(anchor) => self
                .tabs
                .iter()
                .position(|candidate| candidate.id == anchor)
                .expect("reorder anchor validated before mutation"),
            None => self.tabs.len(),
        };
        self.tabs.insert(destination, moved);
    }
}

#[derive(Debug)]
pub struct BrowserApp {
    windows: BTreeMap<BrowserWindowId, BrowserWindow>,
    next_window_id: u64,
    next_tab_id: u64,
    next_tab_activation_id: u64,
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
            next_tab_activation_id: 1,
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
    ) -> Result<TabCloseResult, BrowserModelError> {
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
    ) -> Result<Option<TabActivationIntent>, BrowserModelError> {
        let browser_window = self
            .windows
            .get_mut(&window)
            .ok_or(BrowserModelError::UnknownWindow(window))?;

        if browser_window.tab(tab).is_none() {
            return Err(BrowserModelError::UnknownTab { window, tab });
        }
        Ok(browser_window.set_active_tab(tab))
    }

    pub fn move_tab_before(
        &mut self,
        window: BrowserWindowId,
        tab: TabId,
        before: Option<TabId>,
    ) -> Result<(), BrowserModelError> {
        let browser_window = self
            .windows
            .get_mut(&window)
            .ok_or(BrowserModelError::UnknownWindow(window))?;

        if browser_window.tab(tab).is_none() {
            return Err(BrowserModelError::UnknownTab { window, tab });
        }
        if let Some(anchor) = before
            && browser_window.tab(anchor).is_none()
        {
            return Err(BrowserModelError::UnknownTab {
                window,
                tab: anchor,
            });
        }

        browser_window.move_tab_before(tab, before);
        Ok(())
    }

    pub fn begin_tab_activation(
        &mut self,
        window: BrowserWindowId,
        tab: TabId,
    ) -> Result<TabActivationStart, BrowserModelError> {
        self.ensure_tab(window, tab)?;
        let active = self
            .windows
            .get(&window)
            .expect("target tab validation also validates the window")
            .active_tab_id()
            .ok_or(BrowserModelError::NoActiveTab { window })?;
        let id = self.allocate_tab_activation_id()?;
        let intent = TabActivationIntent::new(id, active, tab);
        Ok(self
            .windows
            .get_mut(&window)
            .expect("window validated before activation")
            .begin_tab_activation(intent))
    }

    pub fn commit_tab_activation(
        &mut self,
        window: BrowserWindowId,
        activation: TabActivationId,
    ) -> Result<TabId, BrowserModelError> {
        let browser_window = self
            .windows
            .get_mut(&window)
            .ok_or(BrowserModelError::UnknownWindow(window))?;
        let expected = browser_window
            .pending_tab_activation()
            .map(TabActivationIntent::id);
        if expected != Some(activation) {
            return Err(BrowserModelError::StaleTabActivation {
                window,
                expected,
                actual: activation,
            });
        }

        let intent = browser_window
            .pending_tab_activation()
            .expect("pending activation validated before commit");
        if browser_window.active_tab_id() != Some(intent.from())
            || browser_window.tab(intent.to()).is_none()
        {
            return Err(BrowserModelError::StaleTabActivation {
                window,
                expected: Some(activation),
                actual: activation,
            });
        }

        let intent = browser_window
            .take_tab_activation(activation)
            .expect("pending activation remains current after validation");
        let target = intent.to();
        let invalidated = browser_window.set_active_tab(target);
        debug_assert!(
            invalidated.is_none(),
            "committed activation was removed before active-tab mutation"
        );
        Ok(target)
    }

    pub fn cancel_tab_activation(
        &mut self,
        window: BrowserWindowId,
        activation: TabActivationId,
    ) -> Result<TabActivationIntent, BrowserModelError> {
        let browser_window = self
            .windows
            .get_mut(&window)
            .ok_or(BrowserModelError::UnknownWindow(window))?;
        let expected = browser_window
            .pending_tab_activation()
            .map(TabActivationIntent::id);
        browser_window.take_tab_activation(activation).ok_or(
            BrowserModelError::StaleTabActivation {
                window,
                expected,
                actual: activation,
            },
        )
    }

    pub fn begin_address_bar_edit(
        &mut self,
        window: BrowserWindowId,
    ) -> Result<Option<TabId>, BrowserModelError> {
        let browser_window = self
            .windows
            .get_mut(&window)
            .ok_or(BrowserModelError::UnknownWindow(window))?;
        Ok(browser_window.begin_address_bar_edit())
    }

    pub fn set_address_bar_text(
        &mut self,
        window: BrowserWindowId,
        text: impl Into<String>,
    ) -> Result<(), BrowserModelError> {
        let browser_window = self
            .windows
            .get_mut(&window)
            .ok_or(BrowserModelError::UnknownWindow(window))?;
        if browser_window.set_address_bar_text(text.into()) {
            Ok(())
        } else {
            Err(BrowserModelError::AddressBarNotEditing { window })
        }
    }

    pub fn cancel_address_bar_edit(
        &mut self,
        window: BrowserWindowId,
    ) -> Result<bool, BrowserModelError> {
        let browser_window = self
            .windows
            .get_mut(&window)
            .ok_or(BrowserModelError::UnknownWindow(window))?;
        Ok(browser_window.cancel_address_bar_edit())
    }

    pub fn submit_address_bar_edit(
        &mut self,
        window: BrowserWindowId,
    ) -> Result<AddressBarSubmission, BrowserModelError> {
        let browser_window = self
            .windows
            .get_mut(&window)
            .ok_or(BrowserModelError::UnknownWindow(window))?;
        browser_window
            .submit_address_bar_edit()
            .ok_or(BrowserModelError::AddressBarNotEditing { window })
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
        self.begin_existing_navigation(window, tab, target, |entry| NavigationIntentKind::Reload {
            entry,
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

    fn allocate_tab_activation_id(&mut self) -> Result<TabActivationId, BrowserModelError> {
        let id = TabActivationId(self.next_tab_activation_id);
        self.next_tab_activation_id = self
            .next_tab_activation_id
            .checked_add(1)
            .ok_or(BrowserModelError::TabActivationIdExhausted)?;
        Ok(id)
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
        assert_eq!(
            app.set_active_tab(window, first)
                .expect("activate first tab"),
            None
        );

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
    fn address_bar_displays_pending_location_before_committed_history() {
        let mut app = BrowserApp::bootstrap().expect("bootstrap");
        let (window, tab) = bootstrap_ids(&app);
        let committed = app
            .begin_navigation(window, tab, "https://committed.example/")
            .expect("committed navigation")
            .intent()
            .id();
        app.commit_navigation(window, tab, committed, "https://committed.example/final")
            .expect("commit navigation");

        assert_eq!(
            app.window(window).expect("window").address_bar_text(),
            "https://committed.example/final"
        );

        let pending = app
            .begin_navigation(window, tab, "https://pending.example/")
            .expect("pending navigation")
            .intent()
            .id();
        assert_eq!(
            app.window(window).expect("window").address_bar_text(),
            "https://pending.example/"
        );

        app.fail_navigation(window, tab, pending, "fixture failure")
            .expect("fail pending navigation");
        assert_eq!(
            app.window(window).expect("window").address_bar_text(),
            "https://committed.example/final"
        );
    }

    #[test]
    fn address_bar_edit_preserves_user_text_while_navigation_changes_underneath() {
        let mut app = BrowserApp::bootstrap().expect("bootstrap");
        let (window, tab) = bootstrap_ids(&app);
        let initial = app
            .begin_navigation(window, tab, "https://initial.example/")
            .expect("initial navigation")
            .intent()
            .id();
        app.commit_navigation(window, tab, initial, "https://initial.example/")
            .expect("initial commit");

        assert_eq!(
            app.begin_address_bar_edit(window).expect("begin edit"),
            Some(tab)
        );
        app.set_address_bar_text(window, "user typed query")
            .expect("edit text");
        app.begin_navigation(window, tab, "https://background.example/")
            .expect("background navigation");

        assert_eq!(
            app.window(window).expect("window").address_bar_text(),
            "user typed query"
        );

        assert!(app.cancel_address_bar_edit(window).expect("cancel edit"));
        assert_eq!(
            app.window(window).expect("window").address_bar_text(),
            "https://background.example/"
        );
    }

    #[test]
    fn switching_tabs_cancels_window_owned_address_bar_edit() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first tab");
        let second = app.create_tab(window).expect("second tab");

        let first_navigation = app
            .begin_navigation(window, first, "https://first.example/")
            .expect("first navigation")
            .intent()
            .id();
        app.commit_navigation(window, first, first_navigation, "https://first.example/")
            .expect("first commit");

        let second_navigation = app
            .begin_navigation(window, second, "https://second.example/")
            .expect("second navigation")
            .intent()
            .id();
        app.commit_navigation(window, second, second_navigation, "https://second.example/")
            .expect("second commit");

        app.begin_address_bar_edit(window)
            .expect("begin edit")
            .expect("active first tab");
        app.set_address_bar_text(window, "unfinished edit")
            .expect("edit text");

        assert_eq!(
            app.set_active_tab(window, second)
                .expect("activate second tab"),
            None
        );

        let browser_window = app.window(window).expect("window");
        assert!(!browser_window.address_bar().is_editing());
        assert_eq!(browser_window.address_bar_text(), "https://second.example/");
    }

    #[test]
    fn closing_the_edited_tab_clears_edit_before_selecting_neighbor() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first tab");
        let second = app.create_tab(window).expect("second tab");
        let navigation = app
            .begin_navigation(window, second, "https://second.example/")
            .expect("second navigation")
            .intent()
            .id();
        app.commit_navigation(window, second, navigation, "https://second.example/")
            .expect("second commit");

        app.begin_address_bar_edit(window)
            .expect("begin edit")
            .expect("active first tab");
        app.set_address_bar_text(window, "unfinished edit")
            .expect("edit text");
        app.close_tab(window, first).expect("close edited tab");

        let browser_window = app.window(window).expect("window");
        assert_eq!(browser_window.active_tab_id(), Some(second));
        assert!(!browser_window.address_bar().is_editing());
        assert_eq!(browser_window.address_bar_text(), "https://second.example/");
    }

    #[test]
    fn address_bar_submission_is_raw_chrome_input_not_implicit_navigation() {
        let mut app = BrowserApp::bootstrap().expect("bootstrap");
        let (window, tab) = bootstrap_ids(&app);

        app.begin_address_bar_edit(window)
            .expect("begin edit")
            .expect("active tab");
        app.set_address_bar_text(window, "  example search terms  ")
            .expect("edit text");
        let submission = app
            .submit_address_bar_edit(window)
            .expect("submit address bar");

        assert_eq!(submission.tab(), tab);
        assert_eq!(submission.text(), "  example search terms  ");
        let navigation = app
            .window(window)
            .and_then(|browser_window| browser_window.tab(tab))
            .expect("tab")
            .navigation();
        assert!(navigation.pending().is_none());
        assert!(navigation.history().is_empty());
        assert_eq!(app.window(window).expect("window").address_bar_text(), "");
        assert_eq!(
            app.submit_address_bar_edit(window),
            Err(BrowserModelError::AddressBarNotEditing { window })
        );
    }

    #[test]
    fn tab_reorder_uses_stable_anchor_identity_and_preserves_active_tab() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let second = app.create_tab(window).expect("second");
        let third = app.create_tab(window).expect("third");

        app.move_tab_before(window, third, Some(first))
            .expect("move third before first");
        assert_eq!(
            app.window(window)
                .expect("window")
                .tabs()
                .iter()
                .map(Tab::id)
                .collect::<Vec<_>>(),
            vec![third, first, second]
        );
        assert_eq!(
            app.window(window).and_then(BrowserWindow::active_tab_id),
            Some(first)
        );

        app.move_tab_before(window, first, None)
            .expect("move first to end");
        assert_eq!(
            app.window(window)
                .expect("window")
                .tabs()
                .iter()
                .map(Tab::id)
                .collect::<Vec<_>>(),
            vec![third, second, first]
        );
        assert_eq!(
            app.window(window).and_then(BrowserWindow::active_tab_id),
            Some(first)
        );
    }

    #[test]
    fn tab_reorder_preserves_address_bar_edit_bound_to_tab_identity() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let second = app.create_tab(window).expect("second");

        app.begin_address_bar_edit(window)
            .expect("begin edit")
            .expect("active first tab");
        app.set_address_bar_text(window, "unfinished edit")
            .expect("edit text");

        app.move_tab_before(window, second, Some(first))
            .expect("move second before edited tab");

        let browser_window = app.window(window).expect("window");
        assert_eq!(browser_window.active_tab_id(), Some(first));
        assert_eq!(browser_window.address_bar().editing_tab(), Some(first));
        assert_eq!(browser_window.address_bar_text(), "unfinished edit");
        assert_eq!(
            browser_window
                .tabs()
                .iter()
                .map(Tab::id)
                .collect::<Vec<_>>(),
            vec![second, first]
        );
    }

    #[test]
    fn invalid_reorder_anchor_is_rejected_before_tab_order_changes() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let second = app.create_tab(window).expect("second");
        let missing = TabId(999);

        assert_eq!(
            app.move_tab_before(window, second, Some(missing)),
            Err(BrowserModelError::UnknownTab {
                window,
                tab: missing,
            })
        );
        assert_eq!(
            app.window(window)
                .expect("window")
                .tabs()
                .iter()
                .map(Tab::id)
                .collect::<Vec<_>>(),
            vec![first, second]
        );

        app.move_tab_before(window, first, Some(first))
            .expect("self anchor is a no-op");
        assert_eq!(
            app.window(window)
                .expect("window")
                .tabs()
                .iter()
                .map(Tab::id)
                .collect::<Vec<_>>(),
            vec![first, second]
        );
    }

    #[test]
    fn tab_activation_begin_does_not_change_active_tab_or_address_edit() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let second = app.create_tab(window).expect("second");

        app.begin_address_bar_edit(window)
            .expect("begin edit")
            .expect("active first tab");
        app.set_address_bar_text(window, "unfinished edit")
            .expect("edit text");

        let start = app
            .begin_tab_activation(window, second)
            .expect("begin activation");

        assert_eq!(start.intent().from(), first);
        assert_eq!(start.intent().to(), second);
        assert!(start.superseded().is_none());

        let browser_window = app.window(window).expect("window");
        assert_eq!(browser_window.active_tab_id(), Some(first));
        assert_eq!(
            browser_window.pending_tab_activation(),
            Some(start.intent())
        );
        assert_eq!(browser_window.address_bar().editing_tab(), Some(first));
        assert_eq!(browser_window.address_bar_text(), "unfinished edit");
    }

    #[test]
    fn tab_activation_commit_changes_active_tab_only_after_exact_commit() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let second = app.create_tab(window).expect("second");

        app.begin_address_bar_edit(window)
            .expect("begin edit")
            .expect("active first tab");
        app.set_address_bar_text(window, "unfinished edit")
            .expect("edit text");

        let activation = app
            .begin_tab_activation(window, second)
            .expect("begin activation")
            .intent()
            .id();

        assert_eq!(
            app.window(window).and_then(BrowserWindow::active_tab_id),
            Some(first)
        );
        assert_eq!(
            app.commit_tab_activation(window, activation)
                .expect("commit activation"),
            second
        );

        let browser_window = app.window(window).expect("window");
        assert_eq!(browser_window.active_tab_id(), Some(second));
        assert!(browser_window.pending_tab_activation().is_none());
        assert!(!browser_window.address_bar().is_editing());
    }

    #[test]
    fn newer_tab_activation_supersedes_older_and_rejects_stale_commit() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let second = app.create_tab(window).expect("second");
        let third = app.create_tab(window).expect("third");

        let stale = app
            .begin_tab_activation(window, second)
            .expect("first activation");
        let current = app
            .begin_tab_activation(window, third)
            .expect("second activation");

        assert!(current.intent().id().get() > stale.intent().id().get());
        assert_eq!(current.intent().from(), first);
        assert_eq!(current.superseded(), Some(stale.intent()));
        assert_eq!(
            app.commit_tab_activation(window, stale.intent().id()),
            Err(BrowserModelError::StaleTabActivation {
                window,
                expected: Some(current.intent().id()),
                actual: stale.intent().id(),
            })
        );
        assert_eq!(
            app.window(window).and_then(BrowserWindow::active_tab_id),
            Some(first)
        );

        app.commit_tab_activation(window, current.intent().id())
            .expect("commit current activation");
        assert_eq!(
            app.window(window).and_then(BrowserWindow::active_tab_id),
            Some(third)
        );
    }

    #[test]
    fn closing_activation_target_invalidates_pending_transition() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let second = app.create_tab(window).expect("second");
        let activation = app
            .begin_tab_activation(window, second)
            .expect("activation")
            .intent()
            .id();

        let closed = app.close_tab(window, second).expect("close target");
        assert_eq!(
            closed.invalidated_activation().map(TabActivationIntent::id),
            Some(activation)
        );

        let browser_window = app.window(window).expect("window");
        assert_eq!(browser_window.active_tab_id(), Some(first));
        assert!(browser_window.pending_tab_activation().is_none());
        assert_eq!(
            app.commit_tab_activation(window, activation),
            Err(BrowserModelError::StaleTabActivation {
                window,
                expected: None,
                actual: activation,
            })
        );
    }

    #[test]
    fn closing_activation_source_invalidates_transition_before_neighbor_selection() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let second = app.create_tab(window).expect("second");
        let third = app.create_tab(window).expect("third");
        let activation = app
            .begin_tab_activation(window, third)
            .expect("activation")
            .intent()
            .id();

        let closed = app.close_tab(window, first).expect("close source");
        assert_eq!(
            closed.invalidated_activation().map(TabActivationIntent::id),
            Some(activation)
        );
        assert_eq!(closed.active_tab(), Some(second));

        let browser_window = app.window(window).expect("window");
        assert_eq!(browser_window.active_tab_id(), Some(second));
        assert!(browser_window.pending_tab_activation().is_none());
        assert_eq!(
            app.commit_tab_activation(window, activation),
            Err(BrowserModelError::StaleTabActivation {
                window,
                expected: None,
                actual: activation,
            })
        );
    }

    #[test]
    fn closing_unrelated_tab_preserves_pending_activation() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let second = app.create_tab(window).expect("second");
        let third = app.create_tab(window).expect("third");
        let activation = app
            .begin_tab_activation(window, second)
            .expect("activation")
            .intent();

        let closed = app.close_tab(window, third).expect("close unrelated tab");
        assert!(closed.invalidated_activation().is_none());

        let browser_window = app.window(window).expect("window");
        assert_eq!(browser_window.active_tab_id(), Some(first));
        assert_eq!(browser_window.pending_tab_activation(), Some(activation));
        app.commit_tab_activation(window, activation.id())
            .expect("commit preserved activation");
        assert_eq!(
            app.window(window).and_then(BrowserWindow::active_tab_id),
            Some(second)
        );
    }

    #[test]
    fn reorder_preserves_pending_activation_by_stable_tab_identity() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let second = app.create_tab(window).expect("second");
        let third = app.create_tab(window).expect("third");
        let activation = app
            .begin_tab_activation(window, third)
            .expect("activation")
            .intent();

        app.move_tab_before(window, third, Some(second))
            .expect("reorder target");
        app.move_tab_before(window, first, None)
            .expect("reorder source");

        assert_eq!(
            app.window(window).expect("window").pending_tab_activation(),
            Some(activation)
        );
        app.commit_tab_activation(window, activation.id())
            .expect("commit after reorder");
        assert_eq!(
            app.window(window).and_then(BrowserWindow::active_tab_id),
            Some(third)
        );
    }

    #[test]
    fn immediate_active_tab_change_cancels_pending_activation() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let second = app.create_tab(window).expect("second");
        let third = app.create_tab(window).expect("third");
        let activation = app
            .begin_tab_activation(window, second)
            .expect("activation")
            .intent()
            .id();

        let invalidated = app
            .set_active_tab(window, third)
            .expect("immediate active change");
        assert_eq!(invalidated.map(TabActivationIntent::id), Some(activation));

        let browser_window = app.window(window).expect("window");
        assert_eq!(browser_window.active_tab_id(), Some(third));
        assert!(browser_window.pending_tab_activation().is_none());
        assert_eq!(
            app.commit_tab_activation(window, activation),
            Err(BrowserModelError::StaleTabActivation {
                window,
                expected: None,
                actual: activation,
            })
        );
        assert_ne!(first, third);
    }

    #[test]
    fn activation_to_current_tab_supersedes_pending_switch_without_dropping_edit() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let second = app.create_tab(window).expect("second");

        app.begin_address_bar_edit(window)
            .expect("begin edit")
            .expect("active first tab");
        app.set_address_bar_text(window, "unfinished edit")
            .expect("edit text");

        let pending = app
            .begin_tab_activation(window, second)
            .expect("pending switch");
        let stay = app
            .begin_tab_activation(window, first)
            .expect("stay on current tab");

        assert_eq!(stay.superseded(), Some(pending.intent()));
        assert_eq!(stay.intent().from(), first);
        assert_eq!(stay.intent().to(), first);
        app.commit_tab_activation(window, stay.intent().id())
            .expect("commit current-tab activation");

        let browser_window = app.window(window).expect("window");
        assert_eq!(browser_window.active_tab_id(), Some(first));
        assert_eq!(browser_window.address_bar().editing_tab(), Some(first));
        assert_eq!(browser_window.address_bar_text(), "unfinished edit");
    }

    #[test]
    fn stale_activation_cancel_cannot_remove_newer_transition() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let _first = app.create_tab(window).expect("first");
        let second = app.create_tab(window).expect("second");
        let third = app.create_tab(window).expect("third");

        let stale = app
            .begin_tab_activation(window, second)
            .expect("stale activation")
            .intent();
        let current = app
            .begin_tab_activation(window, third)
            .expect("current activation")
            .intent();

        assert_eq!(
            app.cancel_tab_activation(window, stale.id()),
            Err(BrowserModelError::StaleTabActivation {
                window,
                expected: Some(current.id()),
                actual: stale.id(),
            })
        );
        assert_eq!(
            app.window(window).expect("window").pending_tab_activation(),
            Some(current)
        );
    }

    #[test]
    fn tab_activation_can_be_cancelled_without_changing_active_tab() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let second = app.create_tab(window).expect("second");
        let start = app
            .begin_tab_activation(window, second)
            .expect("activation");

        assert_eq!(
            app.cancel_tab_activation(window, start.intent().id())
                .expect("cancel activation"),
            start.intent()
        );
        assert_eq!(
            app.window(window).and_then(BrowserWindow::active_tab_id),
            Some(first)
        );
        assert!(
            app.window(window)
                .expect("window")
                .pending_tab_activation()
                .is_none()
        );
    }

    #[test]
    fn tab_activation_identity_is_monotonic_across_windows() {
        let mut app = BrowserApp::new();
        let first_window = app.create_window().expect("first window");
        let first_active = app.create_tab(first_window).expect("first active");
        let first_target = app.create_tab(first_window).expect("first target");
        let second_window = app.create_window().expect("second window");
        let second_active = app.create_tab(second_window).expect("second active");
        let second_target = app.create_tab(second_window).expect("second target");

        let first = app
            .begin_tab_activation(first_window, first_target)
            .expect("first activation")
            .intent()
            .id();
        let second = app
            .begin_tab_activation(second_window, second_target)
            .expect("second activation")
            .intent()
            .id();

        assert!(second.get() > first.get());
        assert_ne!(first_active, first_target);
        assert_ne!(second_active, second_target);
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
