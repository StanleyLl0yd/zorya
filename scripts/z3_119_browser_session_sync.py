from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    text = target.read_text(encoding="utf-8")
    if old not in text:
        raise SystemExit(f"anchor not found in {path}: {old[:120]!r}")
    if text.count(old) != 1:
        raise SystemExit(f"anchor is not unique in {path}: {old[:120]!r}")
    target.write_text(text.replace(old, new, 1), encoding="utf-8")


module = r'''use crate::app::{BrowserApp, BrowserWindowId, TabId};
use crate::profile_runtime::{
    ProfileId, ProfileRuntime, ProfileSessionRestoreRuntimeError,
};
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
    NonEmptySnapshot {
        windows: usize,
        tabs: usize,
    },
    BindingsOutOfSync,
}

impl fmt::Display for ProfileSessionRestoreSyncError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Runtime(error) => error.fmt(formatter),
            Self::SessionRestore(error) => error.fmt(formatter),
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileSessionRestoreSync {
    profile: ProfileId,
    windows: BTreeMap<BrowserWindowId, SessionWindowId>,
    tabs: BTreeMap<(BrowserWindowId, TabId), SessionTabId>,
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
            debug_assert!(removed.is_some(), "bindings were validated before synchronization");
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
                debug_assert!(removed.is_some(), "bindings were validated before synchronization");
                next_tabs.remove(&(browser_window_id, browser_tab));
            }

            let mut desired_order = Vec::with_capacity(committed_tabs.len());
            for (browser_tab, location) in committed_tabs {
                let session_tab = match next_tabs
                    .get(&(browser_window_id, browser_tab))
                    .copied()
                {
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
        assert_eq!(runtime.session_restore_mutation_revision(profile).unwrap(), 0);
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
        assert_eq!(runtime.session_restore_mutation_revision(profile).unwrap(), 1);

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
        assert_eq!(runtime.session_restore_mutation_revision(profile).unwrap(), 1);
        assert_eq!(sync.session_tab(window, tab), Some(session_tab));

        browser
            .commit_navigation(window, tab, pending, "https://committed.example/")
            .unwrap();
        assert!(sync.synchronize(&mut runtime, &browser).unwrap().changed());
        assert_eq!(sync.session_tab(window, tab), Some(session_tab));
        assert_eq!(runtime.session_restore_mutation_revision(profile).unwrap(), 2);
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

        browser.move_tab_before(window, second, Some(first)).unwrap();
        browser.set_active_tab(window, second).unwrap();
        assert!(sync.synchronize(&mut runtime, &browser).unwrap().changed());
        let persisted = runtime
            .active_session_restore(profile)
            .unwrap()
            .window(session_window)
            .unwrap();
        assert_eq!(
            persisted.tabs().iter().map(|tab| tab.id()).collect::<Vec<_>>(),
            vec![second_session, first_session]
        );
        assert_eq!(persisted.active_tab(), Some(second_session));
        assert_eq!(runtime.session_restore_mutation_revision(profile).unwrap(), 2);

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
        assert_eq!(runtime.session_restore_mutation_revision(profile).unwrap(), 3);
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
        assert_eq!(runtime.active_session_restore(profile).unwrap(), &before_snapshot);
        assert_eq!(runtime.session_restore_mutation_revision(profile).unwrap(), before_revision);
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
'''

new_module = Path("src/session_restore_sync.rs")
if new_module.exists():
    raise SystemExit("src/session_restore_sync.rs already exists")
new_module.write_text(module, encoding="utf-8")

replace_once(
    "src/lib.rs",
    "mod session_restore;\nmod tab_activation;",
    "mod session_restore;\nmod session_restore_sync;\nmod tab_activation;",
)
replace_once(
    "src/lib.rs",
    "pub use session_restore::{\n    MAX_SESSION_LOCATION_BYTES, MAX_SESSION_RESTORE_RECORD_BYTES, MAX_SESSION_TABS,\n    MAX_SESSION_TABS_PER_WINDOW, MAX_SESSION_WINDOWS, SESSION_RESTORE_SCHEMA_VERSION,\n    SessionRestoreCleanupWarning, SessionRestoreError, SessionRestoreLoad, SessionRestoreRecovery,\n    SessionRestoreSave, SessionRestoreSnapshot, SessionRestoreStore, SessionTab, SessionTabId,\n    SessionWindow, SessionWindowId,\n};\n",
    "pub use session_restore::{\n    MAX_SESSION_LOCATION_BYTES, MAX_SESSION_RESTORE_RECORD_BYTES, MAX_SESSION_TABS,\n    MAX_SESSION_TABS_PER_WINDOW, MAX_SESSION_WINDOWS, SESSION_RESTORE_SCHEMA_VERSION,\n    SessionRestoreCleanupWarning, SessionRestoreError, SessionRestoreLoad, SessionRestoreRecovery,\n    SessionRestoreSave, SessionRestoreSnapshot, SessionRestoreStore, SessionTab, SessionTabId,\n    SessionWindow, SessionWindowId,\n};\npub use session_restore_sync::{\n    ProfileSessionRestoreSync, ProfileSessionRestoreSyncError, ProfileSessionRestoreSyncOutcome,\n};\n",
)

