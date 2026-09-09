use crate::profile_lock::{ProfileLock, ProfileLockError};
use crate::profile_metadata::{
    ProfileDisplayName, ProfileDisplayNameError, ProfileMetadata, ProfileMetadataError,
    load_or_create_profile_metadata, load_profile_metadata, save_profile_metadata,
};
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
pub const PROFILE_READY_DIRECTORY_NAME: &str = ".zorya-profile.ready";

const LEGACY_DEFAULT_DIRECTORY_NAME: &str = "Default";
const GENERATED_PROFILE_DIRECTORY_PREFIX: &str = "Profile-";
const PROFILE_IDENTITY_RECORD_PREFIX: &str = "v1-";
const PROFILE_IDENTITY_HEX_DIGITS: usize = 32;
static NEXT_PROFILE_STORAGE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileStorageId(u128);

impl ProfileStorageId {
    pub const fn get(self) -> u128 {
        self.0
    }

    pub(crate) const fn from_raw(value: u128) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
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
                write!(
                    formatter,
                    "profile identity is malformed: {}",
                    path.display()
                )
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
            Self::SystemClockBeforeUnixEpoch => formatter
                .write_str("system clock precedes Unix epoch while allocating profile identity"),
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
    MissingMetadata {
        root: PathBuf,
    },
    Metadata {
        root: PathBuf,
        error: ProfileMetadataError,
    },
    RootIdentityMismatch {
        root: PathBuf,
        expected: ProfileStorageId,
        actual: ProfileStorageId,
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
            Self::MissingMetadata { root } => write!(
                formatter,
                "profile {} has no persisted display metadata",
                root.display()
            ),
            Self::Metadata { root, error } => write!(
                formatter,
                "failed to inspect profile metadata for {}: {error}",
                root.display()
            ),
            Self::RootIdentityMismatch {
                root,
                expected,
                actual,
            } => write!(
                formatter,
                "generated profile root {} encodes identity {expected}, but persisted identity is {actual}",
                root.display()
            ),
        }
    }
}

impl std::error::Error for ProfileCatalogError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Identity { error, .. } => Some(error),
            Self::Metadata { error, .. } => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileCatalogEntry {
    root: PathBuf,
    storage_id: Option<ProfileStorageId>,
    metadata: Option<ProfileMetadata>,
}

impl ProfileCatalogEntry {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub const fn storage_id(&self) -> Option<ProfileStorageId> {
        self.storage_id
    }

    pub const fn metadata(&self) -> Option<&ProfileMetadata> {
        self.metadata.as_ref()
    }

    pub fn display_name(&self) -> Option<&str> {
        self.metadata
            .as_ref()
            .map(|metadata| metadata.display_name().as_str())
    }

