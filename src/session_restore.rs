use crate::profile_lock::{ProfileLock, ProfileLockError};
use std::collections::BTreeSet;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const SESSION_RESTORE_SCHEMA_VERSION: u32 = 1;
pub const MAX_SESSION_WINDOWS: usize = 16;
pub const MAX_SESSION_TABS_PER_WINDOW: usize = 128;
pub const MAX_SESSION_TABS: usize = 512;
pub const MAX_SESSION_LOCATION_BYTES: usize = 4 * 1024;
pub const MAX_SESSION_RESTORE_RECORD_BYTES: usize = 3 * 1024 * 1024;

const SESSION_MAGIC: [u8; 8] = *b"ZRYSES01";
const SESSION_DIRECTORY: &str = "session";
const SESSION_FILE_PREFIX: &str = "session-";
const SESSION_FILE_SUFFIX: &str = ".bin";
const SESSION_GENERATION_DIGITS: usize = 20;
const SESSION_RETAINED_GENERATIONS: usize = 3;
const MAX_SESSION_DIRECTORY_ENTRIES: usize = 128;
const MAX_DISCOVERED_GENERATIONS: usize = 64;
const MAX_PENDING_FILE_ATTEMPTS: usize = 8;
const CHECKSUM_BYTES: usize = 8;
const PENDING_FILE_PREFIX: &str = ".pending-session-";
const MIN_RECORD_BYTES: usize = 8 + 4 + 8 + 8 + 8 + 4 + CHECKSUM_BYTES;
static NEXT_PENDING_FILE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionWindowId(u64);

impl SessionWindowId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionTabId(u64);

impl SessionTabId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionTab {
    id: SessionTabId,
    location: String,
}

impl SessionTab {
    pub const fn id(&self) -> SessionTabId {
        self.id
    }

