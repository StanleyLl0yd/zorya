use crate::browsing_history::{
    BrowsingHistoryRecovery, BrowsingHistorySnapshot, BrowsingHistoryStore,
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
pub enum ProfileRuntimeError {
    ProfileIdExhausted,
    ProfileSelectionIdExhausted,
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
    active: Option<ActiveProfile>,
    pending: Option<ProfileSelectionIntent>,
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
            active: None,
            pending: None,
        }
    }

    pub const fn active_profile(&self) -> Option<&ActiveProfile> {
        self.active.as_ref()
    }

    pub const fn pending_selection(&self) -> Option<&ProfileSelectionIntent> {
        self.pending.as_ref()
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

        let first = runtime
            .commit_selection(loaded)
            .unwrap()
            .active_profile();
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
}
