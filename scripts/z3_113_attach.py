from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    text = target.read_text(encoding="utf-8")
    if text.count(old) != 1:
        raise SystemExit(f"expected exactly one match in {path}: {old[:80]!r}")
    target.write_text(text.replace(old, new, 1), encoding="utf-8")


profile = "src/profile_runtime.rs"
replace_once(
    profile,
    "use crate::profile_lock::{ProfileLock, ProfileLockError};\n",
    "use crate::profile_lock::{ProfileLock, ProfileLockError};\n"
    "use crate::session_restore::{\n"
    "    SessionRestoreError, SessionRestoreRecovery, SessionRestoreSnapshot, SessionRestoreStore,\n"
    "};\n",
)
replace_once(
    profile,
    "    bookmarks: BookmarksSnapshot,\n    bookmarks_recovery: Option<BookmarksRecovery>,\n}\n\nimpl PreparedProfile {",
    "    bookmarks: BookmarksSnapshot,\n"
    "    bookmarks_recovery: Option<BookmarksRecovery>,\n"
    "    session_restore: SessionRestoreSnapshot,\n"
    "    session_restore_recovery: Option<SessionRestoreRecovery>,\n"
    "}\n\nimpl PreparedProfile {",
)
replace_once(
    profile,
    "            let bookmarks_recovery = bookmarks_load.recovery().cloned();\n"
    "            let bookmarks = bookmarks_load.into_snapshot();\n\n"
    "            Ok((\n",
    "            let bookmarks_recovery = bookmarks_load.recovery().cloned();\n"
    "            let bookmarks = bookmarks_load.into_snapshot();\n\n"
    "            let session_restore_store = SessionRestoreStore::open(intent.root.clone())\n"
    "                .map_err(ProfilePreparationError::SessionRestore)?;\n"
    "            let session_restore_load = session_restore_store\n"
    "                .load()\n"
    "                .map_err(ProfilePreparationError::SessionRestore)?;\n"
    "            let session_restore_recovery = session_restore_load.recovery().cloned();\n"
    "            let session_restore = session_restore_load.into_snapshot();\n\n"
    "            Ok((\n",
)
replace_once(
    profile,
    "                bookmarks,\n                bookmarks_recovery,\n            ))\n",
    "                bookmarks,\n"
    "                bookmarks_recovery,\n"
    "                session_restore,\n"
    "                session_restore_recovery,\n"
    "            ))\n",
)
replace_once(
    profile,
    "            bookmarks,\n            bookmarks_recovery,\n        ) = match prepared {",
    "            bookmarks,\n"
    "            bookmarks_recovery,\n"
    "            session_restore,\n"
    "            session_restore_recovery,\n"
    "        ) = match prepared {",
)
replace_once(
    profile,
    "            bookmarks,\n            bookmarks_recovery,\n        })\n",
    "            bookmarks,\n"
    "            bookmarks_recovery,\n"
    "            session_restore,\n"
    "            session_restore_recovery,\n"
    "        })\n",
)
replace_once(
    profile,
    "    pub const fn bookmarks_recovery(&self) -> Option<&BookmarksRecovery> {\n"
    "        self.bookmarks_recovery.as_ref()\n"
    "    }\n\n"
    "    #[cfg(any(test, target_os = \"windows\"))]",
    "    pub const fn bookmarks_recovery(&self) -> Option<&BookmarksRecovery> {\n"
    "        self.bookmarks_recovery.as_ref()\n"
    "    }\n\n"
    "    pub const fn session_restore(&self) -> &SessionRestoreSnapshot {\n"
    "        &self.session_restore\n"
    "    }\n\n"
    "    pub const fn session_restore_recovery(&self) -> Option<&SessionRestoreRecovery> {\n"
    "        self.session_restore_recovery.as_ref()\n"
    "    }\n\n"
    "    #[cfg(any(test, target_os = \"windows\"))]",
)
replace_once(
    profile,
    "    BrowsingHistory(crate::browsing_history::BrowsingHistoryError),\n    Bookmarks(BookmarksError),\n    LockRelease {",
    "    BrowsingHistory(crate::browsing_history::BrowsingHistoryError),\n"
    "    Bookmarks(BookmarksError),\n"
    "    SessionRestore(SessionRestoreError),\n"
    "    LockRelease {",
)
replace_once(
    profile,
    "            Self::BrowsingHistory(error) => error.fmt(formatter),\n"
    "            Self::Bookmarks(error) => error.fmt(formatter),\n"
    "            Self::LockRelease {",
    "            Self::BrowsingHistory(error) => error.fmt(formatter),\n"
    "            Self::Bookmarks(error) => error.fmt(formatter),\n"
    "            Self::SessionRestore(error) => error.fmt(formatter),\n"
    "            Self::LockRelease {",
)
replace_once(
    profile,
    "            Self::BrowsingHistory(error) => Some(error),\n"
    "            Self::Bookmarks(error) => Some(error),\n"
    "            Self::LockRelease { preparation, .. } => Some(preparation.as_ref()),",
    "            Self::BrowsingHistory(error) => Some(error),\n"
    "            Self::Bookmarks(error) => Some(error),\n"
    "            Self::SessionRestore(error) => Some(error),\n"
    "            Self::LockRelease { preparation, .. } => Some(preparation.as_ref()),",
)
replace_once(
    profile,
    "    bookmarks: BookmarksRuntimeState,\n    settings_revision: u64,",
    "    bookmarks: BookmarksRuntimeState,\n"
    "    session_restore: SessionRestoreSnapshot,\n"
    "    session_restore_recovery: Option<SessionRestoreRecovery>,\n"
    "    settings_revision: u64,",
)
replace_once(
    profile,
    "    pub const fn bookmarks_recovery(&self) -> Option<&BookmarksRecovery> {\n"
    "        self.bookmarks.recovery()\n"
    "    }\n"
    "}\n\n#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]\n"
    "pub struct ProfileSettingsSaveId",
    "    pub const fn bookmarks_recovery(&self) -> Option<&BookmarksRecovery> {\n"
    "        self.bookmarks.recovery()\n"
    "    }\n\n"
    "    pub const fn session_restore(&self) -> &SessionRestoreSnapshot {\n"
    "        &self.session_restore\n"
    "    }\n\n"
    "    pub const fn session_restore_recovery(&self) -> Option<&SessionRestoreRecovery> {\n"
    "        self.session_restore_recovery.as_ref()\n"
    "    }\n"
    "}\n\n#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]\n"
    "pub struct ProfileSettingsSaveId",
)
replace_once(
    profile,
    "            bookmarks: BookmarksRuntimeState::new(prepared.bookmarks, prepared.bookmarks_recovery),\n"
    "            settings_revision: 0,",
    "            bookmarks: BookmarksRuntimeState::new(prepared.bookmarks, prepared.bookmarks_recovery),\n"
    "            session_restore: prepared.session_restore,\n"
    "            session_restore_recovery: prepared.session_restore_recovery,\n"
    "            settings_revision: 0,",
)

