use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub const PROFILE_LOCK_FILE_NAME: &str = ".zorya-profile.lock";
pub const MAX_PROFILE_LOCK_BYTES: usize = 256;

const PROFILE_LOCK_MAGIC: &str = "ZORYA_PROFILE_LOCK_V1";
const PROFILE_LOCK_ACQUIRE_RETRIES: usize = 4;
static NEXT_PROFILE_LOCK_OWNER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileLockOwner {
    process_id: u32,
    owner_id: u64,
}

impl ProfileLockOwner {
    pub const fn process_id(self) -> u32 {
        self.process_id
    }

    pub const fn owner_id(self) -> u64 {
        self.owner_id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileLockError {
    Io {
        operation: &'static str,
        path: PathBuf,
        kind: io::ErrorKind,
    },
    Held {
        owner: ProfileLockOwner,
    },
    Corrupt {
        path: PathBuf,
    },
    OwnershipLost {
        expected: ProfileLockOwner,
        actual: Option<ProfileLockOwner>,
    },
    RootMismatch {
        lock_root: PathBuf,
        requested_root: PathBuf,
    },
    RecoveryTargetChanged {
        expected: ProfileLockOwner,
        actual: Option<ProfileLockOwner>,
    },
    OwnerIdExhausted,
}

impl fmt::Display for ProfileLockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                kind,
            } => write!(
                formatter,
                "{operation} failed for {}: {kind}",
                path.display()
            ),
            Self::Held { owner } => write!(
                formatter,
                "profile is already owned by process {} owner {}",
                owner.process_id(),
                owner.owner_id()
            ),
            Self::Corrupt { path } => write!(
                formatter,
                "profile lock record is malformed: {}",
                path.display()
            ),
            Self::OwnershipLost { expected, actual } => match actual {
                Some(actual) => write!(
                    formatter,
                    "profile ownership {}:{} was replaced by {}:{}",
                    expected.process_id(),
                    expected.owner_id(),
                    actual.process_id(),
                    actual.owner_id()
                ),
                None => write!(
                    formatter,
                    "profile ownership {}:{} is no longer present",
                    expected.process_id(),
                    expected.owner_id()
                ),
            },
            Self::RootMismatch {
                lock_root,
                requested_root,
            } => write!(
                formatter,
                "profile lock for {} cannot authorize storage root {}",
                lock_root.display(),
                requested_root.display()
            ),
            Self::RecoveryTargetChanged { expected, actual } => match actual {
                Some(actual) => write!(
                    formatter,
                    "abandoned profile lock {}:{} changed to {}:{} before recovery",
                    expected.process_id(),
                    expected.owner_id(),
                    actual.process_id(),
                    actual.owner_id()
                ),
                None => write!(
                    formatter,
                    "abandoned profile lock {}:{} disappeared before recovery",
                    expected.process_id(),
                    expected.owner_id()
                ),
            },
            Self::OwnerIdExhausted => {
                formatter.write_str("profile lock owner identifier space is exhausted")
            }
        }
    }
}

impl std::error::Error for ProfileLockError {}

#[derive(Clone, Debug)]
pub struct ProfileLock {
    inner: Arc<ProfileLockInner>,
}

impl PartialEq for ProfileLock {
    fn eq(&self, other: &Self) -> bool {
        self.inner.root == other.inner.root && self.inner.owner == other.inner.owner
    }
}

impl Eq for ProfileLock {}

#[derive(Debug)]
struct ProfileLockInner {
    root: PathBuf,
    path: PathBuf,
    owner: ProfileLockOwner,
}

