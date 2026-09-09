use crate::profile_runtime::{
    ProfileId, ProfileRuntime, ProfileRuntimeError, ProfileSettingsSaveIntent,
};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileSettingsSavePolicy {
    debounce_millis: u64,
    max_dirty_millis: u64,
    mutation_threshold: u64,
}

impl ProfileSettingsSavePolicy {
    pub fn new(
        debounce_millis: u64,
        max_dirty_millis: u64,
        mutation_threshold: u64,
    ) -> Result<Self, ProfileSettingsSavePolicyError> {
        if debounce_millis == 0 {
            return Err(ProfileSettingsSavePolicyError::ZeroDebounce);
        }
        if max_dirty_millis == 0 {
            return Err(ProfileSettingsSavePolicyError::ZeroMaxDirtyAge);
        }
        if mutation_threshold == 0 {
            return Err(ProfileSettingsSavePolicyError::ZeroMutationThreshold);
        }
        if max_dirty_millis < debounce_millis {
            return Err(ProfileSettingsSavePolicyError::MaxDirtyAgeBeforeDebounce {
                debounce_millis,
                max_dirty_millis,
            });
        }
        Ok(Self {
            debounce_millis,
            max_dirty_millis,
            mutation_threshold,
        })
    }

    pub const fn debounce_millis(self) -> u64 {
        self.debounce_millis
    }

    pub const fn max_dirty_millis(self) -> u64 {
        self.max_dirty_millis
    }

    pub const fn mutation_threshold(self) -> u64 {
        self.mutation_threshold
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileSettingsSavePolicyError {
    ZeroDebounce,
    ZeroMaxDirtyAge,
    ZeroMutationThreshold,
    MaxDirtyAgeBeforeDebounce {
        debounce_millis: u64,
        max_dirty_millis: u64,
    },
}

impl fmt::Display for ProfileSettingsSavePolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDebounce => formatter.write_str("settings save debounce must be nonzero"),
            Self::ZeroMaxDirtyAge => {
                formatter.write_str("settings save maximum dirty age must be nonzero")
            }
            Self::ZeroMutationThreshold => {
                formatter.write_str("settings save mutation threshold must be nonzero")
            }
            Self::MaxDirtyAgeBeforeDebounce {
                debounce_millis,
                max_dirty_millis,
            } => write!(
                formatter,
                "settings save maximum dirty age {max_dirty_millis}ms is shorter than debounce {debounce_millis}ms"
            ),
        }
    }
}

impl std::error::Error for ProfileSettingsSavePolicyError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileSettingsSaveUrgency {
    Normal,
    Flush,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ProfileSettingsSaveScheduler {
    policy: ProfileSettingsSavePolicy,
    profile: Option<ProfileId>,
    observed_mutation_revision: u64,
    first_dirty_millis: Option<u64>,
    last_mutation_millis: Option<u64>,
    last_now_millis: Option<u64>,
}

impl ProfileSettingsSaveScheduler {
    pub const fn new(policy: ProfileSettingsSavePolicy) -> Self {
        Self {
            policy,
            profile: None,
            observed_mutation_revision: 0,
            first_dirty_millis: None,
            last_mutation_millis: None,
            last_now_millis: None,
        }
    }

    pub const fn policy(&self) -> ProfileSettingsSavePolicy {
        self.policy
    }

    pub const fn tracked_profile(&self) -> Option<ProfileId> {
        self.profile
    }

    pub fn next_save_due_millis(&self) -> Option<u64> {
        let debounce_due = self
            .last_mutation_millis
            .map(|last| last.saturating_add(self.policy.debounce_millis));
        let max_age_due = self
            .first_dirty_millis
            .map(|first| first.saturating_add(self.policy.max_dirty_millis));
        match (debounce_due, max_age_due) {
            (Some(debounce), Some(max_age)) => Some(debounce.min(max_age)),
            (Some(debounce), None) => Some(debounce),
            (None, Some(max_age)) => Some(max_age),
            (None, None) => None,
        }
    }

