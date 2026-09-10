use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use zorya::{PreparedProfile, ProfileRuntime, SessionRestoreError, SessionRestoreStore};

static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(1);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "zorya-session-runtime-{}-{label}-{id}",
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

fn load_profile(runtime: &mut ProfileRuntime, root: &Path) -> zorya::ProfileId {
    let selection = runtime.begin_selection(root).unwrap().into_intent();
    let prepared = PreparedProfile::load(&selection).unwrap();
    runtime.commit_selection(prepared).unwrap().active_profile()
}

#[test]
fn successful_save_cleans_only_captured_session_mutations() {
    let root = TempRoot::new("newer-mutation");
    let mut runtime = ProfileRuntime::new();
    let profile = load_profile(&mut runtime, root.path());

    runtime.add_session_window(profile).unwrap();
    let first = runtime
        .begin_session_restore_save_if_dirty(profile)
        .unwrap()
        .unwrap();
    assert_eq!(first.mutation_revision(), 1);

    runtime.add_session_window(profile).unwrap();
    assert_eq!(
        runtime.session_restore_unsaved_mutations(profile).unwrap(),
        2
    );

    let completion = first.execute();
    assert!(completion.result().is_ok());
    runtime.complete_session_restore_save(completion).unwrap();
    assert_eq!(
        runtime
            .active_session_restore(profile)
            .unwrap()
            .generation(),
        1
    );
    assert_eq!(runtime.active_session_restore(profile).unwrap().len(), 2);
    assert!(runtime.session_restore_is_dirty(profile).unwrap());
    assert_eq!(
        runtime.session_restore_unsaved_mutations(profile).unwrap(),
        1
    );

    let completion = runtime
        .begin_session_restore_save_if_dirty(profile)
        .unwrap()
        .unwrap()
        .execute();
    runtime.complete_session_restore_save(completion).unwrap();
    assert_eq!(
        runtime
            .active_session_restore(profile)
            .unwrap()
            .generation(),
        2
    );
    assert!(!runtime.session_restore_is_dirty(profile).unwrap());

    let persisted = SessionRestoreStore::open(root.path())
        .unwrap()
        .load()
        .unwrap()
        .into_snapshot();
    assert_eq!(persisted.generation(), 2);
    assert_eq!(persisted.len(), 2);
}

#[test]
fn failed_and_noop_session_mutations_do_not_advance_revision() {
    let root = TempRoot::new("failure-atomic");
    let mut runtime = ProfileRuntime::new();
    let profile = load_profile(&mut runtime, root.path());
    let window = runtime.add_session_window(profile).unwrap();
    let tab = runtime
        .add_session_tab(profile, window, "https://example.test/original")
        .unwrap();
    assert_eq!(
        runtime.session_restore_mutation_revision(profile).unwrap(),
        2
    );

    assert!(
        !runtime
            .update_session_tab_location(profile, window, tab, "https://example.test/original")
            .unwrap()
    );
    assert_eq!(
        runtime.session_restore_mutation_revision(profile).unwrap(),
        2
    );

    let oversized = "x".repeat(zorya::MAX_SESSION_LOCATION_BYTES + 1);
    assert!(matches!(
        runtime.update_session_tab_location(profile, window, tab, oversized),
        Err(zorya::ProfileSessionRestoreRuntimeError::SessionRestore(
            SessionRestoreError::LocationTooLarge { .. }
        ))
    ));
    assert_eq!(
        runtime.session_restore_mutation_revision(profile).unwrap(),
        2
    );
}

#[test]
fn failed_save_completion_clears_only_ownership_and_leaves_session_dirty() {
    let root = TempRoot::new("save-failure");
    let mut runtime = ProfileRuntime::new();
    let profile = load_profile(&mut runtime, root.path());
    runtime.add_session_window(profile).unwrap();
    let intent = runtime.begin_session_restore_save(profile).unwrap();

    let session_dir = root.path().join("session");
    fs::write(
        session_dir.join("session-00000000000000000001.bin"),
        b"corrupt external generation",
    )
    .unwrap();

    let completion = intent.execute();
    assert!(completion.result().is_err());
    runtime.complete_session_restore_save(completion).unwrap();
    assert!(runtime.pending_session_restore_save().is_none());
    assert!(runtime.session_restore_is_dirty(profile).unwrap());
    assert_eq!(
        runtime
            .active_session_restore(profile)
            .unwrap()
            .generation(),
        0
    );
}
