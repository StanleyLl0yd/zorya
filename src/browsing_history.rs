use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const BROWSING_HISTORY_SCHEMA_VERSION: u32 = 1;
pub const MAX_BROWSING_HISTORY_VISITS: usize = 2_048;
pub const MAX_HISTORY_LOCATION_BYTES: usize = 4 * 1024;
pub const MAX_BROWSING_HISTORY_RECORD_BYTES: usize = 9 * 1024 * 1024;

const HISTORY_MAGIC: [u8; 8] = *b"ZRYHST01";
const HISTORY_DIRECTORY: &str = "history";
const HISTORY_FILE_PREFIX: &str = "history-";
const HISTORY_FILE_SUFFIX: &str = ".bin";
const HISTORY_GENERATION_DIGITS: usize = 20;
const HISTORY_RETAINED_GENERATIONS: usize = 3;
const MAX_HISTORY_DIRECTORY_ENTRIES: usize = 128;
const MAX_DISCOVERED_GENERATIONS: usize = 64;
const MAX_PENDING_FILE_ATTEMPTS: usize = 8;
const CHECKSUM_BYTES: usize = 8;
const PENDING_FILE_PREFIX: &str = ".pending-history-";
const MIN_RECORD_BYTES: usize = 8 + 4 + 8 + 8 + 4 + CHECKSUM_BYTES;
static NEXT_PENDING_FILE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BrowsingHistoryVisitId(u64);

impl BrowsingHistoryVisitId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowsingHistoryVisit {
    id: BrowsingHistoryVisitId,
    visited_unix_millis: u64,
    location: String,
}

impl BrowsingHistoryVisit {
    pub const fn id(&self) -> BrowsingHistoryVisitId {
        self.id
    }

    pub const fn visited_unix_millis(&self) -> u64 {
        self.visited_unix_millis
    }

