use crate::browsing_history::{
    BrowsingHistoryError, BrowsingHistoryRecovery, BrowsingHistorySave, BrowsingHistorySnapshot,
    BrowsingHistoryStore,
};
use crate::profile::{ProfileStorageError, ProfileStore, SettingsRecovery, SettingsSnapshot};
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
    settings: ProductSettings,
    settings_recovery: Option<SettingsRecovery>,
    browsing_history: BrowsingHistorySnapshot,
    browsing_history_recovery: Option<BrowsingHistoryRecovery>,
}

impl PreparedProfile {
    pub fn load(intent: &ProfileSelectionIntent) -> Result<Self, ProfilePreparationError> {
        let store =
            ProfileStore::open(intent.root.clone()).map_err(ProfilePreparationError::Storage)?;
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

        Ok(Self {
            selection: intent.id,
            root: intent.root.clone(),
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfilePreparationError {
    Storage(ProfileStorageError),
    Settings(ProfileSettingsError),
    BrowsingHistory(crate::browsing_history::BrowsingHistoryError),
}

impl fmt::Display for ProfilePreparationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => error.fmt(formatter),
            Self::Settings(error) => error.fmt(formatter),
            Self::BrowsingHistory(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ProfilePreparationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Storage(error) => Some(error),
            Self::Settings(error) => Some(error),
            Self::BrowsingHistory(error) => Some(error),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveProfile {
    id: ProfileId,
    root: PathBuf,
    settings: ProductSettings,
    settings_recovery: Option<SettingsRecovery>,
    browsing_history: BrowsingHistorySnapshot,
    browsing_history_recovery: Option<BrowsingHistoryRecovery>,
}

impl ActiveProfile {
    pub const fn id(&self) -> ProfileId {
        self.id
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileHistorySaveId(u64);

impl ProfileHistorySaveId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileHistorySaveIntent {
    id: ProfileHistorySaveId,
    profile: ProfileId,
    root: PathBuf,
    snapshot: BrowsingHistorySnapshot,
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

    pub fn execute(self) -> ProfileHistorySaveCompletion {
        let base_generation = self.snapshot.generation();
        let result = BrowsingHistoryStore::open(self.root.clone())
            .and_then(|store| store.save(&self.snapshot));
        ProfileHistorySaveCompletion {
            id: self.id,
            profile: self.profile,
            root: self.root,
            base_generation,
            result,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileHistorySaveCompletion {
    id: ProfileHistorySaveId,
    profile: ProfileId,
    root: PathBuf,
    base_generation: u64,
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileSelectionCommit {
    active_profile: ProfileId,
    replaced_profile: Option<ActiveProfile>,
    invalidated_browsing_history_save: Option<ProfileHistorySaveId>,
}

impl ProfileSelectionCommit {
    pub const fn active_profile(&self) -> ProfileId {
        self.active_profile
    }

    pub const fn replaced_profile(&self) -> Option<&ActiveProfile> {
        self.replaced_profile.as_ref()
    }

    pub const fn invalidated_browsing_history_save(&self) -> Option<ProfileHistorySaveId> {
        self.invalidated_browsing_history_save
    }

    pub fn into_replaced_profile(self) -> Option<ActiveProfile> {
        self.replaced_profile
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileRuntimeError {
    ProfileIdExhausted,
    ProfileSelectionIdExhausted,
    ProfileHistorySaveIdExhausted,
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
    StaleSelection {
        expected: Option<ProfileSelectionId>,
        actual: ProfileSelectionId,
    },
    SelectionTargetMismatch {
        selection: ProfileSelectionId,
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
            Self::ProfileHistorySaveIdExhausted => {
                formatter.write_str("profile browsing-history save identifier space is exhausted")
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileRuntime {
    next_profile_id: u64,
    next_selection_id: u64,
    next_history_save_id: u64,
    active: Option<ActiveProfile>,
    pending: Option<ProfileSelectionIntent>,
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
            next_history_save_id: 1,
            active: None,
            pending: None,
            pending_history_save: None,
        }
    }

    pub const fn active_profile(&self) -> Option<&ActiveProfile> {
        self.active.as_ref()
    }

    pub const fn pending_selection(&self) -> Option<&ProfileSelectionIntent> {
        self.pending.as_ref()
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
    ) -> Result<ProfileSelectionCommit, ProfileRuntimeError> {
        let expected = self.pending.as_ref().map(ProfileSelectionIntent::id);
        if expected != Some(prepared.selection) {
            return Err(ProfileRuntimeError::StaleSelection {
                expected,
                actual: prepared.selection,
            });
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|intent| intent.root != prepared.root)
        {
            return Err(ProfileRuntimeError::SelectionTargetMismatch {
                selection: prepared.selection,
            });
        }

        let id = ProfileId(self.next_profile_id);
        self.next_profile_id = self
            .next_profile_id
            .checked_add(1)
            .ok_or(ProfileRuntimeError::ProfileIdExhausted)?;
        self.pending = None;
        let active = ActiveProfile {
            id,
            root: prepared.root,
            settings: prepared.settings,
            settings_recovery: prepared.settings_recovery,
            browsing_history: prepared.browsing_history,
            browsing_history_recovery: prepared.browsing_history_recovery,
        };
        let invalidated_browsing_history_save =
            self.pending_history_save.take().map(|pending| pending.id);
        let replaced_profile = self.active.replace(active);
        Ok(ProfileSelectionCommit {
            active_profile: id,
            replaced_profile,
            invalidated_browsing_history_save,
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
        Ok(&mut self
            .active
            .as_mut()
            .expect("active profile was validated")
            .settings)
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
        Ok(&mut self
            .active
            .as_mut()
            .expect("active profile was validated")
            .browsing_history)
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
        let snapshot = active.browsing_history.clone();
        let base_generation = snapshot.generation();
        let id = ProfileHistorySaveId(self.next_history_save_id);
        self.next_history_save_id = self
            .next_history_save_id
            .checked_add(1)
            .ok_or(ProfileRuntimeError::ProfileHistorySaveIdExhausted)?;
        self.pending_history_save = Some(PendingProfileHistorySave {
            id,
            profile,
            base_generation,
        });
        Ok(ProfileHistorySaveIntent {
            id,
            profile,
            root,
            snapshot,
        })
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
        let active = self
            .active
            .as_ref()
            .ok_or(ProfileRuntimeError::BrowsingHistorySaveTargetMismatch {
                save: completion.id,
            })?;
        if pending.profile != completion.profile
            || pending.base_generation != completion.base_generation
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
        }
        Ok(completion)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        PreparedProfile {
            selection: intent.id,
            root: intent.root.clone(),
            settings: ProductSettings::from_snapshot(SettingsSnapshot::default()).unwrap(),
            settings_recovery: None,
            browsing_history: BrowsingHistorySnapshot::default(),
            browsing_history_recovery: None,
        }
    }

    fn load_profile(runtime: &mut ProfileRuntime, root: &Path) -> ProfileId {
        let intent = runtime.begin_selection(root).unwrap().into_intent();
        let prepared = PreparedProfile::load(&intent).unwrap();
        runtime.commit_selection(prepared).unwrap().active_profile()
    }

    #[test]
    fn selection_is_two_phase_and_supersession_is_stale_safe() {
        let mut runtime = ProfileRuntime::new();
        let first = runtime.begin_selection("first").unwrap().into_intent();
        let first_prepared = prepared(&first);
        let second_start = runtime.begin_selection("second").unwrap();
        assert_eq!(second_start.superseded(), Some(&first));
        let second = second_start.into_intent();

        assert_eq!(
            runtime.commit_selection(first_prepared),
            Err(ProfileRuntimeError::StaleSelection {
                expected: Some(second.id()),
                actual: first.id(),
            })
        );
        assert!(runtime.active_profile().is_none());

        let commit = runtime.commit_selection(prepared(&second)).unwrap();
        assert_eq!(commit.active_profile().get(), 1);
        assert!(commit.replaced_profile().is_none());
        assert_eq!(
            runtime.active_profile().unwrap().root(),
            Path::new("second")
        );
    }

    #[test]
    fn committed_profile_survives_pending_selection_and_exact_cancel() {
        let mut runtime = ProfileRuntime::new();
        let first = runtime.begin_selection("first").unwrap().into_intent();
        let first_id = runtime
            .commit_selection(prepared(&first))
            .unwrap()
            .active_profile();
        let second = runtime.begin_selection("second").unwrap().into_intent();

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
        let mut runtime = ProfileRuntime::new();
        let first = runtime.begin_selection("first").unwrap().into_intent();
        let first_id = runtime
            .commit_selection(prepared(&first))
            .unwrap()
            .active_profile();
        let second = runtime.begin_selection("second").unwrap().into_intent();
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
        let intent = runtime.begin_selection("profile").unwrap().into_intent();
        runtime.next_profile_id = u64::MAX;
        assert_eq!(
            runtime.commit_selection(prepared(&intent)),
            Err(ProfileRuntimeError::ProfileIdExhausted)
        );
        assert_eq!(runtime.pending_selection(), Some(&intent));
        assert!(runtime.active_profile().is_none());
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
    fn prepared_profile_loads_recoverable_store_into_typed_settings() {
        let root = TempRoot::new();
        let store = ProfileStore::open(root.path()).unwrap();
        let mut raw = store.load_settings().unwrap().into_snapshot();
        raw.set(COLOR_SCHEME_KEY, "light").unwrap();
        raw.set("future.same_schema.key", "preserved").unwrap();
        let saved = store.save_settings(&raw).unwrap().into_snapshot();
        assert_eq!(saved.generation(), 1);

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
        let saved = store.save(&snapshot).unwrap().into_snapshot();
        assert_eq!(saved.generation(), 1);

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
        runtime
            .active_browsing_history_mut(first)
            .unwrap()
            .record_visit(456, "https://example.test/second")
            .unwrap();
        assert_eq!(runtime.active_browsing_history(first).unwrap().len(), 2);

        let replacement = runtime
            .begin_selection("replacement")
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
        runtime
            .complete_browsing_history_save(completion)
            .unwrap();
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
        runtime
            .complete_browsing_history_save(completion)
            .unwrap();
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
        runtime
            .complete_browsing_history_save(completion)
            .unwrap();
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
        store.save(&external).unwrap();

        let completion = intent.execute();
        assert!(matches!(
            completion.result(),
            Err(BrowsingHistoryError::StaleGeneration {
                current: 1,
                provided: 0,
            })
        ));
        runtime
            .complete_browsing_history_save(completion)
            .unwrap();
        let active = runtime.active_browsing_history(profile).unwrap();
        assert_eq!(active.generation(), 0);
        assert_eq!(active.len(), 1);
        assert!(runtime.pending_browsing_history_save().is_none());
    }

    #[test]
    fn profile_replacement_invalidates_pending_history_save_and_rejects_completion() {
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
        let commit = runtime.commit_selection(prepared).unwrap();
        let second = commit.active_profile();
        assert_eq!(commit.invalidated_browsing_history_save(), Some(save));
        assert!(runtime.pending_browsing_history_save().is_none());

        let completion = intent.execute();
        assert_eq!(
            runtime.complete_browsing_history_save(completion),
            Err(ProfileRuntimeError::StaleBrowsingHistorySave {
                expected: None,
                actual: save,
            })
        );
        assert_eq!(
            runtime.active_browsing_history(second).unwrap().generation(),
            0
        );
        assert!(runtime.active_browsing_history(second).unwrap().is_empty());
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
