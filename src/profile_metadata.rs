use crate::profile_catalog::{ProfileIdentityError, ProfileStorageId, load_profile_storage_id};
use crate::profile_lock::{ProfileLock, ProfileLockError};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const PROFILE_METADATA_SCHEMA_VERSION: u32 = 1;
pub const PROFILE_METADATA_DIRECTORY_NAME: &str = ".zorya-profile.meta";
pub const MAX_PROFILE_METADATA_DIRECTORY_ENTRIES: usize = 4;
pub const MAX_PROFILE_DISPLAY_NAME_BYTES: usize = 64;

const PROFILE_METADATA_RECORD_PREFIX: &str = "v1-g";
const PROFILE_METADATA_GENERATION_HEX_DIGITS: usize = 16;
const PROFILE_METADATA_STORAGE_ID_HEX_DIGITS: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileDisplayNameError {
    Empty,
    TooLong { bytes: usize, limit: usize },
    LeadingOrTrailingWhitespace,
    ControlCharacter,
}

impl fmt::Display for ProfileDisplayNameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("profile display name cannot be empty"),
            Self::TooLong { bytes, limit } => write!(
                formatter,
                "profile display name is {bytes} bytes; limit is {limit}"
            ),
            Self::LeadingOrTrailingWhitespace => {
                formatter.write_str("profile display name cannot start or end with whitespace")
            }
            Self::ControlCharacter => {
                formatter.write_str("profile display name cannot contain control characters")
            }
        }
    }
}

impl std::error::Error for ProfileDisplayNameError {}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileDisplayName(String);

