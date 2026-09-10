use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use zorya::{BookmarksStore, PreparedProfile, ProfileLock, ProfileRuntime};

static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(1);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Self {
        let id = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "zorya-bookmarks-profile-attachment-{}-{id}",
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
fn corrupt_newest_bookmarks_recovery_survives_profile_commit() {
    let root = TempRoot::new();
    let store = BookmarksStore::open(root.path()).unwrap();
    let mut snapshot = store.load().unwrap().into_snapshot();
    let bookmark = snapshot
        .add_bookmark("Example", "https://example.test/bookmark")
        .unwrap();

    let lock = ProfileLock::acquire(root.path()).unwrap();
    let saved = store.save(&lock, &snapshot).unwrap().into_snapshot();
    assert_eq!(saved.generation(), 1);
    lock.release().unwrap();

    let corrupt_newest = root
        .path()
        .join("bookmarks")
        .join("bookmarks-00000000000000000002.bin");
    fs::write(corrupt_newest, b"corrupt newest generation").unwrap();

    let mut runtime = ProfileRuntime::new();
    let intent = runtime.begin_selection(root.path()).unwrap().into_intent();
    let prepared = PreparedProfile::load(&intent).unwrap();

    assert_eq!(prepared.bookmarks().generation(), 1);
    assert_eq!(
        prepared.bookmarks().bookmark(bookmark).unwrap().title(),
        "Example"
    );
    assert_eq!(
        prepared
            .bookmarks_recovery()
            .expect("corrupt newest generation must be reported")
            .skipped_generations(),
        &[2]
    );

    let profile = runtime.commit_selection(prepared).unwrap().active_profile();
    let active = runtime.active_profile().unwrap();
    assert_eq!(active.id(), profile);
    assert_eq!(active.bookmarks().generation(), 1);
    assert_eq!(
        active.bookmarks().bookmark(bookmark).unwrap().title(),
        "Example"
    );
    assert_eq!(
        active
            .bookmarks_recovery()
            .expect("recovery report must survive exact profile commit")
            .skipped_generations(),
        &[2]
    );
}
