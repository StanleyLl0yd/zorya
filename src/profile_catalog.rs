use crate::profile_lock::{ProfileLock, ProfileLockError};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const PROFILE_IDENTITY_SCHEMA_VERSION: u32 = 1;
pub const PROFILE_IDENTITY_DIRECTORY_NAME: &str = ".zorya-profile.id";
pub const MAX_PROFILE_IDENTITY_DIRECTORY_ENTRIES: usize = 4;
pub const MAX_PROFILE_CATALOG_DIRECTORY_ENTRIES: usize = 128;
pub const MAX_DISCOVERED_PROFILES: usize = 32;

const LEGACY_DEFAULT_DIRECTORY_NAME: &str = "Default";
const PROFILE_IDENTITY_RECORD_PREFIX: &str = "v1-";
const PROFILE_IDENTITY_HEX_DIGITS: usize = 32;
static NEXT_PROFILE_STORAGE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileStorageId(u128);

impl ProfileStorageId {
    pub const fn get(self) -> u128 {
        self.0
    }
}

impl fmt::Display for ProfileStorageId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:032x}", self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileIdentityError {
    Lock(ProfileLockError),
    Io {
        operation: &'static str,
        path: PathBuf,
        kind: io::ErrorKind,
    },
    DirectoryEntryLimitExceeded {
        found: usize,
        limit: usize,
    },
    Corrupt {
        path: PathBuf,
    },
    UnsupportedSchema {
        path: PathBuf,
        schema: u32,
    },
    ConcurrentWrite {
        path: PathBuf,
    },
    SystemClockBeforeUnixEpoch,
    StorageIdTimeRangeExceeded,
    StorageIdSequenceExhausted,
}

impl fmt::Display for ProfileIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lock(error) => error.fmt(formatter),
            Self::Io {
                operation,
                path,
                kind,
            } => write!(
                formatter,
                "{operation} failed for {}: {kind}",
                path.display()
            ),
            Self::DirectoryEntryLimitExceeded { found, limit } => write!(
                formatter,
                "profile identity directory contains {found} entries; limit is {limit}"
            ),
            Self::Corrupt { path } => {
                write!(formatter, "profile identity is malformed: {}", path.display())
            }
            Self::UnsupportedSchema { path, schema } => write!(
                formatter,
                "profile identity {} uses unsupported schema {schema}",
                path.display()
            ),
            Self::ConcurrentWrite { path } => write!(
                formatter,
                "profile identity was created concurrently: {}",
                path.display()
            ),
            Self::SystemClockBeforeUnixEpoch => {
                formatter.write_str("system clock precedes Unix epoch while allocating profile identity")
            }
            Self::StorageIdTimeRangeExceeded => {
                formatter.write_str("system time exceeds profile storage identity range")
            }
            Self::StorageIdSequenceExhausted => {
                formatter.write_str("profile storage identity sequence is exhausted")
            }
        }
    }
}

impl std::error::Error for ProfileIdentityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Lock(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileCatalogError {
    Io {
        operation: &'static str,
        path: PathBuf,
        kind: io::ErrorKind,
    },
    DirectoryEntryLimitExceeded {
        found: usize,
        limit: usize,
    },
    ProfileLimitExceeded {
        found: usize,
        limit: usize,
    },
    UnsupportedEntry {
        path: PathBuf,
    },
    MissingIdentity {
        root: PathBuf,
    },
    DuplicateIdentity {
        storage_id: ProfileStorageId,
        first_root: PathBuf,
        second_root: PathBuf,
    },
    Identity {
        root: PathBuf,
        error: ProfileIdentityError,
    },
}

impl fmt::Display for ProfileCatalogError {
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
            Self::DirectoryEntryLimitExceeded { found, limit } => write!(
                formatter,
                "profile catalog contains {found} entries; scan limit is {limit}"
            ),
            Self::ProfileLimitExceeded { found, limit } => write!(
                formatter,
                "profile catalog contains {found} profiles; limit is {limit}"
            ),
            Self::UnsupportedEntry { path } => write!(
                formatter,
                "profile catalog contains unsupported filesystem entry {}",
                path.display()
            ),
            Self::MissingIdentity { root } => write!(
                formatter,
                "profile {} has no persisted storage identity",
                root.display()
            ),
            Self::DuplicateIdentity {
                storage_id,
                first_root,
                second_root,
            } => write!(
                formatter,
                "profile storage identity {storage_id} is duplicated by {} and {}",
                first_root.display(),
                second_root.display()
            ),
            Self::Identity { root, error } => write!(
                formatter,
                "failed to inspect profile identity for {}: {error}",
                root.display()
            ),
        }
    }
}

