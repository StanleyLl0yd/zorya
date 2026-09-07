use crate::app::{BrowserWindow, TabId};
use crate::tab_activation::TabActivationIntent;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabStripItemSnapshot {
    tab: TabId,
    committed_active: bool,
    pending_activation_target: bool,
    display_location: Option<String>,
    loading: bool,
}

impl TabStripItemSnapshot {
    pub const fn tab(&self) -> TabId {
        self.tab
    }

    pub const fn committed_active(&self) -> bool {
        self.committed_active
    }

    pub const fn pending_activation_target(&self) -> bool {
        self.pending_activation_target
    }

    pub fn display_location(&self) -> Option<&str> {
        self.display_location.as_deref()
    }

    pub const fn loading(&self) -> bool {
        self.loading
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabStripSnapshot {
    items: Vec<TabStripItemSnapshot>,
    committed_active: Option<TabId>,
    pending_activation: Option<TabActivationIntent>,
}

impl TabStripSnapshot {
    pub fn items(&self) -> &[TabStripItemSnapshot] {
        &self.items
    }

    pub const fn committed_active(&self) -> Option<TabId> {
        self.committed_active
    }

    pub const fn pending_activation(&self) -> Option<TabActivationIntent> {
        self.pending_activation
    }

    pub const fn pending_target(&self) -> Option<TabId> {
        match self.pending_activation {
            Some(intent) => Some(intent.to()),
            None => None,
        }
    }
}

impl BrowserWindow {
    pub fn tab_strip_snapshot(&self) -> TabStripSnapshot {
        let committed_active = self.active_tab_id();
        let pending_activation = self.pending_tab_activation();
        let pending_target = pending_activation.map(TabActivationIntent::to);
        let items = self
            .tabs()
            .iter()
            .map(|tab| TabStripItemSnapshot {
                tab: tab.id(),
                committed_active: committed_active == Some(tab.id()),
                pending_activation_target: pending_target == Some(tab.id()),
                display_location: tab.navigation().display_location().map(str::to_owned),
                loading: tab.navigation().is_loading(),
            })
            .collect();

        TabStripSnapshot {
            items,
            committed_active,
            pending_activation,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BrowserApp;

    fn commit_location(
        app: &mut BrowserApp,
        window: crate::BrowserWindowId,
        tab: TabId,
        location: &str,
    ) {
        let navigation = app
            .begin_navigation(window, tab, location)
            .expect("begin navigation")
            .intent()
            .id();
        app.commit_navigation(window, tab, navigation, location)
            .expect("commit navigation");
    }

    #[test]
    fn tab_strip_preserves_product_order_and_stable_identity() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first tab");
        let second = app.create_tab(window).expect("second tab");
        let third = app.create_tab(window).expect("third tab");

        app.move_tab_before(window, third, Some(second))
            .expect("reorder third before second");

        let snapshot = app.window(window).expect("window").tab_strip_snapshot();
        let order = snapshot
            .items()
            .iter()
            .map(TabStripItemSnapshot::tab)
            .collect::<Vec<_>>();

        assert_eq!(order, vec![first, third, second]);
        assert_eq!(snapshot.committed_active(), Some(first));
        assert!(snapshot.items()[0].committed_active());
        assert!(!snapshot.items()[1].committed_active());
        assert!(!snapshot.items()[2].committed_active());
    }

    #[test]
    fn pending_activation_marks_target_without_changing_committed_active_tab() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first tab");
        let second = app.create_tab(window).expect("second tab");
        let third = app.create_tab(window).expect("third tab");

        let first_activation = app
            .begin_tab_activation(window, second)
            .expect("first activation")
            .intent();
        let first_snapshot = app.window(window).expect("window").tab_strip_snapshot();

        assert_eq!(first_snapshot.committed_active(), Some(first));
        assert_eq!(first_snapshot.pending_target(), Some(second));
        assert!(first_snapshot.items()[0].committed_active());
        assert!(first_snapshot.items()[1].pending_activation_target());
        assert!(!first_snapshot.items()[1].committed_active());

        let second_activation = app
            .begin_tab_activation(window, third)
            .expect("superseding activation");
        assert_eq!(second_activation.superseded(), Some(first_activation));

        let second_snapshot = app.window(window).expect("window").tab_strip_snapshot();
        assert_eq!(second_snapshot.committed_active(), Some(first));
        assert_eq!(second_snapshot.pending_target(), Some(third));
        assert!(!second_snapshot.items()[1].pending_activation_target());
        assert!(second_snapshot.items()[2].pending_activation_target());
    }

    #[test]
    fn closing_pending_activation_target_removes_pending_tab_strip_state() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first tab");
        let second = app.create_tab(window).expect("second tab");

        let activation = app
            .begin_tab_activation(window, second)
            .expect("activation")
            .intent();
        let closed = app.close_tab(window, second).expect("close target tab");

        assert_eq!(closed.invalidated_activation(), Some(activation));

        let snapshot = app.window(window).expect("window").tab_strip_snapshot();
        assert_eq!(snapshot.committed_active(), Some(first));
        assert_eq!(snapshot.pending_activation(), None);
        assert_eq!(snapshot.pending_target(), None);
        assert_eq!(snapshot.items().len(), 1);
        assert_eq!(snapshot.items()[0].tab(), first);
    }

    #[test]
    fn tab_strip_projects_loading_and_location_per_tab_independently() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first tab");
        let second = app.create_tab(window).expect("second tab");

        commit_location(&mut app, window, first, "https://first.example/");
        commit_location(&mut app, window, second, "https://second.example/");
        app.begin_navigation(window, second, "https://pending.example/")
            .expect("pending navigation");

        let snapshot = app.window(window).expect("window").tab_strip_snapshot();
        let first_item = snapshot
            .items()
            .iter()
            .find(|item| item.tab() == first)
            .expect("first item");
        let second_item = snapshot
            .items()
            .iter()
            .find(|item| item.tab() == second)
            .expect("second item");

        assert_eq!(first_item.display_location(), Some("https://first.example/"));
        assert!(!first_item.loading());
        assert_eq!(
            second_item.display_location(),
            Some("https://pending.example/")
        );
        assert!(second_item.loading());
    }

    #[test]
    fn address_bar_edit_text_is_not_a_tab_strip_label_source() {
        let mut app = BrowserApp::bootstrap().expect("bootstrap");
        let window = app.windows().next().expect("window").id();
        let tab = app
            .window(window)
            .and_then(BrowserWindow::active_tab_id)
            .expect("active tab");

        commit_location(&mut app, window, tab, "https://committed.example/");
        app.begin_address_bar_edit(window)
            .expect("begin address edit")
            .expect("active tab");
        app.set_address_bar_text(window, "private unfinished address-bar text")
            .expect("set address-bar text");

        let snapshot = app.window(window).expect("window").tab_strip_snapshot();

        assert_eq!(
            snapshot.items()[0].display_location(),
            Some("https://committed.example/")
        );
        assert_eq!(
            app.window(window)
                .expect("window")
                .chrome_snapshot()
                .expect("chrome snapshot")
                .address_text(),
            "private unfinished address-bar text"
        );
    }
}
