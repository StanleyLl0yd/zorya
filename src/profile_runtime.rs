use crate::app::BrowserNavigationCommit;
use crate::browsing_history::{
    BrowsingHistoryError, BrowsingHistoryRecord, BrowsingHistoryRecovery, BrowsingHistorySave,
    BrowsingHistorySnapshot, BrowsingHistoryStore,
};
use crate::profile::{
    ProfileStorageError, ProfileStore, SettingsRecovery, SettingsSave, SettingsSnapshot,
};
use crate::profile_lock::{ProfileLock, ProfileLockError};
use std::fmt;
use std::path::{Path, PathBuf};

const COLOR_SCHEME_KEY: &str = "ui.color_scheme";
const CONFIRM_CLOSE_MULTIPLE_TABS_KEY: &str = "tabs.confirm_close_multiple";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileId(u64);

impl ProfileId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileSelectionId(u64);

impl ProfileSelectionId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileSelectionIntent {
    id: ProfileSelectionId,
    root: PathBuf,
}

impl ProfileSelectionIntent {
    pub const fn id(&self) -> ProfileSelectionId {
        self.id
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileSelectionStart {
    intent: ProfileSelectionIntent,
    superseded: Option<ProfileSelectionIntent>,
}

impl ProfileSelectionStart {
    pub const fn intent(&self) -> &ProfileSelectionIntent {
        &self.intent
    }

    pub fn into_intent(self) -> ProfileSelectionIntent {
        self.intent
    }

    pub const fn superseded(&self) -> Option<&ProfileSelectionIntent> {
        self.superseded.as_ref()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorSchemePreference {
    #[default]
    System,
    Light,
    Dark,
}

impl ColorSchemePreference {
    const fn encoded(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    fn decode(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductSettings {
    snapshot: SettingsSnapshot,
    color_scheme: ColorSchemePreference,
    confirm_close_multiple_tabs: bool,
}

impl ProductSettings {
    pub fn from_snapshot(snapshot: SettingsSnapshot) -> Result<Self, ProfileSettingsError> {
        let color_scheme = match snapshot.get(COLOR_SCHEME_KEY) {
            Some(value) => ColorSchemePreference::decode(value).ok_or_else(|| {
                ProfileSettingsError::InvalidValue {
                    key: COLOR_SCHEME_KEY,
                    value: value.to_owned(),
                }
            })?,
            None => ColorSchemePreference::default(),
        };
        let confirm_close_multiple_tabs = match snapshot.get(CONFIRM_CLOSE_MULTIPLE_TABS_KEY) {
            Some("true") => true,
            Some("false") => false,
            Some(value) => {
                return Err(ProfileSettingsError::InvalidValue {
                    key: CONFIRM_CLOSE_MULTIPLE_TABS_KEY,
                    value: value.to_owned(),
                });
            }
            None => true,
        };
        Ok(Self {
            snapshot,
            color_scheme,
            confirm_close_multiple_tabs,
        })
    }

    pub fn generation(&self) -> u64 {
        self.snapshot.generation()
    }

    pub const fn color_scheme(&self) -> ColorSchemePreference {
        self.color_scheme
    }

    pub const fn confirm_close_multiple_tabs(&self) -> bool {
        self.confirm_close_multiple_tabs
    }

    pub const fn snapshot(&self) -> &SettingsSnapshot {
        &self.snapshot
    }

    pub fn into_snapshot(self) -> SettingsSnapshot {
        self.snapshot
    }

    pub fn set_color_scheme(
        &mut self,
        preference: ColorSchemePreference,
    ) -> Result<(), ProfileSettingsError> {
        self.snapshot
            .set(COLOR_SCHEME_KEY, preference.encoded())
            .map_err(ProfileSettingsError::Storage)?;
        self.color_scheme = preference;
        Ok(())
    }

    pub fn reset_color_scheme(&mut self) {
        self.snapshot.remove(COLOR_SCHEME_KEY);
        self.color_scheme = ColorSchemePreference::default();
    }

    pub fn set_confirm_close_multiple_tabs(
        &mut self,
        confirm: bool,
    ) -> Result<(), ProfileSettingsError> {
        self.snapshot
            .set(
                CONFIRM_CLOSE_MULTIPLE_TABS_KEY,
                if confirm { "true" } else { "false" },
            )
            .map_err(ProfileSettingsError::Storage)?;
        self.confirm_close_multiple_tabs = confirm;
        Ok(())
    }

    pub fn reset_confirm_close_multiple_tabs(&mut self) {
        self.snapshot.remove(CONFIRM_CLOSE_MULTIPLE_TABS_KEY);
        self.confirm_close_multiple_tabs = true;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileSettingsError {
    Storage(ProfileStorageError),
    InvalidValue { key: &'static str, value: String },
}

impl fmt::Display for ProfileSettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => error.fmt(formatter),
            Self::InvalidValue { key, value } => {
                write!(
                    formatter,
                    "invalid value {value:?} for profile setting {key}"
                )
            }
        }
    }
}

impl std::error::Error for ProfileSettingsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::InvalidValue { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedProfile {
    selection: ProfileSelectionId,
    root: PathBuf,
    lock: ProfileLock,
    settings: ProductSettings,
    settings_recovery: Option<SettingsRecovery>,
    browsing_history: BrowsingHistorySnapshot,
    browsing_history_recovery: Option<BrowsingHistoryRecovery>,
}

impl PreparedProfile {
    pub fn load(intent: &ProfileSelectionIntent) -> Result<Self, ProfilePreparationError> {
        let lock =
            ProfileLock::acquire(intent.root.clone()).map_err(ProfilePreparationError::Lock)?;
        let prepared = (|| {
            let store = ProfileStore::open(intent.root.clone())
                .map_err(ProfilePreparationError::Storage)?;
            let load = store
                .load_settings()
                .map_err(ProfilePreparationError::Storage)?;
            let settings_recovery = load.recovery().cloned();
            let settings = ProductSettings::from_snapshot(load.into_snapshot())
                .map_err(ProfilePreparationError::Settings)?;

            let history_store = BrowsingHistoryStore::open(intent.root.clone())
                .map_err(ProfilePreparationError::BrowsingHistory)?;
            let history_load = history_store
                .load()
                .map_err(ProfilePreparationError::BrowsingHistory)?;
            let browsing_history_recovery = history_load.recovery().cloned();
            let browsing_history = history_load.into_snapshot();

            Ok((
                settings,
                settings_recovery,
                browsing_history,
                browsing_history_recovery,
            ))
        })();

        let (settings, settings_recovery, browsing_history, browsing_history_recovery) =
            match prepared {
                Ok(prepared) => prepared,
                Err(error) => {
                    return match lock.release() {
                        Ok(()) => Err(error),
                        Err(release) => Err(ProfilePreparationError::LockRelease {
                            preparation: Box::new(error),
                            release,
                        }),
                    };
                }
            };

        Ok(Self {
            selection: intent.id,
            root: intent.root.clone(),
            lock,
            settings,
            settings_recovery,
            browsing_history,
            browsing_history_recovery,
        })
    }

    pub const fn selection(&self) -> ProfileSelectionId {
        self.selection
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub const fn settings(&self) -> &ProductSettings {
        &self.settings
    }

    pub const fn settings_recovery(&self) -> Option<&SettingsRecovery> {
        self.settings_recovery.as_ref()
    }

    pub const fn browsing_history(&self) -> &BrowsingHistorySnapshot {
        &self.browsing_history
    }

    pub const fn browsing_history_recovery(&self) -> Option<&BrowsingHistoryRecovery> {
        self.browsing_history_recovery.as_ref()
    }

    #[cfg(any(test, target_os = "windows"))]
    pub(crate) fn into_profile_lock(self) -> ProfileLock {
        self.lock
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfilePreparationError {
    Lock(ProfileLockError),
    Storage(ProfileStorageError),
    Settings(ProfileSettingsError),
    BrowsingHistory(crate::browsing_history::BrowsingHistoryError),
    LockRelease {
        preparation: Box<ProfilePreparationError>,
        release: ProfileLockError,
    },
}

impl fmt::Display for ProfilePreparationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Lock(error) => error.fmt(formatter),
            Self::Storage(error) => error.fmt(formatter),
            Self::Settings(error) => error.fmt(formatter),
            Self::BrowsingHistory(error) => error.fmt(formatter),
            Self::LockRelease {
                preparation,
                release,
            } => write!(
                formatter,
                "profile preparation failed: {preparation}; failed to release acquired profile lock: {release}"
            ),
        }
    }
}

impl std::error::Error for ProfilePreparationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Lock(error) => Some(error),
            Self::Storage(error) => Some(error),
            Self::Settings(error) => Some(error),
            Self::BrowsingHistory(error) => Some(error),
            Self::LockRelease { preparation, .. } => Some(preparation.as_ref()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveProfile {
    id: ProfileId,
    root: PathBuf,
    lock: ProfileLock,
    settings: ProductSettings,
    settings_recovery: Option<SettingsRecovery>,
    browsing_history: BrowsingHistorySnapshot,
    browsing_history_recovery: Option<BrowsingHistoryRecovery>,
    settings_revision: u64,
    durable_settings_revision: u64,
    browsing_history_revision: u64,
    durable_browsing_history_revision: u64,
}

impl ActiveProfile {
    pub const fn id(&self) -> ProfileId {
        self.id
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    #[cfg(target_os = "windows")]
    pub(crate) const fn profile_lock(&self) -> &ProfileLock {
        &self.lock
    }

    #[cfg(any(test, target_os = "windows"))]
    pub(crate) fn into_profile_lock(self) -> ProfileLock {
        self.lock
    }

    pub const fn settings(&self) -> &ProductSettings {
        &self.settings
    }

    pub const fn settings_recovery(&self) -> Option<&SettingsRecovery> {
        self.settings_recovery.as_ref()
    }

    pub const fn browsing_history(&self) -> &BrowsingHistorySnapshot {
        &self.browsing_history
    }

    pub const fn browsing_history_recovery(&self) -> Option<&BrowsingHistoryRecovery> {
        self.browsing_history_recovery.as_ref()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileSettingsSaveId(u64);

impl ProfileSettingsSaveId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ProfileSettingsSaveIntent {
    id: ProfileSettingsSaveId,
    profile: ProfileId,
    root: PathBuf,
    lock: ProfileLock,
    snapshot: SettingsSnapshot,
    mutation_revision: u64,
}

impl ProfileSettingsSaveIntent {
    pub const fn id(&self) -> ProfileSettingsSaveId {
        self.id
    }

    pub const fn profile(&self) -> ProfileId {
        self.profile
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub const fn snapshot(&self) -> &SettingsSnapshot {
        &self.snapshot
    }

    pub const fn mutation_revision(&self) -> u64 {
        self.mutation_revision
    }

    pub fn execute(self) -> ProfileSettingsSaveCompletion {
        let base_generation = self.snapshot.generation();
        let result = ProfileStore::open(self.root.clone())
            .and_then(|store| store.save_settings(&self.lock, &self.snapshot));
        ProfileSettingsSaveCompletion {
            id: self.id,
            profile: self.profile,
            root: self.root,
            base_generation,
            mutation_revision: self.mutation_revision,
            result,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ProfileSettingsSaveCompletion {
    id: ProfileSettingsSaveId,
    profile: ProfileId,
    root: PathBuf,
    base_generation: u64,
    mutation_revision: u64,
    result: Result<SettingsSave, ProfileStorageError>,
}

impl ProfileSettingsSaveCompletion {
    pub const fn id(&self) -> ProfileSettingsSaveId {
        self.id
    }

    pub const fn profile(&self) -> ProfileId {
        self.profile
    }

    pub const fn base_generation(&self) -> u64 {
        self.base_generation
    }

    pub const fn mutation_revision(&self) -> u64 {
        self.mutation_revision
    }

    pub const fn result(&self) -> &Result<SettingsSave, ProfileStorageError> {
        &self.result
    }

    pub fn into_result(self) -> Result<SettingsSave, ProfileStorageError> {
        self.result
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PendingProfileSettingsSave {
    id: ProfileSettingsSaveId,
    profile: ProfileId,
    base_generation: u64,
    mutation_revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileHistorySaveId(u64);

impl ProfileHistorySaveId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ProfileHistorySaveIntent {
    id: ProfileHistorySaveId,
    profile: ProfileId,
    root: PathBuf,
    lock: ProfileLock,
    snapshot: BrowsingHistorySnapshot,
    mutation_revision: u64,
}

impl ProfileHistorySaveIntent {
    pub const fn id(&self) -> ProfileHistorySaveId {
        self.id
    }

    pub const fn profile(&self) -> ProfileId {
        self.profile
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub const fn snapshot(&self) -> &BrowsingHistorySnapshot {
        &self.snapshot
    }

    pub const fn mutation_revision(&self) -> u64 {
        self.mutation_revision
    }

    pub fn execute(self) -> ProfileHistorySaveCompletion {
        let base_generation = self.snapshot.generation();
        let result = BrowsingHistoryStore::open(self.root.clone())
            .and_then(|store| store.save(&self.lock, &self.snapshot));
        ProfileHistorySaveCompletion {
            id: self.id,
            profile: self.profile,
            root: self.root,
            base_generation,
            mutation_revision: self.mutation_revision,
            result,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ProfileHistorySaveCompletion {
    id: ProfileHistorySaveId,
    profile: ProfileId,
    root: PathBuf,
    base_generation: u64,
    mutation_revision: u64,
    result: Result<BrowsingHistorySave, BrowsingHistoryError>,
}

impl ProfileHistorySaveCompletion {
    pub const fn id(&self) -> ProfileHistorySaveId {
        self.id
    }

    pub const fn profile(&self) -> ProfileId {
        self.profile
    }

    pub const fn base_generation(&self) -> u64 {
        self.base_generation
    }

    pub const fn mutation_revision(&self) -> u64 {
        self.mutation_revision
    }

    pub const fn result(&self) -> &Result<BrowsingHistorySave, BrowsingHistoryError> {
        &self.result
    }

    pub fn into_result(self) -> Result<BrowsingHistorySave, BrowsingHistoryError> {
        self.result
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PendingProfileHistorySave {
    id: ProfileHistorySaveId,
    profile: ProfileId,
    base_generation: u64,
    mutation_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileSelectionCommit {
    active_profile: ProfileId,
    replaced_profile: Option<ActiveProfile>,
}

impl ProfileSelectionCommit {
    pub const fn active_profile(&self) -> ProfileId {
        self.active_profile
    }

    pub const fn replaced_profile(&self) -> Option<&ActiveProfile> {
        self.replaced_profile.as_ref()
    }

    pub fn into_replaced_profile(self) -> Option<ActiveProfile> {
        self.replaced_profile
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use = "a rejected prepared profile still owns its exact profile lock"]
pub struct ProfileSelectionCommitError {
    error: ProfileRuntimeError,
    prepared: Box<PreparedProfile>,
}

impl ProfileSelectionCommitError {
    pub const fn error(&self) -> &ProfileRuntimeError {
        &self.error
    }

    pub fn prepared(&self) -> &PreparedProfile {
        self.prepared.as_ref()
    }

    pub fn into_parts(self) -> (ProfileRuntimeError, PreparedProfile) {
        (self.error, *self.prepared)
    }
}

impl fmt::Display for ProfileSelectionCommitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for ProfileSelectionCommitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileRuntimeError {
    ProfileIdExhausted,
    ProfileSelectionIdExhausted,
    ProfileSettingsSaveIdExhausted,
    ProfileSettingsMutationRevisionExhausted,
    SettingsSaveAlreadyPending {
        pending: ProfileSettingsSaveId,
    },
    StaleSettingsSave {
        expected: Option<ProfileSettingsSaveId>,
        actual: ProfileSettingsSaveId,
    },
    SettingsSaveTargetMismatch {
        save: ProfileSettingsSaveId,
    },
    SettingsSaveGenerationMismatch {
        expected_generation: u64,
        current_generation: u64,
        saved_generation: u64,
    },
    ProfileHistorySaveIdExhausted,
    ProfileHistoryMutationRevisionExhausted,
    BrowsingHistorySaveAlreadyPending {
        pending: ProfileHistorySaveId,
    },
    StaleBrowsingHistorySave {
        expected: Option<ProfileHistorySaveId>,
        actual: ProfileHistorySaveId,
    },
    BrowsingHistorySaveTargetMismatch {
        save: ProfileHistorySaveId,
    },
    BrowsingHistorySaveGenerationMismatch {
        expected_generation: u64,
        current_generation: u64,
        saved_generation: u64,
    },
    BrowsingHistory(BrowsingHistoryError),
    StaleSelection {
        expected: Option<ProfileSelectionId>,
        actual: ProfileSelectionId,
    },
    SelectionTargetMismatch {
        selection: ProfileSelectionId,
    },
    ActiveProfileNotDurable {
        profile: ProfileId,
    },
    StaleProfile {
        expected: Option<ProfileId>,
        actual: ProfileId,
    },
}

impl fmt::Display for ProfileRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProfileIdExhausted => {
                formatter.write_str("profile identifier space is exhausted")
            }
            Self::ProfileSelectionIdExhausted => {
                formatter.write_str("profile selection identifier space is exhausted")
            }
            Self::ProfileSettingsSaveIdExhausted => {
                formatter.write_str("profile settings save identifier space is exhausted")
            }
            Self::ProfileSettingsMutationRevisionExhausted => {
                formatter.write_str("profile settings mutation revision space is exhausted")
            }
            Self::SettingsSaveAlreadyPending { pending } => write!(
                formatter,
                "profile settings save {} is already pending",
                pending.get()
            ),
            Self::StaleSettingsSave { expected, actual } => match expected {
                Some(expected) => write!(
                    formatter,
                    "profile settings save {} is stale; current save is {}",
                    actual.get(),
                    expected.get()
                ),
                None => write!(
                    formatter,
                    "profile settings save {} is stale; no save is pending",
                    actual.get()
                ),
            },
            Self::SettingsSaveTargetMismatch { save } => write!(
                formatter,
                "profile settings save {} does not match its active profile target",
                save.get()
            ),
            Self::SettingsSaveGenerationMismatch {
                expected_generation,
                current_generation,
                saved_generation,
            } => write!(
                formatter,
                "profile settings save expected in-memory generation {expected_generation}, found {current_generation}, saved {saved_generation}"
            ),
            Self::ProfileHistorySaveIdExhausted => {
                formatter.write_str("profile browsing-history save identifier space is exhausted")
            }
            Self::ProfileHistoryMutationRevisionExhausted => {
                formatter.write_str("profile browsing-history mutation revision space is exhausted")
            }
            Self::BrowsingHistorySaveAlreadyPending { pending } => write!(
                formatter,
                "profile browsing-history save {} is already pending",
                pending.get()
            ),
            Self::StaleBrowsingHistorySave { expected, actual } => match expected {
                Some(expected) => write!(
                    formatter,
                    "profile browsing-history save {} is stale; current save is {}",
                    actual.get(),
                    expected.get()
                ),
                None => write!(
                    formatter,
                    "profile browsing-history save {} is stale; no save is pending",
                    actual.get()
                ),
            },
            Self::BrowsingHistorySaveTargetMismatch { save } => write!(
                formatter,
                "profile browsing-history save {} does not match its active profile target",
                save.get()
            ),
            Self::BrowsingHistorySaveGenerationMismatch {
                expected_generation,
                current_generation,
                saved_generation,
            } => write!(
                formatter,
                "profile browsing-history save expected in-memory generation {expected_generation}, found {current_generation}, saved {saved_generation}"
            ),
            Self::BrowsingHistory(error) => {
                write!(formatter, "browsing-history update failed: {error}")
            }
            Self::StaleSelection { expected, actual } => match expected {
                Some(expected) => write!(
                    formatter,
                    "profile selection {} is stale; current selection is {}",
                    actual.get(),
                    expected.get()
                ),
                None => write!(
                    formatter,
                    "profile selection {} is stale; no profile selection is pending",
                    actual.get()
                ),
            },
            Self::SelectionTargetMismatch { selection } => write!(
                formatter,
                "prepared profile target does not match profile selection {}",
                selection.get()
            ),
            Self::ActiveProfileNotDurable { profile } => write!(
                formatter,
                "active profile {} still has pending or unsaved persistence work",
                profile.get()
            ),
            Self::StaleProfile { expected, actual } => match expected {
                Some(expected) => write!(
                    formatter,
                    "profile {} is stale; active profile is {}",
                    actual.get(),
                    expected.get()
                ),
                None => write!(
                    formatter,
                    "profile {} is stale; no profile is active",
                    actual.get()
                ),
            },
        }
    }
}

impl std::error::Error for ProfileRuntimeError {}

#[derive(Debug, PartialEq, Eq)]
pub struct ProfileRuntime {
    next_profile_id: u64,
    next_selection_id: u64,
    next_settings_save_id: u64,
    next_history_save_id: u64,
    active: Option<ActiveProfile>,
    pending: Option<ProfileSelectionIntent>,
    pending_settings_save: Option<PendingProfileSettingsSave>,
    pending_history_save: Option<PendingProfileHistorySave>,
}

impl Default for ProfileRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl ProfileRuntime {
    pub const fn new() -> Self {
        Self {
            next_profile_id: 1,
            next_selection_id: 1,
            next_settings_save_id: 1,
            next_history_save_id: 1,
            active: None,
            pending: None,
            pending_settings_save: None,
            pending_history_save: None,
        }
    }

    pub const fn active_profile(&self) -> Option<&ActiveProfile> {
        self.active.as_ref()
    }

    pub const fn pending_selection(&self) -> Option<&ProfileSelectionIntent> {
        self.pending.as_ref()
    }

    pub const fn pending_settings_save(&self) -> Option<ProfileSettingsSaveId> {
        match self.pending_settings_save {
            Some(pending) => Some(pending.id),
            None => None,
        }
    }

    pub const fn pending_browsing_history_save(&self) -> Option<ProfileHistorySaveId> {
        match self.pending_history_save {
            Some(pending) => Some(pending.id),
            None => None,
        }
    }

    pub fn begin_selection(
        &mut self,
        root: impl Into<PathBuf>,
    ) -> Result<ProfileSelectionStart, ProfileRuntimeError> {
        let id = ProfileSelectionId(self.next_selection_id);
        self.next_selection_id = self
            .next_selection_id
            .checked_add(1)
            .ok_or(ProfileRuntimeError::ProfileSelectionIdExhausted)?;
        let intent = ProfileSelectionIntent {
            id,
            root: root.into(),
        };
        let superseded = self.pending.replace(intent.clone());
        Ok(ProfileSelectionStart { intent, superseded })
    }

    pub fn cancel_selection(
        &mut self,
        selection: ProfileSelectionId,
    ) -> Result<ProfileSelectionIntent, ProfileRuntimeError> {
        let expected = self.pending.as_ref().map(ProfileSelectionIntent::id);
        if expected != Some(selection) {
            return Err(ProfileRuntimeError::StaleSelection {
                expected,
                actual: selection,
            });
        }
        Ok(self
            .pending
            .take()
            .expect("pending selection was validated"))
    }

    pub fn commit_selection(
        &mut self,
        prepared: PreparedProfile,
    ) -> Result<ProfileSelectionCommit, ProfileSelectionCommitError> {
        let expected = self.pending.as_ref().map(ProfileSelectionIntent::id);
        if expected != Some(prepared.selection) {
            return Err(ProfileSelectionCommitError {
                error: ProfileRuntimeError::StaleSelection {
                    expected,
                    actual: prepared.selection,
                },
                prepared: Box::new(prepared),
            });
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|intent| intent.root != prepared.root)
        {
            let selection = prepared.selection;
            return Err(ProfileSelectionCommitError {
                error: ProfileRuntimeError::SelectionTargetMismatch { selection },
                prepared: Box::new(prepared),
            });
        }

        if let Some(active) = self.active.as_ref() {
            let persistence_pending =
                self.pending_settings_save.is_some() || self.pending_history_save.is_some();
            let persistence_dirty = active.settings_revision != active.durable_settings_revision
                || active.browsing_history_revision != active.durable_browsing_history_revision;
            if persistence_pending || persistence_dirty {
                return Err(ProfileSelectionCommitError {
                    error: ProfileRuntimeError::ActiveProfileNotDurable { profile: active.id },
                    prepared: Box::new(prepared),
                });
            }
        }

        let id = ProfileId(self.next_profile_id);
        let Some(next_profile_id) = self.next_profile_id.checked_add(1) else {
            return Err(ProfileSelectionCommitError {
                error: ProfileRuntimeError::ProfileIdExhausted,
                prepared: Box::new(prepared),
            });
        };
        self.next_profile_id = next_profile_id;
        self.pending = None;
        let active = ActiveProfile {
            id,
            root: prepared.root,
            lock: prepared.lock,
            settings: prepared.settings,
            settings_recovery: prepared.settings_recovery,
            browsing_history: prepared.browsing_history,
            browsing_history_recovery: prepared.browsing_history_recovery,
            settings_revision: 0,
            durable_settings_revision: 0,
            browsing_history_revision: 0,
            durable_browsing_history_revision: 0,
        };
        let replaced_profile = self.active.replace(active);
        Ok(ProfileSelectionCommit {
            active_profile: id,
            replaced_profile,
        })
    }

    pub fn active_settings_mut(
        &mut self,
        profile: ProfileId,
    ) -> Result<&mut ProductSettings, ProfileRuntimeError> {
        let expected = self.active.as_ref().map(ActiveProfile::id);
        if expected != Some(profile) {
            return Err(ProfileRuntimeError::StaleProfile {
                expected,
                actual: profile,
            });
        }
        let active = self.active.as_mut().expect("active profile was validated");
        active.settings_revision = active
            .settings_revision
            .checked_add(1)
            .ok_or(ProfileRuntimeError::ProfileSettingsMutationRevisionExhausted)?;
        Ok(&mut active.settings)
    }

    pub fn settings_unsaved_mutations(
        &self,
        profile: ProfileId,
    ) -> Result<u64, ProfileRuntimeError> {
        let expected = self.active.as_ref().map(ActiveProfile::id);
        if expected != Some(profile) {
            return Err(ProfileRuntimeError::StaleProfile {
                expected,
                actual: profile,
            });
        }
        let active = self.active.as_ref().expect("active profile was validated");
        Ok(active
            .settings_revision
            .checked_sub(active.durable_settings_revision)
            .expect("durable settings revision cannot exceed current revision"))
    }

    pub fn settings_is_dirty(&self, profile: ProfileId) -> Result<bool, ProfileRuntimeError> {
        Ok(self.settings_unsaved_mutations(profile)? != 0)
    }

    pub fn settings_mutation_revision(
        &self,
        profile: ProfileId,
    ) -> Result<u64, ProfileRuntimeError> {
        let expected = self.active.as_ref().map(ActiveProfile::id);
        if expected != Some(profile) {
            return Err(ProfileRuntimeError::StaleProfile {
                expected,
                actual: profile,
            });
        }
        Ok(self
            .active
            .as_ref()
            .expect("active profile was validated")
            .settings_revision)
    }

    pub fn begin_settings_save_if_dirty(
        &mut self,
        profile: ProfileId,
    ) -> Result<Option<ProfileSettingsSaveIntent>, ProfileRuntimeError> {
        if !self.settings_is_dirty(profile)? {
            return Ok(None);
        }
        self.begin_settings_save(profile).map(Some)
    }

    pub fn begin_settings_save(
        &mut self,
        profile: ProfileId,
    ) -> Result<ProfileSettingsSaveIntent, ProfileRuntimeError> {
        let expected = self.active.as_ref().map(ActiveProfile::id);
        if expected != Some(profile) {
            return Err(ProfileRuntimeError::StaleProfile {
                expected,
                actual: profile,
            });
        }
        if let Some(pending) = self.pending_settings_save {
            return Err(ProfileRuntimeError::SettingsSaveAlreadyPending {
                pending: pending.id,
            });
        }

        let active = self.active.as_ref().expect("active profile was validated");
        let root = active.root.clone();
        let lock = active.lock.clone();
        let snapshot = active.settings.snapshot().clone();
        let base_generation = snapshot.generation();
        let mutation_revision = active.settings_revision;
        let id = ProfileSettingsSaveId(self.next_settings_save_id);
        self.next_settings_save_id = self
            .next_settings_save_id
            .checked_add(1)
            .ok_or(ProfileRuntimeError::ProfileSettingsSaveIdExhausted)?;
        self.pending_settings_save = Some(PendingProfileSettingsSave {
            id,
            profile,
            base_generation,
            mutation_revision,
        });
        Ok(ProfileSettingsSaveIntent {
            id,
            profile,
            root,
            lock,
            snapshot,
            mutation_revision,
        })
    }

    pub fn cancel_settings_save(
        &mut self,
        save: ProfileSettingsSaveId,
    ) -> Result<(), ProfileRuntimeError> {
        let expected = self.pending_settings_save.map(|pending| pending.id);
        if expected != Some(save) {
            return Err(ProfileRuntimeError::StaleSettingsSave {
                expected,
                actual: save,
            });
        }
        self.pending_settings_save = None;
        Ok(())
    }

    pub fn complete_settings_save(
        &mut self,
        completion: ProfileSettingsSaveCompletion,
    ) -> Result<ProfileSettingsSaveCompletion, ProfileRuntimeError> {
        let expected = self.pending_settings_save.map(|pending| pending.id);
        if expected != Some(completion.id) {
            return Err(ProfileRuntimeError::StaleSettingsSave {
                expected,
                actual: completion.id,
            });
        }

        let pending = self
            .pending_settings_save
            .expect("pending settings save was validated");
        let active =
            self.active
                .as_ref()
                .ok_or(ProfileRuntimeError::SettingsSaveTargetMismatch {
                    save: completion.id,
                })?;
        if pending.profile != completion.profile
            || pending.base_generation != completion.base_generation
            || pending.mutation_revision != completion.mutation_revision
            || active.id != completion.profile
            || active.root != completion.root
        {
            return Err(ProfileRuntimeError::SettingsSaveTargetMismatch {
                save: completion.id,
            });
        }

        self.pending_settings_save = None;
        if let Ok(saved) = &completion.result {
            let active = self
                .active
                .as_mut()
                .expect("settings save target was validated");
            let current_generation = active.settings.generation();
            let saved_generation = saved.snapshot().generation();
            if !active
                .settings
                .snapshot
                .advance_generation_after_save(pending.base_generation, saved_generation)
            {
                return Err(ProfileRuntimeError::SettingsSaveGenerationMismatch {
                    expected_generation: pending.base_generation,
                    current_generation,
                    saved_generation,
                });
            }
            active.durable_settings_revision = pending.mutation_revision;
        }
        Ok(completion)
    }

    pub fn record_committed_navigation(
        &mut self,
        profile: ProfileId,
        visited_unix_millis: u64,
        commit: BrowserNavigationCommit,
    ) -> Result<BrowsingHistoryRecord, ProfileRuntimeError> {
        let expected = self.active.as_ref().map(ActiveProfile::id);
        if expected != Some(profile) {
            return Err(ProfileRuntimeError::StaleProfile {
                expected,
                actual: profile,
            });
        }

        let active = self.active.as_mut().expect("active profile was validated");
        let next_revision = active
            .browsing_history_revision
            .checked_add(1)
            .ok_or(ProfileRuntimeError::ProfileHistoryMutationRevisionExhausted)?;
        let record = active
            .browsing_history
            .record_visit(visited_unix_millis, commit.location())
            .map_err(ProfileRuntimeError::BrowsingHistory)?;
        active.browsing_history_revision = next_revision;
        Ok(record)
    }

    pub fn browsing_history_unsaved_mutations(
        &self,
        profile: ProfileId,
    ) -> Result<u64, ProfileRuntimeError> {
        let expected = self.active.as_ref().map(ActiveProfile::id);
        if expected != Some(profile) {
            return Err(ProfileRuntimeError::StaleProfile {
                expected,
                actual: profile,
            });
        }
        let active = self.active.as_ref().expect("active profile was validated");
        Ok(active
            .browsing_history_revision
            .checked_sub(active.durable_browsing_history_revision)
            .expect("durable history revision cannot exceed current revision"))
    }

    pub fn browsing_history_is_dirty(
        &self,
        profile: ProfileId,
    ) -> Result<bool, ProfileRuntimeError> {
        Ok(self.browsing_history_unsaved_mutations(profile)? != 0)
    }

    pub fn browsing_history_mutation_revision(
        &self,
        profile: ProfileId,
    ) -> Result<u64, ProfileRuntimeError> {
        let expected = self.active.as_ref().map(ActiveProfile::id);
        if expected != Some(profile) {
            return Err(ProfileRuntimeError::StaleProfile {
                expected,
                actual: profile,
            });
        }
        Ok(self
            .active
            .as_ref()
            .expect("active profile was validated")
            .browsing_history_revision)
    }

    pub fn active_browsing_history(
        &self,
        profile: ProfileId,
    ) -> Result<&BrowsingHistorySnapshot, ProfileRuntimeError> {
        let expected = self.active.as_ref().map(ActiveProfile::id);
        if expected != Some(profile) {
            return Err(ProfileRuntimeError::StaleProfile {
                expected,
                actual: profile,
            });
        }
        Ok(&self
            .active
            .as_ref()
            .expect("active profile was validated")
            .browsing_history)
    }

    pub fn active_browsing_history_mut(
        &mut self,
        profile: ProfileId,
    ) -> Result<&mut BrowsingHistorySnapshot, ProfileRuntimeError> {
        let expected = self.active.as_ref().map(ActiveProfile::id);
        if expected != Some(profile) {
            return Err(ProfileRuntimeError::StaleProfile {
                expected,
                actual: profile,
            });
        }
        let active = self.active.as_mut().expect("active profile was validated");
        active.browsing_history_revision = active
            .browsing_history_revision
            .checked_add(1)
            .ok_or(ProfileRuntimeError::ProfileHistoryMutationRevisionExhausted)?;
        Ok(&mut active.browsing_history)
    }

    pub fn begin_browsing_history_save_if_dirty(
        &mut self,
        profile: ProfileId,
    ) -> Result<Option<ProfileHistorySaveIntent>, ProfileRuntimeError> {
        if !self.browsing_history_is_dirty(profile)? {
            return Ok(None);
        }
        self.begin_browsing_history_save(profile).map(Some)
    }

    pub fn begin_browsing_history_save(
        &mut self,
        profile: ProfileId,
    ) -> Result<ProfileHistorySaveIntent, ProfileRuntimeError> {
        let expected = self.active.as_ref().map(ActiveProfile::id);
        if expected != Some(profile) {
            return Err(ProfileRuntimeError::StaleProfile {
                expected,
                actual: profile,
            });
        }
        if let Some(pending) = self.pending_history_save {
            return Err(ProfileRuntimeError::BrowsingHistorySaveAlreadyPending {
                pending: pending.id,
            });
        }

        let active = self.active.as_ref().expect("active profile was validated");
        let root = active.root.clone();
        let lock = active.lock.clone();
        let snapshot = active.browsing_history.clone();
        let base_generation = snapshot.generation();
        let mutation_revision = active.browsing_history_revision;
        let id = ProfileHistorySaveId(self.next_history_save_id);
        self.next_history_save_id = self
            .next_history_save_id
            .checked_add(1)
            .ok_or(ProfileRuntimeError::ProfileHistorySaveIdExhausted)?;
        self.pending_history_save = Some(PendingProfileHistorySave {
            id,
            profile,
            base_generation,
            mutation_revision,
        });
        Ok(ProfileHistorySaveIntent {
            id,
            profile,
            root,
            lock,
            snapshot,
            mutation_revision,
        })
    }

    pub fn cancel_browsing_history_save(
        &mut self,
        save: ProfileHistorySaveId,
    ) -> Result<(), ProfileRuntimeError> {
        let expected = self.pending_history_save.map(|pending| pending.id);
        if expected != Some(save) {
            return Err(ProfileRuntimeError::StaleBrowsingHistorySave {
                expected,
                actual: save,
            });
        }
        self.pending_history_save = None;
        Ok(())
    }

    pub fn complete_browsing_history_save(
        &mut self,
        completion: ProfileHistorySaveCompletion,
    ) -> Result<ProfileHistorySaveCompletion, ProfileRuntimeError> {
        let expected = self.pending_history_save.map(|pending| pending.id);
        if expected != Some(completion.id) {
            return Err(ProfileRuntimeError::StaleBrowsingHistorySave {
                expected,
                actual: completion.id,
            });
        }

        let pending = self
            .pending_history_save
            .expect("pending history save was validated");
        let active =
            self.active
                .as_ref()
                .ok_or(ProfileRuntimeError::BrowsingHistorySaveTargetMismatch {
                    save: completion.id,
                })?;
        if pending.profile != completion.profile
            || pending.base_generation != completion.base_generation
            || pending.mutation_revision != completion.mutation_revision
            || active.id != completion.profile
            || active.root != completion.root
        {
            return Err(ProfileRuntimeError::BrowsingHistorySaveTargetMismatch {
                save: completion.id,
            });
        }

        self.pending_history_save = None;
        if let Ok(saved) = &completion.result {
            let active = self
                .active
                .as_mut()
                .expect("history save target was validated");
            let current_generation = active.browsing_history.generation();
            let saved_generation = saved.snapshot().generation();
            if !active
                .browsing_history
                .advance_generation_after_save(pending.base_generation, saved_generation)
            {
                return Err(ProfileRuntimeError::BrowsingHistorySaveGenerationMismatch {
                    expected_generation: pending.base_generation,
                    current_generation,
                    saved_generation,
                });
            }
            active.durable_browsing_history_revision = pending.mutation_revision;
        }
        Ok(completion)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BrowserApp, BrowserWindow, MAX_HISTORY_LOCATION_BYTES};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(1);

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let id = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir()
                .join(format!("zorya-profile-runtime-{}-{id}", std::process::id()));
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

    fn prepared(intent: &ProfileSelectionIntent) -> PreparedProfile {
        PreparedProfile::load(intent).unwrap()
    }

    fn load_profile(runtime: &mut ProfileRuntime, root: &Path) -> ProfileId {
        let intent = runtime.begin_selection(root).unwrap().into_intent();
        let prepared = PreparedProfile::load(&intent).unwrap();
        runtime.commit_selection(prepared).unwrap().active_profile()
    }

    fn committed_navigation(location: &str) -> BrowserNavigationCommit {
        let mut app = BrowserApp::bootstrap().unwrap();
        let window = app.windows().next().unwrap().id();
        let tab = app
            .window(window)
            .and_then(BrowserWindow::active_tab_id)
            .unwrap();
        let navigation = app
            .begin_navigation(window, tab, location)
            .unwrap()
            .intent()
            .id();
        app.commit_navigation(window, tab, navigation, location)
            .unwrap()
    }

    #[test]
    fn selection_is_two_phase_and_supersession_is_stale_safe() {
        let first_root = TempRoot::new();
        let second_root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let first = runtime
            .begin_selection(first_root.path())
            .unwrap()
            .into_intent();
        let first_prepared = prepared(&first);
        let second_start = runtime.begin_selection(second_root.path()).unwrap();
        assert_eq!(second_start.superseded(), Some(&first));
        let second = second_start.into_intent();

        let rejection = runtime.commit_selection(first_prepared).unwrap_err();
        assert_eq!(
            rejection.error(),
            &ProfileRuntimeError::StaleSelection {
                expected: Some(second.id()),
                actual: first.id(),
            }
        );
        rejection
            .into_parts()
            .1
            .into_profile_lock()
            .release()
            .unwrap();
        assert!(runtime.active_profile().is_none());
        let reacquired = ProfileLock::acquire(first_root.path()).unwrap();
        reacquired.release().unwrap();

        let commit = runtime.commit_selection(prepared(&second)).unwrap();
        assert_eq!(commit.active_profile().get(), 1);
        assert!(commit.replaced_profile().is_none());
        assert_eq!(runtime.active_profile().unwrap().root(), second_root.path());
    }

    #[test]
    fn committed_profile_survives_pending_selection_and_exact_cancel() {
        let first_root = TempRoot::new();
        let second_root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let first = runtime
            .begin_selection(first_root.path())
            .unwrap()
            .into_intent();
        let first_id = runtime
            .commit_selection(prepared(&first))
            .unwrap()
            .active_profile();
        let second = runtime
            .begin_selection(second_root.path())
            .unwrap()
            .into_intent();

        assert_eq!(runtime.active_profile().unwrap().id(), first_id);
        assert_eq!(
            runtime.cancel_selection(first.id()),
            Err(ProfileRuntimeError::StaleSelection {
                expected: Some(second.id()),
                actual: first.id(),
            })
        );
        assert_eq!(runtime.cancel_selection(second.id()).unwrap(), second);
        assert_eq!(runtime.active_profile().unwrap().id(), first_id);
    }

    #[test]
    fn replacing_profile_returns_old_identity_and_rejects_old_mutation() {
        let first_root = TempRoot::new();
        let second_root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let first = runtime
            .begin_selection(first_root.path())
            .unwrap()
            .into_intent();
        let first_id = runtime
            .commit_selection(prepared(&first))
            .unwrap()
            .active_profile();
        let second = runtime
            .begin_selection(second_root.path())
            .unwrap()
            .into_intent();
        let commit = runtime.commit_selection(prepared(&second)).unwrap();
        let second_id = commit.active_profile();

        assert_ne!(first_id, second_id);
        assert_eq!(commit.replaced_profile().unwrap().id(), first_id);
        assert_eq!(
            runtime.active_settings_mut(first_id),
            Err(ProfileRuntimeError::StaleProfile {
                expected: Some(second_id),
                actual: first_id,
            })
        );

        let replaced = commit.into_replaced_profile().unwrap();
        let lock = replaced.into_profile_lock();
        lock.release().unwrap();
        let reacquired = ProfileLock::acquire(first_root.path()).unwrap();
        reacquired.release().unwrap();
    }

    #[test]
    fn identifier_exhaustion_does_not_replace_pending_or_active_state() {
        let mut runtime = ProfileRuntime::new();
        runtime.next_selection_id = u64::MAX;
        assert_eq!(
            runtime.begin_selection("never"),
            Err(ProfileRuntimeError::ProfileSelectionIdExhausted)
        );
        assert!(runtime.pending_selection().is_none());

        runtime.next_selection_id = 1;
        let root = TempRoot::new();
        let intent = runtime.begin_selection(root.path()).unwrap().into_intent();
        runtime.next_profile_id = u64::MAX;
        let rejection = runtime.commit_selection(prepared(&intent)).unwrap_err();
        assert_eq!(rejection.error(), &ProfileRuntimeError::ProfileIdExhausted);
        rejection
            .into_parts()
            .1
            .into_profile_lock()
            .release()
            .unwrap();
        assert_eq!(runtime.pending_selection(), Some(&intent));
        assert!(runtime.active_profile().is_none());
        let reacquired = ProfileLock::acquire(root.path()).unwrap();
        reacquired.release().unwrap();
    }

    #[test]
    fn typed_settings_preserve_unknown_entries_and_generation() {
        let mut raw = SettingsSnapshot::default();
        raw.set("future.same_schema.key", "keep-me").unwrap();
        let mut settings = ProductSettings::from_snapshot(raw).unwrap();
        let generation = settings.generation();

        settings
            .set_color_scheme(ColorSchemePreference::Dark)
            .unwrap();
        settings.set_confirm_close_multiple_tabs(false).unwrap();

        assert_eq!(settings.generation(), generation);
        assert_eq!(
            settings.snapshot().get("future.same_schema.key"),
            Some("keep-me")
        );
        assert_eq!(settings.snapshot().get(COLOR_SCHEME_KEY), Some("dark"));
        assert_eq!(
            settings.snapshot().get(CONFIRM_CLOSE_MULTIPLE_TABS_KEY),
            Some("false")
        );
    }

    #[test]
    fn typed_settings_defaults_do_not_materialize_raw_keys() {
        let settings = ProductSettings::from_snapshot(SettingsSnapshot::default()).unwrap();
        assert_eq!(settings.color_scheme(), ColorSchemePreference::System);
        assert!(settings.confirm_close_multiple_tabs());
        assert!(settings.snapshot().is_empty());
    }

    #[test]
    fn malformed_known_typed_value_fails_visibly() {
        let mut raw = SettingsSnapshot::default();
        raw.set(COLOR_SCHEME_KEY, "sepia").unwrap();
        assert_eq!(
            ProductSettings::from_snapshot(raw),
            Err(ProfileSettingsError::InvalidValue {
                key: COLOR_SCHEME_KEY,
                value: "sepia".to_owned(),
            })
        );
    }

    #[test]
    fn preparation_failure_after_acquire_releases_exact_lock() {
        let root = TempRoot::new();
        let store = ProfileStore::open(root.path()).unwrap();
        let mut raw = store.load_settings().unwrap().into_snapshot();
        raw.set(COLOR_SCHEME_KEY, "sepia").unwrap();
        let lock = ProfileLock::acquire(root.path()).unwrap();
        store.save_settings(&lock, &raw).unwrap();
        lock.release().unwrap();

        let mut runtime = ProfileRuntime::new();
        let intent = runtime.begin_selection(root.path()).unwrap().into_intent();
        assert_eq!(
            PreparedProfile::load(&intent).unwrap_err(),
            ProfilePreparationError::Settings(ProfileSettingsError::InvalidValue {
                key: COLOR_SCHEME_KEY,
                value: "sepia".to_owned(),
            })
        );

        let reacquired = ProfileLock::acquire(root.path()).unwrap();
        reacquired.release().unwrap();
    }

    #[test]
    fn prepared_profile_loads_recoverable_store_into_typed_settings() {
        let root = TempRoot::new();
        let store = ProfileStore::open(root.path()).unwrap();
        let mut raw = store.load_settings().unwrap().into_snapshot();
        raw.set(COLOR_SCHEME_KEY, "light").unwrap();
        raw.set("future.same_schema.key", "preserved").unwrap();
        let lock = ProfileLock::acquire(root.path()).unwrap();
        let saved = store.save_settings(&lock, &raw).unwrap().into_snapshot();
        assert_eq!(saved.generation(), 1);
        lock.release().unwrap();

        let mut runtime = ProfileRuntime::new();
        let intent = runtime.begin_selection(root.path()).unwrap().into_intent();
        let prepared = PreparedProfile::load(&intent).unwrap();
        assert_eq!(
            prepared.settings().color_scheme(),
            ColorSchemePreference::Light
        );
        assert_eq!(prepared.settings().generation(), 1);
        assert_eq!(
            prepared.settings().snapshot().get("future.same_schema.key"),
            Some("preserved")
        );
        runtime.commit_selection(prepared).unwrap();
        assert_eq!(runtime.active_profile().unwrap().id().get(), 1);
    }

    #[test]
    fn prepared_profile_loads_browsing_history_and_runtime_gates_access() {
        let root = TempRoot::new();
        let store = BrowsingHistoryStore::open(root.path()).unwrap();
        let mut snapshot = store.load().unwrap().into_snapshot();
        snapshot
            .record_visit(123, "https://example.test/first")
            .unwrap();
        let lock = ProfileLock::acquire(root.path()).unwrap();
        let saved = store.save(&lock, &snapshot).unwrap().into_snapshot();
        assert_eq!(saved.generation(), 1);
        lock.release().unwrap();

        let mut runtime = ProfileRuntime::new();
        let intent = runtime.begin_selection(root.path()).unwrap().into_intent();
        let loaded = PreparedProfile::load(&intent).unwrap();
        assert_eq!(loaded.browsing_history().generation(), 1);
        assert_eq!(loaded.browsing_history().len(), 1);
        assert_eq!(
            loaded.browsing_history().visits()[0].location(),
            "https://example.test/first"
        );
        assert!(loaded.browsing_history_recovery().is_none());

        let first = runtime.commit_selection(loaded).unwrap().active_profile();
        assert!(!runtime.browsing_history_is_dirty(first).unwrap());
        assert_eq!(
            runtime.browsing_history_unsaved_mutations(first).unwrap(),
            0
        );
        runtime
            .active_browsing_history_mut(first)
            .unwrap()
            .record_visit(456, "https://example.test/second")
            .unwrap();
        assert_eq!(runtime.active_browsing_history(first).unwrap().len(), 2);
        assert!(runtime.browsing_history_is_dirty(first).unwrap());
        assert_eq!(
            runtime.browsing_history_unsaved_mutations(first).unwrap(),
            1
        );

        let save = runtime.begin_browsing_history_save(first).unwrap();
        runtime
            .complete_browsing_history_save(save.execute())
            .unwrap();
        assert!(!runtime.browsing_history_is_dirty(first).unwrap());

        let replacement_root = TempRoot::new();
        let replacement = runtime
            .begin_selection(replacement_root.path())
            .unwrap()
            .into_intent();
        let second = runtime
            .commit_selection(prepared(&replacement))
            .unwrap()
            .active_profile();
        assert_ne!(first, second);
        assert!(matches!(
            runtime.active_browsing_history(first),
            Err(ProfileRuntimeError::StaleProfile {
                expected: Some(expected),
                actual,
            }) if expected == second && actual == first
        ));
    }

    #[test]
    fn committed_navigation_records_one_visit_for_exact_active_profile() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let commit = committed_navigation("https://example.test/committed");

        let record = runtime
            .record_committed_navigation(profile, 1_234, commit)
            .unwrap();

        assert_eq!(record.id().get(), 1);
        let history = runtime.active_browsing_history(profile).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history.visits()[0].id(), record.id());
        assert_eq!(history.visits()[0].visited_unix_millis(), 1_234);
        assert_eq!(
            history.visits()[0].location(),
            "https://example.test/committed"
        );
        assert_eq!(history.generation(), 0);
    }

    #[test]
    fn stale_profile_cannot_record_committed_navigation() {
        let first_root = TempRoot::new();
        let second_root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let first = load_profile(&mut runtime, first_root.path());
        let second = load_profile(&mut runtime, second_root.path());
        let commit = committed_navigation("https://example.test/stale");

        assert_eq!(
            runtime.record_committed_navigation(first, 1, commit),
            Err(ProfileRuntimeError::StaleProfile {
                expected: Some(second),
                actual: first,
            })
        );
        assert!(runtime.active_browsing_history(second).unwrap().is_empty());
    }

    #[test]
    fn rejected_committed_location_does_not_partially_mutate_history() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let oversized = "x".repeat(MAX_HISTORY_LOCATION_BYTES + 1);
        let invalid = committed_navigation(&oversized);

        assert!(matches!(
            runtime.record_committed_navigation(profile, 1, invalid),
            Err(ProfileRuntimeError::BrowsingHistory(
                BrowsingHistoryError::LocationTooLarge { .. }
            ))
        ));
        assert!(runtime.active_browsing_history(profile).unwrap().is_empty());
        assert!(!runtime.browsing_history_is_dirty(profile).unwrap());
        assert_eq!(
            runtime.browsing_history_unsaved_mutations(profile).unwrap(),
            0
        );

        let valid = committed_navigation("https://example.test/valid");
        let record = runtime
            .record_committed_navigation(profile, 2, valid)
            .unwrap();
        assert_eq!(record.id().get(), 1);
    }

    #[test]
    fn loaded_settings_start_clean_and_mutable_access_marks_dirty() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());

        assert!(!runtime.settings_is_dirty(profile).unwrap());
        assert_eq!(runtime.settings_unsaved_mutations(profile).unwrap(), 0);
        assert!(
            runtime
                .begin_settings_save_if_dirty(profile)
                .unwrap()
                .is_none()
        );

        runtime
            .active_settings_mut(profile)
            .unwrap()
            .set_color_scheme(ColorSchemePreference::Dark)
            .unwrap();

        assert!(runtime.settings_is_dirty(profile).unwrap());
        assert_eq!(runtime.settings_unsaved_mutations(profile).unwrap(), 1);
        let intent = runtime
            .begin_settings_save_if_dirty(profile)
            .unwrap()
            .expect("dirty settings should schedule a save");
        assert_eq!(intent.mutation_revision(), 1);
    }

    #[test]
    fn successful_settings_save_cleans_only_captured_mutations() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());

        runtime
            .active_settings_mut(profile)
            .unwrap()
            .set_color_scheme(ColorSchemePreference::Dark)
            .unwrap();
        let first = runtime
            .begin_settings_save_if_dirty(profile)
            .unwrap()
            .unwrap();
        assert_eq!(first.mutation_revision(), 1);

        runtime
            .active_settings_mut(profile)
            .unwrap()
            .set_confirm_close_multiple_tabs(false)
            .unwrap();
        assert_eq!(runtime.settings_unsaved_mutations(profile).unwrap(), 2);

        let first_completion = first.execute();
        assert!(first_completion.result().is_ok());
        runtime.complete_settings_save(first_completion).unwrap();

        let active = runtime.active_profile().unwrap();
        assert_eq!(active.settings().generation(), 1);
        assert_eq!(
            active.settings().color_scheme(),
            ColorSchemePreference::Dark
        );
        assert!(!active.settings().confirm_close_multiple_tabs());
        assert!(runtime.settings_is_dirty(profile).unwrap());
        assert_eq!(runtime.settings_unsaved_mutations(profile).unwrap(), 1);

        let persisted = ProfileStore::open(root.path())
            .unwrap()
            .load_settings()
            .unwrap()
            .into_snapshot();
        assert_eq!(persisted.generation(), 1);
        assert_eq!(persisted.get(COLOR_SCHEME_KEY), Some("dark"));
        assert_eq!(persisted.get(CONFIRM_CLOSE_MULTIPLE_TABS_KEY), None);

        let second = runtime
            .begin_settings_save_if_dirty(profile)
            .unwrap()
            .unwrap()
            .execute();
        assert!(second.result().is_ok());
        runtime.complete_settings_save(second).unwrap();

        assert!(!runtime.settings_is_dirty(profile).unwrap());
        assert_eq!(runtime.settings_unsaved_mutations(profile).unwrap(), 0);
        assert_eq!(runtime.active_profile().unwrap().settings().generation(), 2);
        let persisted = ProfileStore::open(root.path())
            .unwrap()
            .load_settings()
            .unwrap()
            .into_snapshot();
        assert_eq!(persisted.generation(), 2);
        assert_eq!(
            persisted.get(CONFIRM_CLOSE_MULTIPLE_TABS_KEY),
            Some("false")
        );
    }

    #[test]
    fn failed_settings_save_keeps_runtime_generation_and_dirty_state() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        runtime
            .active_settings_mut(profile)
            .unwrap()
            .set_color_scheme(ColorSchemePreference::Dark)
            .unwrap();
        let intent = runtime.begin_settings_save(profile).unwrap();

        let store = ProfileStore::open(root.path()).unwrap();
        let external = store.load_settings().unwrap().into_snapshot();
        let lock = runtime.active.as_ref().unwrap().lock.clone();
        store.save_settings(&lock, &external).unwrap();

        let completion = intent.execute();
        assert!(matches!(
            completion.result(),
            Err(ProfileStorageError::StaleSettingsGeneration {
                current: 1,
                provided: 0,
            })
        ));
        runtime.complete_settings_save(completion).unwrap();

        assert_eq!(runtime.active_profile().unwrap().settings().generation(), 0);
        assert!(runtime.pending_settings_save().is_none());
        assert!(runtime.settings_is_dirty(profile).unwrap());
        assert_eq!(runtime.settings_unsaved_mutations(profile).unwrap(), 1);
    }

    #[test]
    fn overlapping_settings_save_is_rejected_until_exact_completion() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        runtime
            .active_settings_mut(profile)
            .unwrap()
            .set_color_scheme(ColorSchemePreference::Dark)
            .unwrap();
        let first = runtime.begin_settings_save(profile).unwrap();
        let save = first.id();

        assert_eq!(
            runtime.begin_settings_save(profile),
            Err(ProfileRuntimeError::SettingsSaveAlreadyPending { pending: save })
        );

        runtime.complete_settings_save(first.execute()).unwrap();
        assert!(runtime.begin_settings_save(profile).is_ok());
    }

    #[test]
    fn cancelling_exact_settings_save_preserves_dirty_state_for_retry() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        runtime
            .active_settings_mut(profile)
            .unwrap()
            .set_color_scheme(ColorSchemePreference::Dark)
            .unwrap();

        let first = runtime
            .begin_settings_save_if_dirty(profile)
            .unwrap()
            .unwrap();
        let first_id = first.id();
        assert_eq!(runtime.pending_settings_save(), Some(first_id));

        runtime.cancel_settings_save(first_id).unwrap();

        assert!(runtime.pending_settings_save().is_none());
        assert!(runtime.settings_is_dirty(profile).unwrap());
        assert_eq!(runtime.settings_unsaved_mutations(profile).unwrap(), 1);
        let retry = runtime
            .begin_settings_save_if_dirty(profile)
            .unwrap()
            .expect("cancelled dirty settings save should be retryable");
        assert_ne!(retry.id(), first_id);
    }

    #[test]
    fn stale_settings_save_cancellation_cannot_clear_current_pending_save() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        runtime
            .active_settings_mut(profile)
            .unwrap()
            .set_color_scheme(ColorSchemePreference::Dark)
            .unwrap();

        let first = runtime.begin_settings_save(profile).unwrap().id();
        runtime.cancel_settings_save(first).unwrap();
        let current = runtime.begin_settings_save(profile).unwrap().id();

        assert_eq!(
            runtime.cancel_settings_save(first),
            Err(ProfileRuntimeError::StaleSettingsSave {
                expected: Some(current),
                actual: first,
            })
        );
        assert_eq!(runtime.pending_settings_save(), Some(current));
    }

    #[test]
    fn profile_replacement_waits_for_pending_settings_save() {
        let first_root = TempRoot::new();
        let second_root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let first = load_profile(&mut runtime, first_root.path());
        runtime
            .active_settings_mut(first)
            .unwrap()
            .set_color_scheme(ColorSchemePreference::Dark)
            .unwrap();
        let intent = runtime.begin_settings_save(first).unwrap();
        let save = intent.id();

        let selection = runtime
            .begin_selection(second_root.path())
            .unwrap()
            .into_intent();
        let prepared = PreparedProfile::load(&selection).unwrap();
        let rejection = runtime.commit_selection(prepared).unwrap_err();
        assert_eq!(
            rejection.error(),
            &ProfileRuntimeError::ActiveProfileNotDurable { profile: first }
        );
        assert_eq!(runtime.pending_settings_save(), Some(save));
        assert_eq!(runtime.active_profile().unwrap().id(), first);
        let prepared = rejection.into_parts().1;

        runtime.complete_settings_save(intent.execute()).unwrap();
        assert!(!runtime.settings_is_dirty(first).unwrap());
        let commit = runtime.commit_selection(prepared).unwrap();
        let second = commit.active_profile();
        assert_ne!(first, second);
        let replaced = commit.into_replaced_profile().unwrap();
        replaced.into_profile_lock().release().unwrap();
        let reacquired = ProfileLock::acquire(first_root.path()).unwrap();
        reacquired.release().unwrap();
    }

    #[test]
    fn settings_mutation_revision_exhaustion_precedes_mutable_access() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        runtime.active.as_mut().unwrap().settings_revision = u64::MAX;
        let before = runtime.active_profile().unwrap().settings().clone();

        assert_eq!(
            runtime.active_settings_mut(profile).map(|_| ()),
            Err(ProfileRuntimeError::ProfileSettingsMutationRevisionExhausted)
        );
        assert_eq!(runtime.active_profile().unwrap().settings(), &before);
    }

    #[test]
    fn settings_save_id_exhaustion_does_not_create_pending_work() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        runtime
            .active_settings_mut(profile)
            .unwrap()
            .set_color_scheme(ColorSchemePreference::Dark)
            .unwrap();
        runtime.next_settings_save_id = u64::MAX;

        assert_eq!(
            runtime.begin_settings_save(profile),
            Err(ProfileRuntimeError::ProfileSettingsSaveIdExhausted)
        );
        assert!(runtime.pending_settings_save().is_none());
        assert!(runtime.settings_is_dirty(profile).unwrap());
    }

    #[test]
    fn loaded_history_starts_clean_and_successful_record_marks_dirty() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());

        assert!(!runtime.browsing_history_is_dirty(profile).unwrap());
        assert_eq!(
            runtime.browsing_history_unsaved_mutations(profile).unwrap(),
            0
        );
        assert!(
            runtime
                .begin_browsing_history_save_if_dirty(profile)
                .unwrap()
                .is_none()
        );

        runtime
            .record_committed_navigation(
                profile,
                100,
                committed_navigation("https://example.test/dirty"),
            )
            .unwrap();

        assert!(runtime.browsing_history_is_dirty(profile).unwrap());
        assert_eq!(
            runtime.browsing_history_unsaved_mutations(profile).unwrap(),
            1
        );
        let intent = runtime
            .begin_browsing_history_save_if_dirty(profile)
            .unwrap()
            .expect("dirty history should schedule a save");
        assert_eq!(intent.mutation_revision(), 1);
    }

    #[test]
    fn successful_save_cleans_only_mutations_captured_by_its_snapshot() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());

        runtime
            .record_committed_navigation(
                profile,
                100,
                committed_navigation("https://example.test/first"),
            )
            .unwrap();
        let intent = runtime
            .begin_browsing_history_save_if_dirty(profile)
            .unwrap()
            .unwrap();
        assert_eq!(intent.mutation_revision(), 1);

        runtime
            .record_committed_navigation(
                profile,
                200,
                committed_navigation("https://example.test/second"),
            )
            .unwrap();
        assert_eq!(
            runtime.browsing_history_unsaved_mutations(profile).unwrap(),
            2
        );

        let completion = intent.execute();
        assert_eq!(completion.mutation_revision(), 1);
        runtime.complete_browsing_history_save(completion).unwrap();
        assert!(runtime.browsing_history_is_dirty(profile).unwrap());
        assert_eq!(
            runtime.browsing_history_unsaved_mutations(profile).unwrap(),
            1
        );

        let completion = runtime
            .begin_browsing_history_save_if_dirty(profile)
            .unwrap()
            .unwrap()
            .execute();
        runtime.complete_browsing_history_save(completion).unwrap();
        assert!(!runtime.browsing_history_is_dirty(profile).unwrap());
        assert_eq!(
            runtime.browsing_history_unsaved_mutations(profile).unwrap(),
            0
        );
        assert!(
            runtime
                .begin_browsing_history_save_if_dirty(profile)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn mutable_history_access_conservatively_marks_history_dirty() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());

        let _ = runtime.active_browsing_history_mut(profile).unwrap();

        assert!(runtime.browsing_history_is_dirty(profile).unwrap());
        assert_eq!(
            runtime.browsing_history_unsaved_mutations(profile).unwrap(),
            1
        );
    }

    #[test]
    fn mutation_revision_exhaustion_precedes_history_mutation_or_mutable_access() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        runtime.active.as_mut().unwrap().browsing_history_revision = u64::MAX;

        let before = runtime.active_browsing_history(profile).unwrap().clone();
        assert_eq!(
            runtime.record_committed_navigation(
                profile,
                100,
                committed_navigation("https://example.test/exhausted"),
            ),
            Err(ProfileRuntimeError::ProfileHistoryMutationRevisionExhausted)
        );
        assert_eq!(runtime.active_browsing_history(profile).unwrap(), &before);
        assert_eq!(
            runtime.active_browsing_history_mut(profile).map(|_| ()),
            Err(ProfileRuntimeError::ProfileHistoryMutationRevisionExhausted)
        );
        assert_eq!(runtime.active_browsing_history(profile).unwrap(), &before);
    }

    #[test]
    fn history_save_advances_generation_without_losing_newer_in_memory_visits() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        runtime
            .active_browsing_history_mut(profile)
            .unwrap()
            .record_visit(100, "https://example.test/first")
            .unwrap();

        let intent = runtime.begin_browsing_history_save(profile).unwrap();
        let first_save = intent.id();
        assert_eq!(intent.snapshot().generation(), 0);
        assert_eq!(intent.snapshot().len(), 1);
        assert_eq!(runtime.pending_browsing_history_save(), Some(first_save));

        runtime
            .active_browsing_history_mut(profile)
            .unwrap()
            .record_visit(200, "https://example.test/second")
            .unwrap();

        let completion = intent.execute();
        assert!(completion.result().is_ok());
        runtime.complete_browsing_history_save(completion).unwrap();
        let active = runtime.active_browsing_history(profile).unwrap();
        assert_eq!(active.generation(), 1);
        assert_eq!(active.len(), 2);
        assert!(runtime.pending_browsing_history_save().is_none());

        let persisted = BrowsingHistoryStore::open(root.path())
            .unwrap()
            .load()
            .unwrap()
            .into_snapshot();
        assert_eq!(persisted.generation(), 1);
        assert_eq!(persisted.len(), 1);

        let completion = runtime
            .begin_browsing_history_save(profile)
            .unwrap()
            .execute();
        runtime.complete_browsing_history_save(completion).unwrap();
        let persisted = BrowsingHistoryStore::open(root.path())
            .unwrap()
            .load()
            .unwrap()
            .into_snapshot();
        assert_eq!(persisted.generation(), 2);
        assert_eq!(persisted.len(), 2);
    }

    #[test]
    fn overlapping_history_save_is_rejected_until_exact_completion() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let intent = runtime.begin_browsing_history_save(profile).unwrap();

        assert!(matches!(
            runtime.begin_browsing_history_save(profile),
            Err(ProfileRuntimeError::BrowsingHistorySaveAlreadyPending { pending })
                if pending == intent.id()
        ));

        let completion = intent.execute();
        runtime.complete_browsing_history_save(completion).unwrap();
        assert!(runtime.begin_browsing_history_save(profile).is_ok());
    }

    #[test]
    fn concurrent_history_writer_failure_does_not_advance_runtime_generation() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        runtime
            .active_browsing_history_mut(profile)
            .unwrap()
            .record_visit(100, "https://example.test/runtime")
            .unwrap();
        let intent = runtime.begin_browsing_history_save(profile).unwrap();

        let store = BrowsingHistoryStore::open(root.path()).unwrap();
        let external = store.load().unwrap().into_snapshot();
        let lock = runtime.active.as_ref().unwrap().lock.clone();
        store.save(&lock, &external).unwrap();

        let completion = intent.execute();
        assert!(matches!(
            completion.result(),
            Err(BrowsingHistoryError::StaleGeneration {
                current: 1,
                provided: 0,
            })
        ));
        runtime.complete_browsing_history_save(completion).unwrap();
        let active = runtime.active_browsing_history(profile).unwrap();
        assert_eq!(active.generation(), 0);
        assert_eq!(active.len(), 1);
        assert!(runtime.pending_browsing_history_save().is_none());
        assert!(runtime.browsing_history_is_dirty(profile).unwrap());
        assert_eq!(
            runtime.browsing_history_unsaved_mutations(profile).unwrap(),
            1
        );
    }

    #[test]
    fn profile_replacement_waits_for_pending_history_save() {
        let first_root = TempRoot::new();
        let second_root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let first = load_profile(&mut runtime, first_root.path());
        runtime
            .active_browsing_history_mut(first)
            .unwrap()
            .record_visit(100, "https://example.test/old")
            .unwrap();
        let intent = runtime.begin_browsing_history_save(first).unwrap();
        let save = intent.id();

        let selection = runtime
            .begin_selection(second_root.path())
            .unwrap()
            .into_intent();
        let prepared = PreparedProfile::load(&selection).unwrap();
        let rejection = runtime.commit_selection(prepared).unwrap_err();
        assert_eq!(
            rejection.error(),
            &ProfileRuntimeError::ActiveProfileNotDurable { profile: first }
        );
        assert_eq!(runtime.pending_browsing_history_save(), Some(save));
        assert_eq!(runtime.active_profile().unwrap().id(), first);
        let prepared = rejection.into_parts().1;

        runtime
            .complete_browsing_history_save(intent.execute())
            .unwrap();
        assert!(!runtime.browsing_history_is_dirty(first).unwrap());
        let commit = runtime.commit_selection(prepared).unwrap();
        let second = commit.active_profile();
        assert_ne!(first, second);
        let replaced = commit.into_replaced_profile().unwrap();
        replaced.into_profile_lock().release().unwrap();
        let reacquired = ProfileLock::acquire(first_root.path()).unwrap();
        reacquired.release().unwrap();
    }

    #[test]
    fn cancelling_exact_history_save_preserves_dirty_state_for_retry() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        runtime
            .record_committed_navigation(
                profile,
                100,
                committed_navigation("https://example.test/dirty"),
            )
            .unwrap();

        let first = runtime
            .begin_browsing_history_save_if_dirty(profile)
            .unwrap()
            .unwrap();
        let first_id = first.id();
        assert_eq!(runtime.pending_browsing_history_save(), Some(first_id));

        runtime.cancel_browsing_history_save(first_id).unwrap();

        assert!(runtime.pending_browsing_history_save().is_none());
        assert!(runtime.browsing_history_is_dirty(profile).unwrap());
        assert_eq!(
            runtime.browsing_history_unsaved_mutations(profile).unwrap(),
            1
        );
        let retry = runtime
            .begin_browsing_history_save_if_dirty(profile)
            .unwrap()
            .expect("cancelled dirty save should be retryable");
        assert_ne!(retry.id(), first_id);
    }

    #[test]
    fn stale_history_save_cancellation_cannot_clear_current_pending_save() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        runtime
            .record_committed_navigation(
                profile,
                100,
                committed_navigation("https://example.test/dirty"),
            )
            .unwrap();

        let first = runtime.begin_browsing_history_save(profile).unwrap().id();
        runtime.cancel_browsing_history_save(first).unwrap();
        let current = runtime.begin_browsing_history_save(profile).unwrap().id();

        assert_eq!(
            runtime.cancel_browsing_history_save(first),
            Err(ProfileRuntimeError::StaleBrowsingHistorySave {
                expected: Some(current),
                actual: first,
            })
        );
        assert_eq!(runtime.pending_browsing_history_save(), Some(current));
    }

    #[test]
    fn history_save_id_exhaustion_does_not_create_pending_work() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        runtime.next_history_save_id = u64::MAX;

        assert_eq!(
            runtime.begin_browsing_history_save(profile),
            Err(ProfileRuntimeError::ProfileHistorySaveIdExhausted)
        );
        assert!(runtime.pending_browsing_history_save().is_none());
    }
}