impl std::error::Error for ProfileCatalogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Identity { error, .. } => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileCatalogEntry {
    root: PathBuf,
    storage_id: Option<ProfileStorageId>,
}

impl ProfileCatalogEntry {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub const fn storage_id(&self) -> Option<ProfileStorageId> {
        self.storage_id
    }

    pub const fn requires_legacy_bootstrap(&self) -> bool {
        self.storage_id.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileCatalog {
    root: PathBuf,
}

impl ProfileCatalog {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, ProfileCatalogError> {
        let root = root.into();
        fs::create_dir_all(&root)
            .map_err(|error| catalog_io_error("create profile catalog root", &root, error))?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn discover(&self) -> Result<Vec<ProfileCatalogEntry>, ProfileCatalogError> {
        let entries = fs::read_dir(&self.root)
            .map_err(|error| catalog_io_error("read profile catalog root", &self.root, error))?;
        let mut found_entries = 0_usize;
        let mut profiles = Vec::new();
        let mut identities = BTreeMap::<ProfileStorageId, PathBuf>::new();

        for entry in entries {
            found_entries = found_entries.saturating_add(1);
            if found_entries > MAX_PROFILE_CATALOG_DIRECTORY_ENTRIES {
                return Err(ProfileCatalogError::DirectoryEntryLimitExceeded {
                    found: found_entries,
                    limit: MAX_PROFILE_CATALOG_DIRECTORY_ENTRIES,
                });
            }

            let entry = entry.map_err(|error| {
                catalog_io_error("read profile catalog entry", &self.root, error)
            })?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|error| {
                catalog_io_error("inspect profile catalog entry", &path, error)
            })?;
            if file_type.is_symlink() {
                return Err(ProfileCatalogError::UnsupportedEntry { path });
            }
            if !file_type.is_dir() {
                continue;
            }

            let prospective = profiles.len().saturating_add(1);
            if prospective > MAX_DISCOVERED_PROFILES {
                return Err(ProfileCatalogError::ProfileLimitExceeded {
                    found: prospective,
                    limit: MAX_DISCOVERED_PROFILES,
                });
            }

            let storage_id = load_profile_storage_id(&path).map_err(|error| {
                ProfileCatalogError::Identity {
                    root: path.clone(),
                    error,
                }
            })?;
            if storage_id.is_none()
                && entry.file_name().as_os_str() != OsStr::new(LEGACY_DEFAULT_DIRECTORY_NAME)
            {
                return Err(ProfileCatalogError::MissingIdentity { root: path });
            }

            if let Some(storage_id) = storage_id {
                if let Some(first_root) = identities.insert(storage_id, path.clone()) {
                    return Err(ProfileCatalogError::DuplicateIdentity {
                        storage_id,
                        first_root,
                        second_root: path,
                    });
                }
            }

            profiles.push(ProfileCatalogEntry {
                root: entry.path(),
                storage_id,
            });
        }

        profiles.sort_by(|left, right| left.root.cmp(&right.root));
        Ok(profiles)
    }
}

pub(crate) fn load_or_create_profile_storage_id(
    lock: &ProfileLock,
) -> Result<ProfileStorageId, ProfileIdentityError> {
    lock.verify().map_err(ProfileIdentityError::Lock)?;
    let root = lock.root();
    if let Some(storage_id) = load_profile_storage_id(root)? {
        return Ok(storage_id);
    }

    let identity_directory = root.join(PROFILE_IDENTITY_DIRECTORY_NAME);
    match fs::create_dir(&identity_directory) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(identity_io_error(
                "create profile identity directory",
                &identity_directory,
                error,
            ));
        }
    }

    lock.verify().map_err(ProfileIdentityError::Lock)?;
    if load_profile_storage_id(root)?.is_some() {
        return Err(ProfileIdentityError::ConcurrentWrite {
            path: identity_directory,
        });
    }

    let storage_id = next_profile_storage_id()?;
    let record_path = identity_directory.join(identity_record_name(storage_id));
    lock.verify().map_err(ProfileIdentityError::Lock)?;
    match fs::create_dir(&record_path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(ProfileIdentityError::ConcurrentWrite { path: record_path });
        }
        Err(error) => {
            return Err(identity_io_error(
                "publish profile identity",
                &record_path,
                error,
            ));
        }
    }
    lock.verify().map_err(ProfileIdentityError::Lock)?;
    Ok(storage_id)
}

