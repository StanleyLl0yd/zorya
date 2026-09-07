use crate::app::{BrowserWindow, TabId};
use crate::navigation::{NavigationControls, ReloadControl};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowChromeSnapshot {
    active_tab: TabId,
    address_text: String,
    address_bar_editing: bool,
    navigation_controls: NavigationControls,
}

impl WindowChromeSnapshot {
    pub(crate) fn new(
        active_tab: TabId,
        address_text: String,
        address_bar_editing: bool,
        navigation_controls: NavigationControls,
    ) -> Self {
        Self {
            active_tab,
            address_text,
            address_bar_editing,
            navigation_controls,
        }
    }

    pub const fn active_tab(&self) -> TabId {
        self.active_tab
    }

    pub fn address_text(&self) -> &str {
        &self.address_text
    }

    pub const fn address_bar_editing(&self) -> bool {
        self.address_bar_editing
    }

    pub const fn navigation_controls(&self) -> NavigationControls {
        self.navigation_controls
    }

    pub const fn is_loading(&self) -> bool {
        matches!(self.navigation_controls.reload(), ReloadControl::Stop)
    }
}


impl BrowserWindow {
    pub fn chrome_snapshot(&self) -> Option<WindowChromeSnapshot> {
        let active_tab = self.active_tab()?;
        let active_tab_id = active_tab.id();

        Some(WindowChromeSnapshot::new(
            active_tab_id,
            self.address_bar_text().to_owned(),
            self.address_bar().editing_tab() == Some(active_tab_id),
            active_tab.navigation().controls(),
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddressBarEdit {
    tab: TabId,
    text: String,
}

impl AddressBarEdit {
    pub const fn tab(&self) -> TabId {
        self.tab
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddressBarSubmission {
    tab: TabId,
    text: String,
}

impl AddressBarSubmission {
    pub const fn tab(&self) -> TabId {
        self.tab
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn into_text(self) -> String {
        self.text
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AddressBarState {
    edit: Option<AddressBarEdit>,
}

impl AddressBarState {
    pub fn edit(&self) -> Option<&AddressBarEdit> {
        self.edit.as_ref()
    }

    pub const fn editing_tab(&self) -> Option<TabId> {
        match &self.edit {
            Some(edit) => Some(edit.tab),
            None => None,
        }
    }

    pub const fn is_editing(&self) -> bool {
        self.edit.is_some()
    }

    pub(crate) fn begin(&mut self, tab: TabId, text: String) {
        self.edit = Some(AddressBarEdit { tab, text });
    }

    pub(crate) fn set_text(&mut self, text: String) -> bool {
        let Some(edit) = &mut self.edit else {
            return false;
        };
        edit.text = text;
        true
    }

    pub(crate) fn cancel(&mut self) -> bool {
        self.edit.take().is_some()
    }

    pub(crate) fn submit(&mut self) -> Option<AddressBarSubmission> {
        self.edit.take().map(|edit| AddressBarSubmission {
            tab: edit.tab,
            text: edit.text,
        })
    }

    pub(crate) fn cancel_for_tab(&mut self, tab: TabId) -> bool {
        if self.editing_tab() == Some(tab) {
            self.cancel()
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::BrowserApp;

    #[test]
    fn submission_preserves_verbatim_user_text_and_target_tab() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let tab = app.create_tab(window).expect("tab");
        let mut state = AddressBarState::default();
        state.begin(tab, "  Example Search?  ".into());
        state.set_text("  Example Search?  ".into());

        let submission = state.submit().expect("active edit");

        assert_eq!(submission.tab(), tab);
        assert_eq!(submission.text(), "  Example Search?  ");
        assert!(!state.is_editing());
    }

    #[test]
    fn cancel_for_tab_does_not_destroy_another_tabs_edit() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first tab");
        let second = app.create_tab(window).expect("second tab");
        let mut state = AddressBarState::default();
        state.begin(second, "https://example.test/".into());

        assert!(!state.cancel_for_tab(first));
        assert!(state.is_editing());
        assert!(state.cancel_for_tab(second));
        assert!(!state.is_editing());
    }

    #[test]
    fn chrome_snapshot_projects_pending_navigation_as_loading_stop_state() {
        let mut app = BrowserApp::bootstrap().expect("bootstrap");
        let window = app.windows().next().expect("window").id();
        let tab = app
            .window(window)
            .and_then(BrowserWindow::active_tab_id)
            .expect("active tab");

        let committed = app
            .begin_navigation(window, tab, "https://committed.example/")
            .expect("begin committed navigation")
            .intent()
            .id();
        app.commit_navigation(
            window,
            tab,
            committed,
            "https://committed.example/final",
        )
        .expect("commit navigation");

        let pending = app
            .begin_navigation(window, tab, "https://pending.example/")
            .expect("begin pending navigation")
            .intent()
            .id();

        let snapshot = app
            .window(window)
            .and_then(BrowserWindow::chrome_snapshot)
            .expect("chrome snapshot");
        assert_eq!(snapshot.active_tab(), tab);
        assert_eq!(snapshot.address_text(), "https://pending.example/");
        assert!(!snapshot.address_bar_editing());
        assert!(snapshot.is_loading());
        assert_eq!(
            snapshot.navigation_controls().reload(),
            ReloadControl::Stop
        );

        app.fail_navigation(window, tab, pending, "fixture failure")
            .expect("fail navigation");

        let snapshot = app
            .window(window)
            .and_then(BrowserWindow::chrome_snapshot)
            .expect("chrome snapshot after failure");
        assert_eq!(
            snapshot.address_text(),
            "https://committed.example/final"
        );
        assert!(!snapshot.is_loading());
        assert_eq!(
            snapshot.navigation_controls().reload(),
            ReloadControl::Reload
        );
    }

    #[test]
    fn chrome_snapshot_keeps_address_edit_text_while_navigation_changes() {
        let mut app = BrowserApp::bootstrap().expect("bootstrap");
        let window = app.windows().next().expect("window").id();
        let tab = app
            .window(window)
            .and_then(BrowserWindow::active_tab_id)
            .expect("active tab");

        let committed = app
            .begin_navigation(window, tab, "https://initial.example/")
            .expect("begin navigation")
            .intent()
            .id();
        app.commit_navigation(window, tab, committed, "https://initial.example/")
            .expect("commit navigation");

        app.begin_address_bar_edit(window)
            .expect("begin address edit")
            .expect("active tab");
        app.set_address_bar_text(window, "typed but not submitted")
            .expect("set address text");
        app.begin_navigation(window, tab, "https://background.example/")
            .expect("begin background navigation");

        let snapshot = app
            .window(window)
            .and_then(BrowserWindow::chrome_snapshot)
            .expect("chrome snapshot");
        assert_eq!(snapshot.active_tab(), tab);
        assert_eq!(snapshot.address_text(), "typed but not submitted");
        assert!(snapshot.address_bar_editing());
        assert!(snapshot.is_loading());
    }

    #[test]
    fn pending_tab_activation_does_not_change_committed_chrome_snapshot() {
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
        app.commit_navigation(
            window,
            second,
            second_navigation,
            "https://second.example/",
        )
        .expect("second commit");

        let activation = app
            .begin_tab_activation(window, second)
            .expect("begin activation")
            .intent();

        let pending_snapshot = app
            .window(window)
            .and_then(BrowserWindow::chrome_snapshot)
            .expect("pending snapshot");
        assert_eq!(pending_snapshot.active_tab(), first);
        assert_eq!(pending_snapshot.address_text(), "https://first.example/");

        app.commit_tab_activation(window, activation.id())
            .expect("commit activation");

        let committed_snapshot = app
            .window(window)
            .and_then(BrowserWindow::chrome_snapshot)
            .expect("committed snapshot");
        assert_eq!(committed_snapshot.active_tab(), second);
        assert_eq!(
            committed_snapshot.address_text(),
            "https://second.example/"
        );
    }
}
