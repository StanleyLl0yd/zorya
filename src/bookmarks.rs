use crate::profile_lock::{ProfileLock, ProfileLockError};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const BOOKMARKS_SCHEMA_VERSION: u32 = 1;
pub const MAX_BOOKMARKS: usize = 1_024;
pub const MAX_BOOKMARK_TITLE_BYTES: usize = 1_024;
pub const MAX_BOOKMARK_LOCATION_BYTES: usize = 4 * 1_024;
pub const MAX_BOOKMARKS_RECORD_BYTES: usize = 6 * 1_024 * 1_024;

const BOOKMARKS_MAGIC: [u8; 8] = *b"ZRYBMK01";
const BOOKMARKS_DIRECTORY: &str = "bookmarks";
const BOOKMARKS_FILE_PREFIX: &str = "bookmarks-";
const BOOKMARKS_FILE_SUFFIX: &str = ".bin";
const BOOKMARKS_GENERATION_DIGITS: usize = 20;
const BOOKMARKS_RETAINED_GENERATIONS: usize = 3;
const MAX_BOOKMARKS_DIRECTORY_ENTRIES: usize = 128;
const MAX_DISCOVERED_GENERATIONS: usize = 64;
const MAX_PENDING_FILE_ATTEMPTS: usize = 8;
const CHECKSUM_BYTES: usize = 8;
const PENDING_FILE_PREFIX: &str = ".pending-bookmarks-";
const MIN_RECORD_BYTES: usize = 8 + 4 + 8 + 8 + 4 + CHECKSUM_BYTES;
static NEXT_PENDING_FILE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BookmarkId(u64);

impl BookmarkId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bookmark {
    id: BookmarkId,
    title: String,
    location: String,
}