pub fn load_profile_storage_id(
    root: impl AsRef<Path>,
) -> Result<Option<ProfileStorageId>, ProfileIdentityError> {
    let root = root.as_ref();
    let identity_directory = root.join(PROFILE_IDENTITY_DIRECTORY_NAME);
    let entries = match fs::read_dir(&identity_directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(identity_io_error(
                "read profile identity directory",
                &identity_directory,
                error,
            ));
        }
    };

    let mut found = Vec::new();
    for entry in entries {
        let prospective = found.len().saturating_add(1);
        if prospective > MAX_PROFILE_IDENTITY_DIRECTORY_ENTRIES {
            return Err(ProfileIdentityError::DirectoryEntryLimitExceeded {
                found: prospective,
                limit: MAX_PROFILE_IDENTITY_DIRECTORY_ENTRIES,
            });
        }
        let entry = entry.map_err(|error| {
            identity_io_error(
                "read profile identity entry",
                &identity_directory,
                error,
            )
        })?;
        found.push(entry);
    }

    if found.is_empty() {
        return Ok(None);
    }
    if found.len() != 1 {
        return Err(ProfileIdentityError::Corrupt {
            path: identity_directory,
        });
    }

    let entry = found.pop().expect("one identity entry was validated");
    let path = entry.path();
    let file_type = entry
        .file_type()
        .map_err(|error| identity_io_error("inspect profile identity entry", &path, error))?;
    if !file_type.is_dir() || file_type.is_symlink() {
        return Err(ProfileIdentityError::Corrupt { path });
    }
    parse_identity_record_name(&entry.file_name(), &path).map(Some)
}

fn next_profile_storage_id() -> Result<ProfileStorageId, ProfileIdentityError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ProfileIdentityError::SystemClockBeforeUnixEpoch)?;
    let unix_nanos = u64::try_from(elapsed.as_nanos())
        .map_err(|_| ProfileIdentityError::StorageIdTimeRangeExceeded)?;
    let sequence = NEXT_PROFILE_STORAGE_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .map_err(|_| ProfileIdentityError::StorageIdSequenceExhausted)?;
    let creator = (u64::from(std::process::id()) << 32) ^ sequence;
    let value = (u128::from(unix_nanos) << 64) | u128::from(creator);
    Ok(ProfileStorageId(value))
}

fn identity_record_name(storage_id: ProfileStorageId) -> String {
    format!("{PROFILE_IDENTITY_RECORD_PREFIX}{storage_id}")
}

fn parse_identity_record_name(
    name: &OsStr,
    path: &Path,
) -> Result<ProfileStorageId, ProfileIdentityError> {
    let Some(name) = name.to_str() else {
        return Err(ProfileIdentityError::Corrupt {
            path: path.to_owned(),
        });
    };
    let Some((version, encoded)) = name.split_once('-') else {
        return Err(ProfileIdentityError::Corrupt {
            path: path.to_owned(),
        });
    };
    let Some(version) = version.strip_prefix('v') else {
        return Err(ProfileIdentityError::Corrupt {
            path: path.to_owned(),
        });
    };
    let version = version
        .parse::<u32>()
        .map_err(|_| ProfileIdentityError::Corrupt {
            path: path.to_owned(),
        })?;
    if version != PROFILE_IDENTITY_SCHEMA_VERSION {
        return Err(ProfileIdentityError::UnsupportedSchema {
            path: path.to_owned(),
            schema: version,
        });
    }
    if encoded.len() != PROFILE_IDENTITY_HEX_DIGITS
        || !encoded.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(ProfileIdentityError::Corrupt {
            path: path.to_owned(),
        });
    }
    let value = u128::from_str_radix(encoded, 16).map_err(|_| ProfileIdentityError::Corrupt {
        path: path.to_owned(),
    })?;
    if value == 0 {
        return Err(ProfileIdentityError::Corrupt {
            path: path.to_owned(),
        });
    }
    Ok(ProfileStorageId(value))
}

fn identity_io_error(
    operation: &'static str,
    path: &Path,
    error: io::Error,
) -> ProfileIdentityError {
    ProfileIdentityError::Io {
        operation,
        path: path.to_owned(),
        kind: error.kind(),
    }
}

