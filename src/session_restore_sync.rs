use crate::app::{BrowserApp, BrowserModelError, BrowserWindowId, TabId};
use crate::profile_runtime::{ProfileId, ProfileRuntime, ProfileSessionRestoreRuntimeError};
use crate::session_restore::{
    SessionRestoreError, SessionRestoreSnapshot, SessionTabId, SessionWindowId,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileSessionRestoreSyncOutcome {
    changed: bool,
    windows: usize,
    tabs: usize,
}

impl ProfileSessionRestoreSyncOutcome {
    pub const fn changed(self) -> bool {
        self.changed
    }

    pub const fn windows(self) -> usize {
        self.windows
    }

    pub const fn tabs(self) -> usize {
        self.tabs
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileSessionRestoreSyncError {
    Runtime(ProfileSessionRestoreRuntimeError),
    SessionRestore(SessionRestoreError),
    BrowserModel(BrowserModelError),
    NonEmptySnapshot { windows: usize, tabs: usize },
    BindingsOutOfSync,
}

impl fmt::Display for ProfileSessionRestoreSyncError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Runtime(error) => error.fmt(formatter),
            Self::SessionRestore(error) => error.fmt(formatter),
            Self::BrowserModel(error) => error.fmt(formatter),
            Self::NonEmptySnapshot { windows, tabs } => write!(
                formatter,
                "cannot attach an empty browser-session binding set to a persisted session containing {windows} windows and {tabs} tabs"
            ),
            Self::BindingsOutOfSync => formatter.write_str(
                "browser-session bindings no longer match the active session-restore snapshot",
            ),
        }
    }
}

impl std::error::Error for ProfileSessionRestoreSyncError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Runtime(error) => Some(error),
            Self::SessionRestore(error) => Some(error),
            Self::BrowserModel(error) => Some(error),
            Self::NonEmptySnapshot { .. } | Self::BindingsOutOfSync => None,
        }
    }
}

impl From<ProfileSessionRestoreRuntimeError> for ProfileSessionRestoreSyncError {
    fn from(error: ProfileSessionRestoreRuntimeError) -> Self {
        Self::Runtime(error)
    }
}

impl From<SessionRestoreError> for ProfileSessionRestoreSyncError {
    fn from(error: SessionRestoreError) -> Self {
        Self::SessionRestore(error)
    }
}

