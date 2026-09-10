use std::fs;
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
