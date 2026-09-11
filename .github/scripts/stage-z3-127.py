from pathlib import Path

sync_path = Path("src/session_restore_sync.rs")
lib_path = Path("src/lib.rs")
architecture_path = Path("docs/ARCHITECTURE.md")
roadmap_path = Path("docs/ROADMAP.md")

sync = sync_path.read_text()

if "pub struct ProfileSessionRestoreStartup" not in sync:
    sync = sync.replace(
        "use crate::app::{BrowserApp, BrowserWindowId, TabId};",
        "use crate::app::{BrowserApp, BrowserModelError, BrowserWindowId, TabId};",
        1,
    )

    sync = sync.replace(
        "    Runtime(ProfileSessionRestoreRuntimeError),\n    SessionRestore(SessionRestoreError),\n",
        "    Runtime(ProfileSessionRestoreRuntimeError),\n    SessionRestore(SessionRestoreError),\n    BrowserModel(BrowserModelError),\n",
        1,
    )

    sync = sync.replace(
        "            Self::Runtime(error) => error.fmt(formatter),\n            Self::SessionRestore(error) => error.fmt(formatter),\n",
        "            Self::Runtime(error) => error.fmt(formatter),\n            Self::SessionRestore(error) => error.fmt(formatter),\n            Self::BrowserModel(error) => error.fmt(formatter),\n",
        1,
    )

    sync = sync.replace(
        "            Self::Runtime(error) => Some(error),\n            Self::SessionRestore(error) => Some(error),\n            Self::NonEmptySnapshot { .. } | Self::BindingsOutOfSync => None,\n",
        "            Self::Runtime(error) => Some(error),\n            Self::SessionRestore(error) => Some(error),\n            Self::BrowserModel(error) => Some(error),\n            Self::NonEmptySnapshot { .. } | Self::BindingsOutOfSync => None,\n",
        1,
    )

    from_marker = "impl From<SessionRestoreError> for ProfileSessionRestoreSyncError {\n    fn from(error: SessionRestoreError) -> Self {\n        Self::SessionRestore(error)\n    }\n}\n"
    from_insert = from_marker + "\nimpl From<BrowserModelError> for ProfileSessionRestoreSyncError {\n    fn from(error: BrowserModelError) -> Self {\n        Self::BrowserModel(error)\n    }\n}\n"
    assert from_marker in sync
    sync = sync.replace(from_marker, from_insert, 1)

    struct_marker = "#[derive(Clone, Debug, PartialEq, Eq)]\npub struct ProfileSessionRestoreSync {\n    profile: ProfileId,\n    windows: BTreeMap<BrowserWindowId, SessionWindowId>,\n    tabs: BTreeMap<(BrowserWindowId, TabId), SessionTabId>,\n}\n"
    startup = struct_marker + "\n#[derive(Debug)]\npub struct ProfileSessionRestoreStartup {\n    browser: BrowserApp,\n    sync: ProfileSessionRestoreSync,\n    restored: bool,\n}\n\nimpl ProfileSessionRestoreStartup {\n    pub const fn browser(&self) -> &BrowserApp {\n        &self.browser\n    }\n\n    pub fn browser_mut(&mut self) -> &mut BrowserApp {\n        &mut self.browser\n    }\n\n    pub const fn sync(&self) -> &ProfileSessionRestoreSync {\n        &self.sync\n    }\n\n    pub fn sync_mut(&mut self) -> &mut ProfileSessionRestoreSync {\n        &mut self.sync\n    }\n\n    pub const fn restored(&self) -> bool {\n        self.restored\n    }\n\n    pub fn into_parts(self) -> (BrowserApp, ProfileSessionRestoreSync) {\n        (self.browser, self.sync)\n    }\n}\n"
    assert struct_marker in sync
    sync = sync.replace(struct_marker, startup, 1)

    attach_end = "        Ok(Self {\n            profile,\n            windows: BTreeMap::new(),\n            tabs: BTreeMap::new(),\n        })\n    }\n\n    pub const fn profile(&self) -> ProfileId {\n"
    restore = "        Ok(Self {\n            profile,\n            windows: BTreeMap::new(),\n            tabs: BTreeMap::new(),\n        })\n    }\n\n    pub fn restore_browser(\n        runtime: &ProfileRuntime,\n        profile: ProfileId,\n    ) -> Result<ProfileSessionRestoreStartup, ProfileSessionRestoreSyncError> {\n        let snapshot = runtime.active_session_restore(profile)?.clone();\n        if snapshot.is_empty() {\n            return Ok(ProfileSessionRestoreStartup {\n                browser: BrowserApp::bootstrap()?,\n                sync: Self {\n                    profile,\n                    windows: BTreeMap::new(),\n                    tabs: BTreeMap::new(),\n                },\n                restored: false,\n            });\n        }\n\n        let mut browser = BrowserApp::new();\n        let mut windows = BTreeMap::new();\n        let mut tabs = BTreeMap::new();\n\n        for session_window in snapshot.windows() {\n            let browser_window = browser.create_window()?;\n            windows.insert(browser_window, session_window.id());\n            let mut active_browser_tab = None;\n\n            for session_tab in session_window.tabs() {\n                let browser_tab = browser.create_tab(browser_window)?;\n                let navigation = browser\n                    .begin_navigation(browser_window, browser_tab, session_tab.location())?\n                    .intent()\n                    .id();\n                browser.commit_navigation(\n                    browser_window,\n                    browser_tab,\n                    navigation,\n                    session_tab.location(),\n                )?;\n                tabs.insert((browser_window, browser_tab), session_tab.id());\n                if session_window.active_tab() == Some(session_tab.id()) {\n                    active_browser_tab = Some(browser_tab);\n                }\n            }\n\n            if let Some(active_browser_tab) = active_browser_tab {\n                browser.set_active_tab(browser_window, active_browser_tab)?;\n            }\n        }\n\n        let sync = Self {\n            profile,\n            windows,\n            tabs,\n        };\n        debug_assert!(\n            sync.bindings_match_snapshot(&snapshot),\n            \"fresh restore bindings must exactly cover the persisted snapshot\"\n        );\n        Ok(ProfileSessionRestoreStartup {\n            browser,\n            sync,\n            restored: true,\n        })\n    }\n\n    pub const fn profile(&self) -> ProfileId {\n"
    assert attach_end in sync
    sync = sync.replace(attach_end, restore, 1)

    test_marker = "    #[test]\n    fn attach_rejects_non_empty_persisted_state() {\n"
    tests = r'''    #[test]
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
        assert_eq!(runtime.active_session_restore(profile).unwrap(), &before_snapshot);
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
        assert_eq!(startup.sync().session_window(windows[0].id()), Some(first_window));
        assert_eq!(startup.sync().session_window(windows[1].id()), Some(empty_window));
        assert_eq!(startup.sync().session_window(windows[2].id()), Some(third_window));

        assert_eq!(windows[0].tabs().len(), 2);
        assert_eq!(
            startup.sync().session_tab(windows[0].id(), windows[0].tabs()[0].id()),
            Some(first_tab)
        );
        assert_eq!(
            startup.sync().session_tab(windows[0].id(), windows[0].tabs()[1].id()),
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
            startup.sync().session_tab(windows[2].id(), windows[2].tabs()[0].id()),
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

        assert_eq!(runtime.active_session_restore(profile).unwrap(), &before_snapshot);
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
        let browser_tab = browser
            .window(browser_window)
            .unwrap()
            .tabs()[0]
            .id();

        let first = sync.synchronize(&mut runtime, &browser).unwrap();
        assert!(!first.changed());
        assert_eq!(
            runtime.session_restore_mutation_revision(profile).unwrap(),
            before_revision
        );
        assert_eq!(sync.session_window(browser_window), Some(session_window));
        assert_eq!(sync.session_tab(browser_window, browser_tab), Some(session_tab));

        commit_location(
            &mut browser,
            browser_window,
            browser_tab,
            "https://changed.example/final",
        );
        let second = sync.synchronize(&mut runtime, &browser).unwrap();
        assert!(second.changed());
        assert_eq!(sync.session_tab(browser_window, browser_tab), Some(session_tab));
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

'''
    assert test_marker in sync
    sync = sync.replace(test_marker, tests + test_marker, 1)

    sync_path.write_text(sync)