impl From<BrowserModelError> for ProfileSessionRestoreSyncError {
    fn from(error: BrowserModelError) -> Self {
        Self::BrowserModel(error)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileSessionRestoreSync {
    profile: ProfileId,
    windows: BTreeMap<BrowserWindowId, SessionWindowId>,
    tabs: BTreeMap<(BrowserWindowId, TabId), SessionTabId>,
}

#[derive(Debug)]
pub struct ProfileSessionRestoreStartup {
    browser: BrowserApp,
    sync: ProfileSessionRestoreSync,
    restored: bool,
}

impl ProfileSessionRestoreStartup {
    pub const fn browser(&self) -> &BrowserApp {
        &self.browser
    }

    pub fn browser_mut(&mut self) -> &mut BrowserApp {
        &mut self.browser
    }

    pub const fn sync(&self) -> &ProfileSessionRestoreSync {
        &self.sync
    }

    pub fn sync_mut(&mut self) -> &mut ProfileSessionRestoreSync {
        &mut self.sync
    }

    pub const fn restored(&self) -> bool {
        self.restored
    }

    pub fn into_parts(self) -> (BrowserApp, ProfileSessionRestoreSync) {
        (self.browser, self.sync)
    }
}

impl ProfileSessionRestoreSync {
    pub fn attach_empty(
        runtime: &ProfileRuntime,
        profile: ProfileId,
    ) -> Result<Self, ProfileSessionRestoreSyncError> {
        let snapshot = runtime.active_session_restore(profile)?;
        if !snapshot.is_empty() {
            return Err(ProfileSessionRestoreSyncError::NonEmptySnapshot {
                windows: snapshot.len(),
                tabs: snapshot.total_tabs(),
            });
        }
        Ok(Self {
            profile,
            windows: BTreeMap::new(),
            tabs: BTreeMap::new(),
        })
    }

    pub fn restore_browser(
        runtime: &ProfileRuntime,
        profile: ProfileId,
    ) -> Result<ProfileSessionRestoreStartup, ProfileSessionRestoreSyncError> {
        let snapshot = runtime.active_session_restore(profile)?.clone();
        if snapshot.is_empty() {
            return Ok(ProfileSessionRestoreStartup {
                browser: BrowserApp::bootstrap()?,
                sync: Self {
                    profile,
                    windows: BTreeMap::new(),
                    tabs: BTreeMap::new(),
                },
                restored: false,
            });
        }

        let mut browser = BrowserApp::new();
        let mut windows = BTreeMap::new();
        let mut tabs = BTreeMap::new();

        for session_window in snapshot.windows() {
            let browser_window = browser.create_window()?;
            windows.insert(browser_window, session_window.id());
            let mut active_browser_tab = None;

            for session_tab in session_window.tabs() {
                let browser_tab = browser.create_tab(browser_window)?;
                let navigation = browser
                    .begin_navigation(browser_window, browser_tab, session_tab.location())?
                    .intent()
                    .id();
                browser.commit_navigation(
                    browser_window,
                    browser_tab,
                    navigation,
                    session_tab.location(),
                )?;
                tabs.insert((browser_window, browser_tab), session_tab.id());
                if session_window.active_tab() == Some(session_tab.id()) {
                    active_browser_tab = Some(browser_tab);
                }
            }

            if let Some(active_browser_tab) = active_browser_tab {
                browser.set_active_tab(browser_window, active_browser_tab)?;
            }
        }

        let sync = Self {
            profile,
            windows,
            tabs,
        };
        debug_assert!(
            sync.bindings_match_snapshot(&snapshot),
            "fresh restore bindings must exactly cover the persisted snapshot"
        );
        Ok(ProfileSessionRestoreStartup {
            browser,
            sync,
            restored: true,
        })
    }

    pub const fn profile(&self) -> ProfileId {
        self.profile
    }

    pub fn session_window(&self, browser_window: BrowserWindowId) -> Option<SessionWindowId> {
        self.windows.get(&browser_window).copied()
    }

    pub fn session_tab(
        &self,
        browser_window: BrowserWindowId,
        browser_tab: TabId,
    ) -> Option<SessionTabId> {
        self.tabs.get(&(browser_window, browser_tab)).copied()
    }

    pub fn synchronize(
        &mut self,
        runtime: &mut ProfileRuntime,
        browser: &BrowserApp,
    ) -> Result<ProfileSessionRestoreSyncOutcome, ProfileSessionRestoreSyncError> {
        let current = runtime.active_session_restore(self.profile)?.clone();
        if !self.bindings_match_snapshot(&current) {
            return Err(ProfileSessionRestoreSyncError::BindingsOutOfSync);
        }

        let mut candidate = current.clone();
        let mut next_windows = self.windows.clone();
        let mut next_tabs = self.tabs.clone();
        let live_windows = browser
            .windows()
            .map(|window| window.id())
            .collect::<BTreeSet<_>>();

        let closed_windows = next_windows
            .keys()
            .copied()
            .filter(|window| !live_windows.contains(window))
            .collect::<Vec<_>>();
        for browser_window in closed_windows {
            let session_window = next_windows
                .remove(&browser_window)
                .expect("closed browser window was collected from bindings");
            let removed = candidate.remove_window(session_window);
            debug_assert!(
                removed.is_some(),
                "bindings were validated before synchronization"
            );
            next_tabs.retain(|(window, _), _| *window != browser_window);
        }

        for browser_window in browser.windows() {
            let browser_window_id = browser_window.id();
            let committed_tabs = browser_window
                .tabs()
                .iter()
                .filter_map(|tab| {
                    tab.navigation()
                        .current_entry()
                        .map(|entry| (tab.id(), entry.location()))
                })
                .collect::<Vec<_>>();

            if committed_tabs.is_empty() && !next_windows.contains_key(&browser_window_id) {
                continue;
            }

            let session_window = match next_windows.get(&browser_window_id).copied() {
                Some(window) => window,
                None => {
                    let window = candidate.add_window()?;
                    next_windows.insert(browser_window_id, window);
                    window
                }
            };

            let committed_ids = committed_tabs
                .iter()
                .map(|(tab, _)| *tab)
                .collect::<BTreeSet<_>>();
            let stale_tabs = next_tabs
                .iter()
                .filter_map(|((window, tab), session_tab)| {
                    (*window == browser_window_id && !committed_ids.contains(tab))
                        .then_some((*tab, *session_tab))
                })
                .collect::<Vec<_>>();
            for (browser_tab, session_tab) in stale_tabs {
                let removed = candidate.remove_tab(session_window, session_tab)?;
                debug_assert!(
                    removed.is_some(),
                    "bindings were validated before synchronization"
                );
                next_tabs.remove(&(browser_window_id, browser_tab));
            }

            let mut desired_order = Vec::with_capacity(committed_tabs.len());
            for (browser_tab, location) in committed_tabs {
                let session_tab = match next_tabs.get(&(browser_window_id, browser_tab)).copied() {
                    Some(tab) => {
                        candidate.update_tab_location(session_window, tab, location.to_owned())?;
                        tab
                    }
                    None => {
                        let tab = candidate.add_tab(session_window, location.to_owned())?;
                        next_tabs.insert((browser_window_id, browser_tab), tab);
                        tab
                    }
                };
                desired_order.push(session_tab);
            }

            for (index, desired) in desired_order.iter().copied().enumerate() {
                let current_at_index = candidate
                    .window(session_window)
                    .expect("session window exists during synchronization")
                    .tabs()[index]
                    .id();
                if current_at_index != desired {
                    candidate.move_tab_before(session_window, desired, Some(current_at_index))?;
                }
            }

            if let Some(active_browser_tab) = browser_window.active_tab_id()
                && let Some(active_session_tab) = next_tabs
                    .get(&(browser_window_id, active_browser_tab))
                    .copied()
            {
                candidate.set_active_tab(session_window, active_session_tab)?;
            }
        }

        let changed = runtime.replace_session_restore_snapshot(self.profile, candidate)?;
        self.windows = next_windows;
        self.tabs = next_tabs;
        let snapshot = runtime.active_session_restore(self.profile)?;
        Ok(ProfileSessionRestoreSyncOutcome {
            changed,
            windows: snapshot.len(),
            tabs: snapshot.total_tabs(),
        })
    }

    fn bindings_match_snapshot(&self, snapshot: &SessionRestoreSnapshot) -> bool {
        if snapshot.len() != self.windows.len() {
            return false;
        }
        let bound_windows = self.windows.values().copied().collect::<BTreeSet<_>>();
        let snapshot_windows = snapshot
            .windows()
            .iter()
            .map(|window| window.id())
            .collect::<BTreeSet<_>>();
        if bound_windows != snapshot_windows {
            return false;
        }

        for (browser_window, session_window) in &self.windows {
            let Some(snapshot_window) = snapshot.window(*session_window) else {
                return false;
            };
            let bound_tabs = self
                .tabs
                .iter()
                .filter_map(|((window, _), session_tab)| {
                    (*window == *browser_window).then_some(*session_tab)
                })
                .collect::<BTreeSet<_>>();
            let snapshot_tabs = snapshot_window
                .tabs()
                .iter()
                .map(|tab| tab.id())
                .collect::<BTreeSet<_>>();
            if bound_tabs != snapshot_tabs {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile_runtime::PreparedProfile;
    use crate::session_restore::MAX_SESSION_LOCATION_BYTES;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(1);

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let id = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "zorya-session-restore-browser-sync-{}-{id}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            Self(root)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn load_profile(runtime: &mut ProfileRuntime, root: &Path) -> ProfileId {
        let intent = runtime.begin_selection(root).unwrap().into_intent();
        let prepared = PreparedProfile::load(&intent).unwrap();
        runtime.commit_selection(prepared).unwrap().active_profile()
    }

    fn bootstrap_ids(browser: &BrowserApp) -> (BrowserWindowId, TabId) {
        let window = browser.windows().next().unwrap();
        (window.id(), window.active_tab_id().unwrap())
    }

    fn commit_location(
        browser: &mut BrowserApp,
        window: BrowserWindowId,
        tab: TabId,
        location: &str,
    ) {
        let navigation = browser
            .begin_navigation(window, tab, location)
            .unwrap()
            .intent()
            .id();
        browser
            .commit_navigation(window, tab, navigation, location)
            .unwrap();
    }

    #[test]
    fn empty_snapshot_bootstraps_without_mutating_runtime() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let before_snapshot = runtime.active_session_restore(profile).unwrap().clone();
        let before_revision = runtime.session_restore_mutation_revision(profile).unwrap();
        let before_unsaved = runtime.session_restore_unsaved_mutations(profile).unwrap();

        let startup = ProfileSessionRestoreSync::restore_browser(&runtime, profile).unwrap();

        assert!(!startup.restored());
        let windows = startup.browser().windows().collect::<Vec<_>>();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].tabs().len(), 1);
        assert_eq!(windows[0].active_tab_id(), Some(windows[0].tabs()[0].id()));
        assert!(windows[0].tabs()[0].navigation().current_entry().is_none());
        assert!(windows[0].tabs()[0].navigation().pending().is_none());
        assert!(windows[0].address_bar().edit().is_none());
        assert_eq!(startup.sync().session_window(windows[0].id()), None);
        assert_eq!(
            startup
                .sync()
                .session_tab(windows[0].id(), windows[0].tabs()[0].id()),
            None
        );
        assert_eq!(
            runtime.active_session_restore(profile).unwrap(),
            &before_snapshot
        );
        assert_eq!(
            runtime.session_restore_mutation_revision(profile).unwrap(),
            before_revision
        );
        assert_eq!(
            runtime.session_restore_unsaved_mutations(profile).unwrap(),
            before_unsaved
        );
    }

    #[test]
    fn restore_preserves_order_locations_active_tabs_empty_windows_and_exact_bindings() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let first_window = runtime.add_session_window(profile).unwrap();
        let first_tab = runtime
            .add_session_tab(profile, first_window, "https://first.example/")
            .unwrap();
        let second_tab = runtime
            .add_session_tab(profile, first_window, "https://second.example/")
            .unwrap();
        runtime
            .set_active_session_tab(profile, first_window, second_tab)
            .unwrap();
        let empty_window = runtime.add_session_window(profile).unwrap();
        let third_window = runtime.add_session_window(profile).unwrap();
        let third_tab = runtime
            .add_session_tab(profile, third_window, "https://third.example/path")
            .unwrap();
        let before_snapshot = runtime.active_session_restore(profile).unwrap().clone();
        let before_revision = runtime.session_restore_mutation_revision(profile).unwrap();
        let before_unsaved = runtime.session_restore_unsaved_mutations(profile).unwrap();

        let startup = ProfileSessionRestoreSync::restore_browser(&runtime, profile).unwrap();

        assert!(startup.restored());
        let windows = startup.browser().windows().collect::<Vec<_>>();
        assert_eq!(windows.len(), 3);
        assert_eq!(
            startup.sync().session_window(windows[0].id()),
            Some(first_window)
        );
        assert_eq!(
            startup.sync().session_window(windows[1].id()),
            Some(empty_window)
        );
        assert_eq!(
            startup.sync().session_window(windows[2].id()),
            Some(third_window)
        );

        assert_eq!(windows[0].tabs().len(), 2);
        assert_eq!(
            startup
                .sync()
                .session_tab(windows[0].id(), windows[0].tabs()[0].id()),
            Some(first_tab)
        );
        assert_eq!(
            startup
                .sync()
                .session_tab(windows[0].id(), windows[0].tabs()[1].id()),
            Some(second_tab)
        );
        assert_eq!(
            windows[0].tabs()[0]
                .navigation()
                .current_entry()
                .unwrap()
                .location(),
            "https://first.example/"
        );
        assert_eq!(
            windows[0].tabs()[1]
                .navigation()
                .current_entry()
                .unwrap()
                .location(),
            "https://second.example/"
        );
        assert_eq!(windows[0].active_tab_id(), Some(windows[0].tabs()[1].id()));
        for tab in windows[0].tabs() {
            assert!(tab.navigation().pending().is_none());
        }
        assert!(windows[0].address_bar().edit().is_none());

        assert!(windows[1].tabs().is_empty());
        assert_eq!(windows[1].active_tab_id(), None);
        assert!(windows[1].address_bar().edit().is_none());

        assert_eq!(windows[2].tabs().len(), 1);
        assert_eq!(
            startup
                .sync()
                .session_tab(windows[2].id(), windows[2].tabs()[0].id()),
            Some(third_tab)
        );
        assert_eq!(
            windows[2].tabs()[0]
                .navigation()
                .current_entry()
                .unwrap()
                .location(),
            "https://third.example/path"
        );
        assert_eq!(windows[2].active_tab_id(), Some(windows[2].tabs()[0].id()));
        assert!(windows[2].tabs()[0].navigation().pending().is_none());
        assert!(windows[2].address_bar().edit().is_none());

        assert_eq!(
            runtime.active_session_restore(profile).unwrap(),
            &before_snapshot
        );
        assert_eq!(
            runtime.session_restore_mutation_revision(profile).unwrap(),
            before_revision
        );
        assert_eq!(
            runtime.session_restore_unsaved_mutations(profile).unwrap(),
            before_unsaved
        );
    }