impl Drop for ProfileLockInner {
    fn drop(&mut self) {
        if matches!(
            read_owner_if_present(&self.path),
            Ok(Some(owner)) if owner == self.owner
        ) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

impl ProfileLock {
    pub fn acquire(root: impl Into<PathBuf>) -> Result<Self, ProfileLockError> {
        let root = root.into();
        fs::create_dir_all(&root)
            .map_err(|error| io_error("create profile root", &root, error))?;
        let path = root.join(PROFILE_LOCK_FILE_NAME);
        let owner = next_owner()?;
        let record = encode_owner(owner);

        for _ in 0..PROFILE_LOCK_ACQUIRE_RETRIES {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    if let Err(error) = write_lock_record(&mut file, &record, &path) {
                        let _ = fs::remove_file(&path);
                        return Err(error);
                    }
                    return Ok(Self {
                        inner: Arc::new(ProfileLockInner {
                            root,
                            path,
                            owner,
                        }),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    match read_owner_if_present(&path)? {
                        Some(owner) => return Err(ProfileLockError::Held { owner }),
                        None => continue,
                    }
                }
                Err(error) => {
                    return Err(io_error("create profile lock", &path, error));
                }
            }
        }

        match read_owner_if_present(&path)? {
            Some(owner) => Err(ProfileLockError::Held { owner }),
            None => Err(ProfileLockError::Io {
                operation: "acquire profile lock",
                path,
                kind: io::ErrorKind::WouldBlock,
            }),
        }
    }

    pub fn recover_abandoned(
        root: impl Into<PathBuf>,
        expected: ProfileLockOwner,
    ) -> Result<Self, ProfileLockError> {
        let root = root.into();
        fs::create_dir_all(&root)
            .map_err(|error| io_error("create profile root", &root, error))?;
        let path = root.join(PROFILE_LOCK_FILE_NAME);

        match read_owner_if_present(&path)? {
            Some(actual) if actual == expected => {}
            actual => {
                return Err(ProfileLockError::RecoveryTargetChanged { expected, actual });
            }
        }

        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error("remove abandoned profile lock", &path, error)),
        }

        Self::acquire(root)
    }

    pub fn verify(&self) -> Result<(), ProfileLockError> {
        match read_owner_if_present(&self.inner.path)? {
            Some(actual) if actual == self.inner.owner => Ok(()),
            actual => Err(ProfileLockError::OwnershipLost {
                expected: self.inner.owner,
                actual,
            }),
        }
    }

    pub fn verify_for_root(&self, root: &Path) -> Result<(), ProfileLockError> {
        if self.inner.root != root {
            return Err(ProfileLockError::RootMismatch {
                lock_root: self.inner.root.clone(),
                requested_root: root.to_owned(),
            });
        }
        self.verify()
    }

    pub const fn owner(&self) -> ProfileLockOwner {
        self.inner.owner
    }

    pub fn root(&self) -> &Path {
        &self.inner.root
    }
}

fn next_owner() -> Result<ProfileLockOwner, ProfileLockError> {
    let owner_id = NEXT_PROFILE_LOCK_OWNER
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .map_err(|_| ProfileLockError::OwnerIdExhausted)?;
    Ok(ProfileLockOwner {
        process_id: std::process::id(),
        owner_id,
    })
}

fn encode_owner(owner: ProfileLockOwner) -> Vec<u8> {
    format!(
        "{PROFILE_LOCK_MAGIC}\npid={}\nowner={}\n",
        owner.process_id(),
        owner.owner_id()
    )
    .into_bytes()
}

fn write_lock_record(
    file: &mut File,
    record: &[u8],
    path: &Path,
) -> Result<(), ProfileLockError> {
    file.write_all(record)
        .map_err(|error| io_error("write profile lock", path, error))?;
    file.sync_all()
        .map_err(|error| io_error("sync profile lock", path, error))
}

fn read_owner_if_present(path: &Path) -> Result<Option<ProfileLockOwner>, ProfileLockError> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error("open profile lock", path, error)),
    };

    let mut bytes = Vec::new();
    file.by_ref()
        .take((MAX_PROFILE_LOCK_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error("read profile lock", path, error))?;
    if bytes.len() > MAX_PROFILE_LOCK_BYTES {
        return Err(ProfileLockError::Corrupt {
            path: path.to_owned(),
        });
    }

    parse_owner(&bytes).map(Some).ok_or_else(|| ProfileLockError::Corrupt {
        path: path.to_owned(),
    })
}

