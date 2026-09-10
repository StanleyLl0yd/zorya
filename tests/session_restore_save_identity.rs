use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use zorya::{PreparedProfile, ProfileRuntime};

static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(1);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "zorya-session-save-identity-{}-{label}-{id}",
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
fn session_restore_save_identity_is_process_monotonic_across_profile_replacement() {
    let first_root = TempRoot::new("first");
    let second_root = TempRoot::new("second");
    let mut runtime = ProfileRuntime::new();

    let first = load_profile(&mut runtime, first_root.path());
    runtime.add_session_window(first).unwrap();
    let first_save = runtime.begin_session_restore_save(first).unwrap();
    let first_save_id = first_save.id();
    runtime
        .complete_session_restore_save(first_save.execute())
        .unwrap();

    let selection = runtime
        .begin_selection(second_root.path())
        .unwrap()
        .into_intent();
    let prepared = PreparedProfile::load(&selection).unwrap();
    let second = runtime.commit_selection(prepared).unwrap().active_profile();
    assert_ne!(second, first);

    runtime.add_session_window(second).unwrap();
    let second_save = runtime.begin_session_restore_save(second).unwrap();
    assert!(second_save.id().get() > first_save_id.get());
}