move_method = r'''    pub fn move_tab_before(
        &mut self,
        window: SessionWindowId,
        tab: SessionTabId,
        before: Option<SessionTabId>,
    ) -> Result<bool, SessionRestoreError> {
        let target = self
            .windows
            .iter_mut()
            .find(|candidate| candidate.id == window)
            .ok_or(SessionRestoreError::WindowNotFound { window })?;
        let from = target
            .tabs
            .iter()
            .position(|candidate| candidate.id == tab)
            .ok_or(SessionRestoreError::TabNotFound { window, tab })?;
        if before == Some(tab) {
            return Ok(false);
        }
        let anchor = before
            .map(|anchor| {
                target
                    .tabs
                    .iter()
                    .position(|candidate| candidate.id == anchor)
                    .ok_or(SessionRestoreError::TabNotFound {
                        window,
                        tab: anchor,
                    })
            })
            .transpose()?;
        let already_positioned = match anchor {
            Some(anchor) => from.checked_add(1) == Some(anchor),
            None => from.checked_add(1) == Some(target.tabs.len()),
        };
        if already_positioned {
            return Ok(false);
        }

        let moved = target.tabs.remove(from);
        let destination = match before {
            Some(anchor) => target
                .tabs
                .iter()
                .position(|candidate| candidate.id == anchor)
                .expect("tab-order anchor was validated before mutation"),
            None => target.tabs.len(),
        };
        target.tabs.insert(destination, moved);
        Ok(true)
    }

'''
replace_once(
    "src/session_restore.rs",
    "    pub fn remove_tab(\n        &mut self,\n        window: SessionWindowId,\n        tab: SessionTabId,\n    ) -> Result<Option<SessionTab>, SessionRestoreError> {",
    move_method + "    pub fn remove_tab(\n        &mut self,\n        window: SessionWindowId,\n        tab: SessionTabId,\n    ) -> Result<Option<SessionTab>, SessionRestoreError> {",
)

replace_once(
    "src/profile_runtime/session_restore_persistence.rs",
    "    MutationRevisionExhausted,\n    SaveIdExhausted,",
    "    MutationRevisionExhausted,\n    SnapshotGenerationMismatch {\n        expected_generation: u64,\n        provided_generation: u64,\n    },\n    SaveIdExhausted,",
)
replace_once(
    "src/profile_runtime/session_restore_persistence.rs",
    "            Self::MutationRevisionExhausted => {\n                formatter.write_str(\"profile session-restore mutation revision space is exhausted\")\n            }\n            Self::SaveIdExhausted => {",
    "            Self::MutationRevisionExhausted => {\n                formatter.write_str(\"profile session-restore mutation revision space is exhausted\")\n            }\n            Self::SnapshotGenerationMismatch {\n                expected_generation,\n                provided_generation,\n            } => write!(\n                formatter,\n                \"replacement session snapshot generation {provided_generation} does not match active generation {expected_generation}\"\n            ),\n            Self::SaveIdExhausted => {",
)
replace_once(
    "src/profile_runtime/session_restore_persistence.rs",
    "    pub fn session_restore_unsaved_mutations(\n        &self,\n        profile: ProfileId,\n    ) -> Result<u64, ProfileSessionRestoreRuntimeError> {",
    r'''    pub(crate) fn replace_session_restore_snapshot(
        &mut self,
        profile: ProfileId,
        snapshot: SessionRestoreSnapshot,
    ) -> Result<bool, ProfileSessionRestoreRuntimeError> {
        let state = self.active_session_restore_state_mut(profile)?;
        if snapshot.generation() != state.snapshot.generation() {
            return Err(ProfileSessionRestoreRuntimeError::SnapshotGenerationMismatch {
                expected_generation: state.snapshot.generation(),
                provided_generation: snapshot.generation(),
            });
        }
        if snapshot == state.snapshot {
            return Ok(false);
        }
        let next_revision = state
            .mutation_revision
            .checked_add(1)
            .ok_or(ProfileSessionRestoreRuntimeError::MutationRevisionExhausted)?;
        state.snapshot = snapshot;
        state.mutation_revision = next_revision;
        Ok(true)
    }

    pub fn session_restore_unsaved_mutations(
        &self,
        profile: ProfileId,
    ) -> Result<u64, ProfileSessionRestoreRuntimeError> {''',
)

replace_once(
    "docs/ARCHITECTURE.md",
    "Native browser-state synchronization and startup restoration remain later Z3 work.",
    "`ProfileSessionRestoreSync` now provides a portable exact-profile bridge from live `BrowserApp` state into that runtime snapshot. It binds runtime window/tab IDs only in memory to distinct persisted session IDs, reads only committed `HistoryEntry` locations, preserves persisted IDs across committed location changes, active-tab changes and reorder, prunes closed state, and stages the complete projection on cloned snapshot/bindings before one bulk runtime replacement so validation failure cannot partially mutate either side and a real synchronization advances at most one session mutation revision. Empty attachment is rejected when persisted session state already exists, preventing a fresh native `about:blank` model from overwriting an unrestored session; restored-binding attachment, Windows wiring and startup restoration remain later Z3 work.",
)
replace_once(
    "docs/ROADMAP.md",
    "native browser-state synchronization and startup restore remain later Z3 work;",
    "a portable failure-atomic live-browser synchronization layer now binds runtime window/tab IDs only in memory to stable persisted session IDs, projects only committed locations, mirrors committed tab order/activation/closure and advances at most one runtime revision per real synchronization while refusing empty attachment over an unrestored persisted session; native synchronization wiring and startup restore remain later Z3 work;",
)
