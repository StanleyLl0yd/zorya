use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const SETTINGS_SCHEMA_VERSION: u32 = 1;
pub const MAX_SETTINGS_RECORD_BYTES: usize = 64 * 1024;
pub const MAX_SETTINGS_ENTRIES: usize = 256;
pub const MAX_SETTING_KEY_BYTES: usize = 64;
pub const MAX_SETTING_VALUE_BYTES: usize = 4 * 1024;

const SETTINGS_MAGIC: [u8; 8] = *b"ZRYSET01";
const SETTINGS_DIRECTORY: &str = "settings";
const SETTINGS_FILE_PREFIX: &str = "settings-";
const SETTINGS_FILE_SUFFIX: &str = ".bin";
const SETTINGS_GENERATION_DIGITS: usize = 20;
const SETTINGS_RETAINED_GENERATIONS: usize = 3;
const MAX_SETTINGS_DIRECTORY_ENTRIES: usize = 128;
const MAX_DISCOVERED_GENERATIONS: usize = 64;
const MAX_PENDING_FILE_ATTEMPTS: usize = 8;
const CHECKSUM_BYTES: usize = 8;
const PENDING_FILE_PREFIX: &str = ".pending-settings-";
static NEXT_PENDING_FILE: AtomicU64 = AtomicU64::new(1);
const MIN_RECORD_BYTES: usize = 8 + 4 + 8 + 4 + CHECKSUM_BYTES;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsSnapshot {
    generation: u64,
    entries: BTreeMap<String, String>,
}