impl Bookmark {
    pub const fn id(&self) -> BookmarkId {
        self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn location(&self) -> &str {
        &self.location
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookmarksSnapshot {
    generation: u64,
    next_bookmark_id: u64,
    bookmarks: Vec<Bookmark>,
}

impl Default for BookmarksSnapshot {
    fn default() -> Self {
        Self {
            generation: 0,
            next_bookmark_id: 1,
            bookmarks: Vec::new(),
        }
    }
}

impl BookmarksSnapshot {
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub fn len(&self) -> usize {
        self.bookmarks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bookmarks.is_empty()
    }

    pub fn bookmarks(&self) -> &[Bookmark] {
        &self.bookmarks
    }

    pub fn bookmark(&self, id: BookmarkId) -> Option<&Bookmark> {
        self.bookmarks.iter().find(|bookmark| bookmark.id == id)
    }

    pub fn add_bookmark(
        &mut self,
        title: impl Into<String>,
        location: impl Into<String>,
    ) -> Result<BookmarkId, BookmarksError> {
        let title = title.into();
        let location = location.into();
        validate_title(&title)?;
        validate_location(&location)?;
        if self.bookmarks.len() >= MAX_BOOKMARKS {
            return Err(BookmarksError::BookmarkLimitExceeded {
                found: self.bookmarks.len().saturating_add(1),
                limit: MAX_BOOKMARKS,
            });
        }
        let id = BookmarkId(self.next_bookmark_id);
        self.next_bookmark_id = self
            .next_bookmark_id
            .checked_add(1)
            .ok_or(BookmarksError::BookmarkIdExhausted)?;
        self.bookmarks.push(Bookmark {
            id,
            title,
            location,
        });
        Ok(id)
    }

    pub fn update_bookmark(
        &mut self,
        id: BookmarkId,
        title: impl Into<String>,
        location: impl Into<String>,
    ) -> Result<bool, BookmarksError> {
        let title = title.into();
        let location = location.into();
        validate_title(&title)?;
        validate_location(&location)?;
        let Some(bookmark) = self.bookmarks.iter_mut().find(|bookmark| bookmark.id == id) else {
            return Ok(false);
        };
        bookmark.title = title;
        bookmark.location = location;
        Ok(true)
    }

    pub fn remove_bookmark(&mut self, id: BookmarkId) -> Option<Bookmark> {
        let index = self.bookmarks.iter().position(|bookmark| bookmark.id == id)?;
        Some(self.bookmarks.remove(index))
    }

    pub fn clear(&mut self) {
        self.bookmarks.clear();
    }

    pub(crate) fn advance_generation_after_save(
        &mut self,
        expected_generation: u64,
        saved_generation: u64,
    ) -> bool {
        if self.generation != expected_generation || saved_generation <= expected_generation {
            return false;
        }
        self.generation = saved_generation;
        true
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookmarksRecovery {
    skipped_generations: Vec<u64>,
}

impl BookmarksRecovery {
    pub fn skipped_generations(&self) -> &[u64] {
        &self.skipped_generations
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookmarksLoad {
    snapshot: BookmarksSnapshot,
    recovery: Option<BookmarksRecovery>,
}

impl BookmarksLoad {
    pub const fn snapshot(&self) -> &BookmarksSnapshot {
        &self.snapshot
    }

    pub fn into_snapshot(self) -> BookmarksSnapshot {
        self.snapshot
    }

    pub const fn recovery(&self) -> Option<&BookmarksRecovery> {
        self.recovery.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookmarksCleanupWarning {
    generations: Vec<u64>,
    pending_files: Vec<PathBuf>,
}

impl BookmarksCleanupWarning {
    pub fn generations(&self) -> &[u64] {
        &self.generations
    }

    pub fn pending_files(&self) -> &[PathBuf] {
        &self.pending_files
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookmarksSave {
    snapshot: BookmarksSnapshot,
    cleanup_warning: Option<BookmarksCleanupWarning>,
}

impl BookmarksSave {
    pub const fn snapshot(&self) -> &BookmarksSnapshot {
        &self.snapshot
    }

    pub fn into_snapshot(self) -> BookmarksSnapshot {
        self.snapshot
    }

    pub const fn cleanup_warning(&self) -> Option<&BookmarksCleanupWarning> {
        self.cleanup_warning.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BookmarksError {
    Lock(ProfileLockError),
    Io {
        operation: &'static str,
        path: PathBuf,
        kind: io::ErrorKind,
    },
    TitleTooLarge {
        bytes: usize,
        limit: usize,
    },
    EmptyLocation,
    LocationTooLarge {
        bytes: usize,
        limit: usize,
    },
    BookmarkLimitExceeded {
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
    BookmarkIdExhausted,
    ConcurrentWrite {
        generation: u64,
    },
    PendingFileCollisionLimit {
        attempts: usize,
    },
}

impl fmt::Display for BookmarksError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lock(error) => error.fmt(formatter),
            Self::Io {
                operation,
                path,
                kind,
            } => write!(formatter, "{operation} failed for {}: {kind}", path.display()),
            Self::TitleTooLarge { bytes, limit } => write!(
                formatter,
                "bookmark title requires {bytes} bytes; limit is {limit}"
            ),
            Self::EmptyLocation => formatter.write_str("bookmark location is empty"),
            Self::LocationTooLarge { bytes, limit } => write!(
                formatter,
                "bookmark location requires {bytes} bytes; limit is {limit}"
            ),
            Self::BookmarkLimitExceeded { found, limit } => {
                write!(formatter, "bookmarks contain {found} entries; limit is {limit}")
            }
            Self::RecordTooLarge { bytes, limit } => write!(
                formatter,
                "bookmarks record requires {bytes} bytes; limit is {limit}"
            ),
            Self::DirectoryEntryLimitExceeded { found, limit } => write!(
                formatter,
                "bookmarks directory contains {found} entries; scan limit is {limit}"
            ),
            Self::GenerationFileLimitExceeded { found, limit } => write!(
                formatter,
                "bookmarks directory contains {found} generations; generation limit is {limit}"
            ),
            Self::UnsupportedSchema { generation, schema } => write!(
                formatter,
                "bookmarks generation {generation} uses unsupported schema {schema}"
            ),
            Self::NoValidGeneration {
                corrupt_generations,
            } => write!(
                formatter,
                "no valid bookmarks generation remains after corrupt generations {corrupt_generations:?}"
            ),
            Self::StaleGeneration { current, provided } => write!(
                formatter,
                "bookmarks generation {provided} is stale; current generation is {current}"
            ),
            Self::GenerationExhausted => {
                formatter.write_str("bookmarks generation space is exhausted")
            }
            Self::BookmarkIdExhausted => {
                formatter.write_str("bookmark identifier space is exhausted")
            }
            Self::ConcurrentWrite { generation } => {
                write!(formatter, "bookmarks generation {generation} was created concurrently")
            }
            Self::PendingFileCollisionLimit { attempts } => write!(
                formatter,
                "could not allocate a unique pending bookmarks file after {attempts} attempts"
            ),
        }
    }
}

impl std::error::Error for BookmarksError {}

#[derive(Clone, Debug)]
pub struct BookmarksStore {
    root: PathBuf,
    bookmarks_directory: PathBuf,
}

impl BookmarksStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, BookmarksError> {
        let root = root.into();
        create_directory(&root)?;
        let bookmarks_directory = root.join(BOOKMARKS_DIRECTORY);
        create_directory(&bookmarks_directory)?;
        Ok(Self {
            root,
            bookmarks_directory,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn load(&self) -> Result<BookmarksLoad, BookmarksError> {
        let generations = self.discover_generations()?;
        if generations.is_empty() {
            return Ok(BookmarksLoad {
                snapshot: BookmarksSnapshot::default(),
                recovery: None,
            });
        }

        let mut corrupt_generations = Vec::new();
        for generation in generations {
            let path = self.bookmarks_path(generation);
            let bytes = read_bounded(&path, MAX_BOOKMARKS_RECORD_BYTES)?;
            match decode_bookmarks(&bytes, generation) {
                Ok(snapshot) => {
                    let recovery = (!corrupt_generations.is_empty()).then_some(BookmarksRecovery {
                        skipped_generations: corrupt_generations,
                    });
                    return Ok(BookmarksLoad { snapshot, recovery });
                }
                Err(DecodeError::UnsupportedSchema(schema)) => {
                    return Err(BookmarksError::UnsupportedSchema { generation, schema });
                }
                Err(DecodeError::Corrupt) => corrupt_generations.push(generation),
            }
        }

        Err(BookmarksError::NoValidGeneration {
            corrupt_generations,
        })
    }

    pub fn save(
        &self,
        lock: &ProfileLock,
        snapshot: &BookmarksSnapshot,
    ) -> Result<BookmarksSave, BookmarksError> {
        lock.verify_for_root(&self.root)
            .map_err(BookmarksError::Lock)?;
        validate_snapshot(snapshot)?;
        let loaded = self.load()?;
        let current_generation = loaded.snapshot().generation();
        if current_generation != snapshot.generation {
            return Err(BookmarksError::StaleGeneration {
                current: current_generation,
                provided: snapshot.generation,
            });
        }

        let recovered = loaded
            .recovery()
            .map(BookmarksRecovery::skipped_generations)
            .unwrap_or(&[]);
        let generations = self.discover_generations_with_reserve(2)?;
        if let Some(unexpected) = unexpected_generation(current_generation, recovered, &generations)
        {
            return Err(BookmarksError::ConcurrentWrite {
                generation: unexpected,
            });
        }

        let previous_max = generations.first().copied().unwrap_or(0);
        let generation = previous_max
            .checked_add(1)
            .ok_or(BookmarksError::GenerationExhausted)?;
        let bytes = encode_bookmarks(snapshot, generation)?;
        let final_path = self.bookmarks_path(generation);
        let (pending_path, mut file) = self.create_pending_file(generation)?;

        if let Err(error) = file.write_all(&bytes) {
            let _ = fs::remove_file(&pending_path);
            return Err(io_error("write pending bookmarks generation", &pending_path, error));
        }
        if let Err(error) = file.sync_all() {
            let _ = fs::remove_file(&pending_path);
            return Err(io_error("sync pending bookmarks generation", &pending_path, error));
        }
        drop(file);

        if let Err(error) = lock.verify_for_root(&self.root) {
            let _ = fs::remove_file(&pending_path);
            return Err(BookmarksError::Lock(error));
        }

        match fs::hard_link(&pending_path, &final_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(&pending_path);
                return Err(BookmarksError::ConcurrentWrite { generation });
            }
            Err(error) => {
                let _ = fs::remove_file(&pending_path);
                return Err(io_error("publish bookmarks generation", &final_path, error));
            }
        }

        let mut pending_cleanup = Vec::new();
        if let Err(error) = fs::remove_file(&pending_path)
            && error.kind() != io::ErrorKind::NotFound
        {
            pending_cleanup.push(pending_path);
        }

        let mut saved = snapshot.clone();
        saved.generation = generation;
        let failed_generations = self.cleanup_generations(current_generation, &generations);
        let cleanup_warning = (!failed_generations.is_empty() || !pending_cleanup.is_empty())
            .then_some(BookmarksCleanupWarning {
                generations: failed_generations,
                pending_files: pending_cleanup,
            });
        Ok(BookmarksSave {
            snapshot: saved,
            cleanup_warning,
        })
    }

    fn discover_generations(&self) -> Result<Vec<u64>, BookmarksError> {
        self.discover_generations_with_reserve(0)
    }

    fn discover_generations_with_reserve(
        &self,
        reserved_entries: usize,
    ) -> Result<Vec<u64>, BookmarksError> {
        let entries = fs::read_dir(&self.bookmarks_directory)
            .map_err(|error| io_error("read bookmarks directory", &self.bookmarks_directory, error))?;
        let mut generations = Vec::new();
        let mut entry_count = 0usize;
        for entry in entries {
            entry_count = entry_count.saturating_add(1);
            let prospective = entry_count.saturating_add(reserved_entries);
            if prospective > MAX_BOOKMARKS_DIRECTORY_ENTRIES {
                return Err(BookmarksError::DirectoryEntryLimitExceeded {
                    found: prospective,
                    limit: MAX_BOOKMARKS_DIRECTORY_ENTRIES,
                });
            }
            let entry = entry.map_err(|error| {
                io_error(
                    "read bookmarks directory entry",
                    &self.bookmarks_directory,
                    error,
                )
            })?;
            let file_type = entry.file_type().map_err(|error| {
                io_error("inspect bookmarks directory entry", &entry.path(), error)
            })?;
            if !file_type.is_file() {
                continue;
            }
            if let Some(generation) = parse_generation_file_name(&entry.file_name()) {
                generations.push(generation);
                if generations.len() > MAX_DISCOVERED_GENERATIONS {
                    return Err(BookmarksError::GenerationFileLimitExceeded {
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
            .take(BOOKMARKS_RETAINED_GENERATIONS.saturating_sub(2))
            .collect::<Vec<_>>();
        let mut failed = Vec::new();
        for &generation in previous_generations {
            if generation == previous_current_generation || retained_older.contains(&generation) {
                continue;
            }
            let path = self.bookmarks_path(generation);
            if let Err(error) = fs::remove_file(&path)
                && error.kind() != io::ErrorKind::NotFound
            {
                failed.push(generation);
            }
        }
        failed
    }

    fn create_pending_file(&self, generation: u64) -> Result<(PathBuf, File), BookmarksError> {
        for _ in 0..MAX_PENDING_FILE_ATTEMPTS {
            let sequence = NEXT_PENDING_FILE.fetch_add(1, Ordering::Relaxed);
            let name = format!(
                "{PENDING_FILE_PREFIX}{generation:020}-{:010}-{sequence:020}.tmp",
                std::process::id()
            );
            let path = self.bookmarks_directory.join(name);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok((path, file)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(io_error("create pending bookmarks generation", &path, error));
                }
            }
        }
        Err(BookmarksError::PendingFileCollisionLimit {
            attempts: MAX_PENDING_FILE_ATTEMPTS,
        })
    }

    fn bookmarks_path(&self, generation: u64) -> PathBuf {
        self.bookmarks_directory
            .join(bookmarks_file_name(generation))
    }
}

fn create_directory(path: &Path) -> Result<(), BookmarksError> {
    fs::create_dir_all(path).map_err(|error| io_error("create profile directory", path, error))
}

fn validate_title(title: &str) -> Result<(), BookmarksError> {
    if title.len() > MAX_BOOKMARK_TITLE_BYTES {
        return Err(BookmarksError::TitleTooLarge {
            bytes: title.len(),
            limit: MAX_BOOKMARK_TITLE_BYTES,
        });
    }
    Ok(())
}

fn validate_location(location: &str) -> Result<(), BookmarksError> {
    if location.is_empty() {
        return Err(BookmarksError::EmptyLocation);
    }
    if location.len() > MAX_BOOKMARK_LOCATION_BYTES {
        return Err(BookmarksError::LocationTooLarge {
            bytes: location.len(),
            limit: MAX_BOOKMARK_LOCATION_BYTES,
        });
    }
    Ok(())
}

fn validate_snapshot(snapshot: &BookmarksSnapshot) -> Result<(), BookmarksError> {
    if snapshot.bookmarks.len() > MAX_BOOKMARKS {
        return Err(BookmarksError::BookmarkLimitExceeded {
            found: snapshot.bookmarks.len(),
            limit: MAX_BOOKMARKS,
        });
    }
    if snapshot.next_bookmark_id == 0 {
        return Err(BookmarksError::BookmarkIdExhausted);
    }
    let mut previous = 0u64;
    for bookmark in &snapshot.bookmarks {
        validate_title(&bookmark.title)?;
        validate_location(&bookmark.location)?;
        if bookmark.id.0 == 0
            || bookmark.id.0 <= previous
            || bookmark.id.0 >= snapshot.next_bookmark_id
        {
            return Err(BookmarksError::BookmarkIdExhausted);
        }
        previous = bookmark.id.0;
    }
    Ok(())
}

fn unexpected_generation(current: u64, recovered: &[u64], generations: &[u64]) -> Option<u64> {
    generations
        .iter()
        .find(|&&generation| generation > current && !recovered.contains(&generation))
        .copied()
}

fn bookmarks_file_name(generation: u64) -> String {
    format!("{BOOKMARKS_FILE_PREFIX}{generation:020}{BOOKMARKS_FILE_SUFFIX}")
}

fn parse_generation_file_name(name: &std::ffi::OsStr) -> Option<u64> {
    let name = name.to_str()?;
    let digits = name
        .strip_prefix(BOOKMARKS_FILE_PREFIX)?
        .strip_suffix(BOOKMARKS_FILE_SUFFIX)?;
    if digits.len() != BOOKMARKS_GENERATION_DIGITS
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let generation = digits.parse::<u64>().ok()?;
    (generation != 0).then_some(generation)
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, BookmarksError> {
    let file = File::open(path).map_err(|error| io_error("open bookmarks generation", path, error))?;
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error("read bookmarks generation", path, error))?;
    Ok(bytes)
}

fn encode_bookmarks(
    snapshot: &BookmarksSnapshot,
    generation: u64,
) -> Result<Vec<u8>, BookmarksError> {
    validate_snapshot(snapshot)?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&BOOKMARKS_MAGIC);
    bytes.extend_from_slice(&BOOKMARKS_SCHEMA_VERSION.to_le_bytes());
    bytes.extend_from_slice(&generation.to_le_bytes());
    bytes.extend_from_slice(&snapshot.next_bookmark_id.to_le_bytes());
    bytes.extend_from_slice(&(snapshot.bookmarks.len() as u32).to_le_bytes());
    for bookmark in &snapshot.bookmarks {
        bytes.extend_from_slice(&bookmark.id.0.to_le_bytes());
        bytes.extend_from_slice(&(bookmark.title.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(bookmark.location.len() as u32).to_le_bytes());
        bytes.extend_from_slice(bookmark.title.as_bytes());
        bytes.extend_from_slice(bookmark.location.as_bytes());
    }
    let final_size = bytes.len().saturating_add(CHECKSUM_BYTES);
    if final_size > MAX_BOOKMARKS_RECORD_BYTES {
        return Err(BookmarksError::RecordTooLarge {
            bytes: final_size,
            limit: MAX_BOOKMARKS_RECORD_BYTES,
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

fn decode_bookmarks(
    bytes: &[u8],
    expected_generation: u64,
) -> Result<BookmarksSnapshot, DecodeError> {
    if bytes.len() < 12 || bytes[..8] != BOOKMARKS_MAGIC {
        return Err(DecodeError::Corrupt);
    }
    let schema = u32::from_le_bytes(bytes[8..12].try_into().map_err(|_| DecodeError::Corrupt)?);
    if schema != BOOKMARKS_SCHEMA_VERSION {
        return Err(DecodeError::UnsupportedSchema(schema));
    }
    if bytes.len() < MIN_RECORD_BYTES || bytes.len() > MAX_BOOKMARKS_RECORD_BYTES {
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
    if cursor.take_array::<8>()? != BOOKMARKS_MAGIC {
        return Err(DecodeError::Corrupt);
    }
    let decoded_schema = u32::from_le_bytes(cursor.take_array::<4>()?);
    debug_assert_eq!(decoded_schema, BOOKMARKS_SCHEMA_VERSION);
    let generation = u64::from_le_bytes(cursor.take_array::<8>()?);
    if generation == 0 || generation != expected_generation {
        return Err(DecodeError::Corrupt);
    }
    let next_bookmark_id = u64::from_le_bytes(cursor.take_array::<8>()?);
    if next_bookmark_id == 0 {
        return Err(DecodeError::Corrupt);
    }
    let count = u32::from_le_bytes(cursor.take_array::<4>()?) as usize;
    if count > MAX_BOOKMARKS {
        return Err(DecodeError::Corrupt);
    }

    let mut bookmarks = Vec::with_capacity(count);
    let mut previous = 0u64;
    for _ in 0..count {
        let id = u64::from_le_bytes(cursor.take_array::<8>()?);
        let title_len = u32::from_le_bytes(cursor.take_array::<4>()?) as usize;
        let location_len = u32::from_le_bytes(cursor.take_array::<4>()?) as usize;
        if id == 0
            || id <= previous
            || id >= next_bookmark_id
            || title_len > MAX_BOOKMARK_TITLE_BYTES
            || location_len == 0
            || location_len > MAX_BOOKMARK_LOCATION_BYTES
        {
            return Err(DecodeError::Corrupt);
        }
        let title = std::str::from_utf8(cursor.take(title_len)?)
            .map_err(|_| DecodeError::Corrupt)?
            .to_owned();
        let location = std::str::from_utf8(cursor.take(location_len)?)
            .map_err(|_| DecodeError::Corrupt)?
            .to_owned();
        if validate_title(&title).is_err() || validate_location(&location).is_err() {
            return Err(DecodeError::Corrupt);
        }
        bookmarks.push(Bookmark {
            id: BookmarkId(id),
            title,
            location,
        });
        previous = id;
    }
    if !cursor.is_finished() {
        return Err(DecodeError::Corrupt);
    }

    Ok(BookmarksSnapshot {
        generation,
        next_bookmark_id,
        bookmarks,
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

fn io_error(operation: &'static str, path: &Path, error: io::Error) -> BookmarksError {
    BookmarksError::Io {
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
                "zorya-bookmarks-test-{}-{sequence}",
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

    fn snapshot_with(title: &str, location: &str) -> BookmarksSnapshot {
        let mut snapshot = BookmarksSnapshot::default();
        snapshot.add_bookmark(title, location).unwrap();
        snapshot
    }

    fn raw_record(
        generation: u64,
        schema: u32,
        next_bookmark_id: u64,
        bookmarks: &[(u64, &str, &str)],
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&BOOKMARKS_MAGIC);
        bytes.extend_from_slice(&schema.to_le_bytes());
        bytes.extend_from_slice(&generation.to_le_bytes());
        bytes.extend_from_slice(&next_bookmark_id.to_le_bytes());
        bytes.extend_from_slice(&(bookmarks.len() as u32).to_le_bytes());
        for (id, title, location) in bookmarks {
            bytes.extend_from_slice(&id.to_le_bytes());
            bytes.extend_from_slice(&(title.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(location.len() as u32).to_le_bytes());
            bytes.extend_from_slice(title.as_bytes());
            bytes.extend_from_slice(location.as_bytes());
        }
        let checksum = checksum64(&bytes);
        bytes.extend_from_slice(&checksum.to_le_bytes());
        bytes
    }

    #[test]
    fn snapshot_uses_stable_monotonic_identity_across_edit_and_clear() {
        let mut snapshot = BookmarksSnapshot::default();
        let first = snapshot
            .add_bookmark("One", "https://one.test")
            .unwrap();
        assert!(snapshot
            .update_bookmark(first, "Updated", "https://one.test/new")
            .unwrap());
        assert_eq!(snapshot.bookmark(first).unwrap().title(), "Updated");
        snapshot.clear();
        let second = snapshot
            .add_bookmark("Two", "https://two.test")
            .unwrap();
        assert!(second > first);
        assert!(!snapshot
            .update_bookmark(first, "Missing", "https://missing.test")
            .unwrap());
    }

    #[test]
    fn bounds_are_rejected_before_mutation() {
        let mut snapshot = BookmarksSnapshot::default();
        assert!(matches!(
            snapshot.add_bookmark("x".repeat(MAX_BOOKMARK_TITLE_BYTES + 1), "https://one.test"),
            Err(BookmarksError::TitleTooLarge { .. })
        ));
        assert_eq!(
            snapshot.add_bookmark("One", ""),
            Err(BookmarksError::EmptyLocation)
        );
        assert!(matches!(
            snapshot.add_bookmark("One", "x".repeat(MAX_BOOKMARK_LOCATION_BYTES + 1)),
            Err(BookmarksError::LocationTooLarge { .. })
        ));
        assert!(snapshot.is_empty());
    }

    #[test]
    fn codec_round_trips_generation_data_and_next_identity() {
        let mut snapshot = BookmarksSnapshot::default();
        snapshot
            .add_bookmark("One", "https://one.test")
            .unwrap();
        snapshot
            .add_bookmark("Two", "https://two.test/path")
            .unwrap();
        let encoded = encode_bookmarks(&snapshot, 7).unwrap();
        let mut decoded = decode_bookmarks(&encoded, 7).unwrap();
        assert_eq!(decoded.generation(), 7);
        assert_eq!(decoded.bookmarks(), snapshot.bookmarks());
        assert_eq!(
            decoded
                .add_bookmark("Three", "https://three.test")
                .unwrap()
                .get(),
            3
        );
    }

    #[test]
    fn codec_rejects_checksum_failure_identity_order_and_newer_schema() {
        let bytes = raw_record(
            1,
            BOOKMARKS_SCHEMA_VERSION,
            3,
            &[(1, "One", "https://one.test"), (2, "Two", "https://two.test")],
        );
        let mut corrupt = bytes.clone();
        let index = corrupt.len() - CHECKSUM_BYTES - 1;
        corrupt[index] ^= 1;
        assert!(matches!(decode_bookmarks(&corrupt, 1), Err(DecodeError::Corrupt)));

        let duplicate = raw_record(
            1,
            BOOKMARKS_SCHEMA_VERSION,
            3,
            &[(2, "One", "https://one.test"), (2, "Two", "https://two.test")],
        );
        assert!(matches!(decode_bookmarks(&duplicate, 1), Err(DecodeError::Corrupt)));

        let newer = raw_record(4, BOOKMARKS_SCHEMA_VERSION + 1, 1, &[]);
        assert!(matches!(
            decode_bookmarks(&newer, 4),
            Err(DecodeError::UnsupportedSchema(schema)) if schema == BOOKMARKS_SCHEMA_VERSION + 1
        ));
    }

    #[test]
    fn save_load_and_stale_writer_rejection_preserve_bookmarks() {
        let directory = TestDirectory::new();
        let store = BookmarksStore::open(directory.path()).unwrap();
        let lock = ProfileLock::acquire(directory.path()).unwrap();
        let first = store
            .save(&lock, &snapshot_with("One", "https://one.test"))
            .unwrap()
            .into_snapshot();
        let stale = first.clone();
        let mut second_input = first;
        second_input
            .add_bookmark("Two", "https://two.test")
            .unwrap();
        let second = store.save(&lock, &second_input).unwrap().into_snapshot();
        assert_eq!(second.generation(), 2);
        assert_eq!(
            store.save(&lock, &stale),
            Err(BookmarksError::StaleGeneration {
                current: 2,
                provided: 1,
            })
        );
        assert_eq!(store.load().unwrap().snapshot(), &second);
        lock.release().unwrap();
    }

    #[test]
    fn corrupt_newest_generation_recovers_and_skips_forward_on_save() {
        let directory = TestDirectory::new();
        let store = BookmarksStore::open(directory.path()).unwrap();
        let lock = ProfileLock::acquire(directory.path()).unwrap();
        let first = store
            .save(&lock, &snapshot_with("One", "https://one.test"))
            .unwrap()
            .into_snapshot();
        let mut second_input = first.clone();
        second_input
            .add_bookmark("Two", "https://two.test")
            .unwrap();
        let second = store.save(&lock, &second_input).unwrap().into_snapshot();
        let second_path = store.bookmarks_path(second.generation());
        let mut bytes = fs::read(&second_path).unwrap();
        let index = bytes.len() - CHECKSUM_BYTES - 1;
        bytes[index] ^= 1;
        fs::write(&second_path, bytes).unwrap();

        let recovered = store.load().unwrap();
        assert_eq!(recovered.snapshot(), &first);
        assert_eq!(recovered.recovery().unwrap().skipped_generations(), &[2]);
        let third = store.save(&lock, recovered.snapshot()).unwrap().into_snapshot();
        assert_eq!(third.generation(), 3);
        assert_eq!(store.load().unwrap().snapshot(), &third);
        lock.release().unwrap();
    }

    #[test]
    fn unsupported_persisted_schema_fails_closed() {
        let directory = TestDirectory::new();
        let store = BookmarksStore::open(directory.path()).unwrap();
        let path = store.bookmarks_path(1);
        fs::write(
            &path,
            raw_record(1, BOOKMARKS_SCHEMA_VERSION + 1, 1, &[]),
        )
        .unwrap();
        assert_eq!(
            store.load(),
            Err(BookmarksError::UnsupportedSchema {
                generation: 1,
                schema: BOOKMARKS_SCHEMA_VERSION + 1,
            })
        );
    }

    #[test]
    fn pending_files_are_ignored_but_directory_scans_are_bounded() {
        let directory = TestDirectory::new();
        let store = BookmarksStore::open(directory.path()).unwrap();
        let pending = store
            .bookmarks_directory
            .join(".pending-bookmarks-00000000000000000001-0000000001-00000000000000000001.tmp");
        fs::write(pending, b"partial").unwrap();
        assert!(store.load().unwrap().snapshot().is_empty());

        for index in 1..MAX_BOOKMARKS_DIRECTORY_ENTRIES {
            fs::write(store.bookmarks_directory.join(format!("junk-{index}")), b"x").unwrap();
        }
        assert!(matches!(
            store.discover_generations_with_reserve(1),
            Err(BookmarksError::DirectoryEntryLimitExceeded { .. })
        ));
    }

    #[test]
    fn concurrent_saves_have_one_durable_winner() {
        let directory = TestDirectory::new();
        let store = Arc::new(BookmarksStore::open(directory.path()).unwrap());
        let lock = Arc::new(ProfileLock::acquire(directory.path()).unwrap());
        let barrier = Arc::new(Barrier::new(3));
        let workers = [("One", "https://one.test"), ("Two", "https://two.test")]
            .into_iter()
            .map(|(title, location)| {
                let store = Arc::clone(&store);
                let lock = Arc::clone(&lock);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let snapshot = snapshot_with(title, location);
                    barrier.wait();
                    store.save(lock.as_ref(), &snapshot)
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
        assert!(results.iter().filter_map(|result| result.as_ref().err()).all(|error| {
            matches!(
                error,
                BookmarksError::ConcurrentWrite { .. } | BookmarksError::StaleGeneration { .. }
            )
        }));
        assert_eq!(store.load().unwrap().snapshot().generation(), 1);
    }
}