fn parse_owner(bytes: &[u8]) -> Option<ProfileLockOwner> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut lines = text.lines();
    if lines.next()? != PROFILE_LOCK_MAGIC {
        return None;
    }
    let process_id = lines.next()?.strip_prefix("pid=")?.parse().ok()?;
    let owner_id = lines.next()?.strip_prefix("owner=")?.parse().ok()?;
    if owner_id == 0 || lines.next().is_some() {
        return None;
    }
    Some(ProfileLockOwner {
        process_id,
        owner_id,
    })
}

fn io_error(operation: &'static str, path: &Path, error: io::Error) -> ProfileLockError {
    ProfileLockError::Io {
        operation,
        path: path.to_owned(),
        kind: error.kind(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(1);

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let id = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "zorya-profile-lock-{}-{id}",
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
    fn exact_owner_blocks_second_acquisition_until_drop() {
        let root = TempRoot::new();
        let first = ProfileLock::acquire(root.path()).unwrap();

        assert_eq!(
            ProfileLock::acquire(root.path()),
            Err(ProfileLockError::Held {
                owner: first.owner()
            })
        );

        drop(first);
        assert!(ProfileLock::acquire(root.path()).is_ok());
    }

    #[test]
    fn malformed_existing_lock_fails_closed() {
        let root = TempRoot::new();
        fs::create_dir_all(root.path()).unwrap();
        fs::write(root.path().join(PROFILE_LOCK_FILE_NAME), b"not-a-lock").unwrap();

        assert!(matches!(
            ProfileLock::acquire(root.path()),
            Err(ProfileLockError::Corrupt { .. })
        ));
        assert!(root.path().join(PROFILE_LOCK_FILE_NAME).exists());
    }

    #[test]
    fn abandoned_recovery_requires_exact_observed_owner() {
        let root = TempRoot::new();
        let first = ProfileLock::acquire(root.path()).unwrap();
        let owner = first.owner();
        let wrong = ProfileLockOwner {
            process_id: owner.process_id(),
            owner_id: owner.owner_id().checked_add(1).unwrap(),
        };

        assert_eq!(
            ProfileLock::recover_abandoned(root.path(), wrong),
            Err(ProfileLockError::RecoveryTargetChanged {
                expected: wrong,
                actual: Some(owner),
            })
        );
        assert_eq!(
            ProfileLock::acquire(root.path()),
            Err(ProfileLockError::Held { owner })
        );
    }

    #[test]
    fn recovered_owner_cannot_be_unlocked_by_old_guard() {
        let root = TempRoot::new();
        let first = ProfileLock::acquire(root.path()).unwrap();
        let first_owner = first.owner();
        let recovered = ProfileLock::recover_abandoned(root.path(), first_owner).unwrap();
        let recovered_owner = recovered.owner();
        assert_ne!(first_owner, recovered_owner);

        assert_eq!(
            first.verify(),
            Err(ProfileLockError::OwnershipLost {
                expected: first_owner,
                actual: Some(recovered_owner),
            })
        );

        drop(first);
        assert_eq!(
            ProfileLock::acquire(root.path()),
            Err(ProfileLockError::Held {
                owner: recovered_owner
            })
        );

        drop(recovered);
        assert!(ProfileLock::acquire(root.path()).is_ok());
    }

    #[test]
    fn stale_crash_record_is_explicitly_recoverable() {
        let root = TempRoot::new();
        let lock = ProfileLock::acquire(root.path()).unwrap();
        let owner = lock.owner();
        std::mem::forget(lock);

        assert_eq!(
            ProfileLock::acquire(root.path()),
            Err(ProfileLockError::Held { owner })
        );

        let recovered = ProfileLock::recover_abandoned(root.path(), owner).unwrap();
        assert_ne!(recovered.owner(), owner);
        recovered.verify().unwrap();
    }
}
