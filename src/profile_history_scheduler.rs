use crate::profile_runtime::{
    ProfileHistorySaveIntent, ProfileId, ProfileRuntime, ProfileRuntimeError,
};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileHistorySavePolicy {
    debounce_millis: u64,
    max_dirty_millis: u64,
    mutation_threshold: u64,
}

impl ProfileHistorySavePolicy {
    pub fn new(
        debounce_millis: u64,
        max_dirty_millis: u64,
        mutation_threshold: u64,
    ) -> Result<Self, ProfileHistorySavePolicyError> {
        if debounce_millis == 0 {
            return Err(ProfileHistorySavePolicyError::ZeroDebounce);
        }
        if max_dirty_millis == 0 {
            return Err(ProfileHistorySavePolicyError::ZeroMaxDirtyAge);
        }
        if mutation_threshold == 0 {
            return Err(ProfileHistorySavePolicyError::ZeroMutationThreshold);
        }
        if max_dirty_millis < debounce_millis {
            return Err(ProfileHistorySavePolicyError::MaxDirtyAgeBeforeDebounce {
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
pub enum ProfileHistorySavePolicyError {
    ZeroDebounce,
    ZeroMaxDirtyAge,
    ZeroMutationThreshold,
    MaxDirtyAgeBeforeDebounce {
        debounce_millis: u64,
        max_dirty_millis: u64,
    },
}

impl fmt::Display for ProfileHistorySavePolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDebounce => formatter.write_str("history save debounce must be nonzero"),
            Self::ZeroMaxDirtyAge => {
                formatter.write_str("history save maximum dirty age must be nonzero")
            }
            Self::ZeroMutationThreshold => {
                formatter.write_str("history save mutation threshold must be nonzero")
            }
            Self::MaxDirtyAgeBeforeDebounce {
                debounce_millis,
                max_dirty_millis,
            } => write!(
                formatter,
                "history save maximum dirty age {max_dirty_millis}ms is shorter than debounce {debounce_millis}ms"
            ),
        }
    }
}

impl std::error::Error for ProfileHistorySavePolicyError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileHistorySaveUrgency {
    Normal,
    Flush,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ProfileHistorySaveScheduler {
    policy: ProfileHistorySavePolicy,
    profile: Option<ProfileId>,
    observed_mutation_revision: u64,
    first_dirty_millis: Option<u64>,
    last_mutation_millis: Option<u64>,
    last_now_millis: Option<u64>,
}

impl ProfileHistorySaveScheduler {
    pub const fn new(policy: ProfileHistorySavePolicy) -> Self {
        Self {
            policy,
            profile: None,
            observed_mutation_revision: 0,
            first_dirty_millis: None,
            last_mutation_millis: None,
            last_now_millis: None,
        }
    }

    pub const fn policy(&self) -> ProfileHistorySavePolicy {
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
        urgency: ProfileHistorySaveUrgency,
    ) -> Result<Option<ProfileHistorySaveIntent>, ProfileRuntimeError> {
        let mutation_revision = runtime.browsing_history_mutation_revision(profile)?;
        let unsaved_mutations = runtime.browsing_history_unsaved_mutations(profile)?;
        let save_pending = runtime.pending_browsing_history_save().is_some();

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
        let should_save = urgency == ProfileHistorySaveUrgency::Flush
            || time_regressed
            || threshold_due
            || debounce_due
            || max_age_due;

        if !should_save {
            return Ok(None);
        }

        let intent = runtime.begin_browsing_history_save_if_dirty(profile)?;
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
    use crate::{BrowserApp, BrowserWindow, PreparedProfile};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(1);

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let id = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "zorya-history-save-scheduler-{}-{id}",
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
    ) -> ProfileHistorySavePolicy {
        ProfileHistorySavePolicy::new(debounce_millis, max_dirty_millis, mutation_threshold)
            .unwrap()
    }

    fn load_profile(runtime: &mut ProfileRuntime, root: &Path) -> ProfileId {
        let intent = runtime.begin_selection(root).unwrap().into_intent();
        let prepared = PreparedProfile::load(&intent).unwrap();
        runtime.commit_selection(prepared).unwrap().active_profile()
    }

    fn committed_navigation(location: &str) -> crate::BrowserNavigationCommit {
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

    fn record(runtime: &mut ProfileRuntime, profile: ProfileId, visit: u64) {
        runtime
            .record_committed_navigation(
                profile,
                visit,
                committed_navigation(&format!("https://example.test/{visit}")),
            )
            .unwrap();
    }

    fn scheduled(
        scheduler: &mut ProfileHistorySaveScheduler,
        runtime: &mut ProfileRuntime,
        profile: ProfileId,
        now_millis: u64,
        urgency: ProfileHistorySaveUrgency,
    ) -> bool {
        scheduler
            .poll(runtime, profile, now_millis, urgency)
            .unwrap()
            .is_some()
    }

    fn unsaved(runtime: &ProfileRuntime, profile: ProfileId) -> u64 {
        runtime.browsing_history_unsaved_mutations(profile).unwrap()
    }

    #[test]
    fn policy_rejects_zero_and_inverted_bounds() {
        assert_eq!(
            ProfileHistorySavePolicy::new(0, 10, 1),
            Err(ProfileHistorySavePolicyError::ZeroDebounce)
        );
        assert_eq!(
            ProfileHistorySavePolicy::new(1, 0, 1),
            Err(ProfileHistorySavePolicyError::ZeroMaxDirtyAge)
        );
        assert_eq!(
            ProfileHistorySavePolicy::new(1, 10, 0),
            Err(ProfileHistorySavePolicyError::ZeroMutationThreshold)
        );
        assert_eq!(
            ProfileHistorySavePolicy::new(10, 9, 1),
            Err(ProfileHistorySavePolicyError::MaxDirtyAgeBeforeDebounce {
                debounce_millis: 10,
                max_dirty_millis: 9,
            })
        );
    }

    #[test]
    fn clean_history_never_schedules_even_for_flush() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut scheduler = ProfileHistorySaveScheduler::new(policy(10, 100, 10));

        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            0,
            ProfileHistorySaveUrgency::Normal,
        ));
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            1,
            ProfileHistorySaveUrgency::Flush,
        ));
    }

    #[test]
    fn debounce_restarts_after_new_mutation() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut scheduler = ProfileHistorySaveScheduler::new(policy(10, 100, 10));

        record(&mut runtime, profile, 1);
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            0,
            ProfileHistorySaveUrgency::Normal,
        ));
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            9,
            ProfileHistorySaveUrgency::Normal,
        ));

        record(&mut runtime, profile, 2);
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            9,
            ProfileHistorySaveUrgency::Normal,
        ));
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            18,
            ProfileHistorySaveUrgency::Normal,
        ));
        assert!(scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            19,
            ProfileHistorySaveUrgency::Normal,
        ));
    }

    #[test]
    fn next_save_deadline_tracks_debounce_and_max_dirty_age() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut scheduler = ProfileHistorySaveScheduler::new(policy(10, 20, 100));

        assert_eq!(scheduler.next_save_due_millis(), None);

        record(&mut runtime, profile, 1);
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            0,
            ProfileHistorySaveUrgency::Normal,
        ));
        assert_eq!(scheduler.next_save_due_millis(), Some(10));

        record(&mut runtime, profile, 2);
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            9,
            ProfileHistorySaveUrgency::Normal,
        ));
        assert_eq!(scheduler.next_save_due_millis(), Some(19));

        record(&mut runtime, profile, 3);
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            15,
            ProfileHistorySaveUrgency::Normal,
        ));
        assert_eq!(scheduler.next_save_due_millis(), Some(20));

        assert!(scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            20,
            ProfileHistorySaveUrgency::Normal,
        ));
        assert_eq!(scheduler.next_save_due_millis(), None);
    }

    #[test]
    fn mutation_threshold_schedules_without_waiting_for_debounce() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut scheduler = ProfileHistorySaveScheduler::new(policy(100, 1_000, 2));

        record(&mut runtime, profile, 1);
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            0,
            ProfileHistorySaveUrgency::Normal,
        ));
        record(&mut runtime, profile, 2);
        assert!(scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            1,
            ProfileHistorySaveUrgency::Normal,
        ));
    }

    #[test]
    fn maximum_dirty_age_bounds_continuous_mutation() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut scheduler = ProfileHistorySaveScheduler::new(policy(10, 20, 100));

        for now in [0, 5, 10, 15] {
            record(&mut runtime, profile, now + 1);
            assert!(!scheduled(
                &mut scheduler,
                &mut runtime,
                profile,
                now,
                ProfileHistorySaveUrgency::Normal,
            ));
        }

        record(&mut runtime, profile, 21);
        assert!(scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            20,
            ProfileHistorySaveUrgency::Normal,
        ));
    }

    #[test]
    fn flush_schedules_dirty_history_immediately() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut scheduler = ProfileHistorySaveScheduler::new(policy(100, 1_000, 100));

        record(&mut runtime, profile, 1);
        assert!(scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            0,
            ProfileHistorySaveUrgency::Flush,
        ));
    }

    #[test]
    fn in_flight_save_coalesces_newer_mutations_until_completion() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut scheduler = ProfileHistorySaveScheduler::new(policy(100, 1_000, 1));

        record(&mut runtime, profile, 1);
        let first = scheduler
            .poll(&mut runtime, profile, 0, ProfileHistorySaveUrgency::Normal)
            .unwrap()
            .expect("threshold should start first save");

        record(&mut runtime, profile, 2);
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            1,
            ProfileHistorySaveUrgency::Flush,
        ));

        runtime
            .complete_browsing_history_save(first.execute())
            .unwrap();
        assert_eq!(unsaved(&runtime, profile), 1);
        assert!(scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            1,
            ProfileHistorySaveUrgency::Normal,
        ));
    }

    #[test]
    fn profile_replacement_rejects_stale_poll_and_resets_time_ownership() {
        let first_root = TempRoot::new();
        let second_root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let first = load_profile(&mut runtime, first_root.path());
        let mut scheduler = ProfileHistorySaveScheduler::new(policy(10, 100, 10));

        record(&mut runtime, first, 1);
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            first,
            100,
            ProfileHistorySaveUrgency::Normal,
        ));

        let second = load_profile(&mut runtime, second_root.path());
        assert!(matches!(
            scheduler.poll(
                &mut runtime,
                first,
                101,
                ProfileHistorySaveUrgency::Normal,
            ),
            Err(ProfileRuntimeError::StaleProfile {
                expected: Some(expected),
                actual,
            }) if expected == second && actual == first
        ));

        record(&mut runtime, second, 2);
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            second,
            1,
            ProfileHistorySaveUrgency::Normal,
        ));
        assert_eq!(scheduler.tracked_profile(), Some(second));
    }

    #[test]
    fn monotonic_time_regression_saves_dirty_history_conservatively() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut scheduler = ProfileHistorySaveScheduler::new(policy(100, 1_000, 100));

        record(&mut runtime, profile, 1);
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            100,
            ProfileHistorySaveUrgency::Normal,
        ));
        assert!(scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            99,
            ProfileHistorySaveUrgency::Normal,
        ));
    }
}