impl ProfileDisplayName {
    pub fn new(value: impl Into<String>) -> Result<Self, ProfileDisplayNameError> {
        let value = value.into();
        if value.is_empty() {
            return Err(ProfileDisplayNameError::Empty);
        }
        if value.len() > MAX_PROFILE_DISPLAY_NAME_BYTES {
            return Err(ProfileDisplayNameError::TooLong {
                bytes: value.len(),
                limit: MAX_PROFILE_DISPLAY_NAME_BYTES,
            });
        }
        if value.trim() != value {
            return Err(ProfileDisplayNameError::LeadingOrTrailingWhitespace);
        }
        if value.chars().any(char::is_control) {
            return Err(ProfileDisplayNameError::ControlCharacter);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Display for ProfileDisplayName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileMetadata {
    storage_id: ProfileStorageId,
    generation: u64,
    display_name: ProfileDisplayName,
}

impl ProfileMetadata {
    pub const fn storage_id(&self) -> ProfileStorageId {
        self.storage_id
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub const fn display_name(&self) -> &ProfileDisplayName {
        &self.display_name
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileMetadataError {
    Lock(ProfileLockError),
    Identity(ProfileIdentityError),
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
    StorageIdentityMismatch {
        expected: ProfileStorageId,
        actual: ProfileStorageId,
    },
    Missing,
    StaleGeneration {
        expected: u64,
        actual: u64,
    },
    GenerationExhausted,
    ConcurrentWrite {
        path: PathBuf,
    },
    DisplayName(ProfileDisplayNameError),
}

impl fmt::Display for ProfileMetadataError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lock(error) => error.fmt(formatter),
            Self::Identity(error) => error.fmt(formatter),
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
                "profile metadata directory contains {found} entries; limit is {limit}"
            ),
            Self::Corrupt { path } => {
                write!(
                    formatter,
                    "profile metadata is malformed: {}",
                    path.display()
                )
            }
            Self::UnsupportedSchema { path, schema } => write!(
                formatter,
                "profile metadata {} uses unsupported schema {schema}",
                path.display()
            ),
            Self::StorageIdentityMismatch { expected, actual } => write!(
                formatter,
                "profile metadata storage identity {actual} does not match expected {expected}"
            ),
            Self::Missing => formatter.write_str("profile metadata is missing"),
            Self::StaleGeneration { expected, actual } => write!(
                formatter,
                "profile metadata generation is stale; expected {expected}, current is {actual}"
            ),
            Self::GenerationExhausted => {
                formatter.write_str("profile metadata generation space is exhausted")
            }
            Self::ConcurrentWrite { path } => write!(
                formatter,
                "profile metadata was created concurrently: {}",
                path.display()
            ),
            Self::DisplayName(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ProfileMetadataError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Lock(error) => Some(error),
            Self::Identity(error) => Some(error),
            Self::DisplayName(error) => Some(error),
            _ => None,
        }
    }
}

pub fn load_profile_metadata(
    root: impl AsRef<Path>,
    expected_storage_id: ProfileStorageId,
) -> Result<Option<ProfileMetadata>, ProfileMetadataError> {
    let records = scan_metadata_records(root.as_ref(), expected_storage_id)?;
    Ok(records
        .into_iter()
        .next_back()
        .map(|(_, (_, metadata))| metadata))
}

pub(crate) fn load_or_create_profile_metadata(
    lock: &ProfileLock,
    storage_id: ProfileStorageId,
    default_display_name: ProfileDisplayName,
) -> Result<ProfileMetadata, ProfileMetadataError> {
    verify_storage_identity(lock, storage_id)?;
    if let Some(metadata) = load_profile_metadata(lock.root(), storage_id)? {
        return Ok(metadata);
    }

    let directory = ensure_metadata_directory(lock.root())?;
    lock.verify().map_err(ProfileMetadataError::Lock)?;
    if load_profile_metadata(lock.root(), storage_id)?.is_some() {
        return Err(ProfileMetadataError::ConcurrentWrite { path: directory });
    }

    publish_metadata_record(lock, storage_id, 1, default_display_name)
}

pub(crate) fn save_profile_metadata(
    lock: &ProfileLock,
    storage_id: ProfileStorageId,
    expected_generation: u64,
    display_name: ProfileDisplayName,
) -> Result<ProfileMetadata, ProfileMetadataError> {
    verify_storage_identity(lock, storage_id)?;
    let records = scan_metadata_records(lock.root(), storage_id)?;
    let Some((&actual_generation, (_, current))) = records.last_key_value() else {
        return Err(ProfileMetadataError::Missing);
    };
    if actual_generation != expected_generation {
        return Err(ProfileMetadataError::StaleGeneration {
            expected: expected_generation,
            actual: actual_generation,
        });
    }

    let next_generation = actual_generation
        .checked_add(1)
        .ok_or(ProfileMetadataError::GenerationExhausted)?;

    for (generation, (path, _)) in &records {
        if *generation != actual_generation {
            lock.verify().map_err(ProfileMetadataError::Lock)?;
            fs::remove_dir(path)
                .map_err(|error| metadata_io_error("remove old profile metadata", path, error))?;
        }
    }

    lock.verify().map_err(ProfileMetadataError::Lock)?;
    let reloaded =
        load_profile_metadata(lock.root(), storage_id)?.ok_or(ProfileMetadataError::Missing)?;
    if reloaded.generation != current.generation {
        return Err(ProfileMetadataError::StaleGeneration {
            expected: current.generation,
            actual: reloaded.generation,
        });
    }

    publish_metadata_record(lock, storage_id, next_generation, display_name)
}

fn verify_storage_identity(
    lock: &ProfileLock,
    expected_storage_id: ProfileStorageId,
) -> Result<(), ProfileMetadataError> {
    lock.verify().map_err(ProfileMetadataError::Lock)?;
    let actual = load_profile_storage_id(lock.root())
        .map_err(ProfileMetadataError::Identity)?
        .ok_or(ProfileMetadataError::Identity(
            ProfileIdentityError::Corrupt {
                path: lock.root().to_owned(),
            },
        ))?;
    if actual != expected_storage_id {
        return Err(ProfileMetadataError::StorageIdentityMismatch {
            expected: expected_storage_id,
            actual,
        });
    }
    Ok(())
}

fn ensure_metadata_directory(root: &Path) -> Result<PathBuf, ProfileMetadataError> {
    let directory = root.join(PROFILE_METADATA_DIRECTORY_NAME);
    match fs::create_dir(&directory) {
        Ok(()) => Ok(directory),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(&directory).map_err(|error| {
                metadata_io_error("inspect profile metadata directory", &directory, error)
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(ProfileMetadataError::Corrupt { path: directory });
            }
            Ok(directory)
        }
        Err(error) => Err(metadata_io_error(
            "create profile metadata directory",
            &directory,
            error,
        )),
    }
}

fn publish_metadata_record(
    lock: &ProfileLock,
    storage_id: ProfileStorageId,
    generation: u64,
    display_name: ProfileDisplayName,
) -> Result<ProfileMetadata, ProfileMetadataError> {
    let directory = ensure_metadata_directory(lock.root())?;
    let entries = fs::read_dir(&directory)
        .map_err(|error| metadata_io_error("read profile metadata directory", &directory, error))?;
    let mut count = 0_usize;
    for entry in entries {
        entry
            .map_err(|error| metadata_io_error("read profile metadata entry", &directory, error))?;
        count = count.saturating_add(1);
        if count >= MAX_PROFILE_METADATA_DIRECTORY_ENTRIES {
            return Err(ProfileMetadataError::DirectoryEntryLimitExceeded {
                found: count,
                limit: MAX_PROFILE_METADATA_DIRECTORY_ENTRIES,
            });
        }
    }

    let metadata = ProfileMetadata {
        storage_id,
        generation,
        display_name,
    };
    let path = directory.join(metadata_record_name(&metadata));
    lock.verify().map_err(ProfileMetadataError::Lock)?;
    match fs::create_dir(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return Err(ProfileMetadataError::ConcurrentWrite { path });
        }
        Err(error) => {
            return Err(metadata_io_error("publish profile metadata", &path, error));
        }
    }
    lock.verify().map_err(ProfileMetadataError::Lock)?;
    Ok(metadata)
}

fn scan_metadata_records(
    root: &Path,
    expected_storage_id: ProfileStorageId,
) -> Result<BTreeMap<u64, (PathBuf, ProfileMetadata)>, ProfileMetadataError> {
    let directory = root.join(PROFILE_METADATA_DIRECTORY_NAME);
    let metadata = match fs::symlink_metadata(&directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => {
            return Err(metadata_io_error(
                "inspect profile metadata directory",
                &directory,
                error,
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ProfileMetadataError::Corrupt { path: directory });
    }

    let entries = fs::read_dir(&directory)
        .map_err(|error| metadata_io_error("read profile metadata directory", &directory, error))?;
    let mut records = BTreeMap::new();
    let mut found = 0_usize;
    for entry in entries {
        found = found.saturating_add(1);
        if found > MAX_PROFILE_METADATA_DIRECTORY_ENTRIES {
            return Err(ProfileMetadataError::DirectoryEntryLimitExceeded {
                found,
                limit: MAX_PROFILE_METADATA_DIRECTORY_ENTRIES,
            });
        }
        let entry = entry
            .map_err(|error| metadata_io_error("read profile metadata entry", &directory, error))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| metadata_io_error("inspect profile metadata entry", &path, error))?;
        if file_type.is_symlink() || !file_type.is_dir() {
            return Err(ProfileMetadataError::Corrupt { path });
        }
        let metadata = parse_metadata_record_name(&entry.file_name(), &path)?;
        if metadata.storage_id != expected_storage_id {
            return Err(ProfileMetadataError::StorageIdentityMismatch {
                expected: expected_storage_id,
                actual: metadata.storage_id,
            });
        }
        let generation = metadata.generation;
        if records
            .insert(generation, (path.clone(), metadata))
            .is_some()
        {
            return Err(ProfileMetadataError::Corrupt { path });
        }
    }
    Ok(records)
}

fn metadata_record_name(metadata: &ProfileMetadata) -> String {
    format!(
        "{PROFILE_METADATA_RECORD_PREFIX}{:016x}-i{}-n{}",
        metadata.generation,
        metadata.storage_id,
        encode_hex(metadata.display_name.as_str().as_bytes())
    )
}

fn parse_metadata_record_name(
    name: &OsStr,
    path: &Path,
) -> Result<ProfileMetadata, ProfileMetadataError> {
    let Some(name) = name.to_str() else {
        return Err(ProfileMetadataError::Corrupt {
            path: path.to_owned(),
        });
    };
    let Some((version, rest)) = name.split_once("-g") else {
        return Err(ProfileMetadataError::Corrupt {
            path: path.to_owned(),
        });
    };
    let Some(version) = version.strip_prefix('v') else {
        return Err(ProfileMetadataError::Corrupt {
            path: path.to_owned(),
        });
    };
    let version = version
        .parse::<u32>()
        .map_err(|_| ProfileMetadataError::Corrupt {
            path: path.to_owned(),
        })?;
    if version != PROFILE_METADATA_SCHEMA_VERSION {
        return Err(ProfileMetadataError::UnsupportedSchema {
            path: path.to_owned(),
            schema: version,
        });
    }

    let Some((generation, rest)) = rest.split_once("-i") else {
        return Err(ProfileMetadataError::Corrupt {
            path: path.to_owned(),
        });
    };
    if generation.len() != PROFILE_METADATA_GENERATION_HEX_DIGITS
        || !generation.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(ProfileMetadataError::Corrupt {
            path: path.to_owned(),
        });
    }
    let generation =
        u64::from_str_radix(generation, 16).map_err(|_| ProfileMetadataError::Corrupt {
            path: path.to_owned(),
        })?;
    if generation == 0 {
        return Err(ProfileMetadataError::Corrupt {
            path: path.to_owned(),
        });
    }

    let Some((storage_id, encoded_name)) = rest.split_once("-n") else {
        return Err(ProfileMetadataError::Corrupt {
            path: path.to_owned(),
        });
    };
    if storage_id.len() != PROFILE_METADATA_STORAGE_ID_HEX_DIGITS
        || !storage_id.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(ProfileMetadataError::Corrupt {
            path: path.to_owned(),
        });
    }
    let storage_id =
        u128::from_str_radix(storage_id, 16).map_err(|_| ProfileMetadataError::Corrupt {
            path: path.to_owned(),
        })?;
    if storage_id == 0 {
        return Err(ProfileMetadataError::Corrupt {
            path: path.to_owned(),
        });
    }
    let storage_id =
        ProfileStorageId::from_raw(storage_id).ok_or_else(|| ProfileMetadataError::Corrupt {
            path: path.to_owned(),
        })?;

    let bytes = decode_hex(encoded_name, path)?;
    let display_name = String::from_utf8(bytes).map_err(|_| ProfileMetadataError::Corrupt {
        path: path.to_owned(),
    })?;
    let display_name =
        ProfileDisplayName::new(display_name).map_err(ProfileMetadataError::DisplayName)?;

    Ok(ProfileMetadata {
        storage_id,
        generation,
        display_name,
    })
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn decode_hex(encoded: &str, path: &Path) -> Result<Vec<u8>, ProfileMetadataError> {
    if encoded.is_empty()
        || encoded.len() > MAX_PROFILE_DISPLAY_NAME_BYTES * 2
        || encoded.len() % 2 != 0
        || !encoded.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(ProfileMetadataError::Corrupt {
            path: path.to_owned(),
        });
    }
    let mut bytes = Vec::with_capacity(encoded.len() / 2);
    for pair in encoded.as_bytes().chunks_exact(2) {
        let text = std::str::from_utf8(pair).expect("ASCII hex pair is valid UTF-8");
        let byte = u8::from_str_radix(text, 16).map_err(|_| ProfileMetadataError::Corrupt {
            path: path.to_owned(),
        })?;
        bytes.push(byte);
    }
    Ok(bytes)
}

fn metadata_io_error(
    operation: &'static str,
    path: &Path,
    error: io::Error,
) -> ProfileMetadataError {
    ProfileMetadataError::Io {
        operation,
        path: path.to_owned(),
        kind: error.kind(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile_catalog::load_or_create_profile_storage_id;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(1);

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new(label: &str) -> Self {
            let id = NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "zorya-profile-metadata-{label}-{}-{id}",
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

    fn locked_identity(label: &str) -> (TestRoot, ProfileLock, ProfileStorageId) {
        let root = TestRoot::new(label);
        let lock = ProfileLock::acquire(root.path()).unwrap();
        let storage_id = load_or_create_profile_storage_id(&lock).unwrap();
        (root, lock, storage_id)
    }

    #[test]
    fn display_name_validation_is_bounded_and_user_text_safe() {
        assert!(ProfileDisplayName::new("Personal").is_ok());
        assert_eq!(
            ProfileDisplayName::new(""),
            Err(ProfileDisplayNameError::Empty)
        );
        assert_eq!(
            ProfileDisplayName::new(" Personal"),
            Err(ProfileDisplayNameError::LeadingOrTrailingWhitespace)
        );
        assert_eq!(
            ProfileDisplayName::new("Bad\nName"),
            Err(ProfileDisplayNameError::ControlCharacter)
        );
        assert!(matches!(
            ProfileDisplayName::new("x".repeat(MAX_PROFILE_DISPLAY_NAME_BYTES + 1)),
            Err(ProfileDisplayNameError::TooLong { .. })
        ));
    }

    #[test]
    fn metadata_bootstrap_and_rename_are_generation_safe() {
        let (_root, lock, storage_id) = locked_identity("generation");
        let first = load_or_create_profile_metadata(
            &lock,
            storage_id,
            ProfileDisplayName::new("Default").unwrap(),
        )
        .unwrap();
        assert_eq!(first.generation(), 1);
        assert_eq!(first.display_name().as_str(), "Default");

        let second = save_profile_metadata(
            &lock,
            storage_id,
            first.generation(),
            ProfileDisplayName::new("Personal").unwrap(),
        )
        .unwrap();
        assert_eq!(second.generation(), 2);
        assert_eq!(second.display_name().as_str(), "Personal");
        assert_eq!(
            save_profile_metadata(
                &lock,
                storage_id,
                first.generation(),
                ProfileDisplayName::new("Stale").unwrap(),
            ),
            Err(ProfileMetadataError::StaleGeneration {
                expected: 1,
                actual: 2,
            })
        );
        lock.release().unwrap();
    }

    #[test]
    fn repeated_rename_keeps_metadata_directory_bounded() {
        let (root, lock, storage_id) = locked_identity("retention");
        let mut metadata = load_or_create_profile_metadata(
            &lock,
            storage_id,
            ProfileDisplayName::new("Profile 0").unwrap(),
        )
        .unwrap();

        for index in 1..=12 {
            metadata = save_profile_metadata(
                &lock,
                storage_id,
                metadata.generation(),
                ProfileDisplayName::new(format!("Profile {index}")).unwrap(),
            )
            .unwrap();
        }

        let entry_count = fs::read_dir(root.path().join(PROFILE_METADATA_DIRECTORY_NAME))
            .unwrap()
            .count();
        assert!(entry_count <= 2);
        assert_eq!(metadata.generation(), 13);
        assert_eq!(metadata.display_name().as_str(), "Profile 12");
        lock.release().unwrap();
    }

    #[test]
    fn metadata_loader_rejects_wrong_storage_identity() {
        let (root, lock, storage_id) = locked_identity("identity");
        load_or_create_profile_metadata(
            &lock,
            storage_id,
            ProfileDisplayName::new("Default").unwrap(),
        )
        .unwrap();
        let other = ProfileStorageId::from_raw(storage_id.get().wrapping_add(1)).unwrap();
        assert!(matches!(
            load_profile_metadata(root.path(), other),
            Err(ProfileMetadataError::StorageIdentityMismatch { .. })
        ));
        lock.release().unwrap();
    }

    #[test]
    fn metadata_directory_is_bounded_and_rejects_unknown_schema() {
        let (root, lock, storage_id) = locked_identity("bounds");
        let directory = root.path().join(PROFILE_METADATA_DIRECTORY_NAME);
        fs::create_dir_all(directory.join(format!("v2-g{:016x}-i{}-n41", 1_u64, storage_id)))
            .unwrap();
        assert!(matches!(
            load_profile_metadata(root.path(), storage_id),
            Err(ProfileMetadataError::UnsupportedSchema { schema: 2, .. })
        ));
        lock.release().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn metadata_loader_rejects_redirected_directory() {
        use std::os::unix::fs::symlink;

        let (root, lock, storage_id) = locked_identity("symlink");
        let target = root.path().join("meta-target");
        fs::create_dir_all(&target).unwrap();
        symlink(&target, root.path().join(PROFILE_METADATA_DIRECTORY_NAME)).unwrap();
        assert!(matches!(
            load_profile_metadata(root.path(), storage_id),
            Err(ProfileMetadataError::Corrupt { .. })
        ));
        lock.release().unwrap();
    }
}