    pub const fn requires_legacy_bootstrap(&self) -> bool {
        self.storage_id.is_none() || self.metadata.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileCatalogDiscoverIntent {
    root: PathBuf,
}

impl ProfileCatalogDiscoverIntent {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn execute(&self) -> Result<Vec<ProfileCatalogEntry>, ProfileCatalogError> {
        ProfileCatalog::open(self.root.clone())?.discover()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileCatalogCreateIntent {
    catalog_root: PathBuf,
    display_name: ProfileDisplayName,
}

impl ProfileCatalogCreateIntent {
    pub fn new(
        catalog_root: impl Into<PathBuf>,
        display_name: impl Into<String>,
    ) -> Result<Self, ProfileDisplayNameError> {
        Ok(Self {
            catalog_root: catalog_root.into(),
            display_name: ProfileDisplayName::new(display_name)?,
        })
    }

    pub fn catalog_root(&self) -> &Path {
        &self.catalog_root
    }

    pub const fn display_name(&self) -> &ProfileDisplayName {
        &self.display_name
    }

    pub(crate) fn execute(&self) -> Result<ProfileCatalogEntry, ProfileCatalogCreateError> {
        create_profile(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileCatalogCreateError {
    Catalog(ProfileCatalogError),
    Identity(ProfileIdentityError),
    Metadata(ProfileMetadataError),
    Lock(ProfileLockError),
    Io {
        operation: &'static str,
        path: PathBuf,
        kind: io::ErrorKind,
    },
    RootCollision {
        path: PathBuf,
    },
    StorageIdentityCollision {
        storage_id: ProfileStorageId,
        existing_root: PathBuf,
    },
    LockRelease {
        initialization: Option<Box<ProfileCatalogCreateError>>,
        release: ProfileLockError,
    },
}

impl fmt::Display for ProfileCatalogCreateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Catalog(error) => error.fmt(formatter),
            Self::Identity(error) => error.fmt(formatter),
            Self::Metadata(error) => error.fmt(formatter),
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
            Self::RootCollision { path } => write!(
                formatter,
                "generated profile root already exists: {}",
                path.display()
            ),
            Self::StorageIdentityCollision {
                storage_id,
                existing_root,
            } => write!(
                formatter,
                "generated profile storage identity {storage_id} already belongs to {}",
                existing_root.display()
            ),
            Self::LockRelease {
                initialization,
                release,
            } => match initialization {
                Some(initialization) => write!(
                    formatter,
                    "profile creation failed: {initialization}; acquired profile lock also failed to release: {release}"
                ),
                None => write!(
                    formatter,
                    "profile creation initialized its staging root but failed to release the profile lock: {release}"
                ),
            },
        }
    }
}

impl std::error::Error for ProfileCatalogCreateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Catalog(error) => Some(error),
            Self::Identity(error) => Some(error),
            Self::Metadata(error) => Some(error),
            Self::Lock(error) => Some(error),
            Self::LockRelease {
                initialization: Some(initialization),
                ..
            } => Some(initialization.as_ref()),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileCatalogRenameIntent {
    root: PathBuf,
    storage_id: ProfileStorageId,
    expected_generation: u64,
    display_name: ProfileDisplayName,
}

impl ProfileCatalogRenameIntent {
    pub fn new(
        root: impl Into<PathBuf>,
        storage_id: ProfileStorageId,
        expected_generation: u64,
        display_name: impl Into<String>,
    ) -> Result<Self, ProfileDisplayNameError> {
        Ok(Self {
            root: root.into(),
            storage_id,
            expected_generation,
            display_name: ProfileDisplayName::new(display_name)?,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub const fn storage_id(&self) -> ProfileStorageId {
        self.storage_id
    }

    pub const fn expected_generation(&self) -> u64 {
        self.expected_generation
    }

    pub const fn display_name(&self) -> &ProfileDisplayName {
        &self.display_name
    }

    pub(crate) fn execute(&self) -> Result<ProfileMetadata, ProfileCatalogRenameError> {
        rename_profile(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileCatalogRenameError {
    Lock(ProfileLockError),
    Metadata(ProfileMetadataError),
    LockRelease {
        operation: Option<Box<ProfileMetadataError>>,
        committed: Option<ProfileMetadata>,
        release: ProfileLockError,
    },
}

impl fmt::Display for ProfileCatalogRenameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lock(error) => error.fmt(formatter),
            Self::Metadata(error) => error.fmt(formatter),
            Self::LockRelease {
                operation,
                committed,
                release,
            } => {
                if let Some(operation) = operation {
                    write!(
                        formatter,
                        "profile rename failed: {operation}; acquired profile lock also failed to release: {release}"
                    )
                } else if let Some(committed) = committed {
                    write!(
                        formatter,
                        "profile rename committed metadata generation {} but failed to release the profile lock: {release}",
                        committed.generation()
                    )
                } else {
                    write!(
                        formatter,
                        "profile rename failed to release its acquired profile lock: {release}"
                    )
                }
            }
        }
    }
}

impl std::error::Error for ProfileCatalogRenameError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Lock(error) => Some(error),
            Self::Metadata(error) => Some(error),
            Self::LockRelease {
                operation: Some(operation),
                ..
            } => Some(operation.as_ref()),
            _ => None,
        }
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
        let metadata = fs::symlink_metadata(&root)
            .map_err(|error| catalog_io_error("inspect profile catalog root", &root, error))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(ProfileCatalogError::UnsupportedEntry { path: root });
        }
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
            let file_type = entry
                .file_type()
                .map_err(|error| catalog_io_error("inspect profile catalog entry", &path, error))?;
            if file_type.is_symlink() {
                return Err(ProfileCatalogError::UnsupportedEntry { path });
            }
            if !file_type.is_dir() {
                continue;
            }

            let generated_storage_id = generated_profile_root_storage_id(&entry.file_name());
            if generated_storage_id.is_some() && !profile_publication_ready(&path)? {
                continue;
            }

            let prospective = profiles.len().saturating_add(1);
            if prospective > MAX_DISCOVERED_PROFILES {
                return Err(ProfileCatalogError::ProfileLimitExceeded {
                    found: prospective,
                    limit: MAX_DISCOVERED_PROFILES,
                });
            }

            let storage_id =
                load_profile_storage_id(&path).map_err(|error| ProfileCatalogError::Identity {
                    root: path.clone(),
                    error,
                })?;
            let legacy_default =
                entry.file_name().as_os_str() == OsStr::new(LEGACY_DEFAULT_DIRECTORY_NAME);
            if storage_id.is_none() && !legacy_default {
                return Err(ProfileCatalogError::MissingIdentity { root: path });
            }

            if let (Some(expected), Some(actual)) = (generated_storage_id, storage_id)
                && expected != actual
            {
                return Err(ProfileCatalogError::RootIdentityMismatch {
                    root: path,
                    expected,
                    actual,
                });
            }

            let metadata = match storage_id {
                Some(storage_id) => load_profile_metadata(&path, storage_id)
                    .map_err(|error| ProfileCatalogError::Metadata {
                        root: path.clone(),
                        error,
                    })?,
                None => None,
            };
            if metadata.is_none() && !legacy_default {
                return Err(ProfileCatalogError::MissingMetadata { root: path });
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
                metadata,
            });
        }

        profiles.sort_by(|left, right| left.root.cmp(&right.root));
        Ok(profiles)
    }
}

fn generated_profile_root_name(storage_id: ProfileStorageId) -> String {
    format!("{GENERATED_PROFILE_DIRECTORY_PREFIX}{storage_id}")
}

fn generated_profile_root_storage_id(name: &OsStr) -> Option<ProfileStorageId> {
    let name = name.to_str()?;
    let encoded = name.strip_prefix(GENERATED_PROFILE_DIRECTORY_PREFIX)?;
    if encoded.len() != PROFILE_IDENTITY_HEX_DIGITS
        || !encoded.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return None;
    }
    let value = u128::from_str_radix(encoded, 16).ok()?;
    ProfileStorageId::from_raw(value)
}