fn catalog_io_error(
    operation: &'static str,
    path: &Path,
    error: io::Error,
) -> ProfileCatalogError {
    ProfileCatalogError::Io {
        operation,
        path: path.to_owned(),
        kind: error.kind(),
    }
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
            let root = std::env::temp_dir().join(format!(
                "zorya-profile-catalog-{label}-{}-{id}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            Self(root)
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

    fn fixture_identity(root: &Path, id: ProfileStorageId) {
        let directory = root.join(PROFILE_IDENTITY_DIRECTORY_NAME);
        fs::create_dir_all(directory.join(identity_record_name(id))).unwrap();
    }

    #[test]
    fn locked_profile_identity_bootstrap_is_stable_across_reacquisition() {
        let root = TestRoot::new("bootstrap");
        let first_lock = ProfileLock::acquire(root.path()).unwrap();
        let first = load_or_create_profile_storage_id(&first_lock).unwrap();
        assert_ne!(first.get(), 0);
        assert_eq!(load_profile_storage_id(root.path()).unwrap(), Some(first));
        first_lock.release().unwrap();

        let second_lock = ProfileLock::acquire(root.path()).unwrap();
        let second = load_or_create_profile_storage_id(&second_lock).unwrap();
        assert_eq!(second, first);
        second_lock.release().unwrap();
    }

    #[test]
    fn catalog_accepts_only_default_as_legacy_unidentified_profile() {
        let root = TestRoot::new("legacy");
        let catalog = ProfileCatalog::open(root.path()).unwrap();
        let default = root.path().join(LEGACY_DEFAULT_DIRECTORY_NAME);
        fs::create_dir_all(&default).unwrap();

        let entries = catalog.discover().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].root(), default);
        assert!(entries[0].requires_legacy_bootstrap());

        let other = root.path().join("Other");
        fs::create_dir_all(&other).unwrap();
        assert!(matches!(
            catalog.discover(),
            Err(ProfileCatalogError::MissingIdentity { root }) if root == other
        ));
    }

    #[test]
    fn catalog_discovers_persisted_identity_and_rejects_duplicates() {
        let root = TestRoot::new("duplicate");
        let catalog = ProfileCatalog::open(root.path()).unwrap();
        let id = ProfileStorageId(0x1234);
        let first = root.path().join("First");
        fixture_identity(&first, id);

        let entries = catalog.discover().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].storage_id(), Some(id));

        let second = root.path().join("Second");
        fixture_identity(&second, id);
        assert!(matches!(
            catalog.discover(),
            Err(ProfileCatalogError::DuplicateIdentity {
                storage_id,
                first_root,
                second_root,
            }) if storage_id == id
                && ((first_root == first && second_root == second)
                    || (first_root == second && second_root == first))
        ));
    }

    #[test]
    fn identity_loader_fails_closed_on_unknown_version_and_multiple_records() {
        let root = TestRoot::new("identity-errors");
        let directory = root.path().join(PROFILE_IDENTITY_DIRECTORY_NAME);
        fs::create_dir_all(directory.join(format!("v2-{:032x}", 1_u128))).unwrap();

        assert!(matches!(
            load_profile_storage_id(root.path()),
            Err(ProfileIdentityError::UnsupportedSchema { schema: 2, .. })
        ));

        fs::remove_dir_all(&directory).unwrap();
        fs::create_dir_all(directory.join(identity_record_name(ProfileStorageId(1)))).unwrap();
        fs::create_dir_all(directory.join(identity_record_name(ProfileStorageId(2)))).unwrap();
        assert!(matches!(
            load_profile_storage_id(root.path()),
            Err(ProfileIdentityError::Corrupt { .. })
        ));
    }

    #[test]
    fn catalog_profile_count_is_bounded() {
        let root = TestRoot::new("bound");
        let catalog = ProfileCatalog::open(root.path()).unwrap();
        for index in 0..=MAX_DISCOVERED_PROFILES {
            let profile = root.path().join(format!("Profile-{index:03}"));
            fixture_identity(&profile, ProfileStorageId((index + 1) as u128));
        }

        assert!(matches!(
            catalog.discover(),
            Err(ProfileCatalogError::ProfileLimitExceeded {
                found,
                limit: MAX_DISCOVERED_PROFILES,
            }) if found == MAX_DISCOVERED_PROFILES + 1
        ));
    }
}