architecture = "docs/ARCHITECTURE.md"
replace_once(
    architecture,
    "`PreparedProfile` acquires ownership before reading profile identity, display metadata, settings, history or bookmarks.",
    "`PreparedProfile` acquires ownership before reading profile identity, display metadata, settings, history, bookmarks or session restore.",
)
replace_once(
    architecture,
    "The storage primitive is synchronous and therefore belongs only on background/profile workers. Runtime attachment, mutation revisions, save scheduling and native startup restoration are deliberately later Z3 work.",
    "The storage primitive is synchronous and therefore belongs only on background/profile workers. `PreparedProfile::load` now opens the session store while the exact profile lock is held, preserves the exact `SessionRestoreSnapshot` plus optional `SessionRestoreRecovery`, and a successful exact selection commit transfers both unchanged into `ActiveProfile`; mutation revisions, save lifecycle/worker dispatch, scheduling and native startup restoration remain later Z3 work.",
)
replace_once(
    architecture,
    "`PreparedProfile::load` opens recoverable settings, browsing-history and bookmarks state on the background/profile-worker side. A successful exact profile-selection commit transfers the history/bookmark snapshots and any recovery reports into `ActiveProfile`; these profile-local snapshots remain bound to the same process-local stable `ProfileId`, so later bookmark mutation work can reject stale profile identities rather than retargeting a replacement profile.",
    "`PreparedProfile::load` opens recoverable settings, browsing-history, bookmarks and session-restore state on the background/profile-worker side. A successful exact profile-selection commit transfers the history/bookmark/session snapshots and any recovery reports into `ActiveProfile`; these profile-local snapshots remain bound to the same process-local stable `ProfileId`, while persisted session window/tab identities remain distinct from runtime browser identities.",
)

