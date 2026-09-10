use crate::profile_runtime::{
    ProfileBookmarksRuntimeError, ProfileBookmarksSaveIntent, ProfileId, ProfileRuntime,
};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileBookmarksSavePolicy {
    debounce_millis: u64,
    max_dirty_millis: u64,
    mutation_threshold: u64,
}

impl ProfileBookmarksSavePolicy {
    pub fn new(
        debounce_millis: u64,
        max_dirty_millis: u64,
        mutation_threshold: u64,
    ) -> Result<Self, ProfileBookmarksSavePolicyError> {
        if debounce_millis == 0 {
            return Err(ProfileBookmarksSavePolicyError::ZeroDebounce);
        }
        if max_dirty_millis == 0 {
            return Err(ProfileBookmarksSavePolicyError::ZeroMaxDirtyAge);
        }
        if mutation_threshold == 0 {
            return Err(ProfileBookmarksSavePolicyError::ZeroMutationThreshold);
        }
        if max_dirty_millis < debounce_millis {
            return Err(ProfileBookmarksSavePolicyError::MaxDirtyAgeBeforeDebounce {
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
pub enum ProfileBookmarksSavePolicyError {
    ZeroDebounce,
    ZeroMaxDirtyAge,
    ZeroMutationThreshold,
    MaxDirtyAgeBeforeDebounce {
        debounce_millis: u64,
        max_dirty_millis: u64,
    },
}

impl fmt::Display for ProfileBookmarksSavePolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDebounce => formatter.write_str("bookmarks save debounce must be nonzero"),
            Self::ZeroMaxDirtyAge => {
                formatter.write_str("bookmarks save maximum dirty age must be nonzero")
            }
            Self::ZeroMutationThreshold => {
                formatter.write_str("bookmarks save mutation threshold must be nonzero")
            }
            Self::MaxDirtyAgeBeforeDebounce {
                debounce_millis,
                max_dirty_millis,
            } => write!(
                formatter,
                "bookmarks save maximum dirty age {max_dirty_millis}ms is shorter than debounce {debounce_millis}ms"
            ),
        }
    }
}

impl std::error::Error for ProfileBookmarksSavePolicyError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileBookmarksSaveUrgency {
    Normal,
    Flush,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ProfileBookmarksSaveScheduler {
    policy: ProfileBookmarksSavePolicy,
    profile: Option<ProfileId>,
    observed_mutation_revision: u64,
    first_dirty_millis: Option<u64>,
    last_mutation_millis: Option<u64>,
    last_now_millis: Option<u64>,
}

impl ProfileBookmarksSaveScheduler {
    pub const fn new(policy: ProfileBookmarksSavePolicy) -> Self {
        Self {
            policy,
            profile: None,
            observed_mutation_revision: 0,
            first_dirty_millis: None,
            last_mutation_millis: None,
            last_now_millis: None,
        }
    }

    pub const fn policy(&self) -> ProfileBookmarksSavePolicy {
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
        urgency: ProfileBookmarksSaveUrgency,
    ) -> Result<Option<ProfileBookmarksSaveIntent>, ProfileBookmarksRuntimeError> {
        let mutation_revision = runtime.bookmarks_mutation_revision(profile)?;
        let unsaved_mutations = runtime.bookmarks_unsaved_mutations(profile)?;
        let save_pending = runtime.pending_bookmarks_save().is_some();

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
        let should_save = urgency == ProfileBookmarksSaveUrgency::Flush
            || time_regressed
            || threshold_due
            || debounce_due
            || max_age_due;

        if !should_save {
            return Ok(None);
        }

        let intent = runtime.begin_bookmarks_save_if_dirty(profile)?;
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
    use crate::PreparedProfile;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(1);

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let id = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "zorya-bookmarks-save-scheduler-{}-{id}",
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
    ) -> ProfileBookmarksSavePolicy {
        ProfileBookmarksSavePolicy::new(debounce_millis, max_dirty_millis, mutation_threshold)
            .unwrap()
    }

    fn load_profile(runtime: &mut ProfileRuntime, root: &Path) -> ProfileId {
        let intent = runtime.begin_selection(root).unwrap().into_intent();
        let prepared = PreparedProfile::load(&intent).unwrap();
        runtime.commit_selection(prepared).unwrap().active_profile()
    }

    fn add(runtime: &mut ProfileRuntime, profile: ProfileId, suffix: u64) {
        runtime
            .add_bookmark(
                profile,
                format!("Bookmark {suffix}"),
                format!("https://example.test/{suffix}"),
            )
            .unwrap();
    }

    fn scheduled(
        scheduler: &mut ProfileBookmarksSaveScheduler,
        runtime: &mut ProfileRuntime,
        profile: ProfileId,
        now_millis: u64,
        urgency: ProfileBookmarksSaveUrgency,
    ) -> bool {
        scheduler
            .poll(runtime, profile, now_millis, urgency)
            .unwrap()
            .is_some()
    }

    #[test]
    fn policy_rejects_zero_and_inverted_bounds() {
        assert_eq!(
            ProfileBookmarksSavePolicy::new(0, 10, 1),
            Err(ProfileBookmarksSavePolicyError::ZeroDebounce)
        );
        assert_eq!(
            ProfileBookmarksSavePolicy::new(1, 0, 1),
            Err(ProfileBookmarksSavePolicyError::ZeroMaxDirtyAge)
        );
        assert_eq!(
            ProfileBookmarksSavePolicy::new(1, 10, 0),
            Err(ProfileBookmarksSavePolicyError::ZeroMutationThreshold)
        );
        assert_eq!(
            ProfileBookmarksSavePolicy::new(10, 9, 1),
            Err(ProfileBookmarksSavePolicyError::MaxDirtyAgeBeforeDebounce {
                debounce_millis: 10,
                max_dirty_millis: 9,
            })
        );
    }

    #[test]
    fn clean_bookmarks_never_schedule_even_for_flush() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut scheduler = ProfileBookmarksSaveScheduler::new(policy(10, 100, 10));

        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            0,
            ProfileBookmarksSaveUrgency::Normal,
        ));
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            1,
            ProfileBookmarksSaveUrgency::Flush,
        ));
    }

    #[test]
    fn debounce_restarts_after_new_mutation() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut scheduler = ProfileBookmarksSaveScheduler::new(policy(10, 100, 10));

        add(&mut runtime, profile, 1);
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            0,
            ProfileBookmarksSaveUrgency::Normal,
        ));
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            9,
            ProfileBookmarksSaveUrgency::Normal,
        ));

        add(&mut runtime, profile, 2);
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            9,
            ProfileBookmarksSaveUrgency::Normal,
        ));
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            18,
            ProfileBookmarksSaveUrgency::Normal,
        ));
        assert!(scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            19,
            ProfileBookmarksSaveUrgency::Normal,
        ));
    }

    #[test]
    fn next_save_deadline_tracks_debounce_and_max_dirty_age() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut scheduler = ProfileBookmarksSaveScheduler::new(policy(10, 20, 100));

        assert_eq!(scheduler.next_save_due_millis(), None);
        add(&mut runtime, profile, 1);
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            0,
            ProfileBookmarksSaveUrgency::Normal,
        ));
        assert_eq!(scheduler.next_save_due_millis(), Some(10));

        add(&mut runtime, profile, 2);
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            9,
            ProfileBookmarksSaveUrgency::Normal,
        ));
        assert_eq!(scheduler.next_save_due_millis(), Some(19));

        add(&mut runtime, profile, 3);
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            15,
            ProfileBookmarksSaveUrgency::Normal,
        ));
        assert_eq!(scheduler.next_save_due_millis(), Some(20));

        assert!(scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            20,
            ProfileBookmarksSaveUrgency::Normal,
        ));
        assert_eq!(scheduler.next_save_due_millis(), None);
    }

    #[test]
    fn threshold_and_flush_schedule_without_waiting_for_debounce() {
        let threshold_root = TempRoot::new();
        let mut threshold_runtime = ProfileRuntime::new();
        let threshold_profile = load_profile(&mut threshold_runtime, threshold_root.path());
        let mut threshold_scheduler = ProfileBookmarksSaveScheduler::new(policy(100, 1_000, 2));

        add(&mut threshold_runtime, threshold_profile, 1);
        assert!(!scheduled(
            &mut threshold_scheduler,
            &mut threshold_runtime,
            threshold_profile,
            0,
            ProfileBookmarksSaveUrgency::Normal,
        ));
        add(&mut threshold_runtime, threshold_profile, 2);
        assert!(scheduled(
            &mut threshold_scheduler,
            &mut threshold_runtime,
            threshold_profile,
            1,
            ProfileBookmarksSaveUrgency::Normal,
        ));

        let flush_root = TempRoot::new();
        let mut flush_runtime = ProfileRuntime::new();
        let flush_profile = load_profile(&mut flush_runtime, flush_root.path());
        let mut flush_scheduler = ProfileBookmarksSaveScheduler::new(policy(100, 1_000, 10));
        add(&mut flush_runtime, flush_profile, 1);
        assert!(scheduled(
            &mut flush_scheduler,
            &mut flush_runtime,
            flush_profile,
            0,
            ProfileBookmarksSaveUrgency::Flush,
        ));
    }

    #[test]
    fn maximum_dirty_age_bounds_continuous_mutation() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut scheduler = ProfileBookmarksSaveScheduler::new(policy(10, 20, 100));

        for now in [0, 5, 10, 15] {
            add(&mut runtime, profile, now + 1);
            assert!(!scheduled(
                &mut scheduler,
                &mut runtime,
                profile,
                now,
                ProfileBookmarksSaveUrgency::Normal,
            ));
        }
        add(&mut runtime, profile, 21);
        assert!(scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            20,
            ProfileBookmarksSaveUrgency::Normal,
        ));
    }

    #[test]
    fn time_regression_schedules_dirty_state_conservatively() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut scheduler = ProfileBookmarksSaveScheduler::new(policy(100, 1_000, 100));

        add(&mut runtime, profile, 1);
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            100,
            ProfileBookmarksSaveUrgency::Normal,
        ));
        assert!(scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            99,
            ProfileBookmarksSaveUrgency::Normal,
        ));
    }

    #[test]
    fn pending_save_suppresses_overlap_and_newer_mutation_remains_schedulable() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut scheduler = ProfileBookmarksSaveScheduler::new(policy(10, 100, 100));

        add(&mut runtime, profile, 1);
        let first = scheduler
            .poll(
                &mut runtime,
                profile,
                0,
                ProfileBookmarksSaveUrgency::Flush,
            )
            .unwrap()
            .unwrap();
        add(&mut runtime, profile, 2);
        assert!(
            scheduler
                .poll(
                    &mut runtime,
                    profile,
                    1,
                    ProfileBookmarksSaveUrgency::Flush,
                )
                .unwrap()
                .is_none()
        );

        runtime.complete_bookmarks_save(first.execute()).unwrap();
        assert_eq!(runtime.bookmarks_unsaved_mutations(profile).unwrap(), 1);
        assert!(
            scheduler
                .poll(
                    &mut runtime,
                    profile,
                    1,
                    ProfileBookmarksSaveUrgency::Flush,
                )
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn failed_save_reopens_dirty_window_for_retry() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let mut scheduler = ProfileBookmarksSaveScheduler::new(policy(10, 100, 100));

        add(&mut runtime, profile, 1);
        let save = scheduler
            .poll(
                &mut runtime,
                profile,
                0,
                ProfileBookmarksSaveUrgency::Flush,
            )
            .unwrap()
            .unwrap()
            .id();
        runtime.cancel_bookmarks_save(save).unwrap();

        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            1,
            ProfileBookmarksSaveUrgency::Normal,
        ));
        assert_eq!(scheduler.next_save_due_millis(), Some(11));
        assert!(scheduled(
            &mut scheduler,
            &mut runtime,
            profile,
            11,
            ProfileBookmarksSaveUrgency::Normal,
        ));
    }

    #[test]
    fn stale_profile_error_does_not_retarget_scheduler_state() {
        let first_root = TempRoot::new();
        let second_root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let first = load_profile(&mut runtime, first_root.path());
        let mut scheduler = ProfileBookmarksSaveScheduler::new(policy(10, 100, 100));
        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            first,
            0,
            ProfileBookmarksSaveUrgency::Normal,
        ));

        let selection = runtime
            .begin_selection(second_root.path())
            .unwrap()
            .into_intent();
        let prepared = PreparedProfile::load(&selection).unwrap();
        let replacement = runtime.commit_selection(prepared).unwrap();
        let second = replacement.active_profile();
        replacement
            .into_replaced_profile()
            .unwrap()
            .into_profile_lock()
            .release()
            .unwrap();

        assert!(matches!(
            scheduler.poll(
                &mut runtime,
                first,
                1,
                ProfileBookmarksSaveUrgency::Normal,
            ),
            Err(ProfileBookmarksRuntimeError::StaleProfile {
                expected: Some(expected),
                actual,
            }) if expected == second && actual == first
        ));
        assert_eq!(scheduler.tracked_profile(), Some(first));

        assert!(!scheduled(
            &mut scheduler,
            &mut runtime,
            second,
            2,
            ProfileBookmarksSaveUrgency::Normal,
        ));
        assert_eq!(scheduler.tracked_profile(), Some(second));
    }
}
