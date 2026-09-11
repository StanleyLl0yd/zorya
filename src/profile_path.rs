use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const PROFILE_STORAGE_DIRECTORIES: [&str; 4] = ["settings", "history", "bookmarks", "session"];

#[derive(Debug)]
pub(crate) struct ProfilePathError {
    path: PathBuf,
    kind: io::ErrorKind,
}

impl ProfilePathError {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) const fn kind(&self) -> io::ErrorKind {
        self.kind
    }
}

pub(crate) fn prepare_profile_storage_paths(root: &Path) -> Result<(), ProfilePathError> {
    ensure_real_directory(root)?;
    for directory in PROFILE_STORAGE_DIRECTORIES {
        ensure_real_directory(&root.join(directory))?;
    }
    Ok(())
}

pub(crate) fn verify_profile_storage_paths(root: &Path) -> Result<(), ProfilePathError> {
    let metadata = fs::symlink_metadata(root).map_err(|error| path_error(root, error.kind()))?;
    validate_directory(root, &metadata)?;
    verify_storage_children(root)
}

pub(crate) fn verify_not_redirected_if_present(path: &Path) -> Result<(), ProfilePathError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(path_error(path, error.kind())),
    };
    if is_redirect(&metadata) {
        Err(path_error(path, io::ErrorKind::PermissionDenied))
    } else {
        Ok(())
    }
}

pub(crate) fn ensure_real_directory(path: &Path) -> Result<(), ProfilePathError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_directory(path, &metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|error| path_error(path, error.kind()))?;
            let metadata =
                fs::symlink_metadata(path).map_err(|error| path_error(path, error.kind()))?;
            validate_directory(path, &metadata)
        }
        Err(error) => Err(path_error(path, error.kind())),
    }
}

fn verify_storage_children(root: &Path) -> Result<(), ProfilePathError> {
    for directory in PROFILE_STORAGE_DIRECTORIES {
        let path = root.join(directory);
        let metadata =
            fs::symlink_metadata(&path).map_err(|error| path_error(&path, error.kind()))?;
        validate_directory(&path, &metadata)?;
    }
    Ok(())
}

fn validate_directory(path: &Path, metadata: &fs::Metadata) -> Result<(), ProfilePathError> {
    if is_redirect(metadata) {
        return Err(path_error(path, io::ErrorKind::PermissionDenied));
    }
    if !metadata.is_dir() {
        return Err(path_error(path, io::ErrorKind::InvalidData));
    }
    Ok(())
}

fn path_error(path: &Path, kind: io::ErrorKind) -> ProfilePathError {
    ProfilePathError {
        path: path.to_owned(),
        kind,
    }
}

#[cfg(windows)]
fn is_redirect(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_redirect(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(1);

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new(label: &str) -> Self {
            let id = NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "zorya-profile-path-{label}-{}-{id}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn preparation_materializes_real_storage_directories() {
        let root = TestRoot::new("real");
        prepare_profile_storage_paths(root.path()).unwrap();
        assert!(root.path().is_dir());
        for directory in PROFILE_STORAGE_DIRECTORIES {
            let path = root.path().join(directory);
            let metadata = fs::symlink_metadata(path).unwrap();
            assert!(metadata.is_dir());
            assert!(!is_redirect(&metadata));
        }
        verify_profile_storage_paths(root.path()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_unix_root_symlink() {
        use std::os::unix::fs::symlink;

        let base = TestRoot::new("unix-root-symlink");
        let target = base.path().join("target");
        let redirected = base.path().join("redirected");
        fs::create_dir_all(&target).unwrap();
        symlink(&target, &redirected).unwrap();

        let error = prepare_profile_storage_paths(&redirected).unwrap_err();
        assert_eq!(error.path(), redirected);
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);

        fs::remove_file(redirected).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_unix_storage_symlink() {
        use std::os::unix::fs::symlink;

        let root = TestRoot::new("unix-storage-symlink");
        fs::create_dir_all(root.path()).unwrap();
        let target = root.path().join("target");
        let redirected = root.path().join("settings");
        fs::create_dir_all(&target).unwrap();
        symlink(&target, &redirected).unwrap();

        let error = prepare_profile_storage_paths(root.path()).unwrap_err();
        assert_eq!(error.path(), redirected);
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);

        fs::remove_file(redirected).unwrap();
    }

    #[cfg(windows)]
    fn create_junction(target: &Path, junction: &Path) {
        use std::process::Command;

        let status = Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(junction)
            .arg(target)
            .status()
            .unwrap();
        assert!(status.success(), "failed to create test junction");
    }

    #[cfg(windows)]
    #[test]
    fn rejects_windows_root_junction() {
        let base = TestRoot::new("windows-root-junction");
        let target = base.path().join("target");
        let redirected = base.path().join("redirected");
        fs::create_dir_all(&target).unwrap();
        create_junction(&target, &redirected);

        let error = prepare_profile_storage_paths(&redirected).unwrap_err();
        assert_eq!(error.path(), redirected);
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);

        fs::remove_dir(redirected).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn rejects_windows_storage_junction() {
        let root = TestRoot::new("windows-storage-junction");
        fs::create_dir_all(root.path()).unwrap();
        let target = root.path().join("target");
        let redirected = root.path().join("settings");
        fs::create_dir_all(&target).unwrap();
        create_junction(&target, &redirected);

        let error = prepare_profile_storage_paths(root.path()).unwrap_err();
        assert_eq!(error.path(), redirected);
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);

        fs::remove_dir(redirected).unwrap();
    }
}
