use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use zorya::{
    BookmarksError, BookmarksStore, BrowsingHistoryError, BrowsingHistoryStore,
    ProfileStorageError, ProfileStore, SessionRestoreError, SessionRestoreStore,
};

static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(1);

struct RedirectedRoot {
    root: PathBuf,
    redirected: PathBuf,
}

impl RedirectedRoot {
    fn new(child: &str) -> Self {
        let id = NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "zorya-store-redirect-test-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("create test root");
        let target = root.join("redirect-target");
        fs::create_dir_all(&target).expect("create redirect target");
        let redirected = root.join(child);
        create_directory_redirect(&target, &redirected);
        Self { root, redirected }
    }

    fn path(&self) -> &Path {
        &self.root
    }
}

impl Drop for RedirectedRoot {
    fn drop(&mut self) {
        remove_directory_redirect(&self.redirected);
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(unix)]
fn create_directory_redirect(target: &Path, redirected: &Path) {
    std::os::unix::fs::symlink(target, redirected).expect("create test symlink");
}

#[cfg(unix)]
fn remove_directory_redirect(redirected: &Path) {
    let _ = fs::remove_file(redirected);
}

#[cfg(windows)]
fn create_directory_redirect(target: &Path, redirected: &Path) {
    use std::process::Command;

    let status = Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(redirected)
        .arg(target)
        .status()
        .expect("run mklink");
    assert!(status.success(), "failed to create test junction");
}

#[cfg(windows)]
fn remove_directory_redirect(redirected: &Path) {
    let _ = fs::remove_dir(redirected);
}

#[test]
fn settings_store_rejects_redirected_storage_directory() {
    let root = RedirectedRoot::new("settings");
    assert!(matches!(
        ProfileStore::open(root.path()),
        Err(ProfileStorageError::Io {
            kind: io::ErrorKind::PermissionDenied,
            ..
        })
    ));
}

#[test]
fn bookmarks_store_rejects_redirected_storage_directory() {
    let root = RedirectedRoot::new("bookmarks");
    assert!(matches!(
        BookmarksStore::open(root.path()),
        Err(BookmarksError::Io {
            kind: io::ErrorKind::PermissionDenied,
            ..
        })
    ));
}

#[test]
fn browsing_history_store_rejects_redirected_storage_directory() {
    let root = RedirectedRoot::new("history");
    assert!(matches!(
        BrowsingHistoryStore::open(root.path()),
        Err(BrowsingHistoryError::Io {
            kind: io::ErrorKind::PermissionDenied,
            ..
        })
    ));
}

#[test]
fn session_restore_store_rejects_redirected_storage_directory() {
    let root = RedirectedRoot::new("session");
    assert!(matches!(
        SessionRestoreStore::open(root.path()),
        Err(SessionRestoreError::Io {
            kind: io::ErrorKind::PermissionDenied,
            ..
        })
    ));
}