impl SettingsSnapshot {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.entries
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }

    pub fn set(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Option<String>, ProfileStorageError> {
        let key = key.into();
        let value = value.into();
        validate_setting_key(&key)?;
        validate_setting_value(&value)?;
        if !self.entries.contains_key(&key) && self.entries.len() >= MAX_SETTINGS_ENTRIES {
            return Err(ProfileStorageError::SettingsEntryLimitExceeded {
                limit: MAX_SETTINGS_ENTRIES,
            });
        }
        Ok(self.entries.insert(key, value))
    }

    pub fn remove(&mut self, key: &str) -> Option<String> {
        self.entries.remove(key)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsRecovery {
    skipped_generations: Vec<u64>,
}

impl SettingsRecovery {
    pub fn skipped_generations(&self) -> &[u64] {
        &self.skipped_generations
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsLoad {
    snapshot: SettingsSnapshot,
    recovery: Option<SettingsRecovery>,
}

impl SettingsLoad {
    pub fn snapshot(&self) -> &SettingsSnapshot {
        &self.snapshot
    }

    pub fn into_snapshot(self) -> SettingsSnapshot {
        self.snapshot
    }

    pub fn recovery(&self) -> Option<&SettingsRecovery> {
        self.recovery.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsCleanupWarning {
    generations: Vec<u64>,
    pending_files: Vec<PathBuf>,
}

impl SettingsCleanupWarning {
    pub fn generations(&self) -> &[u64] {
        &self.generations
    }

    pub fn pending_files(&self) -> &[PathBuf] {
        &self.pending_files
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsSave {
    snapshot: SettingsSnapshot,
    cleanup_warning: Option<SettingsCleanupWarning>,
}

impl SettingsSave {
    pub fn snapshot(&self) -> &SettingsSnapshot {
        &self.snapshot
    }

    pub fn into_snapshot(self) -> SettingsSnapshot {
        self.snapshot
    }

    pub fn cleanup_warning(&self) -> Option<&SettingsCleanupWarning> {
        self.cleanup_warning.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileStorageError {
    Io {
        operation: &'static str,
        path: PathBuf,
        kind: io::ErrorKind,
    },
    InvalidSettingKey {
        key: String,
    },
    SettingValueTooLarge {
        bytes: usize,
        limit: usize,
    },
    SettingsEntryLimitExceeded {
        limit: usize,
    },
    SettingsRecordTooLarge {
        bytes: usize,
        limit: usize,
    },
    SettingsDirectoryEntryLimitExceeded {
        found: usize,
        limit: usize,
    },
    GenerationFileLimitExceeded {
        found: usize,
        limit: usize,
    },
    UnsupportedSettingsSchema {
        generation: u64,
        schema: u32,
    },
    NoValidSettingsGeneration {
        corrupt_generations: Vec<u64>,
    },
    StaleSettingsGeneration {
        current: u64,
        provided: u64,
    },
    SettingsGenerationExhausted,
    ConcurrentSettingsWrite {
        generation: u64,
    },
    PendingSettingsFileCollisionLimit {
        attempts: usize,
    },
}

impl fmt::Display for ProfileStorageError {
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
            Self::InvalidSettingKey { key } => {
                write!(formatter, "invalid profile setting key {key:?}")
            }
            Self::SettingValueTooLarge { bytes, limit } => write!(
                formatter,
                "profile setting value requires {bytes} bytes; limit is {limit}"
            ),
            Self::SettingsEntryLimitExceeded { limit } => {
                write!(formatter, "profile settings entry limit {limit} exceeded")
            }
            Self::SettingsRecordTooLarge { bytes, limit } => write!(
                formatter,
                "profile settings record requires {bytes} bytes; limit is {limit}"
            ),
            Self::SettingsDirectoryEntryLimitExceeded { found, limit } => write!(
                formatter,
                "profile settings directory contains {found} entries; scan limit is {limit}"
            ),
            Self::GenerationFileLimitExceeded { found, limit } => write!(
                formatter,
                "profile settings directory contains {found} generations; generation limit is {limit}"
            ),
            Self::UnsupportedSettingsSchema { generation, schema } => write!(
                formatter,
                "profile settings generation {generation} uses unsupported schema {schema}"
            ),
            Self::NoValidSettingsGeneration {
                corrupt_generations,
            } => write!(
                formatter,
                "no valid profile settings generation remains after corrupt generations {corrupt_generations:?}"
            ),
            Self::StaleSettingsGeneration { current, provided } => write!(
                formatter,
                "profile settings generation {provided} is stale; current generation is {current}"
            ),
            Self::SettingsGenerationExhausted => {
                formatter.write_str("profile settings generation space is exhausted")
            }
            Self::ConcurrentSettingsWrite { generation } => write!(
                formatter,
                "profile settings generation {generation} was created concurrently"
            ),
            Self::PendingSettingsFileCollisionLimit { attempts } => write!(
                formatter,
                "could not allocate a unique pending settings file after {attempts} attempts"
            ),
        }
    }
}

impl std::error::Error for ProfileStorageError {}

#[derive(Clone, Debug)]
pub struct ProfileStore {
    root: PathBuf,
    settings_directory: PathBuf,
}

impl ProfileStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, ProfileStorageError> {
        let root = root.into();
        create_directory(&root)?;
        let settings_directory = root.join(SETTINGS_DIRECTORY);
        create_directory(&settings_directory)?;
        Ok(Self {
            root,
            settings_directory,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn load_settings(&self) -> Result<SettingsLoad, ProfileStorageError> {
        let generations = self.discover_generations()?;
        if generations.is_empty() {
            return Ok(SettingsLoad {
                snapshot: SettingsSnapshot::default(),
                recovery: None,
            });
        }

        let mut corrupt_generations = Vec::new();
        for generation in generations {
            let path = self.settings_path(generation);
            let bytes = read_bounded(&path, MAX_SETTINGS_RECORD_BYTES)?;
            match decode_settings(&bytes, generation) {
                Ok(snapshot) => {
                    let recovery = (!corrupt_generations.is_empty()).then_some(SettingsRecovery {
                        skipped_generations: corrupt_generations,
                    });
                    return Ok(SettingsLoad { snapshot, recovery });
                }
                Err(DecodeError::UnsupportedSchema(schema)) => {
                    return Err(ProfileStorageError::UnsupportedSettingsSchema {
                        generation,
                        schema,
                    });
                }
                Err(DecodeError::Corrupt) => corrupt_generations.push(generation),
            }
        }

        Err(ProfileStorageError::NoValidSettingsGeneration {
            corrupt_generations,
        })
    }

    pub fn save_settings(
        &self,
        snapshot: &SettingsSnapshot,
    ) -> Result<SettingsSave, ProfileStorageError> {
        validate_snapshot(snapshot)?;
        let loaded = self.load_settings()?;
        let current_generation = loaded.snapshot().generation();
        if current_generation != snapshot.generation {
            return Err(ProfileStorageError::StaleSettingsGeneration {
                current: current_generation,
                provided: snapshot.generation,
            });
        }

        let recovered = loaded
            .recovery()
            .map(|recovery| recovery.skipped_generations())
            .unwrap_or(&[]);
        let generations = self.discover_generations_with_reserve(2)?;
        if let Some(unexpected) =
            unexpected_generation(current_generation, recovered, &generations)
        {
            return Err(ProfileStorageError::ConcurrentSettingsWrite {
                generation: unexpected,
            });
        }
        let previous_max = generations.first().copied().unwrap_or(0);
        let generation = previous_max
            .checked_add(1)
            .ok_or(ProfileStorageError::SettingsGenerationExhausted)?;
        let bytes = encode_settings(snapshot, generation)?;
        let final_path = self.settings_path(generation);
        let (pending_path, mut file) = self.create_pending_settings_file(generation)?;

        if let Err(error) = file.write_all(&bytes) {
            let _ = fs::remove_file(&pending_path);
            return Err(io_error("write pending settings generation", &pending_path, error));
        }
        if let Err(error) = file.sync_all() {
            let _ = fs::remove_file(&pending_path);
            return Err(io_error("sync pending settings generation", &pending_path, error));
        }
        drop(file);

        match fs::hard_link(&pending_path, &final_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(&pending_path);
                return Err(ProfileStorageError::ConcurrentSettingsWrite { generation });
            }
            Err(error) => {
                let _ = fs::remove_file(&pending_path);
                return Err(io_error("publish settings generation", &final_path, error));
            }
        }

        let mut pending_cleanup = Vec::new();
        if let Err(error) = fs::remove_file(&pending_path) {
            if error.kind() != io::ErrorKind::NotFound {
                pending_cleanup.push(pending_path);
            }
        }

        let mut saved = snapshot.clone();
        saved.generation = generation;
        let failed_generations = self.cleanup_generations(generation, &generations);
        let cleanup_warning =
            (!failed_generations.is_empty() || !pending_cleanup.is_empty()).then_some(
                SettingsCleanupWarning {
                    generations: failed_generations,
                    pending_files: pending_cleanup,
                },
            );
        Ok(SettingsSave {
            snapshot: saved,
            cleanup_warning,
        })
    }

    fn discover_generations(&self) -> Result<Vec<u64>, ProfileStorageError> {
        self.discover_generations_with_reserve(0)
    }

    fn discover_generations_with_reserve(
        &self,
        reserved_entries: usize,
    ) -> Result<Vec<u64>, ProfileStorageError> {
        let entries = fs::read_dir(&self.settings_directory).map_err(|error| {
            io_error(
                "read settings directory",
                &self.settings_directory,
                error,
            )
        })?;
        let mut generations = Vec::new();
        let mut entry_count = 0usize;
        for entry in entries {
            entry_count = entry_count.saturating_add(1);
            let prospective = entry_count.saturating_add(reserved_entries);
            if prospective > MAX_SETTINGS_DIRECTORY_ENTRIES {
                return Err(ProfileStorageError::SettingsDirectoryEntryLimitExceeded {
                    found: prospective,
                    limit: MAX_SETTINGS_DIRECTORY_ENTRIES,
                });
            }
            let entry = entry.map_err(|error| {
                io_error(
                    "read settings directory entry",
                    &self.settings_directory,
                    error,
                )
            })?;
            let file_type = entry.file_type().map_err(|error| {
                io_error("inspect settings directory entry", &entry.path(), error)
            })?;
            if !file_type.is_file() {
                continue;
            }
            if let Some(generation) = parse_generation_file_name(&entry.file_name()) {
                generations.push(generation);
                if generations.len() > MAX_DISCOVERED_GENERATIONS {
                    return Err(ProfileStorageError::GenerationFileLimitExceeded {
                        found: generations.len(),
                        limit: MAX_DISCOVERED_GENERATIONS,
                    });
                }
            }
        }
        generations.sort_unstable_by(|left, right| right.cmp(left));
        Ok(generations)
    }

    fn cleanup_generations(
        &self,
        current_generation: u64,
        previous_generations: &[u64],
    ) -> Vec<u64> {
        let mut failed = Vec::new();
        for &generation in previous_generations
            .iter()
            .skip(SETTINGS_RETAINED_GENERATIONS.saturating_sub(1))
            .rev()
        {
            let path = self.settings_path(generation);
            if let Err(error) = fs::remove_file(&path) {
                if error.kind() != io::ErrorKind::NotFound {
                    failed.push(generation);
                }
            }
        }

        debug_assert!(!failed.contains(&current_generation));
        failed
    }

    fn create_pending_settings_file(
        &self,
        generation: u64,
    ) -> Result<(PathBuf, File), ProfileStorageError> {
        for _ in 0..MAX_PENDING_FILE_ATTEMPTS {
            let sequence = NEXT_PENDING_FILE.fetch_add(1, Ordering::Relaxed);
            let name = format!(
                "{PENDING_FILE_PREFIX}{generation:020}-{:010}-{sequence:020}.tmp",
                std::process::id()
            );
            let path = self.settings_directory.join(name);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok((path, file)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(io_error("create pending settings generation", &path, error));
                }
            }
        }
        Err(ProfileStorageError::PendingSettingsFileCollisionLimit {
            attempts: MAX_PENDING_FILE_ATTEMPTS,
        })
    }

    fn settings_path(&self, generation: u64) -> PathBuf {
        self.settings_directory
            .join(settings_file_name(generation))
    }
}

fn create_directory(path: &Path) -> Result<(), ProfileStorageError> {
    fs::create_dir_all(path).map_err(|error| io_error("create profile directory", path, error))
}

fn validate_snapshot(snapshot: &SettingsSnapshot) -> Result<(), ProfileStorageError> {
    if snapshot.entries.len() > MAX_SETTINGS_ENTRIES {
        return Err(ProfileStorageError::SettingsEntryLimitExceeded {
            limit: MAX_SETTINGS_ENTRIES,
        });
    }
    for (key, value) in &snapshot.entries {
        validate_setting_key(key)?;
        validate_setting_value(value)?;
    }
    Ok(())
}

fn validate_setting_key(key: &str) -> Result<(), ProfileStorageError> {
    let valid = !key.is_empty()
        && key.len() <= MAX_SETTING_KEY_BYTES
        && key.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'-' | b'_')
        });
    if valid {
        Ok(())
    } else {
        Err(ProfileStorageError::InvalidSettingKey {
            key: key.to_owned(),
        })
    }
}

fn validate_setting_value(value: &str) -> Result<(), ProfileStorageError> {
    if value.len() <= MAX_SETTING_VALUE_BYTES {
        Ok(())
    } else {
        Err(ProfileStorageError::SettingValueTooLarge {
            bytes: value.len(),
            limit: MAX_SETTING_VALUE_BYTES,
        })
    }
}

fn unexpected_generation(
    current: u64,
    recovered: &[u64],
    generations: &[u64],
) -> Option<u64> {
    generations
        .iter()
        .find(|&&generation| generation > current && !recovered.contains(&generation))
        .copied()
}

fn settings_file_name(generation: u64) -> String {
    format!("{SETTINGS_FILE_PREFIX}{generation:020}{SETTINGS_FILE_SUFFIX}")
}

fn parse_generation_file_name(name: &std::ffi::OsStr) -> Option<u64> {
    let name = name.to_str()?;
    let digits = name
        .strip_prefix(SETTINGS_FILE_PREFIX)?
        .strip_suffix(SETTINGS_FILE_SUFFIX)?;
    if digits.len() != SETTINGS_GENERATION_DIGITS
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let generation = digits.parse::<u64>().ok()?;
    (generation != 0).then_some(generation)
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, ProfileStorageError> {
    let file = File::open(path).map_err(|error| io_error("open settings generation", path, error))?;
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error("read settings generation", path, error))?;
    Ok(bytes)
}

fn encode_settings(
    snapshot: &SettingsSnapshot,
    generation: u64,
) -> Result<Vec<u8>, ProfileStorageError> {
    validate_snapshot(snapshot)?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&SETTINGS_MAGIC);
    bytes.extend_from_slice(&SETTINGS_SCHEMA_VERSION.to_le_bytes());
    bytes.extend_from_slice(&generation.to_le_bytes());
    bytes.extend_from_slice(&(snapshot.entries.len() as u32).to_le_bytes());

    for (key, value) in &snapshot.entries {
        bytes.extend_from_slice(&(key.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
        bytes.extend_from_slice(key.as_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }

    let final_size = bytes.len().saturating_add(CHECKSUM_BYTES);
    if final_size > MAX_SETTINGS_RECORD_BYTES {
        return Err(ProfileStorageError::SettingsRecordTooLarge {
            bytes: final_size,
            limit: MAX_SETTINGS_RECORD_BYTES,
        });
    }
    let checksum = checksum64(&bytes);
    bytes.extend_from_slice(&checksum.to_le_bytes());
    Ok(bytes)
}

enum DecodeError {
    Corrupt,
    UnsupportedSchema(u32),
}

fn decode_settings(bytes: &[u8], expected_generation: u64) -> Result<SettingsSnapshot, DecodeError> {
    if bytes.len() < 12 {
        return Err(DecodeError::Corrupt);
    }
    if bytes[..8] != SETTINGS_MAGIC {
        return Err(DecodeError::Corrupt);
    }
    let schema = u32::from_le_bytes(bytes[8..12].try_into().map_err(|_| DecodeError::Corrupt)?);
    if schema != SETTINGS_SCHEMA_VERSION {
        return Err(DecodeError::UnsupportedSchema(schema));
    }
    if bytes.len() < MIN_RECORD_BYTES || bytes.len() > MAX_SETTINGS_RECORD_BYTES {
        return Err(DecodeError::Corrupt);
    }

    let payload_len = bytes
        .len()
        .checked_sub(CHECKSUM_BYTES)
        .ok_or(DecodeError::Corrupt)?;
    let (payload, checksum_bytes) = bytes.split_at(payload_len);
    let stored_checksum = u64::from_le_bytes(
        checksum_bytes
            .try_into()
            .map_err(|_| DecodeError::Corrupt)?,
    );
    if checksum64(payload) != stored_checksum {
        return Err(DecodeError::Corrupt);
    }

    let mut cursor = RecordCursor::new(payload);
    if cursor.take_array::<8>()? != SETTINGS_MAGIC {
        return Err(DecodeError::Corrupt);
    }
    let decoded_schema = u32::from_le_bytes(cursor.take_array::<4>()?);
    debug_assert_eq!(decoded_schema, SETTINGS_SCHEMA_VERSION);
    let generation = u64::from_le_bytes(cursor.take_array::<8>()?);
    if generation != expected_generation || generation == 0 {
        return Err(DecodeError::Corrupt);
    }
    let count = u32::from_le_bytes(cursor.take_array::<4>()?) as usize;
    if count > MAX_SETTINGS_ENTRIES {
        return Err(DecodeError::Corrupt);
    }

    let mut entries = BTreeMap::new();
    for _ in 0..count {
        let key_len = u16::from_le_bytes(cursor.take_array::<2>()?) as usize;
        let value_len = u32::from_le_bytes(cursor.take_array::<4>()?) as usize;
        if key_len == 0
            || key_len > MAX_SETTING_KEY_BYTES
            || value_len > MAX_SETTING_VALUE_BYTES
        {
            return Err(DecodeError::Corrupt);
        }

        let key = std::str::from_utf8(cursor.take(key_len)?)
            .map_err(|_| DecodeError::Corrupt)?
            .to_owned();
        let value = std::str::from_utf8(cursor.take(value_len)?)
            .map_err(|_| DecodeError::Corrupt)?
            .to_owned();
        if validate_setting_key(&key).is_err() || validate_setting_value(&value).is_err() {
            return Err(DecodeError::Corrupt);
        }
        if entries.insert(key, value).is_some() {
            return Err(DecodeError::Corrupt);
        }
    }

    if !cursor.is_finished() {
        return Err(DecodeError::Corrupt);
    }
    Ok(SettingsSnapshot {
        generation,
        entries,
    })
}

struct RecordCursor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> RecordCursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], DecodeError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or(DecodeError::Corrupt)?;
        let slice = self
            .bytes
            .get(self.position..end)
            .ok_or(DecodeError::Corrupt)?;
        self.position = end;
        Ok(slice)
    }

    fn take_array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        self.take(N)?
            .try_into()
            .map_err(|_| DecodeError::Corrupt)
    }

    fn is_finished(&self) -> bool {
        self.position == self.bytes.len()
    }
}

fn checksum64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn io_error(operation: &'static str, path: &Path, error: io::Error) -> ProfileStorageError {
    ProfileStorageError::Io {
        operation,
        path: path.to_path_buf(),
        kind: error.kind(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "zorya-profile-settings-test-{}-{sequence}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn snapshot_with(key: &str, value: &str) -> SettingsSnapshot {
        let mut snapshot = SettingsSnapshot::default();
        snapshot.set(key, value).expect("valid setting");
        snapshot
    }

    fn raw_record(
        generation: u64,
        schema: u32,
        entries: &[(&str, &str)],
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&SETTINGS_MAGIC);
        bytes.extend_from_slice(&schema.to_le_bytes());
        bytes.extend_from_slice(&generation.to_le_bytes());
        bytes.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        for (key, value) in entries {
            bytes.extend_from_slice(&(key.len() as u16).to_le_bytes());
            bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
            bytes.extend_from_slice(key.as_bytes());
            bytes.extend_from_slice(value.as_bytes());
        }
        let checksum = checksum64(&bytes);
        bytes.extend_from_slice(&checksum.to_le_bytes());
        bytes
    }

    #[test]
    fn settings_snapshot_enforces_key_and_value_bounds() {
        let mut snapshot = SettingsSnapshot::default();
        assert!(snapshot.set("browser.mode", "blank").is_ok());
        assert!(matches!(
            snapshot.set("Browser Mode", "blank"),
            Err(ProfileStorageError::InvalidSettingKey { .. })
        ));
        assert!(matches!(
            snapshot.set("browser.large", "x".repeat(MAX_SETTING_VALUE_BYTES + 1)),
            Err(ProfileStorageError::SettingValueTooLarge { .. })
        ));
    }

    #[test]
    fn settings_codec_round_trips_generation_and_entries() {
        let mut snapshot = snapshot_with("browser.mode", "blank");
        snapshot.set("privacy.mode", "standard").unwrap();

        let encoded = encode_settings(&snapshot, 7).expect("encode");
        let decoded = decode_settings(&encoded, 7).expect("decode");

        assert_eq!(decoded.generation(), 7);
        assert_eq!(decoded.get("browser.mode"), Some("blank"));
        assert_eq!(decoded.get("privacy.mode"), Some("standard"));
    }

    #[test]
    fn duplicate_keys_are_rejected() {
        let bytes = raw_record(
            1,
            SETTINGS_SCHEMA_VERSION,
            &[("browser.mode", "one"), ("browser.mode", "two")],
        );

        assert!(matches!(
            decode_settings(&bytes, 1),
            Err(DecodeError::Corrupt)
        ));
    }

    #[test]
    fn unpublished_pending_file_is_ignored() {
        let directory = TestDirectory::new();
        let store = ProfileStore::open(directory.path()).expect("open profile");
        let first = store
            .save_settings(&snapshot_with("browser.mode", "first"))
            .unwrap()
            .into_snapshot();

        let pending = store.settings_directory.join(format!(
            "{PENDING_FILE_PREFIX}{:020}-{:010}-{:020}.tmp",
            first.generation() + 1,
            std::process::id(),
            999_999u64
        ));
        fs::write(&pending, b"partial future write").unwrap();

        let loaded = store.load_settings().unwrap();
        assert_eq!(loaded.snapshot().generation(), first.generation());
        assert!(loaded.recovery().is_none());

        fs::remove_file(pending).unwrap();
        let mut next = loaded.into_snapshot();
        next.set("browser.mode", "second").unwrap();
        let saved = store.save_settings(&next).unwrap().into_snapshot();
        assert_eq!(saved.generation(), first.generation() + 1);
    }

    #[test]
    fn successful_save_removes_its_pending_publication_file() {
        let directory = TestDirectory::new();
        let store = ProfileStore::open(directory.path()).expect("open profile");
        let saved = store
            .save_settings(&snapshot_with("browser.mode", "first"))
            .unwrap();
        assert!(saved.cleanup_warning().is_none());

        let pending = fs::read_dir(&store.settings_directory)
            .unwrap()
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.starts_with(PENDING_FILE_PREFIX))
            .collect::<Vec<_>>();
        assert!(pending.is_empty());
    }

    #[test]
    fn corrupt_newest_generation_recovers_previous_explicitly() {
        let directory = TestDirectory::new();
        let store = ProfileStore::open(directory.path()).expect("open profile");
        let first = store
            .save_settings(&snapshot_with("browser.mode", "first"))
            .expect("save first")
            .into_snapshot();
        let mut second_input = first.clone();
        second_input.set("browser.mode", "second").unwrap();
        let second = store
            .save_settings(&second_input)
            .expect("save second")
            .into_snapshot();

        OpenOptions::new()
            .write(true)
            .open(store.settings_path(second.generation()))
            .expect("open newest")
            .set_len(7)
            .expect("truncate newest");

        let loaded = store.load_settings().expect("recover previous");
        assert_eq!(loaded.snapshot().generation(), first.generation());
        assert_eq!(loaded.snapshot().get("browser.mode"), Some("first"));
        assert_eq!(
            loaded.recovery().unwrap().skipped_generations(),
            &[second.generation()]
        );
    }

    #[test]
    fn checksum_failure_recovers_previous_generation() {
        let directory = TestDirectory::new();
        let store = ProfileStore::open(directory.path()).expect("open profile");
        let first = store
            .save_settings(&snapshot_with("browser.mode", "first"))
            .expect("save first")
            .into_snapshot();
        let mut second_input = first.clone();
        second_input.set("browser.mode", "second").unwrap();
        let second = store
            .save_settings(&second_input)
            .expect("save second")
            .into_snapshot();

        let path = store.settings_path(second.generation());
        let mut bytes = fs::read(&path).expect("read newest");
        bytes[MIN_RECORD_BYTES - CHECKSUM_BYTES] ^= 0x40;
        fs::write(&path, bytes).expect("corrupt newest");

        let loaded = store.load_settings().expect("recover previous");
        assert_eq!(loaded.snapshot().generation(), first.generation());
        assert_eq!(
            loaded.recovery().unwrap().skipped_generations(),
            &[second.generation()]
        );
    }

    #[test]
    fn oversized_newer_schema_still_blocks_fallback() {
        let mut bytes = raw_record(
            2,
            SETTINGS_SCHEMA_VERSION + 1,
            &[("browser.mode", "future")],
        );
        bytes.resize(MAX_SETTINGS_RECORD_BYTES + 1, 0);

        assert!(matches!(
            decode_settings(&bytes, 2),
            Err(DecodeError::UnsupportedSchema(schema))
                if schema == SETTINGS_SCHEMA_VERSION + 1
        ));
    }

    #[test]
    fn unsupported_newer_schema_blocks_fallback() {
        let directory = TestDirectory::new();
        let store = ProfileStore::open(directory.path()).expect("open profile");
        let first = store
            .save_settings(&snapshot_with("browser.mode", "first"))
            .expect("save first")
            .into_snapshot();
        let next = first.generation() + 1;
        fs::write(
            store.settings_path(next),
            raw_record(next, SETTINGS_SCHEMA_VERSION + 1, &[("browser.mode", "future")]),
        )
        .expect("write newer schema");

        assert!(matches!(
            store.load_settings(),
            Err(ProfileStorageError::UnsupportedSettingsSchema {
                generation,
                schema
            }) if generation == next && schema == SETTINGS_SCHEMA_VERSION + 1
        ));
    }

    #[test]
    fn stale_snapshot_cannot_overwrite_newer_generation() {
        let directory = TestDirectory::new();
        let store = ProfileStore::open(directory.path()).expect("open profile");
        let initial = snapshot_with("browser.mode", "first");
        let first = store.save_settings(&initial).unwrap().into_snapshot();
        let stale = first.clone();
        let mut current = first;
        current.set("browser.mode", "second").unwrap();
        let second = store.save_settings(&current).unwrap().into_snapshot();

        assert!(matches!(
            store.save_settings(&stale),
            Err(ProfileStorageError::StaleSettingsGeneration {
                current,
                provided
            }) if current == second.generation() && provided == stale.generation()
        ));
    }

    #[test]
    fn unexpected_generation_detection_distinguishes_recovery_from_concurrency() {
        assert_eq!(unexpected_generation(4, &[6, 5], &[6, 5, 4, 3]), None);
        assert_eq!(
            unexpected_generation(4, &[5], &[6, 5, 4, 3]),
            Some(6)
        );
    }

    #[test]
    fn stale_snapshot_cannot_overwrite_a_concurrent_generation() {
        let directory = TestDirectory::new();
        let store = ProfileStore::open(directory.path()).expect("open profile");
        let first = store
            .save_settings(&snapshot_with("browser.mode", "first"))
            .unwrap()
            .into_snapshot();
        let concurrent_generation = first.generation() + 1;
        fs::write(
            store.settings_path(concurrent_generation),
            raw_record(
                concurrent_generation,
                SETTINGS_SCHEMA_VERSION,
                &[("browser.mode", "concurrent")],
            ),
        )
        .unwrap();

        assert!(matches!(
            store.save_settings(&first),
            Err(ProfileStorageError::StaleSettingsGeneration {
                current,
                provided
            }) if current == concurrent_generation && provided == first.generation()
        ));
    }

    #[test]
    fn corrupt_high_generation_is_not_reused_on_next_save() {
        let directory = TestDirectory::new();
        let store = ProfileStore::open(directory.path()).expect("open profile");
        let first = store
            .save_settings(&snapshot_with("browser.mode", "first"))
            .unwrap()
            .into_snapshot();
        let corrupt_generation = first.generation() + 1;
        fs::write(store.settings_path(corrupt_generation), b"partial").unwrap();

        let recovered = store.load_settings().unwrap().into_snapshot();
        assert_eq!(recovered.generation(), first.generation());
        let mut next_input = recovered;
        next_input.set("browser.mode", "third").unwrap();
        let saved = store.save_settings(&next_input).unwrap().into_snapshot();

        assert_eq!(saved.generation(), corrupt_generation + 1);
        assert_eq!(saved.get("browser.mode"), Some("third"));
    }

    #[test]
    fn settings_directory_scan_is_bounded_even_for_unrelated_files() {
        let directory = TestDirectory::new();
        let store = ProfileStore::open(directory.path()).expect("open profile");

        for index in 0..=MAX_SETTINGS_DIRECTORY_ENTRIES {
            fs::write(
                store.settings_directory.join(format!("unrelated-{index:03}.tmp")),
                b"x",
            )
            .unwrap();
        }

        assert!(matches!(
            store.load_settings(),
            Err(ProfileStorageError::SettingsDirectoryEntryLimitExceeded {
                found,
                limit
            }) if found == MAX_SETTINGS_DIRECTORY_ENTRIES + 1
                && limit == MAX_SETTINGS_DIRECTORY_ENTRIES
        ));
    }

    #[test]
    fn save_reserves_a_directory_slot_before_creating_a_generation() {
        let directory = TestDirectory::new();
        let store = ProfileStore::open(directory.path()).expect("open profile");

        for index in 0..MAX_SETTINGS_DIRECTORY_ENTRIES {
            fs::write(
                store.settings_directory.join(format!("unrelated-{index:03}.tmp")),
                b"x",
            )
            .unwrap();
        }

        assert!(store.load_settings().is_ok());
        assert!(matches!(
            store.save_settings(&SettingsSnapshot::default()),
            Err(ProfileStorageError::SettingsDirectoryEntryLimitExceeded {
                found,
                limit
            }) if found == MAX_SETTINGS_DIRECTORY_ENTRIES + 1
                && limit == MAX_SETTINGS_DIRECTORY_ENTRIES
        ));
    }

    #[test]
    fn successful_saves_keep_a_bounded_generation_window() {
        let directory = TestDirectory::new();
        let store = ProfileStore::open(directory.path()).expect("open profile");
        let mut snapshot = SettingsSnapshot::default();

        for index in 0..8 {
            snapshot
                .set("browser.sequence", index.to_string())
                .expect("setting");
            snapshot = store.save_settings(&snapshot).unwrap().into_snapshot();
        }

        let generations = store.discover_generations().unwrap();
        assert_eq!(generations.len(), SETTINGS_RETAINED_GENERATIONS);
        assert_eq!(generations[0], snapshot.generation());
    }
}
