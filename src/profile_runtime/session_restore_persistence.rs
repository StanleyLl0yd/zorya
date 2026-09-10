use super::{ActiveProfile, ProfileId, ProfileRuntime};
use crate::profile_lock::ProfileLock;
use crate::session_restore::{
    SessionRestoreError, SessionRestoreRecovery, SessionRestoreSave, SessionRestoreSnapshot,
    SessionRestoreStore, SessionTab, SessionTabId, SessionWindow, SessionWindowId,
};
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProfileSessionRestoreSaveId(u64);

impl ProfileSessionRestoreSaveId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ProfileSessionRestoreSaveIntent {
    id: ProfileSessionRestoreSaveId,
    profile: ProfileId,
    root: PathBuf,
    lock: ProfileLock,
    snapshot: SessionRestoreSnapshot,
    mutation_revision: u64,
}

impl ProfileSessionRestoreSaveIntent {
    pub const fn id(&self) -> ProfileSessionRestoreSaveId {
        self.id
    }

    pub const fn profile(&self) -> ProfileId {
        self.profile
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub const fn snapshot(&self) -> &SessionRestoreSnapshot {
        &self.snapshot
    }

    pub const fn mutation_revision(&self) -> u64 {
        self.mutation_revision
    }

    pub fn execute(self) -> ProfileSessionRestoreSaveCompletion {
        let base_generation = self.snapshot.generation();
        let result = SessionRestoreStore::open(self.root.clone())
            .and_then(|store| store.save(&self.lock, &self.snapshot));
        ProfileSessionRestoreSaveCompletion {
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
pub struct ProfileSessionRestoreSaveCompletion {
    id: ProfileSessionRestoreSaveId,
    profile: ProfileId,
    root: PathBuf,
    base_generation: u64,
    mutation_revision: u64,
    result: Result<SessionRestoreSave, SessionRestoreError>,
}

impl ProfileSessionRestoreSaveCompletion {
    pub const fn id(&self) -> ProfileSessionRestoreSaveId {
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

    pub const fn result(&self) -> &Result<SessionRestoreSave, SessionRestoreError> {
        &self.result
    }

    pub fn into_result(self) -> Result<SessionRestoreSave, SessionRestoreError> {
        self.result
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PendingProfileSessionRestoreSave {
    id: ProfileSessionRestoreSaveId,
    profile: ProfileId,
    base_generation: u64,
    mutation_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SessionRestoreRuntimeState {
    snapshot: SessionRestoreSnapshot,
    recovery: Option<SessionRestoreRecovery>,
    mutation_revision: u64,
    durable_revision: u64,
}

impl SessionRestoreRuntimeState {
    pub(super) const fn new(
        snapshot: SessionRestoreSnapshot,
        recovery: Option<SessionRestoreRecovery>,
    ) -> Self {
        Self {
            snapshot,
            recovery,
            mutation_revision: 0,
            durable_revision: 0,
        }
    }

    pub(super) const fn snapshot(&self) -> &SessionRestoreSnapshot {
        &self.snapshot
    }

    pub(super) const fn recovery(&self) -> Option<&SessionRestoreRecovery> {
        self.recovery.as_ref()
    }

    pub(super) const fn is_dirty(&self) -> bool {
        self.mutation_revision != self.durable_revision
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileSessionRestoreRuntimeError {
    StaleProfile {
        expected: Option<ProfileId>,
        actual: ProfileId,
    },
    MutationRevisionExhausted,
    SnapshotGenerationMismatch {
        expected_generation: u64,
        provided_generation: u64,
    },
    SaveIdExhausted,
    SaveAlreadyPending {
        pending: ProfileSessionRestoreSaveId,
    },
    StaleSave {
        expected: Option<ProfileSessionRestoreSaveId>,
        actual: ProfileSessionRestoreSaveId,
    },
    SaveTargetMismatch {
        save: ProfileSessionRestoreSaveId,
    },
    SaveGenerationMismatch {
        expected_generation: u64,
        current_generation: u64,
        saved_generation: u64,
    },
    SessionRestore(SessionRestoreError),
}

impl fmt::Display for ProfileSessionRestoreRuntimeError {
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
                formatter.write_str("profile session-restore mutation revision space is exhausted")
            }
            Self::SnapshotGenerationMismatch {
                expected_generation,
                provided_generation,
            } => write!(
                formatter,
                "replacement session snapshot generation {provided_generation} does not match active generation {expected_generation}"
            ),
            Self::SaveIdExhausted => {
                formatter.write_str("profile session-restore save identifier space is exhausted")
            }
            Self::SaveAlreadyPending { pending } => write!(
                formatter,
                "profile session-restore save {} is already pending",
                pending.get()
            ),
            Self::StaleSave { expected, actual } => match expected {
                Some(expected) => write!(
                    formatter,
                    "profile session-restore save {} is stale; current save is {}",
                    actual.get(),
                    expected.get()
                ),
                None => write!(
                    formatter,
                    "profile session-restore save {} is stale; no save is pending",
                    actual.get()
                ),
            },
            Self::SaveTargetMismatch { save } => write!(
                formatter,
                "profile session-restore save {} does not match its active profile target",
                save.get()
            ),
            Self::SaveGenerationMismatch {
                expected_generation,
                current_generation,
                saved_generation,
            } => write!(
                formatter,
                "profile session-restore save expected in-memory generation {expected_generation}, found {current_generation}, saved {saved_generation}"
            ),
            Self::SessionRestore(error) => {
                write!(formatter, "session-restore update failed: {error}")
            }
        }
    }
}

impl std::error::Error for ProfileSessionRestoreRuntimeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SessionRestore(error) => Some(error),
            _ => None,
        }
    }
}

impl ProfileRuntime {
    fn active_session_restore_state(
        &self,
        profile: ProfileId,
    ) -> Result<&SessionRestoreRuntimeState, ProfileSessionRestoreRuntimeError> {
        let expected = self.active.as_ref().map(ActiveProfile::id);
        if expected != Some(profile) {
            return Err(ProfileSessionRestoreRuntimeError::StaleProfile {
                expected,
                actual: profile,
            });
        }
        Ok(&self
            .active
            .as_ref()
            .expect("active profile was validated")
            .session_restore)
    }

    fn active_session_restore_state_mut(
        &mut self,
        profile: ProfileId,
    ) -> Result<&mut SessionRestoreRuntimeState, ProfileSessionRestoreRuntimeError> {
        let expected = self.active.as_ref().map(ActiveProfile::id);
        if expected != Some(profile) {
            return Err(ProfileSessionRestoreRuntimeError::StaleProfile {
                expected,
                actual: profile,
            });
        }
        Ok(&mut self
            .active
            .as_mut()
            .expect("active profile was validated")
            .session_restore)
    }

    pub fn active_session_restore(
        &self,
        profile: ProfileId,
    ) -> Result<&SessionRestoreSnapshot, ProfileSessionRestoreRuntimeError> {
        Ok(self.active_session_restore_state(profile)?.snapshot())
    }

    pub(crate) fn replace_session_restore_snapshot(
        &mut self,
        profile: ProfileId,
        snapshot: SessionRestoreSnapshot,
    ) -> Result<bool, ProfileSessionRestoreRuntimeError> {
        let state = self.active_session_restore_state_mut(profile)?;
        if snapshot.generation() != state.snapshot.generation() {
            return Err(
                ProfileSessionRestoreRuntimeError::SnapshotGenerationMismatch {
                    expected_generation: state.snapshot.generation(),
                    provided_generation: snapshot.generation(),
                },
            );
        }
        if snapshot == state.snapshot {
            return Ok(false);
        }
        let next_revision = state
            .mutation_revision
            .checked_add(1)
            .ok_or(ProfileSessionRestoreRuntimeError::MutationRevisionExhausted)?;
        state.snapshot = snapshot;
        state.mutation_revision = next_revision;
        Ok(true)
    }

    pub fn session_restore_unsaved_mutations(
        &self,
        profile: ProfileId,
    ) -> Result<u64, ProfileSessionRestoreRuntimeError> {
        let state = self.active_session_restore_state(profile)?;
        Ok(state
            .mutation_revision
            .checked_sub(state.durable_revision)
            .expect("durable session-restore revision cannot exceed current revision"))
    }

    pub fn session_restore_is_dirty(
        &self,
        profile: ProfileId,
    ) -> Result<bool, ProfileSessionRestoreRuntimeError> {
        Ok(self.session_restore_unsaved_mutations(profile)? != 0)
    }

    pub fn session_restore_mutation_revision(
        &self,
        profile: ProfileId,
    ) -> Result<u64, ProfileSessionRestoreRuntimeError> {
        Ok(self
            .active_session_restore_state(profile)?
            .mutation_revision)
    }

    pub const fn pending_session_restore_save(&self) -> Option<ProfileSessionRestoreSaveId> {
        match self.pending_session_restore_save {
            Some(pending) => Some(pending.id),
            None => None,
        }
    }

    pub fn add_session_window(
        &mut self,
        profile: ProfileId,
    ) -> Result<SessionWindowId, ProfileSessionRestoreRuntimeError> {
        let state = self.active_session_restore_state_mut(profile)?;
        let next_revision = state
            .mutation_revision
            .checked_add(1)
            .ok_or(ProfileSessionRestoreRuntimeError::MutationRevisionExhausted)?;
        let window = state
            .snapshot
            .add_window()
            .map_err(ProfileSessionRestoreRuntimeError::SessionRestore)?;
        state.mutation_revision = next_revision;
        Ok(window)
    }

    pub fn remove_session_window(
        &mut self,
        profile: ProfileId,
        window: SessionWindowId,
    ) -> Result<Option<SessionWindow>, ProfileSessionRestoreRuntimeError> {
        let state = self.active_session_restore_state_mut(profile)?;
        if state.snapshot.window(window).is_none() {
            return Ok(None);
        }
        let next_revision = state
            .mutation_revision
            .checked_add(1)
            .ok_or(ProfileSessionRestoreRuntimeError::MutationRevisionExhausted)?;
        let removed = state.snapshot.remove_window(window);
        debug_assert!(removed.is_some(), "session window existence was validated");
        if removed.is_some() {
            state.mutation_revision = next_revision;
        }
        Ok(removed)
    }

    pub fn add_session_tab(
        &mut self,
        profile: ProfileId,
        window: SessionWindowId,
        location: impl Into<String>,
    ) -> Result<SessionTabId, ProfileSessionRestoreRuntimeError> {
        let state = self.active_session_restore_state_mut(profile)?;
        let next_revision = state
            .mutation_revision
            .checked_add(1)
            .ok_or(ProfileSessionRestoreRuntimeError::MutationRevisionExhausted)?;
        let tab = state
            .snapshot
            .add_tab(window, location)
            .map_err(ProfileSessionRestoreRuntimeError::SessionRestore)?;
        state.mutation_revision = next_revision;
        Ok(tab)
    }

    pub fn update_session_tab_location(
        &mut self,
        profile: ProfileId,
        window: SessionWindowId,
        tab: SessionTabId,
        location: impl Into<String>,
    ) -> Result<bool, ProfileSessionRestoreRuntimeError> {
        let location = location.into();
        let state = self.active_session_restore_state_mut(profile)?;
        let target = state.snapshot.window(window).ok_or({
            ProfileSessionRestoreRuntimeError::SessionRestore(SessionRestoreError::WindowNotFound {
                window,
            })
        })?;
        let target_tab = target
            .tabs()
            .iter()
            .find(|candidate| candidate.id() == tab)
            .ok_or({
                ProfileSessionRestoreRuntimeError::SessionRestore(
                    SessionRestoreError::TabNotFound { window, tab },
                )
            })?;
        if target_tab.location() == location {
            return Ok(false);
        }
        let next_revision = state
            .mutation_revision
            .checked_add(1)
            .ok_or(ProfileSessionRestoreRuntimeError::MutationRevisionExhausted)?;
        let changed = state
            .snapshot
            .update_tab_location(window, tab, location)
            .map_err(ProfileSessionRestoreRuntimeError::SessionRestore)?;
        debug_assert!(changed, "session tab location difference was validated");
        if changed {
            state.mutation_revision = next_revision;
        }
        Ok(changed)
    }

    pub fn set_active_session_tab(
        &mut self,
        profile: ProfileId,
        window: SessionWindowId,
        tab: SessionTabId,
    ) -> Result<bool, ProfileSessionRestoreRuntimeError> {
        let state = self.active_session_restore_state_mut(profile)?;
        let target = state.snapshot.window(window).ok_or({
            ProfileSessionRestoreRuntimeError::SessionRestore(SessionRestoreError::WindowNotFound {
                window,
            })
        })?;
        if !target.tabs().iter().any(|candidate| candidate.id() == tab) {
            return Err(ProfileSessionRestoreRuntimeError::SessionRestore(
                SessionRestoreError::TabNotFound { window, tab },
            ));
        }
        if target.active_tab() == Some(tab) {
            return Ok(false);
        }
        let next_revision = state
            .mutation_revision
            .checked_add(1)
            .ok_or(ProfileSessionRestoreRuntimeError::MutationRevisionExhausted)?;
        let changed = state
            .snapshot
            .set_active_tab(window, tab)
            .map_err(ProfileSessionRestoreRuntimeError::SessionRestore)?;
        debug_assert!(changed, "active session tab difference was validated");
        if changed {
            state.mutation_revision = next_revision;
        }
        Ok(changed)
    }

    pub fn remove_session_tab(
        &mut self,
        profile: ProfileId,
        window: SessionWindowId,
        tab: SessionTabId,
    ) -> Result<Option<SessionTab>, ProfileSessionRestoreRuntimeError> {
        let state = self.active_session_restore_state_mut(profile)?;
        let target = state.snapshot.window(window).ok_or({
            ProfileSessionRestoreRuntimeError::SessionRestore(SessionRestoreError::WindowNotFound {
                window,
            })
        })?;
        if !target.tabs().iter().any(|candidate| candidate.id() == tab) {
            return Ok(None);
        }
        let next_revision = state
            .mutation_revision
            .checked_add(1)
            .ok_or(ProfileSessionRestoreRuntimeError::MutationRevisionExhausted)?;
        let removed = state
            .snapshot
            .remove_tab(window, tab)
            .map_err(ProfileSessionRestoreRuntimeError::SessionRestore)?;
        debug_assert!(removed.is_some(), "session tab existence was validated");
        if removed.is_some() {
            state.mutation_revision = next_revision;
        }
        Ok(removed)
    }

    pub fn begin_session_restore_save_if_dirty(
        &mut self,
        profile: ProfileId,
    ) -> Result<Option<ProfileSessionRestoreSaveIntent>, ProfileSessionRestoreRuntimeError> {
        if !self.session_restore_is_dirty(profile)? {
            return Ok(None);
        }
        self.begin_session_restore_save(profile).map(Some)
    }

    pub fn begin_session_restore_save(
        &mut self,
        profile: ProfileId,
    ) -> Result<ProfileSessionRestoreSaveIntent, ProfileSessionRestoreRuntimeError> {
        let expected = self.active.as_ref().map(ActiveProfile::id);
        if expected != Some(profile) {
            return Err(ProfileSessionRestoreRuntimeError::StaleProfile {
                expected,
                actual: profile,
            });
        }
        if let Some(pending) = self.pending_session_restore_save {
            return Err(ProfileSessionRestoreRuntimeError::SaveAlreadyPending {
                pending: pending.id,
            });
        }

        let active = self.active.as_ref().expect("active profile was validated");
        let root = active.root.clone();
        let lock = active.lock.clone();
        let snapshot = active.session_restore.snapshot.clone();
        let base_generation = snapshot.generation();
        let mutation_revision = active.session_restore.mutation_revision;
        let id = ProfileSessionRestoreSaveId(self.next_session_restore_save_id);
        self.next_session_restore_save_id = self
            .next_session_restore_save_id
            .checked_add(1)
            .ok_or(ProfileSessionRestoreRuntimeError::SaveIdExhausted)?;
        self.pending_session_restore_save = Some(PendingProfileSessionRestoreSave {
            id,
            profile,
            base_generation,
            mutation_revision,
        });
        Ok(ProfileSessionRestoreSaveIntent {
            id,
            profile,
            root,
            lock,
            snapshot,
            mutation_revision,
        })
    }

    pub fn cancel_session_restore_save(
        &mut self,
        save: ProfileSessionRestoreSaveId,
    ) -> Result<(), ProfileSessionRestoreRuntimeError> {
        let expected = self.pending_session_restore_save.map(|pending| pending.id);
        if expected != Some(save) {
            return Err(ProfileSessionRestoreRuntimeError::StaleSave {
                expected,
                actual: save,
            });
        }
        self.pending_session_restore_save = None;
        Ok(())
    }

    pub fn complete_session_restore_save(
        &mut self,
        completion: ProfileSessionRestoreSaveCompletion,
    ) -> Result<ProfileSessionRestoreSaveCompletion, ProfileSessionRestoreRuntimeError> {
        let expected = self.pending_session_restore_save.map(|pending| pending.id);
        if expected != Some(completion.id) {
            return Err(ProfileSessionRestoreRuntimeError::StaleSave {
                expected,
                actual: completion.id,
            });
        }

        let pending = self
            .pending_session_restore_save
            .expect("pending session-restore save was validated");
        let active =
            self.active
                .as_ref()
                .ok_or(ProfileSessionRestoreRuntimeError::SaveTargetMismatch {
                    save: completion.id,
                })?;
        if pending.profile != completion.profile
            || pending.base_generation != completion.base_generation
            || pending.mutation_revision != completion.mutation_revision
            || active.id != completion.profile
            || active.root != completion.root
        {
            return Err(ProfileSessionRestoreRuntimeError::SaveTargetMismatch {
                save: completion.id,
            });
        }

        self.pending_session_restore_save = None;
        if let Ok(saved) = &completion.result {
            let active = self
                .active
                .as_mut()
                .expect("session-restore save target was validated");
            let current_generation = active.session_restore.snapshot.generation();
            let saved_generation = saved.snapshot().generation();
            if !active
                .session_restore
                .snapshot
                .advance_generation_after_save(pending.base_generation, saved_generation)
            {
                return Err(ProfileSessionRestoreRuntimeError::SaveGenerationMismatch {
                    expected_generation: pending.base_generation,
                    current_generation,
                    saved_generation,
                });
            }
            active.session_restore.durable_revision = pending.mutation_revision;
        }
        Ok(completion)
    }
}
