use crate::app::{BrowserApp, BrowserModelError, BrowserWindowId, TabId};
use crate::navigation::{NavigationIntent, NavigationStart};
use crate::tab_activation::{TabActivationStart, TabCycleDirection};
use crate::tab_close::TabCloseStart;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserCommand {
    NewTab,
    CloseTab,
    FocusAddressBar,
    Back,
    Forward,
    ReloadOrStop,
    CycleTab(TabCycleDirection),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowserCommandEffect {
    TabCreated(TabId),
    TabCloseStarted(TabCloseStart),
    AddressBarEditStarted(TabId),
    NavigationStarted(NavigationStart),
    NavigationStopped(NavigationIntent),
    TabActivationStarted(TabActivationStart),
    Unavailable,
}

impl BrowserApp {
    pub fn dispatch_browser_command(
        &mut self,
        window: BrowserWindowId,
        command: BrowserCommand,
    ) -> Result<BrowserCommandEffect, BrowserModelError> {
        if command == BrowserCommand::NewTab {
            return self
                .create_tab(window)
                .map(BrowserCommandEffect::TabCreated);
        }

        if let BrowserCommand::CycleTab(direction) = command {
            return Ok(self
                .begin_tab_cycle(window, direction)?
                .map(BrowserCommandEffect::TabActivationStarted)
                .unwrap_or(BrowserCommandEffect::Unavailable));
        }

        let active_tab = self
            .window(window)
            .ok_or(BrowserModelError::UnknownWindow(window))?
            .active_tab_id()
            .ok_or(BrowserModelError::NoActiveTab { window })?;

        match command {
            BrowserCommand::NewTab => {
                unreachable!("new-tab commands are handled before active-tab routing")
            }
            BrowserCommand::CloseTab => self
                .begin_tab_close(window, active_tab)
                .map(BrowserCommandEffect::TabCloseStarted),
            BrowserCommand::FocusAddressBar => {
                let edited_tab = self
                    .begin_address_bar_edit(window)?
                    .expect("active tab validated before address-bar edit");
                debug_assert_eq!(edited_tab, active_tab);
                Ok(BrowserCommandEffect::AddressBarEditStarted(edited_tab))
            }
            BrowserCommand::Back => Ok(self
                .begin_back_navigation(window, active_tab)?
                .map(BrowserCommandEffect::NavigationStarted)
                .unwrap_or(BrowserCommandEffect::Unavailable)),
            BrowserCommand::Forward => Ok(self
                .begin_forward_navigation(window, active_tab)?
                .map(BrowserCommandEffect::NavigationStarted)
                .unwrap_or(BrowserCommandEffect::Unavailable)),
            BrowserCommand::ReloadOrStop => {
                let is_loading = self
                    .window(window)
                    .and_then(|browser_window| browser_window.tab(active_tab))
                    .expect("active tab validated before reload/stop")
                    .navigation()
                    .is_loading();

                if is_loading {
                    let stopped = self
                        .stop_navigation(window, active_tab)?
                        .expect("loading state implies a pending navigation");
                    Ok(BrowserCommandEffect::NavigationStopped(stopped))
                } else {
                    Ok(self
                        .begin_reload(window, active_tab)?
                        .map(BrowserCommandEffect::NavigationStarted)
                        .unwrap_or(BrowserCommandEffect::Unavailable))
                }
            }
            BrowserCommand::CycleTab(_) => {
                unreachable!("tab-cycle commands are handled before active-tab routing")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NavigationIntentKind, ReloadControl};

    fn bootstrap_ids(app: &BrowserApp) -> (BrowserWindowId, TabId) {
        let window = app.windows().next().expect("bootstrap window");
        (
            window.id(),
            window.active_tab_id().expect("bootstrap active tab"),
        )
    }

    fn commit_location(app: &mut BrowserApp, window: BrowserWindowId, tab: TabId, location: &str) {
        let navigation = app
            .begin_navigation(window, tab, location)
            .expect("begin navigation")
            .intent()
            .id();
        app.commit_navigation(window, tab, navigation, location)
            .expect("commit navigation");
    }

    #[test]
    fn new_tab_command_allocates_stable_identity_without_switching_existing_active_tab() {
        let mut app = BrowserApp::bootstrap().expect("bootstrap");
        let (window, first) = bootstrap_ids(&app);

        let effect = app
            .dispatch_browser_command(window, BrowserCommand::NewTab)
            .expect("new-tab command");
        let BrowserCommandEffect::TabCreated(second) = effect else {
            panic!("new-tab command should return the created identity");
        };

        assert_ne!(second, first);
        let browser_window = app.window(window).expect("window");
        assert_eq!(browser_window.active_tab_id(), Some(first));
        assert_eq!(browser_window.tabs().len(), 2);
        assert_eq!(browser_window.tabs()[1].id(), second);
    }

    #[test]
    fn new_tab_command_can_create_the_first_active_tab_in_an_empty_window() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");

        let effect = app
            .dispatch_browser_command(window, BrowserCommand::NewTab)
            .expect("new-tab command");
        let BrowserCommandEffect::TabCreated(tab) = effect else {
            panic!("new-tab command should return the created identity");
        };

        assert_eq!(
            app.window(window).and_then(|window| window.active_tab_id()),
            Some(tab)
        );
    }

    #[test]
    fn close_tab_command_starts_presentation_aware_active_close() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let second = app.create_tab(window).expect("second");

        let effect = app
            .dispatch_browser_command(window, BrowserCommand::CloseTab)
            .expect("close command");
        let BrowserCommandEffect::TabCloseStarted(TabCloseStart::ActiveWithFallback(close)) =
            effect
        else {
            panic!("active close should start fallback handoff");
        };

        assert_eq!(close.closing_tab(), first);
        assert_eq!(close.fallback_tab(), second);
        let browser_window = app.window(window).expect("window");
        assert_eq!(browser_window.active_tab_id(), Some(first));
        assert!(browser_window.tab(first).is_some());
        assert_eq!(
            browser_window.pending_tab_activation(),
            Some(close.activation().intent())
        );
    }

    #[test]
    fn back_command_targets_committed_tab_while_activation_is_pending() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first tab");
        let second = app.create_tab(window).expect("second tab");

        commit_location(&mut app, window, first, "https://first-a.example/");
        commit_location(&mut app, window, first, "https://first-b.example/");
        commit_location(&mut app, window, second, "https://second.example/");

        let activation = app
            .begin_tab_activation(window, second)
            .expect("pending activation");
        assert_eq!(activation.intent().from(), first);
        assert_eq!(activation.intent().to(), second);

        let effect = app
            .dispatch_browser_command(window, BrowserCommand::Back)
            .expect("back command");
        let BrowserCommandEffect::NavigationStarted(start) = effect else {
            panic!("back should start a navigation");
        };

        assert_eq!(
            start.intent().kind(),
            NavigationIntentKind::TraverseHistory {
                entry: app
                    .window(window)
                    .and_then(|browser_window| browser_window.tab(first))
                    .and_then(|tab| tab.navigation().history().first())
                    .expect("first history entry")
                    .id(),
            }
        );
        assert_eq!(
            app.window(window)
                .and_then(|browser_window| browser_window.tab(first))
                .and_then(|tab| tab.navigation().pending())
                .map(|intent| intent.id()),
            Some(start.intent().id())
        );
        assert!(
            app.window(window)
                .and_then(|browser_window| browser_window.tab(second))
                .expect("second tab")
                .navigation()
                .pending()
                .is_none()
        );
    }

    #[test]
    fn reload_or_stop_stops_the_exact_current_pending_navigation_atomically() {
        let mut app = BrowserApp::bootstrap().expect("bootstrap");
        let (window, tab) = bootstrap_ids(&app);
        commit_location(&mut app, window, tab, "https://committed.example/");

        let pending = app
            .begin_navigation(window, tab, "https://pending.example/")
            .expect("pending navigation")
            .intent()
            .id();

        let effect = app
            .dispatch_browser_command(window, BrowserCommand::ReloadOrStop)
            .expect("stop command");
        let BrowserCommandEffect::NavigationStopped(stopped) = effect else {
            panic!("loading reload/stop command should stop");
        };

        assert_eq!(stopped.id(), pending);
        let controls = app
            .window(window)
            .and_then(|browser_window| browser_window.tab(tab))
            .expect("tab")
            .navigation()
            .controls();
        assert_eq!(controls.reload(), ReloadControl::Reload);
    }

    #[test]
    fn reload_or_stop_reloads_committed_document_when_idle() {
        let mut app = BrowserApp::bootstrap().expect("bootstrap");
        let (window, tab) = bootstrap_ids(&app);
        commit_location(&mut app, window, tab, "https://committed.example/");

        let effect = app
            .dispatch_browser_command(window, BrowserCommand::ReloadOrStop)
            .expect("reload command");
        let BrowserCommandEffect::NavigationStarted(start) = effect else {
            panic!("idle reload/stop command should reload");
        };

        assert!(matches!(
            start.intent().kind(),
            NavigationIntentKind::Reload { .. }
        ));
        assert_eq!(
            start.intent().requested_location(),
            "https://committed.example/"
        );
    }

    #[test]
    fn repeated_cycle_commands_use_pending_target_as_the_next_anchor() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let second = app.create_tab(window).expect("second");
        let third = app.create_tab(window).expect("third");

        let first_effect = app
            .dispatch_browser_command(window, BrowserCommand::CycleTab(TabCycleDirection::Next))
            .expect("first cycle");
        let BrowserCommandEffect::TabActivationStarted(first_start) = first_effect else {
            panic!("first cycle should start activation");
        };
        assert_eq!(first_start.intent().from(), first);
        assert_eq!(first_start.intent().to(), second);

        let second_effect = app
            .dispatch_browser_command(window, BrowserCommand::CycleTab(TabCycleDirection::Next))
            .expect("second cycle");
        let BrowserCommandEffect::TabActivationStarted(second_start) = second_effect else {
            panic!("second cycle should supersede activation");
        };
        assert_eq!(second_start.intent().from(), first);
        assert_eq!(second_start.intent().to(), third);
        assert_eq!(second_start.superseded(), Some(first_start.intent()));
    }

    #[test]
    fn focus_address_bar_uses_committed_active_tab() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let first = app.create_tab(window).expect("first");
        let second = app.create_tab(window).expect("second");
        commit_location(&mut app, window, first, "https://first.example/");
        commit_location(&mut app, window, second, "https://second.example/");

        app.begin_tab_activation(window, second)
            .expect("pending activation");

        let effect = app
            .dispatch_browser_command(window, BrowserCommand::FocusAddressBar)
            .expect("focus address bar");

        assert_eq!(effect, BrowserCommandEffect::AddressBarEditStarted(first));
        let browser_window = app.window(window).expect("window");
        assert_eq!(browser_window.address_bar().editing_tab(), Some(first));
        assert_eq!(browser_window.address_bar_text(), "https://first.example/");
    }

    #[test]
    fn unavailable_history_command_is_a_noop_effect() {
        let mut app = BrowserApp::bootstrap().expect("bootstrap");
        let (window, _) = bootstrap_ids(&app);

        assert_eq!(
            app.dispatch_browser_command(window, BrowserCommand::Back)
                .expect("back command"),
            BrowserCommandEffect::Unavailable
        );
    }
}