fn profile_publication_ready(root: &Path) -> Result<bool, ProfileCatalogError> {
    let marker = root.join(PROFILE_READY_DIRECTORY_NAME);
    let metadata = match fs::symlink_metadata(&marker) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(catalog_io_error(
                "inspect profile publication marker",
                &marker,
                error,
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ProfileCatalogError::UnsupportedEntry { path: marker });
    }
    let mut entries = fs::read_dir(&marker)
        .map_err(|error| catalog_io_error("read profile publication marker", &marker, error))?;
    if entries.next().is_some() {
        return Err(ProfileCatalogError::UnsupportedEntry { path: marker });
    }
    Ok(true)
}

fn create_profile(
    intent: &ProfileCatalogCreateIntent,
) -> Result<ProfileCatalogEntry, ProfileCatalogCreateError> {
    let catalog = ProfileCatalog::open(intent.catalog_root.clone())
        .map_err(ProfileCatalogCreateError::Catalog)?;
    let existing = catalog
        .discover()
        .map_err(ProfileCatalogCreateError::Catalog)?;
    if existing.len() >= MAX_DISCOVERED_PROFILES {
        return Err(ProfileCatalogCreateError::Catalog(
            ProfileCatalogError::ProfileLimitExceeded {
                found: existing.len().saturating_add(1),
                limit: MAX_DISCOVERED_PROFILES,
            },
        ));
    }

    let storage_id = next_profile_storage_id().map_err(ProfileCatalogCreateError::Identity)?;
    if let Some(existing) = existing
        .iter()
        .find(|entry| entry.storage_id() == Some(storage_id))
    {
        return Err(ProfileCatalogCreateError::StorageIdentityCollision {
            storage_id,
            existing_root: existing.root().to_owned(),
        });
    }
    let root = catalog.root.join(generated_profile_root_name(storage_id));
    match fs::create_dir(&root) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(ProfileCatalogCreateError::RootCollision { path: root });
        }
        Err(error) => {
            return Err(profile_create_io_error(
                "create profile staging root",
                &root,
                error,
            ));
        }
    }

    let lock = ProfileLock::acquire(root.clone()).map_err(ProfileCatalogCreateError::Lock)?;
    let initialization = (|| {
        publish_profile_storage_id(&lock, storage_id)
            .map_err(ProfileCatalogCreateError::Identity)?;
        load_or_create_profile_metadata(&lock, storage_id, intent.display_name.clone())
            .map_err(ProfileCatalogCreateError::Metadata)
    })();

    let metadata = match initialization {
        Ok(metadata) => match lock.release() {
            Ok(()) => metadata,
            Err(release) => {
                return Err(ProfileCatalogCreateError::LockRelease {
                    initialization: None,
                    release,
                });
            }
        },
        Err(initialization) => {
            return match lock.release() {
                Ok(()) => Err(initialization),
                Err(release) => Err(ProfileCatalogCreateError::LockRelease {
                    initialization: Some(Box::new(initialization)),
                    release,
                }),
            };
        }
    };

    let ready = root.join(PROFILE_READY_DIRECTORY_NAME);
    match fs::create_dir(&ready) {
        Ok(()) => {}
        Err(error) => {
            return Err(profile_create_io_error(
                "publish completed profile root",
                &ready,
                error,
            ));
        }
    }

    Ok(ProfileCatalogEntry {
        root,
        storage_id: Some(storage_id),
        metadata: Some(metadata),
    })
}