    pub fn location(&self) -> &str {
        &self.location
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionWindow {
    id: SessionWindowId,
    active_tab: Option<SessionTabId>,
    tabs: Vec<SessionTab>,
}

impl SessionWindow {
    pub const fn id(&self) -> SessionWindowId {
        self.id
    }

    pub const fn active_tab(&self) -> Option<SessionTabId> {
        self.active_tab
    }

    pub fn tabs(&self) -> &[SessionTab] {
        &self.tabs
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRestoreSnapshot {
    generation: u64,
    next_window_id: u64,
    next_tab_id: u64,
    windows: Vec<SessionWindow>,
}

impl Default for SessionRestoreSnapshot {
    fn default() -> Self {
        Self {
            generation: 0,
            next_window_id: 1,
            next_tab_id: 1,
            windows: Vec::new(),
        }
    }
}

impl SessionRestoreSnapshot {
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    pub fn windows(&self) -> &[SessionWindow] {
        &self.windows
    }

    pub fn window(&self, id: SessionWindowId) -> Option<&SessionWindow> {
        self.windows.iter().find(|window| window.id == id)
    }

    pub fn total_tabs(&self) -> usize {
        self.windows.iter().map(|window| window.tabs.len()).sum()
    }

    pub fn add_window(&mut self) -> Result<SessionWindowId, SessionRestoreError> {
        if self.windows.len() >= MAX_SESSION_WINDOWS {
            return Err(SessionRestoreError::WindowLimitExceeded {
                found: self.windows.len().saturating_add(1),
                limit: MAX_SESSION_WINDOWS,
            });
        }
        let id = SessionWindowId(self.next_window_id);
        let next_window_id = self
            .next_window_id
            .checked_add(1)
            .ok_or(SessionRestoreError::WindowIdExhausted)?;
        self.windows.push(SessionWindow {
            id,
            active_tab: None,
            tabs: Vec::new(),
        });
        self.next_window_id = next_window_id;
        Ok(id)
    }

    pub fn remove_window(&mut self, id: SessionWindowId) -> Option<SessionWindow> {
        let index = self.windows.iter().position(|window| window.id == id)?;
        Some(self.windows.remove(index))
    }

    pub fn add_tab(
        &mut self,
        window: SessionWindowId,
        location: impl Into<String>,
    ) -> Result<SessionTabId, SessionRestoreError> {
        let location = location.into();
        validate_location(&location)?;
        let window_index = self
            .windows
            .iter()
            .position(|candidate| candidate.id == window)
            .ok_or(SessionRestoreError::WindowNotFound { window })?;
        if self.windows[window_index].tabs.len() >= MAX_SESSION_TABS_PER_WINDOW {
            return Err(SessionRestoreError::TabsPerWindowLimitExceeded {
                found: self.windows[window_index].tabs.len().saturating_add(1),
                limit: MAX_SESSION_TABS_PER_WINDOW,
            });
        }
        let total = self.total_tabs();
        if total >= MAX_SESSION_TABS {
            return Err(SessionRestoreError::TabLimitExceeded {
                found: total.saturating_add(1),
                limit: MAX_SESSION_TABS,
            });
        }
        let id = SessionTabId(self.next_tab_id);
        let next_tab_id = self
            .next_tab_id
            .checked_add(1)
            .ok_or(SessionRestoreError::TabIdExhausted)?;
        let target = &mut self.windows[window_index];
        target.tabs.push(SessionTab { id, location });
        if target.active_tab.is_none() {
            target.active_tab = Some(id);
        }
        self.next_tab_id = next_tab_id;
        Ok(id)
    }

    pub fn update_tab_location(
        &mut self,
        window: SessionWindowId,
        tab: SessionTabId,
        location: impl Into<String>,
    ) -> Result<bool, SessionRestoreError> {
        let location = location.into();
        validate_location(&location)?;
        let target = self
            .windows
            .iter_mut()
            .find(|candidate| candidate.id == window)
            .ok_or(SessionRestoreError::WindowNotFound { window })?;
        let tab = target
            .tabs
            .iter_mut()
            .find(|candidate| candidate.id == tab)
            .ok_or(SessionRestoreError::TabNotFound { window, tab })?;
        if tab.location == location {
            return Ok(false);
        }
        tab.location = location;
        Ok(true)
    }

    pub fn set_active_tab(
        &mut self,
        window: SessionWindowId,
        tab: SessionTabId,
    ) -> Result<bool, SessionRestoreError> {
        let target = self
            .windows
            .iter_mut()
            .find(|candidate| candidate.id == window)
            .ok_or(SessionRestoreError::WindowNotFound { window })?;
        if !target.tabs.iter().any(|candidate| candidate.id == tab) {
            return Err(SessionRestoreError::TabNotFound { window, tab });
        }
        if target.active_tab == Some(tab) {
            return Ok(false);
        }
        target.active_tab = Some(tab);
        Ok(true)
    }

    pub fn remove_tab(
        &mut self,
        window: SessionWindowId,
        tab: SessionTabId,
    ) -> Result<Option<SessionTab>, SessionRestoreError> {
        let target = self
            .windows
            .iter_mut()
            .find(|candidate| candidate.id == window)
            .ok_or(SessionRestoreError::WindowNotFound { window })?;
        let Some(index) = target.tabs.iter().position(|candidate| candidate.id == tab) else {
            return Ok(None);
        };
        let removed = target.tabs.remove(index);
        if target.active_tab == Some(tab) {
            target.active_tab = target.tabs.first().map(SessionTab::id);
        }
        Ok(Some(removed))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRestoreRecovery {
    skipped_generations: Vec<u64>,
}

impl SessionRestoreRecovery {
    pub fn skipped_generations(&self) -> &[u64] {
        &self.skipped_generations
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRestoreLoad {
    snapshot: SessionRestoreSnapshot,
    recovery: Option<SessionRestoreRecovery>,
}

impl SessionRestoreLoad {
    pub const fn snapshot(&self) -> &SessionRestoreSnapshot {
        &self.snapshot
    }

    pub fn into_snapshot(self) -> SessionRestoreSnapshot {
        self.snapshot
    }

    pub const fn recovery(&self) -> Option<&SessionRestoreRecovery> {
        self.recovery.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRestoreCleanupWarning {
    generations: Vec<u64>,
    pending_files: Vec<PathBuf>,
}

impl SessionRestoreCleanupWarning {
    pub fn generations(&self) -> &[u64] {
        &self.generations
    }

    pub fn pending_files(&self) -> &[PathBuf] {
        &self.pending_files
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRestoreSave {
    snapshot: SessionRestoreSnapshot,
    cleanup_warning: Option<SessionRestoreCleanupWarning>,
}

impl SessionRestoreSave {
    pub const fn snapshot(&self) -> &SessionRestoreSnapshot {
        &self.snapshot
    }

    pub fn into_snapshot(self) -> SessionRestoreSnapshot {
        self.snapshot
    }

    pub const fn cleanup_warning(&self) -> Option<&SessionRestoreCleanupWarning> {
        self.cleanup_warning.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionRestoreError {
    Lock(ProfileLockError),
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
    WindowLimitExceeded {
        found: usize,
        limit: usize,
    },
    TabsPerWindowLimitExceeded {
        found: usize,
        limit: usize,
    },
    TabLimitExceeded {
        found: usize,
        limit: usize,
    },
    WindowNotFound {
        window: SessionWindowId,
    },
    TabNotFound {
        window: SessionWindowId,
        tab: SessionTabId,
    },
    InvalidActiveTab {
        window: SessionWindowId,
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
    WindowIdExhausted,
    TabIdExhausted,
    ConcurrentWrite {
        generation: u64,
    },
    PendingFileCollisionLimit {
        attempts: usize,
    },
}

impl fmt::Display for SessionRestoreError {
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
            Self::EmptyLocation => formatter.write_str("session tab location is empty"),
            Self::LocationTooLarge { bytes, limit } => write!(
                formatter,
                "session tab location requires {bytes} bytes; limit is {limit}"
            ),
            Self::WindowLimitExceeded { found, limit } => write!(
                formatter,
                "session contains {found} windows; limit is {limit}"
            ),
            Self::TabsPerWindowLimitExceeded { found, limit } => write!(
                formatter,
                "session window contains {found} tabs; limit is {limit}"
            ),
            Self::TabLimitExceeded { found, limit } => {
                write!(formatter, "session contains {found} tabs; limit is {limit}")
            }
            Self::WindowNotFound { window } => {
                write!(formatter, "session window {} does not exist", window.get())
            }
            Self::TabNotFound { window, tab } => write!(
                formatter,
                "session tab {} does not exist in window {}",
                tab.get(),
                window.get()
            ),
            Self::InvalidActiveTab { window } => write!(
                formatter,
                "session window {} has an invalid active-tab identity",
                window.get()
            ),
            Self::RecordTooLarge { bytes, limit } => write!(
                formatter,
                "session restore record requires {bytes} bytes; limit is {limit}"
            ),
            Self::DirectoryEntryLimitExceeded { found, limit } => write!(
                formatter,
                "session directory contains {found} entries; scan limit is {limit}"
            ),
            Self::GenerationFileLimitExceeded { found, limit } => write!(
                formatter,
                "session directory contains {found} generations; generation limit is {limit}"
            ),
            Self::UnsupportedSchema { generation, schema } => write!(
                formatter,
                "session generation {generation} uses unsupported schema {schema}"
            ),
            Self::NoValidGeneration {
                corrupt_generations,
            } => write!(
                formatter,
                "no valid session generation remains after corrupt generations {corrupt_generations:?}"
            ),
            Self::StaleGeneration { current, provided } => write!(
                formatter,
                "session generation {provided} is stale; current generation is {current}"
            ),
            Self::GenerationExhausted => {
                formatter.write_str("session generation space is exhausted")
            }
            Self::WindowIdExhausted => {
                formatter.write_str("session window identifier space is exhausted")
            }
            Self::TabIdExhausted => {
                formatter.write_str("session tab identifier space is exhausted")
            }
            Self::ConcurrentWrite { generation } => write!(
                formatter,
                "session generation {generation} was created concurrently"
            ),
            Self::PendingFileCollisionLimit { attempts } => write!(
                formatter,
                "could not allocate a unique pending session file after {attempts} attempts"
            ),
        }
    }
}

impl std::error::Error for SessionRestoreError {}

#[derive(Clone, Debug)]
pub struct SessionRestoreStore {
    root: PathBuf,
    session_directory: PathBuf,
}

impl SessionRestoreStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, SessionRestoreError> {
        let root = root.into();
        create_directory(&root)?;
        let session_directory = root.join(SESSION_DIRECTORY);
        create_directory(&session_directory)?;
        Ok(Self {
            root,
            session_directory,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn load(&self) -> Result<SessionRestoreLoad, SessionRestoreError> {
        let generations = self.discover_generations()?;
        if generations.is_empty() {
            return Ok(SessionRestoreLoad {
                snapshot: SessionRestoreSnapshot::default(),
                recovery: None,
            });
        }

        let mut corrupt_generations = Vec::new();
        for generation in generations {
            let path = self.session_path(generation);
            let bytes = read_bounded(&path, MAX_SESSION_RESTORE_RECORD_BYTES)?;
            match decode_session(&bytes, generation) {
                Ok(snapshot) => {
                    let recovery =
                        (!corrupt_generations.is_empty()).then_some(SessionRestoreRecovery {
                            skipped_generations: corrupt_generations,
                        });
                    return Ok(SessionRestoreLoad { snapshot, recovery });
                }
                Err(DecodeError::UnsupportedSchema(schema)) => {
                    return Err(SessionRestoreError::UnsupportedSchema { generation, schema });
                }
                Err(DecodeError::Corrupt) => corrupt_generations.push(generation),
            }
        }

        Err(SessionRestoreError::NoValidGeneration {
            corrupt_generations,
        })
    }

    pub fn save(
        &self,
        lock: &ProfileLock,
        snapshot: &SessionRestoreSnapshot,
    ) -> Result<SessionRestoreSave, SessionRestoreError> {
        lock.verify_for_root(&self.root)
            .map_err(SessionRestoreError::Lock)?;
        validate_snapshot(snapshot)?;
        let loaded = self.load()?;
        let current_generation = loaded.snapshot().generation();
        if current_generation != snapshot.generation {
            return Err(SessionRestoreError::StaleGeneration {
                current: current_generation,
                provided: snapshot.generation,
            });
        }

        let recovered = loaded
            .recovery()
            .map(SessionRestoreRecovery::skipped_generations)
            .unwrap_or(&[]);
        let generations = self.discover_generations_with_reserve(2)?;
        if let Some(unexpected) = unexpected_generation(current_generation, recovered, &generations)
        {
            return Err(SessionRestoreError::ConcurrentWrite {
                generation: unexpected,
            });
        }

        let previous_max = generations.first().copied().unwrap_or(0);
        let generation = previous_max
            .checked_add(1)
            .ok_or(SessionRestoreError::GenerationExhausted)?;
        let bytes = encode_session(snapshot, generation)?;
        let final_path = self.session_path(generation);
        let (pending_path, mut file) = self.create_pending_file(generation)?;

        if let Err(error) = file.write_all(&bytes) {
            let _ = fs::remove_file(&pending_path);
            return Err(io_error(
                "write pending session generation",
                &pending_path,
                error,
            ));
        }
        if let Err(error) = file.sync_all() {
            let _ = fs::remove_file(&pending_path);
            return Err(io_error(
                "sync pending session generation",
                &pending_path,
                error,
            ));
        }
        drop(file);

        if let Err(error) = lock.verify_for_root(&self.root) {
            let _ = fs::remove_file(&pending_path);
            return Err(SessionRestoreError::Lock(error));
        }

        match fs::hard_link(&pending_path, &final_path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let _ = fs::remove_file(&pending_path);
                return Err(SessionRestoreError::ConcurrentWrite { generation });
            }
            Err(error) => {
                let _ = fs::remove_file(&pending_path);
                return Err(io_error("publish session generation", &final_path, error));
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
            .then_some(SessionRestoreCleanupWarning {
                generations: failed_generations,
                pending_files: pending_cleanup,
            });
        Ok(SessionRestoreSave {
            snapshot: saved,
            cleanup_warning,
        })
    }

    fn discover_generations(&self) -> Result<Vec<u64>, SessionRestoreError> {
        self.discover_generations_with_reserve(0)
    }

    fn discover_generations_with_reserve(
        &self,
        reserved_entries: usize,
    ) -> Result<Vec<u64>, SessionRestoreError> {
        let entries = fs::read_dir(&self.session_directory)
            .map_err(|error| io_error("read session directory", &self.session_directory, error))?;
        let mut generations = Vec::new();
        let mut entry_count = 0usize;
        for entry in entries {
            entry_count = entry_count.saturating_add(1);
            let prospective = entry_count.saturating_add(reserved_entries);
            if prospective > MAX_SESSION_DIRECTORY_ENTRIES {
                return Err(SessionRestoreError::DirectoryEntryLimitExceeded {
                    found: prospective,
                    limit: MAX_SESSION_DIRECTORY_ENTRIES,
                });
            }
            let entry = entry.map_err(|error| {
                io_error(
                    "read session directory entry",
                    &self.session_directory,
                    error,
                )
            })?;
            let file_type = entry.file_type().map_err(|error| {
                io_error("inspect session directory entry", &entry.path(), error)
            })?;
            if !file_type.is_file() {
                continue;
            }
            if let Some(generation) = parse_generation_file_name(&entry.file_name()) {
                generations.push(generation);
                if generations.len() > MAX_DISCOVERED_GENERATIONS {
                    return Err(SessionRestoreError::GenerationFileLimitExceeded {
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
            .take(SESSION_RETAINED_GENERATIONS.saturating_sub(2))
            .collect::<Vec<_>>();
        let mut failed = Vec::new();
        for &generation in previous_generations {
            if generation == previous_current_generation || retained_older.contains(&generation) {
                continue;
            }
            let path = self.session_path(generation);
            if let Err(error) = fs::remove_file(&path)
                && error.kind() != io::ErrorKind::NotFound
            {
                failed.push(generation);
            }
        }
        failed
    }

    fn create_pending_file(&self, generation: u64) -> Result<(PathBuf, File), SessionRestoreError> {
        for _ in 0..MAX_PENDING_FILE_ATTEMPTS {
            let sequence = NEXT_PENDING_FILE.fetch_add(1, Ordering::Relaxed);
            let name = format!(
                "{PENDING_FILE_PREFIX}{generation:020}-{:010}-{sequence:020}.tmp",
                std::process::id()
            );
            let path = self.session_directory.join(name);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok((path, file)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(io_error("create pending session generation", &path, error));
                }
            }
        }
        Err(SessionRestoreError::PendingFileCollisionLimit {
            attempts: MAX_PENDING_FILE_ATTEMPTS,
        })
    }

    fn session_path(&self, generation: u64) -> PathBuf {
        self.session_directory.join(session_file_name(generation))
    }
}

fn create_directory(path: &Path) -> Result<(), SessionRestoreError> {
    fs::create_dir_all(path).map_err(|error| io_error("create profile directory", path, error))
}

fn validate_location(location: &str) -> Result<(), SessionRestoreError> {
    if location.is_empty() {
        return Err(SessionRestoreError::EmptyLocation);
    }
    if location.len() > MAX_SESSION_LOCATION_BYTES {
        return Err(SessionRestoreError::LocationTooLarge {
            bytes: location.len(),
            limit: MAX_SESSION_LOCATION_BYTES,
        });
    }
    Ok(())
}

fn validate_snapshot(snapshot: &SessionRestoreSnapshot) -> Result<(), SessionRestoreError> {
    if snapshot.windows.len() > MAX_SESSION_WINDOWS {
        return Err(SessionRestoreError::WindowLimitExceeded {
            found: snapshot.windows.len(),
            limit: MAX_SESSION_WINDOWS,
        });
    }
    if snapshot.next_window_id == 0 {
        return Err(SessionRestoreError::WindowIdExhausted);
    }
    if snapshot.next_tab_id == 0 {
        return Err(SessionRestoreError::TabIdExhausted);
    }

    let mut window_ids = BTreeSet::new();
    let mut tab_ids = BTreeSet::new();
    let mut total_tabs = 0usize;
    for window in &snapshot.windows {
        if window.id.0 == 0
            || window.id.0 >= snapshot.next_window_id
            || !window_ids.insert(window.id.0)
        {
            return Err(SessionRestoreError::WindowIdExhausted);
        }
        if window.tabs.len() > MAX_SESSION_TABS_PER_WINDOW {
            return Err(SessionRestoreError::TabsPerWindowLimitExceeded {
                found: window.tabs.len(),
                limit: MAX_SESSION_TABS_PER_WINDOW,
            });
        }
        total_tabs = total_tabs.saturating_add(window.tabs.len());
        if total_tabs > MAX_SESSION_TABS {
            return Err(SessionRestoreError::TabLimitExceeded {
                found: total_tabs,
                limit: MAX_SESSION_TABS,
            });
        }
        for tab in &window.tabs {
            if tab.id.0 == 0 || tab.id.0 >= snapshot.next_tab_id || !tab_ids.insert(tab.id.0) {
                return Err(SessionRestoreError::TabIdExhausted);
            }
            validate_location(&tab.location)?;
        }
        let active_is_valid = match (window.tabs.is_empty(), window.active_tab) {
            (true, None) => true,
            (false, Some(active)) => window.tabs.iter().any(|tab| tab.id == active),
            _ => false,
        };
        if !active_is_valid {
            return Err(SessionRestoreError::InvalidActiveTab { window: window.id });
        }
    }
    Ok(())
}

fn unexpected_generation(current: u64, recovered: &[u64], generations: &[u64]) -> Option<u64> {
    generations
        .iter()
        .find(|&&generation| generation > current && !recovered.contains(&generation))
        .copied()
}

fn session_file_name(generation: u64) -> String {
    format!("{SESSION_FILE_PREFIX}{generation:020}{SESSION_FILE_SUFFIX}")
}

fn parse_generation_file_name(name: &std::ffi::OsStr) -> Option<u64> {
    let name = name.to_str()?;
    let digits = name
        .strip_prefix(SESSION_FILE_PREFIX)?
        .strip_suffix(SESSION_FILE_SUFFIX)?;
    if digits.len() != SESSION_GENERATION_DIGITS
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let generation = digits.parse::<u64>().ok()?;
    (generation != 0).then_some(generation)
}

fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>, SessionRestoreError> {
    let file =
        File::open(path).map_err(|error| io_error("open session generation", path, error))?;
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| io_error("read session generation", path, error))?;
    Ok(bytes)
}

fn encode_session(
    snapshot: &SessionRestoreSnapshot,
    generation: u64,
) -> Result<Vec<u8>, SessionRestoreError> {
    validate_snapshot(snapshot)?;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&SESSION_MAGIC);
    bytes.extend_from_slice(&SESSION_RESTORE_SCHEMA_VERSION.to_le_bytes());
    bytes.extend_from_slice(&generation.to_le_bytes());
    bytes.extend_from_slice(&snapshot.next_window_id.to_le_bytes());
    bytes.extend_from_slice(&snapshot.next_tab_id.to_le_bytes());
    bytes.extend_from_slice(&(snapshot.windows.len() as u32).to_le_bytes());
    for window in &snapshot.windows {
        bytes.extend_from_slice(&window.id.0.to_le_bytes());
        bytes.extend_from_slice(&window.active_tab.map_or(0, |tab| tab.0).to_le_bytes());
        bytes.extend_from_slice(&(window.tabs.len() as u32).to_le_bytes());
        for tab in &window.tabs {
            bytes.extend_from_slice(&tab.id.0.to_le_bytes());
            bytes.extend_from_slice(&(tab.location.len() as u32).to_le_bytes());
            bytes.extend_from_slice(tab.location.as_bytes());
        }
    }
    let final_len = bytes.len().saturating_add(CHECKSUM_BYTES);
    if final_len > MAX_SESSION_RESTORE_RECORD_BYTES {
        return Err(SessionRestoreError::RecordTooLarge {
            bytes: final_len,
            limit: MAX_SESSION_RESTORE_RECORD_BYTES,
        });
    }
    let checksum = checksum64(&bytes);
    bytes.extend_from_slice(&checksum.to_le_bytes());
    Ok(bytes)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DecodeError {
    UnsupportedSchema(u32),
    Corrupt,
}

fn decode_session(
    bytes: &[u8],
    expected_generation: u64,
) -> Result<SessionRestoreSnapshot, DecodeError> {
    if bytes.len() < MIN_RECORD_BYTES || bytes.len() > MAX_SESSION_RESTORE_RECORD_BYTES {
        return Err(DecodeError::Corrupt);
    }
    let body_len = bytes
        .len()
        .checked_sub(CHECKSUM_BYTES)
        .ok_or(DecodeError::Corrupt)?;
    let stored_checksum = u64::from_le_bytes(
        bytes[body_len..]
            .try_into()
            .map_err(|_| DecodeError::Corrupt)?,
    );
    if checksum64(&bytes[..body_len]) != stored_checksum {
        return Err(DecodeError::Corrupt);
    }

    let mut cursor = RecordCursor::new(&bytes[..body_len]);
    if cursor.take_array::<8>()? != SESSION_MAGIC {
        return Err(DecodeError::Corrupt);
    }
    let schema = u32::from_le_bytes(cursor.take_array::<4>()?);
    if schema != SESSION_RESTORE_SCHEMA_VERSION {
        return Err(DecodeError::UnsupportedSchema(schema));
    }
    let generation = u64::from_le_bytes(cursor.take_array::<8>()?);
    if generation == 0 || generation != expected_generation {
        return Err(DecodeError::Corrupt);
    }
    let next_window_id = u64::from_le_bytes(cursor.take_array::<8>()?);
    let next_tab_id = u64::from_le_bytes(cursor.take_array::<8>()?);
    if next_window_id == 0 || next_tab_id == 0 {
        return Err(DecodeError::Corrupt);
    }
    let window_count = u32::from_le_bytes(cursor.take_array::<4>()?) as usize;
    if window_count > MAX_SESSION_WINDOWS {
        return Err(DecodeError::Corrupt);
    }

    let mut windows = Vec::with_capacity(window_count);
    let mut window_ids = BTreeSet::new();
    let mut tab_ids = BTreeSet::new();
    let mut total_tabs = 0usize;
    for _ in 0..window_count {
        let raw_window_id = u64::from_le_bytes(cursor.take_array::<8>()?);
        let raw_active_tab = u64::from_le_bytes(cursor.take_array::<8>()?);
        let tab_count = u32::from_le_bytes(cursor.take_array::<4>()?) as usize;
        if raw_window_id == 0
            || raw_window_id >= next_window_id
            || !window_ids.insert(raw_window_id)
            || tab_count > MAX_SESSION_TABS_PER_WINDOW
        {
            return Err(DecodeError::Corrupt);
        }
        total_tabs = total_tabs
            .checked_add(tab_count)
            .ok_or(DecodeError::Corrupt)?;
        if total_tabs > MAX_SESSION_TABS {
            return Err(DecodeError::Corrupt);
        }
        let mut tabs = Vec::with_capacity(tab_count);
        for _ in 0..tab_count {
            let raw_tab_id = u64::from_le_bytes(cursor.take_array::<8>()?);
            let location_len = u32::from_le_bytes(cursor.take_array::<4>()?) as usize;
            if raw_tab_id == 0
                || raw_tab_id >= next_tab_id
                || !tab_ids.insert(raw_tab_id)
                || location_len == 0
                || location_len > MAX_SESSION_LOCATION_BYTES
            {
                return Err(DecodeError::Corrupt);
            }
            let location = std::str::from_utf8(cursor.take(location_len)?)
                .map_err(|_| DecodeError::Corrupt)?
                .to_owned();
            if validate_location(&location).is_err() {
                return Err(DecodeError::Corrupt);
            }
            tabs.push(SessionTab {
                id: SessionTabId(raw_tab_id),
                location,
            });
        }
        let active_tab = (raw_active_tab != 0).then_some(SessionTabId(raw_active_tab));
        let active_is_valid = match (tabs.is_empty(), active_tab) {
            (true, None) => true,
            (false, Some(active)) => tabs.iter().any(|tab| tab.id == active),
            _ => false,
        };
        if !active_is_valid {
            return Err(DecodeError::Corrupt);
        }
        windows.push(SessionWindow {
            id: SessionWindowId(raw_window_id),
            active_tab,
            tabs,
        });
    }
    if !cursor.is_finished() {
        return Err(DecodeError::Corrupt);
    }

    let snapshot = SessionRestoreSnapshot {
        generation,
        next_window_id,
        next_tab_id,
        windows,
    };
    validate_snapshot(&snapshot).map_err(|_| DecodeError::Corrupt)?;
    Ok(snapshot)
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

fn io_error(operation: &'static str, path: &Path, error: io::Error) -> SessionRestoreError {
    SessionRestoreError::Io {
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
                "zorya-session-restore-test-{}-{sequence}",
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

    fn snapshot_with(location: &str) -> SessionRestoreSnapshot {
        let mut snapshot = SessionRestoreSnapshot::default();
        let window = snapshot.add_window().unwrap();
        snapshot.add_tab(window, location).unwrap();
        snapshot
    }

    type RawTab<'a> = (u64, &'a str);
    type RawWindow<'a> = (u64, u64, &'a [RawTab<'a>]);

    fn raw_record(
        generation: u64,
        schema: u32,
        next_window_id: u64,
        next_tab_id: u64,
        windows: &[RawWindow<'_>],
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&SESSION_MAGIC);
        bytes.extend_from_slice(&schema.to_le_bytes());
        bytes.extend_from_slice(&generation.to_le_bytes());
        bytes.extend_from_slice(&next_window_id.to_le_bytes());
        bytes.extend_from_slice(&next_tab_id.to_le_bytes());
        bytes.extend_from_slice(&(windows.len() as u32).to_le_bytes());
        for (window, active, tabs) in windows {
            bytes.extend_from_slice(&window.to_le_bytes());
            bytes.extend_from_slice(&active.to_le_bytes());
            bytes.extend_from_slice(&(tabs.len() as u32).to_le_bytes());
            for (tab, location) in *tabs {
                bytes.extend_from_slice(&tab.to_le_bytes());
                bytes.extend_from_slice(&(location.len() as u32).to_le_bytes());
                bytes.extend_from_slice(location.as_bytes());
            }
        }
        let checksum = checksum64(&bytes);
        bytes.extend_from_slice(&checksum.to_le_bytes());
        bytes
    }

    #[test]
    fn snapshot_preserves_order_active_tab_and_monotonic_identities() {
        let mut snapshot = SessionRestoreSnapshot::default();
        let first_window = snapshot.add_window().unwrap();
        let first_tab = snapshot.add_tab(first_window, "https://one.test").unwrap();
        let second_tab = snapshot.add_tab(first_window, "https://two.test").unwrap();
        assert_eq!(
            snapshot.window(first_window).unwrap().active_tab(),
            Some(first_tab)
        );
        assert!(snapshot.set_active_tab(first_window, second_tab).unwrap());
        assert!(
            snapshot
                .update_tab_location(first_window, second_tab, "https://two.test/committed")
                .unwrap()
        );
        let second_window = snapshot.add_window().unwrap();
        let third_tab = snapshot
            .add_tab(second_window, "https://three.test")
            .unwrap();
        assert_eq!(
            snapshot
                .windows()
                .iter()
                .map(SessionWindow::id)
                .collect::<Vec<_>>(),
            vec![first_window, second_window]
        );
        assert_eq!(
            snapshot
                .window(first_window)
                .unwrap()
                .tabs()
                .iter()
                .map(SessionTab::id)
                .collect::<Vec<_>>(),
            vec![first_tab, second_tab]
        );
        assert_eq!(third_tab.get(), 3);

        let removed = snapshot
            .remove_tab(first_window, second_tab)
            .unwrap()
            .unwrap();
        assert_eq!(removed.id(), second_tab);
        assert_eq!(
            snapshot.window(first_window).unwrap().active_tab(),
            Some(first_tab)
        );
        let fourth_tab = snapshot.add_tab(first_window, "https://four.test").unwrap();
        assert!(fourth_tab > third_tab);
        snapshot.remove_window(first_window).unwrap();
        let third_window = snapshot.add_window().unwrap();
        assert!(third_window > second_window);
    }

    #[test]
    fn model_bounds_and_foreign_active_tab_are_rejected_before_mutation() {
        let mut snapshot = SessionRestoreSnapshot::default();
        let first = snapshot.add_window().unwrap();
        let second = snapshot.add_window().unwrap();
        assert_eq!(
            snapshot.add_tab(first, ""),
            Err(SessionRestoreError::EmptyLocation)
        );
        assert!(matches!(
            snapshot.add_tab(first, "x".repeat(MAX_SESSION_LOCATION_BYTES + 1)),
            Err(SessionRestoreError::LocationTooLarge { .. })
        ));
        let first_tab = snapshot.add_tab(first, "https://one.test").unwrap();
        let second_tab = snapshot.add_tab(second, "https://two.test").unwrap();
        assert!(matches!(
            snapshot.set_active_tab(first, second_tab),
            Err(SessionRestoreError::TabNotFound { .. })
        ));
        assert_eq!(
            snapshot.window(first).unwrap().active_tab(),
            Some(first_tab)
        );
    }

    #[test]
    fn codec_round_trips_order_active_state_and_next_identities() {
        let mut snapshot = SessionRestoreSnapshot::default();
        let first = snapshot.add_window().unwrap();
        let first_tab = snapshot.add_tab(first, "https://one.test").unwrap();
        let second_tab = snapshot.add_tab(first, "https://two.test").unwrap();
        snapshot.set_active_tab(first, second_tab).unwrap();
        let second = snapshot.add_window().unwrap();
        snapshot.add_tab(second, "about:blank").unwrap();

        let encoded = encode_session(&snapshot, 7).unwrap();
        let mut decoded = decode_session(&encoded, 7).unwrap();
        assert_eq!(decoded.generation(), 7);
        assert_eq!(decoded.windows(), snapshot.windows());
        assert_eq!(
            decoded.window(first).unwrap().active_tab(),
            Some(second_tab)
        );
        assert_eq!(decoded.add_window().unwrap().get(), 3);
        assert_eq!(
            decoded.add_tab(second, "https://next.test").unwrap().get(),
            4
        );
        assert_eq!(first_tab.get(), 1);
    }

    #[test]
    fn codec_rejects_checksum_duplicate_identity_bad_active_and_newer_schema() {
        let bytes = raw_record(
            1,
            SESSION_RESTORE_SCHEMA_VERSION,
            2,
            3,
            &[(1, 1, &[(1, "https://one.test"), (2, "https://two.test")])],
        );
        let mut corrupt = bytes.clone();
        let index = corrupt.len() - CHECKSUM_BYTES - 1;
        corrupt[index] ^= 1;
        assert_eq!(decode_session(&corrupt, 1), Err(DecodeError::Corrupt));

        let duplicate = raw_record(
            1,
            SESSION_RESTORE_SCHEMA_VERSION,
            3,
            2,
            &[
                (1, 1, &[(1, "https://one.test")]),
                (2, 1, &[(1, "https://duplicate.test")]),
            ],
        );
        assert_eq!(decode_session(&duplicate, 1), Err(DecodeError::Corrupt));

        let bad_active = raw_record(
            1,
            SESSION_RESTORE_SCHEMA_VERSION,
            2,
            3,
            &[(1, 2, &[(1, "https://one.test")])],
        );
        assert_eq!(decode_session(&bad_active, 1), Err(DecodeError::Corrupt));

        let newer = raw_record(4, SESSION_RESTORE_SCHEMA_VERSION + 1, 1, 1, &[]);
        assert!(matches!(
            decode_session(&newer, 4),
            Err(DecodeError::UnsupportedSchema(schema))
                if schema == SESSION_RESTORE_SCHEMA_VERSION + 1
        ));
    }

    #[test]
    fn save_load_stale_writer_and_exact_lock_are_enforced() {
        let directory = TestDirectory::new();
        let other = TestDirectory::new();
        let store = SessionRestoreStore::open(directory.path()).unwrap();
        let wrong_lock = ProfileLock::acquire(other.path()).unwrap();
        assert!(matches!(
            store.save(&wrong_lock, &snapshot_with("https://wrong.test")),
            Err(SessionRestoreError::Lock(_))
        ));
        wrong_lock.release().unwrap();

        let lock = ProfileLock::acquire(directory.path()).unwrap();
        let first = store
            .save(&lock, &snapshot_with("https://one.test"))
            .unwrap()
            .into_snapshot();
        let stale = first.clone();
        let mut second_input = first;
        let window = second_input.windows()[0].id();
        second_input.add_tab(window, "https://two.test").unwrap();
        let second = store.save(&lock, &second_input).unwrap().into_snapshot();
        assert_eq!(second.generation(), 2);
        assert_eq!(
            store.save(&lock, &stale),
            Err(SessionRestoreError::StaleGeneration {
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
        let store = SessionRestoreStore::open(directory.path()).unwrap();
        let lock = ProfileLock::acquire(directory.path()).unwrap();
        let first = store
            .save(&lock, &snapshot_with("https://one.test"))
            .unwrap()
            .into_snapshot();
        let mut second_input = first.clone();
        let window = second_input.windows()[0].id();
        second_input.add_tab(window, "https://two.test").unwrap();
        let second = store.save(&lock, &second_input).unwrap().into_snapshot();
        let second_path = store.session_path(second.generation());
        let bytes = fs::read(&second_path).unwrap();
        fs::write(&second_path, &bytes[..bytes.len() / 2]).unwrap();

        let recovered = store.load().unwrap();
        assert_eq!(recovered.snapshot(), &first);
        assert_eq!(recovered.recovery().unwrap().skipped_generations(), &[2]);
        let third = store
            .save(&lock, recovered.snapshot())
            .unwrap()
            .into_snapshot();
        assert_eq!(third.generation(), 3);
        assert_eq!(store.load().unwrap().snapshot(), &third);
        lock.release().unwrap();
    }

    #[test]
    fn unsupported_schema_and_generation_exhaustion_fail_closed() {
        let schema_directory = TestDirectory::new();
        let schema_store = SessionRestoreStore::open(schema_directory.path()).unwrap();
        fs::write(
            schema_store.session_path(1),
            raw_record(1, SESSION_RESTORE_SCHEMA_VERSION + 1, 1, 1, &[]),
        )
        .unwrap();
        assert_eq!(
            schema_store.load(),
            Err(SessionRestoreError::UnsupportedSchema {
                generation: 1,
                schema: SESSION_RESTORE_SCHEMA_VERSION + 1,
            })
        );

        let exhausted_directory = TestDirectory::new();
        let exhausted_store = SessionRestoreStore::open(exhausted_directory.path()).unwrap();
        fs::write(
            exhausted_store.session_path(u64::MAX),
            raw_record(u64::MAX, SESSION_RESTORE_SCHEMA_VERSION, 1, 1, &[]),
        )
        .unwrap();
        let loaded = exhausted_store.load().unwrap().into_snapshot();
        let lock = ProfileLock::acquire(exhausted_directory.path()).unwrap();
        assert_eq!(
            exhausted_store.save(&lock, &loaded),
            Err(SessionRestoreError::GenerationExhausted)
        );
        lock.release().unwrap();
    }

    #[test]
    fn pending_files_are_ignored_and_directory_scans_are_bounded() {
        let directory = TestDirectory::new();
        let store = SessionRestoreStore::open(directory.path()).unwrap();
        let pending = store
            .session_directory
            .join(".pending-session-00000000000000000001-0000000001-00000000000000000001.tmp");
        fs::write(pending, b"partial").unwrap();
        assert!(store.load().unwrap().snapshot().is_empty());

        for index in 1..MAX_SESSION_DIRECTORY_ENTRIES {
            fs::write(store.session_directory.join(format!("junk-{index}")), b"x").unwrap();
        }
        assert!(matches!(
            store.discover_generations_with_reserve(1),
            Err(SessionRestoreError::DirectoryEntryLimitExceeded { .. })
        ));
    }

    #[test]
    fn retained_generations_remain_bounded() {
        let directory = TestDirectory::new();
        let store = SessionRestoreStore::open(directory.path()).unwrap();
        let lock = ProfileLock::acquire(directory.path()).unwrap();
        let mut snapshot = snapshot_with("https://one.test");
        for _ in 0..6 {
            snapshot = store.save(&lock, &snapshot).unwrap().into_snapshot();
        }
        let generations = store.discover_generations().unwrap();
        assert!(generations.len() <= SESSION_RETAINED_GENERATIONS);
        assert_eq!(generations.first().copied(), Some(6));
        lock.release().unwrap();
    }

    #[test]
    fn concurrent_saves_have_one_durable_winner() {
        let directory = TestDirectory::new();
        let store = Arc::new(SessionRestoreStore::open(directory.path()).unwrap());
        let lock = Arc::new(ProfileLock::acquire(directory.path()).unwrap());
        let barrier = Arc::new(Barrier::new(3));
        let workers = ["https://one.test", "https://two.test"]
            .into_iter()
            .map(|location| {
                let store = Arc::clone(&store);
                let lock = Arc::clone(&lock);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let snapshot = snapshot_with(location);
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
        assert!(
            results
                .iter()
                .filter_map(|result| result.as_ref().err())
                .all(|error| matches!(
                    error,
                    SessionRestoreError::ConcurrentWrite { .. }
                        | SessionRestoreError::StaleGeneration { .. }
                ))
        );
        assert_eq!(store.load().unwrap().snapshot().generation(), 1);
    }
}