    #[test]
    fn first_sync_after_restore_is_no_op_and_later_navigation_keeps_session_tab_identity() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let session_window = runtime.add_session_window(profile).unwrap();
        let session_tab = runtime
            .add_session_tab(profile, session_window, "https://stable.example/")
            .unwrap();
        let before_revision = runtime.session_restore_mutation_revision(profile).unwrap();
        let startup = ProfileSessionRestoreSync::restore_browser(&runtime, profile).unwrap();
        let (mut browser, mut sync) = startup.into_parts();
        let browser_window = browser.windows().next().unwrap().id();
        let browser_tab = browser.window(browser_window).unwrap().tabs()[0].id();

        let first = sync.synchronize(&mut runtime, &browser).unwrap();
        assert!(!first.changed());
        assert_eq!(
            runtime.session_restore_mutation_revision(profile).unwrap(),
            before_revision
        );
        assert_eq!(sync.session_window(browser_window), Some(session_window));
        assert_eq!(
            sync.session_tab(browser_window, browser_tab),
            Some(session_tab)
        );

        commit_location(
            &mut browser,
            browser_window,
            browser_tab,
            "https://changed.example/final",
        );
        let second = sync.synchronize(&mut runtime, &browser).unwrap();
        assert!(second.changed());
        assert_eq!(
            sync.session_tab(browser_window, browser_tab),
            Some(session_tab)
        );
        assert_eq!(
            runtime.session_restore_mutation_revision(profile).unwrap(),
            before_revision + 1
        );
        assert_eq!(
            runtime
                .active_session_restore(profile)
                .unwrap()
                .window(session_window)
                .unwrap()
                .tabs()[0]
                .location(),
            "https://changed.example/final"
        );
    }

    #[test]
    fn restore_rejects_stale_profile_before_exposing_startup() {
        let root = TempRoot::new();
        let mut source_runtime = ProfileRuntime::new();
        let stale = load_profile(&mut source_runtime, root.path());
        let runtime = ProfileRuntime::new();

        assert!(matches!(
            ProfileSessionRestoreSync::restore_browser(&runtime, stale),
            Err(ProfileSessionRestoreSyncError::Runtime(
                ProfileSessionRestoreRuntimeError::StaleProfile { expected: None, actual }
            )) if actual == stale
        ));
    }

    #[test]
    fn attach_rejects_non_empty_persisted_state() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        runtime.add_session_window(profile).unwrap();

        assert_eq!(
            ProfileSessionRestoreSync::attach_empty(&runtime, profile),
            Err(ProfileSessionRestoreSyncError::NonEmptySnapshot {
                windows: 1,
                tabs: 0,
            })
        );
    }

    #[test]
    fn uncommitted_browser_state_is_a_no_op() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let browser = BrowserApp::bootstrap().unwrap();
        let mut sync = ProfileSessionRestoreSync::attach_empty(&runtime, profile).unwrap();

        let outcome = sync.synchronize(&mut runtime, &browser).unwrap();

        assert!(!outcome.changed());
        assert_eq!(outcome.windows(), 0);
        assert_eq!(outcome.tabs(), 0);
        assert_eq!(
            runtime.session_restore_mutation_revision(profile).unwrap(),
            0
        );
    }

    #[test]
    fn committed_location_is_bound_once_and_pending_or_edit_text_is_ignored() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut browser = BrowserApp::bootstrap().unwrap();
        let (window, tab) = bootstrap_ids(&browser);
        let mut sync = ProfileSessionRestoreSync::attach_empty(&runtime, profile).unwrap();

        commit_location(&mut browser, window, tab, "https://a.example/");
        assert!(sync.synchronize(&mut runtime, &browser).unwrap().changed());
        let session_window = sync.session_window(window).unwrap();
        let session_tab = sync.session_tab(window, tab).unwrap();
        assert_eq!(
            runtime
                .active_session_restore(profile)
                .unwrap()
                .window(session_window)
                .unwrap()
                .tabs()[0]
                .location(),
            "https://a.example/"
        );
        assert_eq!(
            runtime.session_restore_mutation_revision(profile).unwrap(),
            1
        );

        browser.begin_address_bar_edit(window).unwrap();
        browser
            .set_address_bar_text(window, "https://edit.example/")
            .unwrap();
        let pending = browser
            .begin_navigation(window, tab, "https://pending.example/")
            .unwrap()
            .intent()
            .id();
        assert!(!sync.synchronize(&mut runtime, &browser).unwrap().changed());
        assert_eq!(
            runtime.session_restore_mutation_revision(profile).unwrap(),
            1
        );
        assert_eq!(sync.session_tab(window, tab), Some(session_tab));

        browser
            .commit_navigation(window, tab, pending, "https://committed.example/")
            .unwrap();
        assert!(sync.synchronize(&mut runtime, &browser).unwrap().changed());
        assert_eq!(sync.session_tab(window, tab), Some(session_tab));
        assert_eq!(
            runtime.session_restore_mutation_revision(profile).unwrap(),
            2
        );
        assert_eq!(
            runtime
                .active_session_restore(profile)
                .unwrap()
                .window(session_window)
                .unwrap()
                .tabs()[0]
                .location(),
            "https://committed.example/"
        );
    }

    #[test]
    fn reorder_activation_and_close_preserve_surviving_session_identity() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut browser = BrowserApp::bootstrap().unwrap();
        let (window, first) = bootstrap_ids(&browser);
        commit_location(&mut browser, window, first, "https://first.example/");
        let second = browser.create_tab(window).unwrap();
        commit_location(&mut browser, window, second, "https://second.example/");
        let mut sync = ProfileSessionRestoreSync::attach_empty(&runtime, profile).unwrap();
        assert!(sync.synchronize(&mut runtime, &browser).unwrap().changed());
        let session_window = sync.session_window(window).unwrap();
        let first_session = sync.session_tab(window, first).unwrap();
        let second_session = sync.session_tab(window, second).unwrap();

        browser
            .move_tab_before(window, second, Some(first))
            .unwrap();
        browser.set_active_tab(window, second).unwrap();
        assert!(sync.synchronize(&mut runtime, &browser).unwrap().changed());
        let persisted = runtime
            .active_session_restore(profile)
            .unwrap()
            .window(session_window)
            .unwrap();
        assert_eq!(
            persisted
                .tabs()
                .iter()
                .map(|tab| tab.id())
                .collect::<Vec<_>>(),
            vec![second_session, first_session]
        );
        assert_eq!(persisted.active_tab(), Some(second_session));
        assert_eq!(
            runtime.session_restore_mutation_revision(profile).unwrap(),
            2
        );

        browser.close_tab(window, first).unwrap();
        assert!(sync.synchronize(&mut runtime, &browser).unwrap().changed());
        let persisted = runtime
            .active_session_restore(profile)
            .unwrap()
            .window(session_window)
            .unwrap();
        assert_eq!(persisted.tabs().len(), 1);
        assert_eq!(persisted.tabs()[0].id(), second_session);
        assert_eq!(persisted.active_tab(), Some(second_session));
        assert_eq!(sync.session_tab(window, first), None);
        assert_eq!(sync.session_tab(window, second), Some(second_session));
        assert_eq!(
            runtime.session_restore_mutation_revision(profile).unwrap(),
            3
        );
    }

    #[test]
    fn validation_failure_is_atomic_for_snapshot_revision_and_bindings() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut browser = BrowserApp::bootstrap().unwrap();
        let (window, first) = bootstrap_ids(&browser);
        commit_location(&mut browser, window, first, "https://stable.example/");
        let mut sync = ProfileSessionRestoreSync::attach_empty(&runtime, profile).unwrap();
        sync.synchronize(&mut runtime, &browser).unwrap();

        let before_snapshot = runtime.active_session_restore(profile).unwrap().clone();
        let before_revision = runtime.session_restore_mutation_revision(profile).unwrap();
        let before_sync = sync.clone();
        let invalid = browser.create_tab(window).unwrap();
        let too_large = "x".repeat(MAX_SESSION_LOCATION_BYTES + 1);
        commit_location(&mut browser, window, invalid, &too_large);

        assert!(matches!(
            sync.synchronize(&mut runtime, &browser),
            Err(ProfileSessionRestoreSyncError::SessionRestore(
                SessionRestoreError::LocationTooLarge { .. }
            ))
        ));
        assert_eq!(
            runtime.active_session_restore(profile).unwrap(),
            &before_snapshot
        );
        assert_eq!(
            runtime.session_restore_mutation_revision(profile).unwrap(),
            before_revision
        );
        assert_eq!(sync, before_sync);
    }

    #[test]
    fn stale_profile_and_external_session_mutation_fail_closed() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut browser = BrowserApp::bootstrap().unwrap();
        let (window, tab) = bootstrap_ids(&browser);
        commit_location(&mut browser, window, tab, "https://stable.example/");
        let mut sync = ProfileSessionRestoreSync::attach_empty(&runtime, profile).unwrap();
        sync.synchronize(&mut runtime, &browser).unwrap();

        runtime.add_session_window(profile).unwrap();
        assert_eq!(
            sync.synchronize(&mut runtime, &browser),
            Err(ProfileSessionRestoreSyncError::BindingsOutOfSync)
        );

        let mut no_profile = ProfileRuntime::new();
        assert!(matches!(
            sync.synchronize(&mut no_profile, &browser),
            Err(ProfileSessionRestoreSyncError::Runtime(
                ProfileSessionRestoreRuntimeError::StaleProfile { actual, .. }
            )) if actual == profile
        ));
    }
}
