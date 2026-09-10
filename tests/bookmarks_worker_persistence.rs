use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use zorya::{PreparedProfile, ProfileRuntime, ProfileWorker};

static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(1);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "zorya-bookmarks-worker-persistence-{}-{label}-{id}",
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

fn dirty_bookmarks_runtime(root: &Path) -> (ProfileRuntime, zorya::ProfileBookmarksSaveIntent) {
    let mut runtime = ProfileRuntime::new();
    let selection = runtime.begin_selection(root).unwrap().into_intent();
    let prepared = PreparedProfile::load(&selection).unwrap();
    let profile = runtime.commit_selection(prepared).unwrap().active_profile();
    runtime
        .add_bookmark(profile, "Example", "https://example.test/")
        .unwrap();
    let intent = runtime
        .begin_bookmarks_save_if_dirty(profile)
        .unwrap()
        .unwrap();
    (runtime, intent)
}

#[test]
fn queue_full_returns_exact_bookmarks_save_for_runtime_cancellation() {
    let blocking_root = TempRoot::new("blocking");
    let queued_root = TempRoot::new("queued");
    let bookmarks_root = TempRoot::new("bookmarks");

    let mut queue_runtime = ProfileRuntime::new();
    let blocking = queue_runtime
        .begin_selection(blocking_root.path())
        .unwrap()
        .into_intent();
    let queued = queue_runtime
        .begin_selection(queued_root.path())
        .unwrap()
        .into_intent();
    let (mut runtime, intent) = dirty_bookmarks_runtime(bookmarks_root.path());
    let profile = intent.profile();
    let save = intent.id();

    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let mut block_first_completion = true;
    let worker = ProfileWorker::spawn(move |_| {
        if block_first_completion {
            block_first_completion = false;
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        }
    })
    .unwrap();

    worker.prepare(blocking).unwrap();
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    worker.prepare(queued).unwrap();
    let error = worker.save_bookmarks(intent).unwrap_err();
    assert!(error.is_full());
    let intent = error.into_work();
    assert_eq!(intent.id(), save);
    assert_eq!(intent.profile(), profile);

    runtime.cancel_bookmarks_save(save).unwrap();
    assert!(runtime.pending_bookmarks_save().is_none());
    assert!(runtime.bookmarks_is_dirty(profile).unwrap());

    release_tx.send(()).unwrap();
}

#[test]
fn disconnected_worker_returns_exact_unavailable_bookmarks_save() {
    let blocking_root = TempRoot::new("disconnect-blocking");
    let bookmarks_root = TempRoot::new("disconnect-bookmarks");

    let mut queue_runtime = ProfileRuntime::new();
    let blocking = queue_runtime
        .begin_selection(blocking_root.path())
        .unwrap()
        .into_intent();
    let (mut runtime, intent) = dirty_bookmarks_runtime(bookmarks_root.path());
    let profile = intent.profile();
    let save = intent.id();

    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let worker = ProfileWorker::spawn(move |_| {
        entered_tx.send(()).unwrap();
        panic!("fixture terminates profile worker");
    })
    .unwrap();

    worker.prepare(blocking).unwrap();
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !worker.is_finished() && Instant::now() < deadline {
        thread::yield_now();
    }
    assert!(worker.is_finished());

    let error = worker.save_bookmarks(intent).unwrap_err();
    assert!(!error.is_full());
    let intent = error.into_work();
    assert_eq!(intent.id(), save);
    assert_eq!(intent.profile(), profile);

    runtime.cancel_bookmarks_save(save).unwrap();
    assert!(runtime.pending_bookmarks_save().is_none());
    assert!(runtime.bookmarks_is_dirty(profile).unwrap());
}