fn rename_profile(
    intent: &ProfileCatalogRenameIntent,
) -> Result<ProfileMetadata, ProfileCatalogRenameError> {
    let lock =
        ProfileLock::acquire(intent.root.clone()).map_err(ProfileCatalogRenameError::Lock)?;
    let operation = save_profile_metadata(
        &lock,
        intent.storage_id,
        intent.expected_generation,
        intent.display_name.clone(),
    );
    let release = lock.release();

    match (operation, release) {
        (Ok(metadata), Ok(())) => Ok(metadata),
        (Err(operation), Ok(())) => Err(ProfileCatalogRenameError::Metadata(operation)),
        (Err(operation), Err(release)) => Err(ProfileCatalogRenameError::LockRelease {
            operation: Some(Box::new(operation)),
            committed: None,
            release,
        }),
        (Ok(metadata), Err(release)) => Err(ProfileCatalogRenameError::LockRelease {
            operation: None,
            committed: Some(metadata),
            release,
        }),
    }
}

fn profile_create_io_error(
    operation: &'static str,
    path: &Path,
    error: io::Error,
) -> ProfileCatalogCreateError {
    ProfileCatalogCreateError::Io {
        operation,
        path: path.to_owned(),
        kind: error.kind(),
    }
}

pub(crate) fn load_or_create_profile_storage_id(
    lock: &ProfileLock,
) -> Result<ProfileStorageId, ProfileIdentityError> {
    lock.verify().map_err(ProfileIdentityError::Lock)?;
    if let Some(storage_id) = load_profile_storage_id(lock.root())? {
        return Ok(storage_id);
    }
    let storage_id = next_profile_storage_id()?;
    publish_profile_storage_id(lock, storage_id)?;
    Ok(storage_id)
}

