use super::{ActiveProfile, ProfileId, ProfileRuntime};
use crate::bookmarks::{
    Bookmark, BookmarkId, BookmarksError, BookmarksRecovery, BookmarksSave, BookmarksSnapshot,
    BookmarksStore,
};
use crate::profile_lock::ProfileLock;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileBookmarksSaveId(u64);

impl ProfileBookmarksSaveId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ProfileBookmarksSaveIntent {
    id: ProfileBookmarksSaveId,
    profile: ProfileId,
    root: PathBuf,
    lock: ProfileLock,
    snapshot: BookmarksSnapshot,
    mutation_revision: u64,
}

impl ProfileBookmarksSaveIntent {
    pub const fn id(&self) -> ProfileBookmarksSaveId {
        self.id
    }

    pub const fn profile(&self) -> ProfileId {
        self.profile
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub const fn snapshot(&self) -> &BookmarksSnapshot {
        &self.snapshot
    }

    pub const fn mutation_revision(&self) -> u64 {
        self.mutation_revision
    }

    pub fn execute(self) -> ProfileBookmarksSaveCompletion {
        let base_generation = self.snapshot.generation();
        let result = BookmarksStore::open(self.root.clone())
            .and_then(|store| store.save(&self.lock, &self.snapshot));
        ProfileBookmarksSaveCompletion {
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
pub struct ProfileBookmarksSaveCompletion {
    id: ProfileBookmarksSaveId,
    profile: ProfileId,
    root: PathBuf,
    base_generation: u64,
    mutation_revision: u64,
    result: Result<BookmarksSave, BookmarksError>,
}

impl ProfileBookmarksSaveCompletion {
    pub const fn id(&self) -> ProfileBookmarksSaveId {
        self.id
    }

    pub const fn profile(&self) -> ProfileId {
        self.profile
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub const fn base_generation(&self) -> u64 {
        self.base_generation
    }

    pub const fn mutation_revision(&self) -> u64 {
        self.mutation_revision
    }

    pub const fn result(&self) -> &Result<BookmarksSave, BookmarksError> {
        &self.result
    }

    pub fn into_result(self) -> Result<BookmarksSave, BookmarksError> {
        self.result
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PendingProfileBookmarksSave {
    id: ProfileBookmarksSaveId,
    profile: ProfileId,
    base_generation: u64,
    mutation_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BookmarksRuntimeState {
    snapshot: BookmarksSnapshot,
    recovery: Option<BookmarksRecovery>,
    mutation_revision: u64,
    durable_revision: u64,
}

impl BookmarksRuntimeState {
    pub(super) const fn new(
        snapshot: BookmarksSnapshot,
        recovery: Option<BookmarksRecovery>,
    ) -> Self {
        Self {
            snapshot,
            recovery,
            mutation_revision: 0,
            durable_revision: 0,
        }
    }

    pub(super) const fn snapshot(&self) -> &BookmarksSnapshot {
        &self.snapshot
    }

    pub(super) const fn recovery(&self) -> Option<&BookmarksRecovery> {
        self.recovery.as_ref()
    }

    pub(super) const fn is_dirty(&self) -> bool {
        self.mutation_revision != self.durable_revision
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileBookmarksRuntimeError {
    StaleProfile {
        expected: Option<ProfileId>,
        actual: ProfileId,
    },
    MutationRevisionExhausted,
    SaveIdExhausted,
    SaveAlreadyPending {
        pending: ProfileBookmarksSaveId,
    },
    StaleSave {
        expected: Option<ProfileBookmarksSaveId>,
        actual: ProfileBookmarksSaveId,
    },
    SaveTargetMismatch {
        save: ProfileBookmarksSaveId,
    },
    SaveGenerationMismatch {
        expected_generation: u64,
        current_generation: u64,
        saved_generation: u64,
    },
    Bookmarks(BookmarksError),
}

impl fmt::Display for ProfileBookmarksRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
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
            Self::MutationRevisionExhausted => {
                formatter.write_str("profile bookmarks mutation revision space is exhausted")
            }
            Self::SaveIdExhausted => {
                formatter.write_str("profile bookmarks save identifier space is exhausted")
            }
            Self::SaveAlreadyPending { pending } => write!(
                formatter,
                "profile bookmarks save {} is already pending",
                pending.get()
            ),
            Self::StaleSave { expected, actual } => match expected {
                Some(expected) => write!(
                    formatter,
                    "profile bookmarks save {} is stale; current save is {}",
                    actual.get(),
                    expected.get()
                ),
                None => write!(
                    formatter,
                    "profile bookmarks save {} is stale; no save is pending",
                    actual.get()
                ),
            },
            Self::SaveTargetMismatch { save } => write!(
                formatter,
                "profile bookmarks save {} does not match its active profile target",
                save.get()
            ),
            Self::SaveGenerationMismatch {
                expected_generation,
                current_generation,
                saved_generation,
            } => write!(
                formatter,
                "profile bookmarks save expected in-memory generation {expected_generation}, found {current_generation}, saved {saved_generation}"
            ),
            Self::Bookmarks(error) => write!(formatter, "bookmarks update failed: {error}"),
        }
    }
}

impl std::error::Error for ProfileBookmarksRuntimeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Bookmarks(error) => Some(error),
            _ => None,
        }
    }
}

impl ProfileRuntime {
    fn active_bookmarks_state(
        &self,
        profile: ProfileId,
    ) -> Result<&BookmarksRuntimeState, ProfileBookmarksRuntimeError> {
        let expected = self.active.as_ref().map(ActiveProfile::id);
        if expected != Some(profile) {
            return Err(ProfileBookmarksRuntimeError::StaleProfile {
                expected,
                actual: profile,
            });
        }
        Ok(&self
            .active
            .as_ref()
            .expect("active profile was validated")
            .bookmarks)
    }

    fn active_bookmarks_state_mut(
        &mut self,
        profile: ProfileId,
    ) -> Result<&mut BookmarksRuntimeState, ProfileBookmarksRuntimeError> {
        let expected = self.active.as_ref().map(ActiveProfile::id);
        if expected != Some(profile) {
            return Err(ProfileBookmarksRuntimeError::StaleProfile {
                expected,
                actual: profile,
            });
        }
        Ok(&mut self
            .active
            .as_mut()
            .expect("active profile was validated")
            .bookmarks)
    }

    pub fn active_bookmarks(
        &self,
        profile: ProfileId,
    ) -> Result<&BookmarksSnapshot, ProfileBookmarksRuntimeError> {
        Ok(self.active_bookmarks_state(profile)?.snapshot())
    }

    pub fn bookmarks_unsaved_mutations(
        &self,
        profile: ProfileId,
    ) -> Result<u64, ProfileBookmarksRuntimeError> {
        let state = self.active_bookmarks_state(profile)?;
        Ok(state
            .mutation_revision
            .checked_sub(state.durable_revision)
            .expect("durable bookmarks revision cannot exceed current revision"))
    }

    pub fn bookmarks_is_dirty(
        &self,
        profile: ProfileId,
    ) -> Result<bool, ProfileBookmarksRuntimeError> {
        Ok(self.bookmarks_unsaved_mutations(profile)? != 0)
    }

    pub fn bookmarks_mutation_revision(
        &self,
        profile: ProfileId,
    ) -> Result<u64, ProfileBookmarksRuntimeError> {
        Ok(self.active_bookmarks_state(profile)?.mutation_revision)
    }

    pub const fn pending_bookmarks_save(&self) -> Option<ProfileBookmarksSaveId> {
        match self.pending_bookmarks_save {
            Some(pending) => Some(pending.id),
            None => None,
        }
    }
    pub fn add_bookmark(
        &mut self,
        profile: ProfileId,
        title: impl Into<String>,
        location: impl Into<String>,
    ) -> Result<BookmarkId, ProfileBookmarksRuntimeError> {
        let state = self.active_bookmarks_state_mut(profile)?;
        let next_revision = state
            .mutation_revision
            .checked_add(1)
            .ok_or(ProfileBookmarksRuntimeError::MutationRevisionExhausted)?;
        let id = state
            .snapshot
            .add_bookmark(title, location)
            .map_err(ProfileBookmarksRuntimeError::Bookmarks)?;
        state.mutation_revision = next_revision;
        Ok(id)
    }

    pub fn update_bookmark(
        &mut self,
        profile: ProfileId,
        bookmark: BookmarkId,
        title: impl Into<String>,
        location: impl Into<String>,
    ) -> Result<bool, ProfileBookmarksRuntimeError> {
        let state = self.active_bookmarks_state_mut(profile)?;
        if state.snapshot.bookmark(bookmark).is_none() {
            return Ok(false);
        }
        let next_revision = state
            .mutation_revision
            .checked_add(1)
            .ok_or(ProfileBookmarksRuntimeError::MutationRevisionExhausted)?;
        let changed = state
            .snapshot
            .update_bookmark(bookmark, title, location)
            .map_err(ProfileBookmarksRuntimeError::Bookmarks)?;
        debug_assert!(changed, "bookmark existence was validated before mutation");
        if changed {
            state.mutation_revision = next_revision;
        }
        Ok(changed)
    }

    pub fn remove_bookmark(
        &mut self,
        profile: ProfileId,
        bookmark: BookmarkId,
    ) -> Result<Option<Bookmark>, ProfileBookmarksRuntimeError> {
        let state = self.active_bookmarks_state_mut(profile)?;
        if state.snapshot.bookmark(bookmark).is_none() {
            return Ok(None);
        }
        let next_revision = state
            .mutation_revision
            .checked_add(1)
            .ok_or(ProfileBookmarksRuntimeError::MutationRevisionExhausted)?;
        let removed = state.snapshot.remove_bookmark(bookmark);
        debug_assert!(
            removed.is_some(),
            "bookmark existence was validated before removal"
        );
        if removed.is_some() {
            state.mutation_revision = next_revision;
        }
        Ok(removed)
    }

    pub fn clear_bookmarks(
        &mut self,
        profile: ProfileId,
    ) -> Result<bool, ProfileBookmarksRuntimeError> {
        let state = self.active_bookmarks_state_mut(profile)?;
        if state.snapshot.is_empty() {
            return Ok(false);
        }
        let next_revision = state
            .mutation_revision
            .checked_add(1)
            .ok_or(ProfileBookmarksRuntimeError::MutationRevisionExhausted)?;
        state.snapshot.clear();
        state.mutation_revision = next_revision;
        Ok(true)
    }

    pub fn begin_bookmarks_save_if_dirty(
        &mut self,
        profile: ProfileId,
    ) -> Result<Option<ProfileBookmarksSaveIntent>, ProfileBookmarksRuntimeError> {
        if !self.bookmarks_is_dirty(profile)? {
            return Ok(None);
        }
        self.begin_bookmarks_save(profile).map(Some)
    }

    pub fn begin_bookmarks_save(
        &mut self,
        profile: ProfileId,
    ) -> Result<ProfileBookmarksSaveIntent, ProfileBookmarksRuntimeError> {
        let expected = self.active.as_ref().map(ActiveProfile::id);
        if expected != Some(profile) {
            return Err(ProfileBookmarksRuntimeError::StaleProfile {
                expected,
                actual: profile,
            });
        }
        if let Some(pending) = self.pending_bookmarks_save {
            return Err(ProfileBookmarksRuntimeError::SaveAlreadyPending {
                pending: pending.id,
            });
        }

        let active = self.active.as_ref().expect("active profile was validated");
        let root = active.root.clone();
        let lock = active.lock.clone();
        let snapshot = active.bookmarks.snapshot.clone();
        let base_generation = snapshot.generation();
        let mutation_revision = active.bookmarks.mutation_revision;
        let id = ProfileBookmarksSaveId(self.next_bookmarks_save_id);
        self.next_bookmarks_save_id = self
            .next_bookmarks_save_id
            .checked_add(1)
            .ok_or(ProfileBookmarksRuntimeError::SaveIdExhausted)?;
        self.pending_bookmarks_save = Some(PendingProfileBookmarksSave {
            id,
            profile,
            base_generation,
            mutation_revision,
        });
        Ok(ProfileBookmarksSaveIntent {
            id,
            profile,
            root,
            lock,
            snapshot,
            mutation_revision,
        })
    }
    pub fn cancel_bookmarks_save(
        &mut self,
        save: ProfileBookmarksSaveId,
    ) -> Result<(), ProfileBookmarksRuntimeError> {
        let expected = self.pending_bookmarks_save.map(|pending| pending.id);
        if expected != Some(save) {
            return Err(ProfileBookmarksRuntimeError::StaleSave {
                expected,
                actual: save,
            });
        }
        self.pending_bookmarks_save = None;
        Ok(())
    }
    pub fn complete_bookmarks_save(
        &mut self,
        completion: ProfileBookmarksSaveCompletion,
    ) -> Result<ProfileBookmarksSaveCompletion, ProfileBookmarksRuntimeError> {
        let expected = self.pending_bookmarks_save.map(|pending| pending.id);
        if expected != Some(completion.id) {
            return Err(ProfileBookmarksRuntimeError::StaleSave {
                expected,
                actual: completion.id,
            });
        }

        let pending = self
            .pending_bookmarks_save
            .expect("pending bookmarks save was validated");
        let active =
            self.active
                .as_ref()
                .ok_or(ProfileBookmarksRuntimeError::SaveTargetMismatch {
                    save: completion.id,
                })?;
        if pending.profile != completion.profile
            || pending.base_generation != completion.base_generation
            || pending.mutation_revision != completion.mutation_revision
            || active.id != completion.profile
            || active.root != completion.root
        {
            return Err(ProfileBookmarksRuntimeError::SaveTargetMismatch {
                save: completion.id,
            });
        }

        self.pending_bookmarks_save = None;
        if let Ok(saved) = &completion.result {
            let active = self
                .active
                .as_mut()
                .expect("bookmarks save target was validated");
            let current_generation = active.bookmarks.snapshot.generation();
            let saved_generation = saved.snapshot().generation();
            if !active
                .bookmarks
                .snapshot
                .advance_generation_after_save(pending.base_generation, saved_generation)
            {
                return Err(ProfileBookmarksRuntimeError::SaveGenerationMismatch {
                    expected_generation: pending.base_generation,
                    current_generation,
                    saved_generation,
                });
            }
            active.bookmarks.durable_revision = pending.mutation_revision;
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
            let root = std::env::temp_dir().join(format!(
                "zorya-bookmarks-runtime-{}-{id}",
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

    fn load_profile(runtime: &mut ProfileRuntime, root: &Path) -> ProfileId {
        let intent = runtime.begin_selection(root).unwrap().into_intent();
        let prepared = super::super::PreparedProfile::load(&intent).unwrap();
        runtime.commit_selection(prepared).unwrap().active_profile()
    }

    #[test]
    fn loaded_bookmarks_start_clean_and_successful_mutation_marks_dirty() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());

        assert!(!runtime.bookmarks_is_dirty(profile).unwrap());
        assert_eq!(runtime.bookmarks_unsaved_mutations(profile).unwrap(), 0);
        assert!(
            runtime
                .begin_bookmarks_save_if_dirty(profile)
                .unwrap()
                .is_none()
        );

        let bookmark = runtime
            .add_bookmark(profile, "Example", "https://example.test/")
            .unwrap();
        assert_eq!(bookmark.get(), 1);
        assert!(runtime.bookmarks_is_dirty(profile).unwrap());
        assert_eq!(runtime.bookmarks_unsaved_mutations(profile).unwrap(), 1);
        assert_eq!(runtime.bookmarks_mutation_revision(profile).unwrap(), 1);
    }

    #[test]
    fn failed_or_noop_mutations_do_not_mark_bookmarks_dirty() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());

        let oversized = "x".repeat(crate::MAX_BOOKMARK_TITLE_BYTES + 1);
        assert!(matches!(
            runtime.add_bookmark(profile, oversized, "https://example.test/"),
            Err(ProfileBookmarksRuntimeError::Bookmarks(
                BookmarksError::TitleTooLarge { .. }
            ))
        ));
        assert!(!runtime.bookmarks_is_dirty(profile).unwrap());

        assert!(!runtime.clear_bookmarks(profile).unwrap());
        assert_eq!(runtime.bookmarks_unsaved_mutations(profile).unwrap(), 0);
    }

    #[test]
    fn successful_save_cleans_only_captured_bookmark_mutations() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        runtime
            .add_bookmark(profile, "First", "https://example.test/first")
            .unwrap();
        let first = runtime
            .begin_bookmarks_save_if_dirty(profile)
            .unwrap()
            .unwrap();
        assert_eq!(first.mutation_revision(), 1);

        runtime
            .add_bookmark(profile, "Second", "https://example.test/second")
            .unwrap();
        assert_eq!(runtime.bookmarks_unsaved_mutations(profile).unwrap(), 2);

        let completion = first.execute();
        assert!(completion.result().is_ok());
        runtime.complete_bookmarks_save(completion).unwrap();
        assert_eq!(runtime.active_bookmarks(profile).unwrap().generation(), 1);
        assert_eq!(runtime.active_bookmarks(profile).unwrap().len(), 2);
        assert!(runtime.bookmarks_is_dirty(profile).unwrap());
        assert_eq!(runtime.bookmarks_unsaved_mutations(profile).unwrap(), 1);

        let completion = runtime
            .begin_bookmarks_save_if_dirty(profile)
            .unwrap()
            .unwrap()
            .execute();
        runtime.complete_bookmarks_save(completion).unwrap();
        assert_eq!(runtime.active_bookmarks(profile).unwrap().generation(), 2);
        assert!(!runtime.bookmarks_is_dirty(profile).unwrap());

        let persisted = BookmarksStore::open(root.path())
            .unwrap()
            .load()
            .unwrap()
            .into_snapshot();
        assert_eq!(persisted.generation(), 2);
        assert_eq!(persisted.len(), 2);
    }

    #[test]
    fn concurrent_bookmarks_writer_failure_preserves_runtime_generation_and_dirty_state() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        runtime
            .add_bookmark(profile, "Runtime", "https://example.test/runtime")
            .unwrap();
        let intent = runtime.begin_bookmarks_save(profile).unwrap();

        let store = BookmarksStore::open(root.path()).unwrap();
        let external = store.load().unwrap().into_snapshot();
        let lock = runtime.active.as_ref().unwrap().lock.clone();
        store.save(&lock, &external).unwrap();

        let completion = intent.execute();
        assert!(matches!(
            completion.result(),
            Err(BookmarksError::StaleGeneration {
                current: 1,
                provided: 0,
            })
        ));
        runtime.complete_bookmarks_save(completion).unwrap();
        assert_eq!(runtime.active_bookmarks(profile).unwrap().generation(), 0);
        assert!(runtime.pending_bookmarks_save().is_none());
        assert!(runtime.bookmarks_is_dirty(profile).unwrap());
        assert_eq!(runtime.bookmarks_unsaved_mutations(profile).unwrap(), 1);
    }

    #[test]
    fn overlapping_and_stale_bookmarks_save_cancellation_are_rejected() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        runtime
            .add_bookmark(profile, "Example", "https://example.test/")
            .unwrap();
        let first = runtime.begin_bookmarks_save(profile).unwrap();
        let first_id = first.id();

        assert_eq!(
            runtime.begin_bookmarks_save(profile),
            Err(ProfileBookmarksRuntimeError::SaveAlreadyPending { pending: first_id })
        );
        runtime.cancel_bookmarks_save(first_id).unwrap();
        assert!(runtime.bookmarks_is_dirty(profile).unwrap());
        let current = runtime.begin_bookmarks_save(profile).unwrap().id();
        assert_ne!(current, first_id);
        assert_eq!(
            runtime.cancel_bookmarks_save(first_id),
            Err(ProfileBookmarksRuntimeError::StaleSave {
                expected: Some(current),
                actual: first_id,
            })
        );
        assert_eq!(runtime.pending_bookmarks_save(), Some(current));
    }

    #[test]
    fn stale_profile_cannot_mutate_or_read_replacement_bookmarks() {
        let first_root = TempRoot::new();
        let second_root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let first = load_profile(&mut runtime, first_root.path());
        let selection = runtime
            .begin_selection(second_root.path())
            .unwrap()
            .into_intent();
        let prepared = super::super::PreparedProfile::load(&selection).unwrap();
        let replacement = runtime.commit_selection(prepared).unwrap();
        let second = replacement.active_profile();
        replacement
            .into_replaced_profile()
            .unwrap()
            .into_profile_lock()
            .release()
            .unwrap();

        assert!(matches!(
            runtime.active_bookmarks(first),
            Err(ProfileBookmarksRuntimeError::StaleProfile {
                expected: Some(expected),
                actual,
            }) if expected == second && actual == first
        ));
        assert!(matches!(
            runtime.add_bookmark(first, "Stale", "https://example.test/stale"),
            Err(ProfileBookmarksRuntimeError::StaleProfile {
                expected: Some(expected),
                actual,
            }) if expected == second && actual == first
        ));
        assert!(runtime.active_bookmarks(second).unwrap().is_empty());
    }

    #[test]
    fn profile_replacement_waits_for_dirty_and_pending_bookmarks() {
        let first_root = TempRoot::new();
        let second_root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let first = load_profile(&mut runtime, first_root.path());
        runtime
            .add_bookmark(first, "Old", "https://example.test/old")
            .unwrap();

        let selection = runtime
            .begin_selection(second_root.path())
            .unwrap()
            .into_intent();
        let prepared = super::super::PreparedProfile::load(&selection).unwrap();
        let rejection = runtime.commit_selection(prepared).unwrap_err();
        assert_eq!(
            rejection.error(),
            &super::super::ProfileRuntimeError::ActiveProfileNotDurable { profile: first }
        );
        let prepared = rejection.into_parts().1;

        let intent = runtime.begin_bookmarks_save(first).unwrap();
        let save = intent.id();
        let rejection = runtime.commit_selection(prepared).unwrap_err();
        assert_eq!(runtime.pending_bookmarks_save(), Some(save));
        let prepared = rejection.into_parts().1;

        runtime.complete_bookmarks_save(intent.execute()).unwrap();
        assert!(!runtime.bookmarks_is_dirty(first).unwrap());
        let replacement = runtime.commit_selection(prepared).unwrap();
        replacement
            .into_replaced_profile()
            .unwrap()
            .into_profile_lock()
            .release()
            .unwrap();
    }

    #[test]
    fn bookmark_revision_and_save_id_exhaustion_are_failure_atomic() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        runtime.active.as_mut().unwrap().bookmarks.mutation_revision = u64::MAX;

        assert_eq!(
            runtime.add_bookmark(profile, "Nope", "https://example.test/nope"),
            Err(ProfileBookmarksRuntimeError::MutationRevisionExhausted)
        );
        assert!(runtime.active_bookmarks(profile).unwrap().is_empty());

        runtime.active.as_mut().unwrap().bookmarks.mutation_revision = 0;
        runtime
            .add_bookmark(profile, "Example", "https://example.test/")
            .unwrap();
        runtime.next_bookmarks_save_id = u64::MAX;
        assert_eq!(
            runtime.begin_bookmarks_save(profile),
            Err(ProfileBookmarksRuntimeError::SaveIdExhausted)
        );
        assert!(runtime.pending_bookmarks_save().is_none());
        assert!(runtime.bookmarks_is_dirty(profile).unwrap());
    }
}
