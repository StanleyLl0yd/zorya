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
    UnknownWindow(BrowserWindowId),
    UnknownTab { window: BrowserWindowId, tab: TabId },
}

impl fmt::Display for BrowserModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WindowIdExhausted => {
                formatter.write_str("browser window identifier space is exhausted")
            }
            Self::TabIdExhausted => formatter.write_str("tab identifier space is exhausted"),
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
        }
    }
}

impl std::error::Error for BrowserModelError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    id: TabId,
}

impl Tab {
    pub const fn id(&self) -> TabId {
        self.id
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

    pub const fn active_tab_id(&self) -> Option<TabId> {
        self.active_tab
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        let active = self.active_tab?;
        self.tabs.iter().find(|tab| tab.id == active)
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
            .insert_tab(Tab { id });
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_creates_one_window_with_one_active_tab() {
        let app = BrowserApp::bootstrap().expect("bootstrap should allocate initial identities");
        let windows = app.windows().collect::<Vec<_>>();

        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].tabs().len(), 1);
        assert_eq!(windows[0].active_tab_id(), Some(windows[0].tabs()[0].id()));
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

        assert_eq!(app.window(window).and_then(BrowserWindow::active_tab_id), Some(second));
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
}