    pub fn poll(
        &mut self,
        runtime: &mut ProfileRuntime,
        profile: ProfileId,
        now_millis: u64,
        urgency: ProfileSettingsSaveUrgency,
    ) -> Result<Option<ProfileSettingsSaveIntent>, ProfileRuntimeError> {
        let mutation_revision = runtime.settings_mutation_revision(profile)?;
        let unsaved_mutations = runtime.settings_unsaved_mutations(profile)?;
        let save_pending = runtime.pending_settings_save().is_some();

        let profile_changed = self.profile != Some(profile);
        let time_regressed = !profile_changed
            && self
                .last_now_millis
                .is_some_and(|last_now| now_millis < last_now);
        let now_millis = if profile_changed {
            now_millis
        } else {
            self.last_now_millis
                .map_or(now_millis, |last_now| last_now.max(now_millis))
        };

        if profile_changed {
            self.profile = Some(profile);
            self.observed_mutation_revision = mutation_revision;
            self.first_dirty_millis = (unsaved_mutations != 0).then_some(now_millis);
            self.last_mutation_millis = self.first_dirty_millis;
            self.last_now_millis = Some(now_millis);
        } else {
            self.last_now_millis = Some(now_millis);
            if mutation_revision != self.observed_mutation_revision {
                self.observed_mutation_revision = mutation_revision;
                if unsaved_mutations != 0 {
                    self.first_dirty_millis.get_or_insert(now_millis);
                    self.last_mutation_millis = Some(now_millis);
                }
            } else if unsaved_mutations != 0 && !save_pending && self.first_dirty_millis.is_none() {
                self.first_dirty_millis = Some(now_millis);
                self.last_mutation_millis = Some(now_millis);
            }
        }

        if unsaved_mutations == 0 {
            self.clear_dirty_window();
            return Ok(None);
        }
        if save_pending {
            return Ok(None);
        }

        let debounce_due = self
            .last_mutation_millis
            .is_some_and(|last| now_millis.saturating_sub(last) >= self.policy.debounce_millis);
        let max_age_due = self
            .first_dirty_millis
            .is_some_and(|first| now_millis.saturating_sub(first) >= self.policy.max_dirty_millis);
        let threshold_due = unsaved_mutations >= self.policy.mutation_threshold;
        let should_save = urgency == ProfileSettingsSaveUrgency::Flush
            || time_regressed
            || threshold_due
            || debounce_due
            || max_age_due;

        if !should_save {
            return Ok(None);
        }

        let intent = runtime.begin_settings_save_if_dirty(profile)?;
        if intent.is_some() {
            self.clear_dirty_window();
        }
        Ok(intent)
    }