    pub fn location(&self) -> &str {
        &self.location
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowsingHistoryRecord {
    id: BrowsingHistoryVisitId,
    evicted: Option<BrowsingHistoryVisit>,
}

impl BrowsingHistoryRecord {
    pub const fn id(&self) -> BrowsingHistoryVisitId {
        self.id
    }

    pub const fn evicted(&self) -> Option<&BrowsingHistoryVisit> {
        self.evicted.as_ref()
    }

    pub fn into_evicted(self) -> Option<BrowsingHistoryVisit> {
        self.evicted
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowsingHistorySnapshot {
    generation: u64,
    next_visit_id: u64,
    visits: Vec<BrowsingHistoryVisit>,
}

impl Default for BrowsingHistorySnapshot {
    fn default() -> Self {
        Self {
            generation: 0,
            next_visit_id: 1,
            visits: Vec::new(),
        }
    }
}

impl BrowsingHistorySnapshot {
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub fn len(&self) -> usize {
        self.visits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.visits.is_empty()
    }

    pub fn visits(&self) -> &[BrowsingHistoryVisit] {
        &self.visits
    }

    pub fn record_visit(
        &mut self,
        visited_unix_millis: u64,
        location: impl Into<String>,
    ) -> Result<BrowsingHistoryRecord, BrowsingHistoryError> {
        let location = location.into();
        validate_location(&location)?;
        let id = BrowsingHistoryVisitId(self.next_visit_id);
        let next_visit_id = self
            .next_visit_id
            .checked_add(1)
            .ok_or(BrowsingHistoryError::VisitIdExhausted)?;
        let evicted = if self.visits.len() == MAX_BROWSING_HISTORY_VISITS {
            Some(self.visits.remove(0))
        } else {
            None
        };
        self.visits.push(BrowsingHistoryVisit {
            id,
            visited_unix_millis,
            location,
        });
        self.next_visit_id = next_visit_id;
        Ok(BrowsingHistoryRecord { id, evicted })
    }

    pub fn remove_visit(&mut self, id: BrowsingHistoryVisitId) -> Option<BrowsingHistoryVisit> {
        let index = self.visits.iter().position(|visit| visit.id == id)?;
        Some(self.visits.remove(index))
    }

    pub fn clear(&mut self) {
        self.visits.clear();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowsingHistoryRecovery {
    skipped_generations: Vec<u64>,
}

impl BrowsingHistoryRecovery {
    pub fn skipped_generations(&self) -> &[u64] {
        &self.skipped_generations
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowsingHistoryLoad {
    snapshot: BrowsingHistorySnapshot,
    recovery: Option<BrowsingHistoryRecovery>,
}

impl BrowsingHistoryLoad {
    pub const fn snapshot(&self) -> &BrowsingHistorySnapshot {
        &self.snapshot
    }

    pub fn into_snapshot(self) -> BrowsingHistorySnapshot {
        self.snapshot
    }

    pub const fn recovery(&self) -> Option<&BrowsingHistoryRecovery> {
        self.recovery.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowsingHistoryCleanupWarning {
    generations: Vec<u64>,
    pending_files: Vec<PathBuf>,
}

impl BrowsingHistoryCleanupWarning {
    pub fn generations(&self) -> &[u64] {
        &self.generations
    }

    pub fn pending_files(&self) -> &[PathBuf] {
        &self.pending_files
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowsingHistorySave {
    snapshot: BrowsingHistorySnapshot,
    cleanup_warning: Option<BrowsingHistoryCleanupWarning>,
}

impl BrowsingHistorySave {
    pub const fn snapshot(&self) -> &BrowsingHistorySnapshot {
        &self.snapshot
    }

    pub fn into_snapshot(self) -> BrowsingHistorySnapshot {
        self.snapshot
    }

    pub const fn cleanup_warning(&self) -> Option<&BrowsingHistoryCleanupWarning> {
        self.cleanup_warning.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BrowsingHistoryError {
    Io {
        operation: &'static str,
        path: PathBuf,
        kind: io::ErrorKind,
    },
    EmptyLocation,
    LocationTooLarge {
        bytes: usize,
        limit: usize,
    },
    VisitLimitExceeded {
        found: usize,
        limit: usize,
    },
    RecordTooLarge {
        bytes: usize,
        limit: usize,
    },
    DirectoryEntryLimitExceeded {
        found: usize,
        limit: usize,
    },
    GenerationFileLimitExceeded {
        found: usize,
        limit: usize,
    },
    UnsupportedSchema {
        generation: u64,
        schema: u32,
    },
    NoValidGeneration {
        corrupt_generations: Vec<u64>,
    },
    StaleGeneration {
        current: u64,
        provided: u64,
    },
    GenerationExhausted,
    VisitIdExhausted,
    ConcurrentWrite {
        generation: u64,
    },
    PendingFileCollisionLimit {
        attempts: usize,
    },
}

impl fmt::Display for BrowsingHistoryError {
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
            Self::EmptyLocation => formatter.write_str("browsing history location is empty"),
            Self::LocationTooLarge { bytes, limit } => write!(
                formatter,
                "browsing history location requires {bytes} bytes; limit is {limit}"
            ),
            Self::VisitLimitExceeded { found, limit } => write!(
                formatter,
                "browsing history contains {found} visits; limit is {limit}"
            ),
            Self::RecordTooLarge { bytes, limit } => write!(
                formatter,
                "browsing history record requires {bytes} bytes; limit is {limit}"
            ),
            Self::DirectoryEntryLimitExceeded { found, limit } => write!(
                formatter,
                "browsing history directory contains {found} entries; scan limit is {limit}"
            ),
            Self::GenerationFileLimitExceeded { found, limit } => write!(
                formatter,
                "browsing history directory contains {found} generations; generation limit is {limit}"
            ),
            Self::UnsupportedSchema { generation, schema } => write!(
                formatter,
                "browsing history generation {generation} uses unsupported schema {schema}"
            ),
            Self::NoValidGeneration {
                corrupt_generations,
            } => write!(
                formatter,
                "no valid browsing history generation remains after corrupt generations {corrupt_generations:?}"
            ),
            Self::StaleGeneration { current, provided } => write!(
                formatter,
                "browsing history generation {provided} is stale; current generation is {current}"
            ),
            Self::GenerationExhausted => {
                formatter.write_str("browsing history generation space is exhausted")
            }
            Self::VisitIdExhausted => {
                formatter.write_str("browsing history visit identifier space is exhausted")
            }
            Self::ConcurrentWrite { generation } => write!(
                formatter,
                "browsing history generation {generation} was created concurrently"
            ),
            Self::PendingFileCollisionLimit { attempts } => write!(
                formatter,
                "could not allocate a unique pending browsing history file after {attempts} attempts"
            ),
        }
    }
}

impl std::error::Error for BrowsingHistoryError {}

#[derive(Clone, Debug)]
pub struct BrowsingHistoryStore {
    root: PathBuf,
    history_directory: PathBuf,
}

impl BrowsingHistoryStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, BrowsingHistoryError> {
        let root = root.into();
        create_directory(&root)?;
        let history_directory = root.join(HISTORY_DIRECTORY);
        create_directory(&history_directory)?;
        Ok(Self {
            root,
            history_directory,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn load(&self) -> Result<BrowsingHistoryLoad, BrowsingHistoryError> {
        let generations = self.discover_generations()?;
        if generations.is_empty() {
            return Ok(BrowsingHistoryLoad {
                snapshot: BrowsingHistorySnapshot::default(),
                recovery: None,
            });
        }

        let mut corrupt_generations = Vec::new();
        for generation in generations {
            let path = self.history_path(generation);
            let bytes = read_bounded(&path, MAX_BROWSING_HISTORY_RECORD_BYTES)?;
            match decode_history(&bytes, generation) {
                Ok(snapshot) => {
                    let recovery =
                        (!corrupt_generations.is_empty()).then_some(BrowsingHistoryRecovery {
                            skipped_generations: corrupt_generations,
                        });
                    return Ok(BrowsingHistoryLoad { snapshot, recovery });
                }
                Err(DecodeError::UnsupportedSchema(schema)) => {
                    return Err(BrowsingHistoryError::UnsupportedSchema { generation, schema });
                }
                Err(DecodeError::Corrupt) => corrupt_generations.push(generation),
            }
        }

        Err(BrowsingHistoryError::NoValidGeneration {
            corrupt_generations,
        })
    }

    pub fn save(
        &self,
        snapshot: &BrowsingHistorySnapshot,
    ) -> Result<BrowsingHistorySave, BrowsingHistoryError> {
        validate_snapshot(snapshot)?;
        let loaded = self.load()?;
        let current_generation = loaded.snapshot().generation();
        if current_generation != snapshot.generation {
            return Err(BrowsingHistoryError::StaleGeneration {
                current: current_generation,
                provided: snapshot.generation,
            });
        }

        let recovered = loaded
            .recovery()
            .map(BrowsingHistoryRecovery::skipped_generations)
            .unwrap_or(&[]);
        let generations = self.discover_generations_with_reserve(2)?;
        if let Some(unexpected) = unexpected_generation(current_generation, recovered, &generations)
        {
            return Err(BrowsingHistoryError::ConcurrentWrite {
                generation: unexpected,
            });
        }

        let previous_max = generations.first().copied().unwrap_or(0);
        let generation = previous_max
            .checked_add(1)
            .ok_or(BrowsingHistoryError::GenerationExhausted)?;
        let bytes = encode_history(snapshot, generation)?;
        let final_path = self.history_path(generation);
        let (pending_path, mut file) = self.create_pending_file(generation)?;

        if let Err(error) = file.write_all(&bytes) {
            let _ = fs::remove_file(&pending_path);
            return Err(io_error(
                "write pending history generation",
                &pending_path,
                error,
            ));
        }
        if let Err(error) = file.sync_all() {
            let _ = fs::remove_file(&pending_path);
            return Err(io_error(
                "sync pending history generation",
                &pending_path,
                error,
            ));
        }
        drop(file);

        match fs::hard_link(&pending_path, &final_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(&pending_path);
                return Err(BrowsingHistoryError::ConcurrentWrite { generation });
            }
            Err(error) => {
                let _ = fs::remove_file(&pending_path);
                return Err(io_error("publish history generation", &final_path, error));
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
        let failed_generations = self.cleanup_generations(current_generation, &generations);
        let cleanup_warning = (!failed_generations.is_empty() || !pending_cleanup.is_empty())
            .then_some(BrowsingHistoryCleanupWarning {
                generations: failed_generations,
                pending_files: pending_cleanup,
            });
        Ok(BrowsingHistorySave {
            snapshot: saved,
            cleanup_warning,
        })
    }

    fn discover_generations(&self) -> Result<Vec<u64>, BrowsingHistoryError> {
        self.discover_generations_with_reserve(0)
    }

    fn discover_generations_with_reserve(
        &self,
        reserved_entries: usize,
    ) -> Result<Vec<u64>, BrowsingHistoryError> {
        let entries = fs::read_dir(&self.history_directory)
            .map_err(|error| io_error("read history directory", &self.history_directory, error))?;
        let mut generations = Vec::new();
        let mut entry_count = 0usize;
        for entry in entries {
            entry_count = entry_count.saturating_add(1);
            let prospective = entry_count.saturating_add(reserved_entries);
            if prospective > MAX_HISTORY_DIRECTORY_ENTRIES {
                return Err(BrowsingHistoryError::DirectoryEntryLimitExceeded {
                    found: prospective,
                    limit: MAX_HISTORY_DIRECTORY_ENTRIES,
                });
            }
            let entry = entry.map_err(|error| {
                io_error(
                    "read history directory entry",
                    &self.history_directory,
                    error,
                )
            })?;
            let file_type = entry.file_type().map_err(|error| {
                io_error("inspect history directory entry", &entry.path(), error)
            })?;
            if !file_type.is_file() {
                continue;
            }
            if let Some(generation) = parse_generation_file_name(&entry.file_name()) {
                generations.push(generation);
                if generations.len() > MAX_DISCOVERED_GENERATIONS {
                    return Err(BrowsingHistoryError::GenerationFileLimitExceeded {
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
        previous_current_generation: u64,
        previous_generations: &[u64],
    ) -> Vec<u64> {
        let retained_older = previous_generations
            .iter()
            .copied()
            .filter(|&generation| generation < previous_current_generation)
            .take(HISTORY_RETAINED_GENERATIONS.saturating_sub(2))
            .collect::<Vec<_>>();
        let mut failed = Vec::new();

        for &generation in previous_generations {
            if generation == previous_current_generation || retained_older.contains(&generation) {
                continue;
            }
            let path = self.history_path(generation);
            if let Err(error) = fs::remove_file(&path) {
                if error.kind() != io::ErrorKind::NotFound {
                    failed.push(generation);
                }
            }
        }
        failed
    }

    fn create_pending_file(
        &self,
        generation: u64,
    ) -> Result<(PathBuf, File), BrowsingHistoryError> {
        for _ in 0..MAX_PENDING_FILE_ATTEMPTS {
            let sequence = NEXT_PENDING_FILE.fetch_add(1, Ordering::Relaxed);
            let name = format!(
                "{PENDING_FILE_PREFIX}{generation:020}-{:010}-{sequence:020}.tmp",
                std::process::id()
            );
            let path = self.history_directory.join(name);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok((path, file)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(io_error("create pending history generation", &path, error));
                }
            }
        }
        Err(BrowsingHistoryError::PendingFileCollisionLimit {
            attempts: MAX_PENDING_FILE_ATTEMPTS,
        })
    }

    fn history_path(&self, generation: u64) -> PathBuf {
        self.history_directory.join(history_file_name(generation))
    }
}

fn create_directory(path: &Path) -> Result<(), BrowsingHistoryError> {
    fs::create_dir_all(path).map_err(|error| io_error("create profile directory", path, error))
}

fn validate_location(location: &str) -> Result<(), BrowsingHistoryError> {
    if location.is_empty() {
        return Err(BrowsingHistoryError::EmptyLocation);
    }
    if location.len() > MAX_HISTORY_LOCATION_BYTES {
        return Err(BrowsingHistoryError::LocationTooLarge {
            bytes: location.len(),
            limit: MAX_HISTORY_LOCATION_BYTES,
        });
    }
    Ok(())
}

fn validate_snapshot(snapshot: &BrowsingHistorySnapshot) -> Result<(), BrowsingHistoryError> {
    if snapshot.visits.len() > MAX_BROWSING_HISTORY_VISITS {
        return Err(BrowsingHistoryError::VisitLimitExceeded {
            found: snapshot.visits.len(),
            limit: MAX_BROWSING_HISTORY_VISITS,
        });
    }
    if snapshot.next_visit_id == 0 {
        return Err(BrowsingHistoryError::VisitIdExhausted);
    }

    let mut previous = 0u64;
    for visit in &snapshot.visits {
        validate_location(&visit.location)?;
        if visit.id.0 == 0 || visit.id.0 <= previous || visit.id.0 >= snapshot.next_visit_id {
            return Err(BrowsingHistoryError::VisitIdExhausted);
        }
        previous = visit.id.0;
    }
    Ok(())
}

fn unexpected_generation(current: u64, recovered: &[u64], generations: &[u64]) -> Option<u64> {
    generations
        .iter()
        .find(|&&generation| generation > current && !recovered.contains(&generation))
        .copied()
}

fn history_file_name(generation: u64) -> String {
    format!("{HISTORY_FILE_PREFIX}{generation:020}{HISTORY_FILE_SUFFIX}")
}

fn parse_generation_file_name(name: &std::ffi::OsStr) -> Option<u64> {
    let name = name.to_str()?;
    let digits = name
        .strip_prefix(HISTORY_FILE_PREFIX)?
        .strip_suffix(HISTORY_FILE_SUFFIX)?;
    if digits.len() != HISTORY_GENERATION_DIGITS
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let generation = digits.parse::<u64>().ok()?;
    (generation != 0).then_some(generation)
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, BrowsingHistoryError> {
    let file =
        File::open(path).map_err(|error| io_error("open history generation", path, error))?;
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error("read history generation", path, error))?;
    Ok(bytes)
}

fn encode_history(
    snapshot: &BrowsingHistorySnapshot,
    generation: u64,
) -> Result<Vec<u8>, BrowsingHistoryError> {
    validate_snapshot(snapshot)?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&HISTORY_MAGIC);
    bytes.extend_from_slice(&BROWSING_HISTORY_SCHEMA_VERSION.to_le_bytes());
    bytes.extend_from_slice(&generation.to_le_bytes());
    bytes.extend_from_slice(&snapshot.next_visit_id.to_le_bytes());
    bytes.extend_from_slice(&(snapshot.visits.len() as u32).to_le_bytes());

    for visit in &snapshot.visits {
        bytes.extend_from_slice(&visit.id.0.to_le_bytes());
        bytes.extend_from_slice(&visit.visited_unix_millis.to_le_bytes());
        bytes.extend_from_slice(&(visit.location.len() as u32).to_le_bytes());
        bytes.extend_from_slice(visit.location.as_bytes());
    }

    let final_size = bytes.len().saturating_add(CHECKSUM_BYTES);
    if final_size > MAX_BROWSING_HISTORY_RECORD_BYTES {
        return Err(BrowsingHistoryError::RecordTooLarge {
            bytes: final_size,
            limit: MAX_BROWSING_HISTORY_RECORD_BYTES,
        });
    }
    let checksum = checksum64(&bytes);
    bytes.extend_from_slice(&checksum.to_le_bytes());
    Ok(bytes)
}

#[derive(Debug)]
enum DecodeError {
    Corrupt,
    UnsupportedSchema(u32),
}

fn decode_history(
    bytes: &[u8],
    expected_generation: u64,
) -> Result<BrowsingHistorySnapshot, DecodeError> {
    if bytes.len() < 12 || bytes[..8] != HISTORY_MAGIC {
        return Err(DecodeError::Corrupt);
    }
    let schema = u32::from_le_bytes(bytes[8..12].try_into().map_err(|_| DecodeError::Corrupt)?);
    if schema != BROWSING_HISTORY_SCHEMA_VERSION {
        return Err(DecodeError::UnsupportedSchema(schema));
    }
    if bytes.len() < MIN_RECORD_BYTES || bytes.len() > MAX_BROWSING_HISTORY_RECORD_BYTES {
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
    if cursor.take_array::<8>()? != HISTORY_MAGIC {
        return Err(DecodeError::Corrupt);
    }
    let decoded_schema = u32::from_le_bytes(cursor.take_array::<4>()?);
    debug_assert_eq!(decoded_schema, BROWSING_HISTORY_SCHEMA_VERSION);
    let generation = u64::from_le_bytes(cursor.take_array::<8>()?);
    if generation == 0 || generation != expected_generation {
        return Err(DecodeError::Corrupt);
    }
    let next_visit_id = u64::from_le_bytes(cursor.take_array::<8>()?);
    if next_visit_id == 0 {
        return Err(DecodeError::Corrupt);
    }
    let count = u32::from_le_bytes(cursor.take_array::<4>()?) as usize;
    if count > MAX_BROWSING_HISTORY_VISITS {
        return Err(DecodeError::Corrupt);
    }

    let mut visits = Vec::with_capacity(count);
    let mut previous = 0u64;
    for _ in 0..count {
        let id = u64::from_le_bytes(cursor.take_array::<8>()?);
        let visited_unix_millis = u64::from_le_bytes(cursor.take_array::<8>()?);
        let location_len = u32::from_le_bytes(cursor.take_array::<4>()?) as usize;
        if id == 0
            || id <= previous
            || id >= next_visit_id
            || location_len == 0
            || location_len > MAX_HISTORY_LOCATION_BYTES
        {
            return Err(DecodeError::Corrupt);
        }
        let location = std::str::from_utf8(cursor.take(location_len)?)
            .map_err(|_| DecodeError::Corrupt)?
            .to_owned();
        if validate_location(&location).is_err() {
            return Err(DecodeError::Corrupt);
        }
        visits.push(BrowsingHistoryVisit {
            id: BrowsingHistoryVisitId(id),
            visited_unix_millis,
            location,
        });
        previous = id;
    }
    if !cursor.is_finished() {
        return Err(DecodeError::Corrupt);
    }

    Ok(BrowsingHistorySnapshot {
        generation,
        next_visit_id,
        visits,
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
        self.take(N)?.try_into().map_err(|_| DecodeError::Corrupt)
    }

    const fn is_finished(&self) -> bool {
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

fn io_error(operation: &'static str, path: &Path, error: io::Error) -> BrowsingHistoryError {
    BrowsingHistoryError::Io {
        operation,
        path: path.to_path_buf(),
        kind: error.kind(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "zorya-browsing-history-test-{}-{sequence}",
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

    fn snapshot_with(location: &str) -> BrowsingHistorySnapshot {
        let mut snapshot = BrowsingHistorySnapshot::default();
        snapshot.record_visit(1_000, location).unwrap();
        snapshot
    }

    fn raw_record(
        generation: u64,
        schema: u32,
        next_visit_id: u64,
        visits: &[(u64, u64, &str)],
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&HISTORY_MAGIC);
        bytes.extend_from_slice(&schema.to_le_bytes());
        bytes.extend_from_slice(&generation.to_le_bytes());
        bytes.extend_from_slice(&next_visit_id.to_le_bytes());
        bytes.extend_from_slice(&(visits.len() as u32).to_le_bytes());
        for (id, visited_unix_millis, location) in visits {
            bytes.extend_from_slice(&id.to_le_bytes());
            bytes.extend_from_slice(&visited_unix_millis.to_le_bytes());
            bytes.extend_from_slice(&(location.len() as u32).to_le_bytes());
            bytes.extend_from_slice(location.as_bytes());
        }
        let checksum = checksum64(&bytes);
        bytes.extend_from_slice(&checksum.to_le_bytes());
        bytes
    }

    #[test]
    fn snapshot_allocates_monotonic_ids_and_reports_eviction() {
        let mut snapshot = BrowsingHistorySnapshot::default();
        for index in 0..MAX_BROWSING_HISTORY_VISITS {
            let record = snapshot
                .record_visit(index as u64, format!("https://example.test/{index}"))
                .unwrap();
            assert!(record.evicted().is_none());
        }
        let first = snapshot.visits()[0].clone();
        let record = snapshot
            .record_visit(99_999, "https://example.test/new")
            .unwrap();

        assert_eq!(snapshot.len(), MAX_BROWSING_HISTORY_VISITS);
        assert_eq!(record.evicted(), Some(&first));
        assert_eq!(record.id().get(), MAX_BROWSING_HISTORY_VISITS as u64 + 1);
        assert_eq!(snapshot.visits()[0].id().get(), 2);
    }

    #[test]
    fn location_bounds_are_enforced_before_mutation() {
        let mut snapshot = BrowsingHistorySnapshot::default();
        assert_eq!(
            snapshot.record_visit(1, ""),
            Err(BrowsingHistoryError::EmptyLocation)
        );
        assert!(matches!(
            snapshot.record_visit(1, "x".repeat(MAX_HISTORY_LOCATION_BYTES + 1)),
            Err(BrowsingHistoryError::LocationTooLarge { .. })
        ));
        assert!(snapshot.is_empty());
    }

    #[test]
    fn clearing_history_does_not_reuse_visit_ids() {
        let mut snapshot = BrowsingHistorySnapshot::default();
        let first = snapshot.record_visit(1, "https://one.test").unwrap().id();
        snapshot.clear();
        let second = snapshot.record_visit(2, "https://two.test").unwrap().id();
        assert!(second > first);
    }

    #[test]
    fn codec_round_trips_visits_generation_and_next_identity() {
        let mut snapshot = BrowsingHistorySnapshot::default();
        snapshot.record_visit(10, "https://one.test").unwrap();
        snapshot.record_visit(20, "https://two.test/path").unwrap();
        let encoded = encode_history(&snapshot, 7).unwrap();
        let mut decoded = decode_history(&encoded, 7).unwrap();

        assert_eq!(decoded.generation(), 7);
        assert_eq!(decoded.visits(), snapshot.visits());
        assert_eq!(
            decoded
                .record_visit(30, "https://three.test")
                .unwrap()
                .id()
                .get(),
            3
        );
    }

    #[test]
    fn codec_rejects_truncation_checksum_failure_and_bad_identity_order() {
        let bytes = raw_record(
            1,
            BROWSING_HISTORY_SCHEMA_VERSION,
            3,
            &[(1, 10, "https://one.test"), (2, 20, "https://two.test")],
        );
        assert!(matches!(
            decode_history(&bytes[..bytes.len() - 1], 1),
            Err(DecodeError::Corrupt)
        ));

        let mut corrupt = bytes.clone();
        let index = corrupt.len() - CHECKSUM_BYTES - 1;
        corrupt[index] ^= 0x01;
        assert!(matches!(
            decode_history(&corrupt, 1),
            Err(DecodeError::Corrupt)
        ));

        let duplicate = raw_record(
            1,
            BROWSING_HISTORY_SCHEMA_VERSION,
            3,
            &[(2, 10, "https://one.test"), (2, 20, "https://two.test")],
        );
        assert!(matches!(
            decode_history(&duplicate, 1),
            Err(DecodeError::Corrupt)
        ));
    }

    #[test]
    fn unsupported_newer_schema_fails_before_current_format_decoding() {
        let bytes = raw_record(4, BROWSING_HISTORY_SCHEMA_VERSION + 1, 0, &[]);
        assert!(matches!(
            decode_history(&bytes, 4),
            Err(DecodeError::UnsupportedSchema(schema))
                if schema == BROWSING_HISTORY_SCHEMA_VERSION + 1
        ));
    }

    #[test]
    fn unpublished_pending_file_is_ignored() {
        let directory = TestDirectory::new();
        let store = BrowsingHistoryStore::open(directory.path()).unwrap();
        let first = store.save(&snapshot_with("https://one.test")).unwrap();

        let pending = store.history_directory.join(format!(
            "{PENDING_FILE_PREFIX}{:020}-{:010}-{:020}.tmp",
            first.snapshot().generation() + 1,
            std::process::id(),
            999_999u64
        ));
        fs::write(&pending, b"partial future write").unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.snapshot().generation(), 1);
        assert_eq!(loaded.snapshot().visits()[0].location(), "https://one.test");
        assert!(loaded.recovery().is_none());
    }

    #[test]
    fn save_load_and_stale_writer_rejection_preserve_data() {
        let directory = TestDirectory::new();
        let store = BrowsingHistoryStore::open(directory.path()).unwrap();
        let first = store
            .save(&snapshot_with("https://one.test"))
            .unwrap()
            .into_snapshot();
        let stale = first.clone();

        let mut second_input = first;
        second_input
            .record_visit(2_000, "https://two.test")
            .unwrap();
        let second = store.save(&second_input).unwrap().into_snapshot();
        assert_eq!(second.generation(), 2);

        assert_eq!(
            store.save(&stale),
            Err(BrowsingHistoryError::StaleGeneration {
                current: 2,
                provided: 1,
            })
        );
        let loaded = store.load().unwrap();
        assert_eq!(loaded.snapshot(), &second);
    }

    #[test]
    fn concurrent_saves_have_one_durable_winner() {
        let directory = TestDirectory::new();
        let store = Arc::new(BrowsingHistoryStore::open(directory.path()).unwrap());
        let barrier = Arc::new(Barrier::new(3));

        let workers = ["https://one.test", "https://two.test"]
            .into_iter()
            .map(|location| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let snapshot = snapshot_with(location);
                    barrier.wait();
                    store.save(&snapshot)
                })
            })
            .collect::<Vec<_>>();

        barrier.wait();
        let results = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
        assert!(
            results
                .iter()
                .filter_map(|result| result.as_ref().err())
                .all(|error| matches!(
                    error,
                    BrowsingHistoryError::ConcurrentWrite { .. }
                        | BrowsingHistoryError::StaleGeneration { .. }
                ))
        );

        let loaded = store.load().unwrap();
        assert_eq!(loaded.snapshot().generation(), 1);
        assert_eq!(loaded.snapshot().len(), 1);
    }

    #[test]
    fn corrupt_newest_generation_recovers_previous_explicitly() {
        let directory = TestDirectory::new();
        let store = BrowsingHistoryStore::open(directory.path()).unwrap();
        let first = store
            .save(&snapshot_with("https://one.test"))
            .unwrap()
            .into_snapshot();
        let mut second_input = first.clone();
        second_input.record_visit(2_000, "https://two.test").unwrap();
        let second = store.save(&second_input).unwrap().into_snapshot();

        let path = store.history_path(second.generation());
        let mut bytes = fs::read(&path).unwrap();
        let index = bytes.len() - CHECKSUM_BYTES - 1;
        bytes[index] ^= 0x01;
        fs::write(&path, bytes).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.snapshot(), &first);
        assert_eq!(
            loaded.recovery().unwrap().skipped_generations(),
            &[second.generation()]
        );
    }

    #[test]
    fn newer_schema_generation_fails_closed_without_fallback() {
        let directory = TestDirectory::new();
        let store = BrowsingHistoryStore::open(directory.path()).unwrap();
        let first = store
            .save(&snapshot_with("https://one.test"))
            .unwrap()
            .into_snapshot();
        let newer_generation = first.generation() + 1;
        fs::write(
            store.history_path(newer_generation),
            raw_record(
                newer_generation,
                BROWSING_HISTORY_SCHEMA_VERSION + 1,
                1,
                &[],
            ),
        )
        .unwrap();

        assert_eq!(
            store.load(),
            Err(BrowsingHistoryError::UnsupportedSchema {
                generation: newer_generation,
                schema: BROWSING_HISTORY_SCHEMA_VERSION + 1,
            })
        );
    }

    #[test]
    fn retained_generation_window_is_bounded() {
        let directory = TestDirectory::new();
        let store = BrowsingHistoryStore::open(directory.path()).unwrap();
        let mut snapshot = BrowsingHistorySnapshot::default();

        for index in 0..7u64 {
            snapshot
                .record_visit(index, format!("https://example.test/{index}"))
                .unwrap();
            snapshot = store.save(&snapshot).unwrap().into_snapshot();
        }

        let generations = store.discover_generations().unwrap();
        assert_eq!(generations.len(), HISTORY_RETAINED_GENERATIONS);
        assert_eq!(generations, vec![7, 6, 5]);
    }

    #[test]
    fn directory_scan_limit_is_enforced() {
        let directory = TestDirectory::new();
        let store = BrowsingHistoryStore::open(directory.path()).unwrap();
        for index in 0..=MAX_HISTORY_DIRECTORY_ENTRIES {
            fs::write(
                store.history_directory.join(format!("noise-{index:03}")),
                b"x",
            )
            .unwrap();
        }

        assert!(matches!(
            store.load(),
            Err(BrowsingHistoryError::DirectoryEntryLimitExceeded { .. })
        ));
    }
}