roadmap = "docs/ROADMAP.md"
replace_once(
    roadmap,
    "- session restore now has a bounded, versioned and recoverable profile-local storage foundation with persisted window/tab identities distinct from runtime browser identities, ordered windows/tabs, exact active-tab membership and committed display-location-only records; immutable generations use exact profile-lock verification, no-overwrite publication, stale/concurrent writer rejection, explicit corruption fallback, fail-closed schema handling and bounded retention; runtime attachment, dirty/save scheduling, worker execution and native startup restore remain later Z3 work;",
    "- session restore now has a bounded, versioned and recoverable profile-local storage foundation with persisted window/tab identities distinct from runtime browser identities, ordered windows/tabs, exact active-tab membership and committed display-location-only records; immutable generations use exact profile-lock verification, no-overwrite publication, stale/concurrent writer rejection, explicit corruption fallback, fail-closed schema handling and bounded retention; locked worker-side profile preparation now loads the exact session snapshot plus recovery report and transfers both unchanged into the committed `ActiveProfile`; mutation/dirty/save lifecycle, worker save dispatch, scheduling and native startup restore remain later Z3 work;",
)

test = Path("tests/session_restore_profile_attachment.rs")
if test.exists():
    raise SystemExit(f"{test} already exists")
test.write_text(
    r'''use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use zorya::{PreparedProfile, ProfileLock, ProfileRuntime, SessionRestoreStore};

static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(1);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Self {
        let id = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "zorya-session-restore-profile-attachment-{}-{id}",
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

#[test]
fn corrupt_newest_session_restore_recovery_survives_profile_commit() {
    let root = TempRoot::new();
    let store = SessionRestoreStore::open(root.path()).unwrap();
    let mut snapshot = store.load().unwrap().into_snapshot();

    let first_window = snapshot.add_window().unwrap();
    let first_tab = snapshot
        .add_tab(first_window, "https://example.test/first")
        .unwrap();
    let active_tab = snapshot
        .add_tab(first_window, "https://example.test/active")
        .unwrap();
    assert!(snapshot.set_active_tab(first_window, active_tab).unwrap());

    let second_window = snapshot.add_window().unwrap();
    let second_tab = snapshot
        .add_tab(second_window, "https://example.test/second-window")
        .unwrap();

    let lock = ProfileLock::acquire(root.path()).unwrap();
    let saved = store.save(&lock, &snapshot).unwrap().into_snapshot();
    assert_eq!(saved.generation(), 1);
    lock.release().unwrap();

    let corrupt_newest = root
        .path()
        .join("session")
        .join("session-00000000000000000002.bin");
    fs::write(corrupt_newest, b"corrupt newest generation").unwrap();

    let mut runtime = ProfileRuntime::new();
    let intent = runtime.begin_selection(root.path()).unwrap().into_intent();
    let prepared = PreparedProfile::load(&intent).unwrap();

    let prepared_session = prepared.session_restore();
    assert_eq!(prepared_session.generation(), 1);
    assert_eq!(prepared_session.windows().len(), 2);
    assert_eq!(prepared_session.windows()[0].id(), first_window);
    assert_eq!(prepared_session.windows()[1].id(), second_window);
    assert_eq!(prepared_session.windows()[0].active_tab(), Some(active_tab));
    assert_eq!(prepared_session.windows()[0].tabs()[0].id(), first_tab);
    assert_eq!(prepared_session.windows()[0].tabs()[1].id(), active_tab);
    assert_eq!(
        prepared_session.windows()[0].tabs()[1].location(),
        "https://example.test/active"
    );
    assert_eq!(prepared_session.windows()[1].tabs()[0].id(), second_tab);
    assert_eq!(
        prepared
            .session_restore_recovery()
            .expect("corrupt newest generation must be reported")
            .skipped_generations(),
        &[2]
    );
    let expected_session = prepared_session.clone();

    let profile = runtime.commit_selection(prepared).unwrap().active_profile();
    let active = runtime.active_profile().unwrap();
    assert_eq!(active.id(), profile);
    assert_eq!(active.session_restore(), &expected_session);
    assert_eq!(
        active
            .session_restore_recovery()
            .expect("recovery report must survive exact profile commit")
            .skipped_generations(),
        &[2]
    );
}
''',
    encoding="utf-8",
)
