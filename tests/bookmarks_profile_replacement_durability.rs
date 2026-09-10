use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use zorya::{BookmarksStore, PreparedProfile, ProfileRuntime, ProfileRuntimeError};

static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(1);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "zorya-bookmarks-replacement-durability-{}-{label}-{id}",
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
fn dirty_bookmarks_block_replacement_until_exact_save_is_durable() {
    let first_root = TempRoot::new("first");
    let second_root = TempRoot::new("second");
    let mut runtime = ProfileRuntime::new();
    let first = load_profile(&mut runtime, first_root.path());

    runtime
        .add_bookmark(first, "Example", "https://example.test/")
        .unwrap();
    assert!(runtime.bookmarks_is_dirty(first).unwrap());

    let selection = runtime
        .begin_selection(second_root.path())
        .unwrap()
        .into_intent();
    let prepared = PreparedProfile::load(&selection).unwrap();
    let rejection = runtime.commit_selection(prepared).unwrap_err();
    assert_eq!(
        rejection.error(),
        &ProfileRuntimeError::ActiveProfileNotDurable { profile: first }
    );
    let prepared = rejection.into_parts().1;

    let completion = runtime
        .begin_bookmarks_save_if_dirty(first)
        .unwrap()
        .unwrap()
        .execute();
    assert!(completion.result().is_ok());
    runtime.complete_bookmarks_save(completion).unwrap();
    assert!(!runtime.bookmarks_is_dirty(first).unwrap());

    let persisted = BookmarksStore::open(first_root.path())
        .unwrap()
        .load()
        .unwrap()
        .into_snapshot();
    assert_eq!(persisted.generation(), 1);
    assert_eq!(persisted.len(), 1);

    let replacement = runtime.commit_selection(prepared).unwrap();
    assert_ne!(replacement.active_profile(), first);
}