lib = lib_path.read_text()
if "ProfileSessionRestoreStartup" not in lib:
    old = "    ProfileSessionRestoreSync, ProfileSessionRestoreSyncError, ProfileSessionRestoreSyncOutcome,\n"
    new = "    ProfileSessionRestoreStartup, ProfileSessionRestoreSync, ProfileSessionRestoreSyncError,\n    ProfileSessionRestoreSyncOutcome,\n"
    assert old in lib
    lib_path.write_text(lib.replace(old, new, 1))

architecture = architecture_path.read_text()
heading = "### Profile session startup restoration"
if heading not in architecture:
    marker = "## Release packaging boundary\n"
    addition = """### Profile session startup restoration\n\n`ProfileSessionRestoreSync::restore_browser` reconstructs persisted session state into a fresh portable `BrowserApp` before live synchronization is enabled. Persisted `SessionWindowId` and `SessionTabId` values remain persistence identities only: restoration allocates fresh process-local browser/window/tab/navigation/history identities and records the exact cross-domain mapping only inside `ProfileSessionRestoreSync`. Persisted locations are installed through model-only begin/commit navigation transitions, leaving no pending navigation or address edit. Empty persisted windows remain empty; a completely empty persisted snapshot uses the ordinary one-window/one-blank-tab bootstrap with no bindings. Restoration reads but never mutates the active snapshot or its generation/revision state and constructs browser state plus bindings locally, so failure cannot partially publish startup state. The resulting bindings make the first synchronization a no-op and preserve persisted tab identity across later committed-location changes. Native startup wiring must establish this restored browser/binding pair before enabling live synchronization, so an initial blank browser cannot overwrite a persisted session.\n\n"""
    assert marker in architecture
    architecture_path.write_text(architecture.replace(marker, addition + marker, 1))

roadmap = roadmap_path.read_text()
old_roadmap = "a portable failure-atomic live-browser synchronization layer now binds runtime window/tab IDs only in memory to stable persisted session IDs, projects only committed locations, mirrors committed tab order/activation/closure and advances at most one runtime revision per real synchronization while refusing empty attachment over an unrestored persisted session; native synchronization wiring and startup restore remain later Z3 work;"
new_roadmap = "a portable failure-atomic live-browser synchronization layer now binds runtime window/tab IDs only in memory to stable persisted session IDs, projects only committed locations, mirrors committed tab order/activation/closure and advances at most one runtime revision per real synchronization while refusing empty attachment over an unrestored persisted session; portable startup restoration now reconstructs a fresh `BrowserApp` from the exact persisted window/tab order, committed locations and active selection with fresh process-local identities, preserves empty persisted windows, returns exact sync bindings, leaves persistence revisions untouched and makes the first live synchronization a no-op; native startup/synchronization wiring remains later Z3 work;"
if new_roadmap not in roadmap:
    assert old_roadmap in roadmap
    roadmap_path.write_text(roadmap.replace(old_roadmap, new_roadmap, 1))