    fn clear_dirty_window(&mut self) {
        self.first_dirty_millis = None;
        self.last_mutation_millis = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColorSchemePreference, PreparedProfile};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(1);

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let id = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "zorya-settings-save-scheduler-{}-{id}",
                std::process::id()
            ));
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

    fn policy(
        debounce_millis: u64,
        max_dirty_millis: u64,
        mutation_threshold: u64,
    ) -> ProfileSettingsSavePolicy {
        ProfileSettingsSavePolicy::new(debounce_millis, max_dirty_millis, mutation_threshold)
            .unwrap()
    }

    fn load_profile(runtime: &mut ProfileRuntime, root: &Path) -> ProfileId {
        let intent = runtime.begin_selection(root).unwrap().into_intent();
        let prepared = PreparedProfile::load(&intent).unwrap();
        runtime.commit_selection(prepared).unwrap().active_profile()
    }

    fn mutate(runtime: &mut ProfileRuntime, profile: ProfileId) {
        runtime
            .active_settings_mut(profile)
            .unwrap()
            .set_color_scheme(ColorSchemePreference::Dark)
            .unwrap();
    }

    #[test]
    fn clean_settings_never_schedule() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut scheduler = ProfileSettingsSaveScheduler::new(policy(10, 100, 3));

        assert!(
            scheduler
                .poll(&mut runtime, profile, 0, ProfileSettingsSaveUrgency::Normal)
                .unwrap()
                .is_none()
        );
        assert_eq!(scheduler.next_save_due_millis(), None);
    }

    #[test]
    fn debounce_schedules_after_quiet_window() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut scheduler = ProfileSettingsSaveScheduler::new(policy(10, 100, 3));

        mutate(&mut runtime, profile);
        assert!(
            scheduler
                .poll(&mut runtime, profile, 5, ProfileSettingsSaveUrgency::Normal)
                .unwrap()
                .is_none()
        );
        assert_eq!(scheduler.next_save_due_millis(), Some(15));
        assert!(
            scheduler
                .poll(
                    &mut runtime,
                    profile,
                    14,
                    ProfileSettingsSaveUrgency::Normal
                )
                .unwrap()
                .is_none()
        );
        assert!(
            scheduler
                .poll(
                    &mut runtime,
                    profile,
                    15,
                    ProfileSettingsSaveUrgency::Normal
                )
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn threshold_schedules_without_waiting_for_debounce() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut scheduler = ProfileSettingsSaveScheduler::new(policy(100, 1_000, 2));

        mutate(&mut runtime, profile);
        mutate(&mut runtime, profile);
        assert!(
            scheduler
                .poll(&mut runtime, profile, 1, ProfileSettingsSaveUrgency::Normal)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn max_dirty_age_bounds_continuous_mutation() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut scheduler = ProfileSettingsSaveScheduler::new(policy(50, 100, 100));

        mutate(&mut runtime, profile);
        scheduler
            .poll(
                &mut runtime,
                profile,
                10,
                ProfileSettingsSaveUrgency::Normal,
            )
            .unwrap();
        mutate(&mut runtime, profile);
        scheduler
            .poll(
                &mut runtime,
                profile,
                80,
                ProfileSettingsSaveUrgency::Normal,
            )
            .unwrap();
        mutate(&mut runtime, profile);
        assert!(
            scheduler
                .poll(
                    &mut runtime,
                    profile,
                    110,
                    ProfileSettingsSaveUrgency::Normal
                )
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn flush_schedules_dirty_settings_immediately() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut scheduler = ProfileSettingsSaveScheduler::new(policy(100, 1_000, 10));

        mutate(&mut runtime, profile);
        assert!(
            scheduler
                .poll(&mut runtime, profile, 1, ProfileSettingsSaveUrgency::Flush)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn pending_save_blocks_overlap_and_newer_mutation_remains_schedulable() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut scheduler = ProfileSettingsSaveScheduler::new(policy(10, 100, 10));

        mutate(&mut runtime, profile);
        let intent = scheduler
            .poll(&mut runtime, profile, 0, ProfileSettingsSaveUrgency::Flush)
            .unwrap()
            .unwrap();
        mutate(&mut runtime, profile);
        assert!(
            scheduler
                .poll(
                    &mut runtime,
                    profile,
                    20,
                    ProfileSettingsSaveUrgency::Normal
                )
                .unwrap()
                .is_none()
        );

        let completion = intent.execute();
        runtime.complete_settings_save(completion).unwrap();
        assert!(
            scheduler
                .poll(
                    &mut runtime,
                    profile,
                    30,
                    ProfileSettingsSaveUrgency::Normal
                )
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn clock_regression_flushes_dirty_state_conservatively() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut scheduler = ProfileSettingsSaveScheduler::new(policy(100, 1_000, 10));

        scheduler
            .poll(
                &mut runtime,
                profile,
                500,
                ProfileSettingsSaveUrgency::Normal,
            )
            .unwrap();
        mutate(&mut runtime, profile);
        assert!(
            scheduler
                .poll(
                    &mut runtime,
                    profile,
                    400,
                    ProfileSettingsSaveUrgency::Normal
                )
                .unwrap()
                .is_some()
        );
    }
}