pub(crate) fn publish_profile_storage_id(
    lock: &ProfileLock,
    storage_id: ProfileStorageId,
) -> Result<(), ProfileIdentityError> {
    lock.verify().map_err(ProfileIdentityError::Lock)?;
    let root = lock.root();
    if load_profile_storage_id(root)?.is_some() {
        return Err(ProfileIdentityError::ConcurrentWrite {
            path: root.join(PROFILE_IDENTITY_DIRECTORY_NAME),
        });
    }

    let identity_directory = root.join(PROFILE_IDENTITY_DIRECTORY_NAME);
    match fs::create_dir(&identity_directory) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(&identity_directory)
                .map_err(|error| identity_io_error("inspect profile identity directory", &identity_directory, error))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(ProfileIdentityError::Corrupt {
                    path: identity_directory,
                });
            }
        }
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
    Ok(())
}

pub fn load_profile_storage_id(
    root: impl AsRef<Path>,
) -> Result<Option<ProfileStorageId>, ProfileIdentityError> {
    let root = root.as_ref();
    let identity_directory = root.join(PROFILE_IDENTITY_DIRECTORY_NAME);
    let metadata = match fs::symlink_metadata(&identity_directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(identity_io_error(
                "inspect profile identity directory",
                &identity_directory,
                error,
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ProfileIdentityError::Corrupt {
            path: identity_directory,
        });
    }

    let entries = fs::read_dir(&identity_directory).map_err(|error| {
        identity_io_error(
            "read profile identity directory",
            &identity_directory,
            error,
        )
    })?;

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
            identity_io_error("read profile identity entry", &identity_directory, error)
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

fn catalog_io_error(operation: &'static str, path: &Path, error: io::Error) -> ProfileCatalogError {
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

    fn fixture_profile(root: &Path, id: ProfileStorageId, display_name: &str) {
        let lock = ProfileLock::acquire(root).unwrap();
        publish_profile_storage_id(&lock, id).unwrap();
        load_or_create_profile_metadata(
            &lock,
            id,
            ProfileDisplayName::new(display_name).unwrap(),
        )
        .unwrap();
        lock.release().unwrap();
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
        fixture_profile(&first, id, "First");

        let entries = catalog.discover().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].storage_id(), Some(id));
        assert_eq!(entries[0].display_name(), Some("First"));

        let second = root.path().join("Second");
        fixture_profile(&second, id, "Second");
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

    #[cfg(unix)]
    #[test]
    fn identity_loader_rejects_symlinked_identity_directory() {
        use std::os::unix::fs::symlink;

        let root = TestRoot::new("identity-symlink");
        let target = root.path().join("identity-target");
        fs::create_dir_all(target.join(identity_record_name(ProfileStorageId(7)))).unwrap();
        symlink(&target, root.path().join(PROFILE_IDENTITY_DIRECTORY_NAME)).unwrap();

        assert!(matches!(
            load_profile_storage_id(root.path()),
            Err(ProfileIdentityError::Corrupt { .. })
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
    fn generated_profile_is_invisible_until_ready_marker_is_published() {
        let root = TestRoot::new("publication");
        let catalog = ProfileCatalog::open(root.path()).unwrap();
        let id = ProfileStorageId(0x42);
        let profile = root.path().join(generated_profile_root_name(id));
        fixture_profile(&profile, id, "Hidden");

        assert!(catalog.discover().unwrap().is_empty());

        fs::create_dir(profile.join(PROFILE_READY_DIRECTORY_NAME)).unwrap();
        let entries = catalog.discover().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].storage_id(), Some(id));
        assert_eq!(entries[0].display_name(), Some("Hidden"));
    }

    #[test]
    fn profile_create_publishes_complete_root_and_metadata() {
        let root = TestRoot::new("create");
        let intent = ProfileCatalogCreateIntent::new(root.path(), "Personal").unwrap();
        let created = intent.execute().unwrap();

        let storage_id = created.storage_id().unwrap();
        assert_eq!(created.display_name(), Some("Personal"));
        assert_eq!(
            created.root().file_name().and_then(|name| name.to_str()),
            Some(generated_profile_root_name(storage_id).as_str())
        );
        assert!(created.root().join(PROFILE_READY_DIRECTORY_NAME).is_dir());

        let entries = ProfileCatalog::open(root.path()).unwrap().discover().unwrap();
        assert_eq!(entries, vec![created]);
    }

    #[test]
    fn rename_updates_metadata_generation_without_moving_profile_root() {
        let root = TestRoot::new("rename");
        let created = ProfileCatalogCreateIntent::new(root.path(), "Personal")
            .unwrap()
            .execute()
            .unwrap();
        let storage_id = created.storage_id().unwrap();
        let generation = created.metadata().unwrap().generation();
        let profile_root = created.root().to_owned();

        let renamed = ProfileCatalogRenameIntent::new(
            &profile_root,
            storage_id,
            generation,
            "Work",
        )
        .unwrap()
        .execute()
        .unwrap();
        assert_eq!(renamed.generation(), generation + 1);
        assert_eq!(renamed.display_name().as_str(), "Work");
        assert!(profile_root.is_dir());

        let stale = ProfileCatalogRenameIntent::new(
            &profile_root,
            storage_id,
            generation,
            "Stale",
        )
        .unwrap()
        .execute()
        .unwrap_err();
        assert!(matches!(
            stale,
            ProfileCatalogRenameError::Metadata(ProfileMetadataError::StaleGeneration {
                expected,
                actual,
            }) if expected == generation && actual == generation + 1
        ));

        let entries = ProfileCatalog::open(root.path()).unwrap().discover().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].root(), profile_root);
        assert_eq!(entries[0].display_name(), Some("Work"));
    }

    #[test]
    fn rename_rejects_wrong_persisted_storage_identity() {
        let root = TestRoot::new("rename-identity");
        let created = ProfileCatalogCreateIntent::new(root.path(), "Personal")
            .unwrap()
            .execute()
            .unwrap();
        let storage_id = created.storage_id().unwrap();
        let wrong = ProfileStorageId::from_raw(storage_id.get().wrapping_add(1)).unwrap();
        let error = ProfileCatalogRenameIntent::new(
            created.root(),
            wrong,
            created.metadata().unwrap().generation(),
            "Wrong",
        )
        .unwrap()
        .execute()
        .unwrap_err();

        assert!(matches!(
            error,
            ProfileCatalogRenameError::Metadata(
                ProfileMetadataError::StorageIdentityMismatch {
                    expected,
                    actual,
                }
            ) if expected == wrong && actual == storage_id
        ));
        assert_eq!(
            ProfileCatalog::open(root.path())
                .unwrap()
                .discover()
                .unwrap()[0]
                .display_name(),
            Some("Personal")
        );
    }

    #[cfg(unix)]
    #[test]
    fn catalog_open_rejects_redirected_root() {
        use std::os::unix::fs::symlink;

        let root = TestRoot::new("catalog-symlink");
        let target = root.path().join("target");
        let redirected = root.path().join("redirected");
        fs::create_dir_all(&target).unwrap();
        symlink(&target, &redirected).unwrap();

        assert!(matches!(
            ProfileCatalog::open(&redirected),
            Err(ProfileCatalogError::UnsupportedEntry { path }) if path == redirected
        ));
    }

    #[test]
    fn catalog_profile_count_is_bounded() {
        let root = TestRoot::new("bound");
        let catalog = ProfileCatalog::open(root.path()).unwrap();
        for index in 0..=MAX_DISCOVERED_PROFILES {
            let profile = root.path().join(format!("Legacy-{index:03}"));
            fixture_profile(
                &profile,
                ProfileStorageId((index + 1) as u128),
                &format!("Profile {index}"),
            );
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
