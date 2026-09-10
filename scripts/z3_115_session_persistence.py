from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    target = Path(path)
    text = target.read_text(encoding="utf-8")
    if text.count(old) != 1:
        raise SystemExit(f"expected exactly one match in {path}: {old[:120]!r}")
    target.write_text(text.replace(old, new, 1), encoding="utf-8")


# Session snapshot: allow a captured save to advance only the storage generation
# of the still-current in-memory snapshot, preserving newer mutations.
replace_once(
    "src/session_restore.rs",
    "        Ok(Some(removed))\n    }\n}\n\n#[derive(Clone, Debug, PartialEq, Eq)]\npub struct SessionRestoreRecovery",
    "        Ok(Some(removed))\n"
    "    }\n\n"
    "    pub(crate) fn advance_generation_after_save(\n"
    "        &mut self,\n"
    "        expected_generation: u64,\n"
    "        saved_generation: u64,\n"
    "    ) -> bool {\n"
    "        if self.generation != expected_generation || saved_generation <= expected_generation {\n"
    "            return false;\n"
    "        }\n"
    "        self.generation = saved_generation;\n"
    "        true\n"
    "    }\n"
    "}\n\n"
    "#[derive(Clone, Debug, PartialEq, Eq)]\n"
    "pub struct SessionRestoreRecovery",
)

module = Path("src/profile_runtime/session_restore_persistence.rs")
if module.exists():
    raise SystemExit(f"{module} already exists")
module.write_text(r'''use super::{ActiveProfile, ProfileId, ProfileRuntime};
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
            Self::MutationRevisionExhausted => formatter
                .write_str("profile session-restore mutation revision space is exhausted"),
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
            Self::SessionRestore(error) => write!(formatter, "session-restore update failed: {error}"),
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
        Ok(self.active_session_restore_state(profile)?.mutation_revision)
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
        let target = state.snapshot.window(window).ok_or_else(|| {
            ProfileSessionRestoreRuntimeError::SessionRestore(SessionRestoreError::WindowNotFound {
                window,
            })
        })?;
        let target_tab = target.tabs().iter().find(|candidate| candidate.id() == tab).ok_or_else(
            || {
                ProfileSessionRestoreRuntimeError::SessionRestore(SessionRestoreError::TabNotFound {
                    window,
                    tab,
                })
            },
        )?;
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
        let target = state.snapshot.window(window).ok_or_else(|| {
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
        let target = state.snapshot.window(window).ok_or_else(|| {
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
        let active = self.active.as_ref().ok_or(
            ProfileSessionRestoreRuntimeError::SaveTargetMismatch {
                save: completion.id,
            },
        )?;
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
''', encoding="utf-8")

# Profile runtime integration.
replace_once(
    "src/profile_runtime.rs",
    "mod bookmarks_persistence;\n\nuse bookmarks_persistence::{BookmarksRuntimeState, PendingProfileBookmarksSave};",
    "mod bookmarks_persistence;\n"
    "mod session_restore_persistence;\n\n"
    "use bookmarks_persistence::{BookmarksRuntimeState, PendingProfileBookmarksSave};\n"
    "use session_restore_persistence::{\n"
    "    PendingProfileSessionRestoreSave, SessionRestoreRuntimeState,\n"
    "};",
)
replace_once(
    "src/profile_runtime.rs",
    "pub use bookmarks_persistence::{\n    ProfileBookmarkToggleOutcome, ProfileBookmarksRuntimeError, ProfileBookmarksSaveCompletion,\n    ProfileBookmarksSaveId, ProfileBookmarksSaveIntent,\n};\n\nconst COLOR_SCHEME_KEY",
    "pub use bookmarks_persistence::{\n"
    "    ProfileBookmarkToggleOutcome, ProfileBookmarksRuntimeError, ProfileBookmarksSaveCompletion,\n"
    "    ProfileBookmarksSaveId, ProfileBookmarksSaveIntent,\n"
    "};\n"
    "pub use session_restore_persistence::{\n"
    "    ProfileSessionRestoreRuntimeError, ProfileSessionRestoreSaveCompletion,\n"
    "    ProfileSessionRestoreSaveId, ProfileSessionRestoreSaveIntent,\n"
    "};\n\n"
    "const COLOR_SCHEME_KEY",
)
replace_once(
    "src/profile_runtime.rs",
    "    bookmarks: BookmarksRuntimeState,\n    session_restore: SessionRestoreSnapshot,\n    session_restore_recovery: Option<SessionRestoreRecovery>,\n    settings_revision: u64,",
    "    bookmarks: BookmarksRuntimeState,\n"
    "    session_restore: SessionRestoreRuntimeState,\n"
    "    settings_revision: u64,",
)
replace_once(
    "src/profile_runtime.rs",
    "    pub const fn session_restore(&self) -> &SessionRestoreSnapshot {\n        &self.session_restore\n    }\n\n    pub const fn session_restore_recovery(&self) -> Option<&SessionRestoreRecovery> {\n        self.session_restore_recovery.as_ref()\n    }",
    "    pub const fn session_restore(&self) -> &SessionRestoreSnapshot {\n"
    "        self.session_restore.snapshot()\n"
    "    }\n\n"
    "    pub const fn session_restore_recovery(&self) -> Option<&SessionRestoreRecovery> {\n"
    "        self.session_restore.recovery()\n"
    "    }",
)
replace_once(
    "src/profile_runtime.rs",
    "    next_bookmarks_save_id: u64,\n    active: Option<ActiveProfile>,",
    "    next_bookmarks_save_id: u64,\n"
    "    next_session_restore_save_id: u64,\n"
    "    active: Option<ActiveProfile>,",
)
replace_once(
    "src/profile_runtime.rs",
    "    pending_bookmarks_save: Option<PendingProfileBookmarksSave>,\n}",
    "    pending_bookmarks_save: Option<PendingProfileBookmarksSave>,\n"
    "    pending_session_restore_save: Option<PendingProfileSessionRestoreSave>,\n"
    "}",
)
replace_once(
    "src/profile_runtime.rs",
    "            next_bookmarks_save_id: 1,\n            active: None,",
    "            next_bookmarks_save_id: 1,\n"
    "            next_session_restore_save_id: 1,\n"
    "            active: None,",
)
replace_once(
    "src/profile_runtime.rs",
    "            pending_bookmarks_save: None,\n        }",
    "            pending_bookmarks_save: None,\n"
    "            pending_session_restore_save: None,\n"
    "        }",
)
replace_once(
    "src/profile_runtime.rs",
    "            let persistence_pending = self.pending_settings_save.is_some()\n                || self.pending_history_save.is_some()\n                || self.pending_bookmarks_save.is_some();\n            let persistence_dirty = active.settings_revision != active.durable_settings_revision\n                || active.browsing_history_revision != active.durable_browsing_history_revision\n                || active.bookmarks.is_dirty();",
    "            let persistence_pending = self.pending_settings_save.is_some()\n"
    "                || self.pending_history_save.is_some()\n"
    "                || self.pending_bookmarks_save.is_some()\n"
    "                || self.pending_session_restore_save.is_some();\n"
    "            let persistence_dirty = active.settings_revision != active.durable_settings_revision\n"
    "                || active.browsing_history_revision != active.durable_browsing_history_revision\n"
    "                || active.bookmarks.is_dirty()\n"
    "                || active.session_restore.is_dirty();",
)
replace_once(
    "src/profile_runtime.rs",
    "            bookmarks: BookmarksRuntimeState::new(prepared.bookmarks, prepared.bookmarks_recovery),\n            session_restore: prepared.session_restore,\n            session_restore_recovery: prepared.session_restore_recovery,\n            settings_revision: 0,",
    "            bookmarks: BookmarksRuntimeState::new(prepared.bookmarks, prepared.bookmarks_recovery),\n"
    "            session_restore: SessionRestoreRuntimeState::new(\n"
    "                prepared.session_restore,\n"
    "                prepared.session_restore_recovery,\n"
    "            ),\n"
    "            settings_revision: 0,",
)

# Bounded worker save command/completion/dispatch.
replace_once(
    "src/profile_worker.rs",
    "    ProfileSelectionId, ProfileSelectionIntent, ProfileSettingsSaveCompletion,\n    ProfileSettingsSaveIntent,\n};",
    "    ProfileSelectionId, ProfileSelectionIntent, ProfileSessionRestoreSaveCompletion,\n"
    "    ProfileSessionRestoreSaveIntent, ProfileSettingsSaveCompletion, ProfileSettingsSaveIntent,\n"
    "};",
)
replace_once(
    "src/profile_worker.rs",
    "    SaveBookmarks(ProfileBookmarksSaveIntent),\n    DiscoverCatalog(ProfileCatalogDiscoverIntent),",
    "    SaveBookmarks(ProfileBookmarksSaveIntent),\n"
    "    SaveSessionRestore(ProfileSessionRestoreSaveIntent),\n"
    "    DiscoverCatalog(ProfileCatalogDiscoverIntent),",
)
replace_once(
    "src/profile_worker.rs",
    "    BookmarksSaved(ProfileBookmarksSaveCompletion),\n    CatalogDiscovered {",
    "    BookmarksSaved(ProfileBookmarksSaveCompletion),\n"
    "    SessionRestoreSaved(ProfileSessionRestoreSaveCompletion),\n"
    "    CatalogDiscovered {",
)
replace_once(
    "src/profile_worker.rs",
    "    pub fn discover_profiles(\n",
    "    pub fn save_session_restore(\n"
    "        &self,\n"
    "        intent: ProfileSessionRestoreSaveIntent,\n"
    "    ) -> Result<(), ProfileWorkerSubmitError<ProfileSessionRestoreSaveIntent>> {\n"
    "        match self\n"
    "            .sender\n"
    "            .try_send(ProfileWorkerCommand::SaveSessionRestore(intent))\n"
    "        {\n"
    "            Ok(()) => Ok(()),\n"
    "            Err(TrySendError::Full(ProfileWorkerCommand::SaveSessionRestore(intent))) => {\n"
    "                Err(ProfileWorkerSubmitError::Full(Box::new(intent)))\n"
    "            }\n"
    "            Err(TrySendError::Disconnected(ProfileWorkerCommand::SaveSessionRestore(\n"
    "                intent,\n"
    "            ))) => Err(ProfileWorkerSubmitError::Unavailable(Box::new(intent))),\n"
    "            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {\n"
    "                unreachable!(\"session-restore-save submission preserves its command variant\")\n"
    "            }\n"
    "        }\n"
    "    }\n\n"
    "    pub fn discover_profiles(\n",
)
replace_once(
    "src/profile_worker.rs",
    "            ProfileWorkerCommand::SaveBookmarks(intent) => {\n                ProfileWorkerCompletion::BookmarksSaved(intent.execute())\n            }\n            ProfileWorkerCommand::DiscoverCatalog(intent) => {",
    "            ProfileWorkerCommand::SaveBookmarks(intent) => {\n"
    "                ProfileWorkerCompletion::BookmarksSaved(intent.execute())\n"
    "            }\n"
    "            ProfileWorkerCommand::SaveSessionRestore(intent) => {\n"
    "                ProfileWorkerCompletion::SessionRestoreSaved(intent.execute())\n"
    "            }\n"
    "            ProfileWorkerCommand::DiscoverCatalog(intent) => {",
)

# Public exports.
replace_once(
    "src/lib.rs",
    "    ProfileSelectionId, ProfileSelectionIntent, ProfileSelectionStart, ProfileSettingsError,\n    ProfileSettingsSaveCompletion, ProfileSettingsSaveId, ProfileSettingsSaveIntent,\n};",
    "    ProfileSelectionId, ProfileSelectionIntent, ProfileSelectionStart,\n"
    "    ProfileSessionRestoreRuntimeError, ProfileSessionRestoreSaveCompletion,\n"
    "    ProfileSessionRestoreSaveId, ProfileSessionRestoreSaveIntent, ProfileSettingsError,\n"
    "    ProfileSettingsSaveCompletion, ProfileSettingsSaveId, ProfileSettingsSaveIntent,\n"
    "};",
)

# Windows deliberately has no session scheduler/native persistence yet; fail closed on an impossible
# completion so adding the worker variant does not silently broaden native behavior.
replace_once(
    "src/platform/windows.rs",
    "            ProfileWorkerCompletion::ProfileRenamed { .. } => self.fail(\n",
    "            ProfileWorkerCompletion::SessionRestoreSaved(_) => self.fail(\n"
    "                event_loop,\n"
    "                \"unexpected session-restore-save completion arrived without native session persistence wiring\",\n"
    "            ),\n"
    "            ProfileWorkerCompletion::ProfileRenamed { .. } => self.fail(\n",
)

# Integration coverage.
def write_new(path: str, text: str) -> None:
    target = Path(path)
    if target.exists():
        raise SystemExit(f"{target} already exists")
    target.write_text(text, encoding="utf-8")

write_new("tests/session_restore_runtime_persistence.rs", r'''use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use zorya::{PreparedProfile, ProfileRuntime, SessionRestoreError, SessionRestoreStore};

static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(1);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "zorya-session-runtime-{}-{label}-{id}",
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

fn load_profile(runtime: &mut ProfileRuntime, root: &Path) -> zorya::ProfileId {
    let selection = runtime.begin_selection(root).unwrap().into_intent();
    let prepared = PreparedProfile::load(&selection).unwrap();
    runtime.commit_selection(prepared).unwrap().active_profile()
}

#[test]
fn successful_save_cleans_only_captured_session_mutations() {
    let root = TempRoot::new("newer-mutation");
    let mut runtime = ProfileRuntime::new();
    let profile = load_profile(&mut runtime, root.path());

    runtime.add_session_window(profile).unwrap();
    let first = runtime
        .begin_session_restore_save_if_dirty(profile)
        .unwrap()
        .unwrap();
    assert_eq!(first.mutation_revision(), 1);

    runtime.add_session_window(profile).unwrap();
    assert_eq!(runtime.session_restore_unsaved_mutations(profile).unwrap(), 2);

    let completion = first.execute();
    assert!(completion.result().is_ok());
    runtime.complete_session_restore_save(completion).unwrap();
    assert_eq!(runtime.active_session_restore(profile).unwrap().generation(), 1);
    assert_eq!(runtime.active_session_restore(profile).unwrap().len(), 2);
    assert!(runtime.session_restore_is_dirty(profile).unwrap());
    assert_eq!(runtime.session_restore_unsaved_mutations(profile).unwrap(), 1);

    let completion = runtime
        .begin_session_restore_save_if_dirty(profile)
        .unwrap()
        .unwrap()
        .execute();
    runtime.complete_session_restore_save(completion).unwrap();
    assert_eq!(runtime.active_session_restore(profile).unwrap().generation(), 2);
    assert!(!runtime.session_restore_is_dirty(profile).unwrap());

    let persisted = SessionRestoreStore::open(root.path())
        .unwrap()
        .load()
        .unwrap()
        .into_snapshot();
    assert_eq!(persisted.generation(), 2);
    assert_eq!(persisted.len(), 2);
}

#[test]
fn failed_and_noop_session_mutations_do_not_advance_revision() {
    let root = TempRoot::new("failure-atomic");
    let mut runtime = ProfileRuntime::new();
    let profile = load_profile(&mut runtime, root.path());
    let window = runtime.add_session_window(profile).unwrap();
    let tab = runtime
        .add_session_tab(profile, window, "https://example.test/original")
        .unwrap();
    assert_eq!(runtime.session_restore_mutation_revision(profile).unwrap(), 2);

    assert!(!runtime
        .update_session_tab_location(profile, window, tab, "https://example.test/original")
        .unwrap());
    assert_eq!(runtime.session_restore_mutation_revision(profile).unwrap(), 2);

    let oversized = "x".repeat(zorya::MAX_SESSION_LOCATION_BYTES + 1);
    assert!(matches!(
        runtime.update_session_tab_location(profile, window, tab, oversized),
        Err(zorya::ProfileSessionRestoreRuntimeError::SessionRestore(
            SessionRestoreError::LocationTooLarge { .. }
        ))
    ));
    assert_eq!(runtime.session_restore_mutation_revision(profile).unwrap(), 2);
}

#[test]
fn failed_save_completion_clears_only_ownership_and_leaves_session_dirty() {
    let root = TempRoot::new("save-failure");
    let mut runtime = ProfileRuntime::new();
    let profile = load_profile(&mut runtime, root.path());
    runtime.add_session_window(profile).unwrap();
    let intent = runtime.begin_session_restore_save(profile).unwrap();

    let session_dir = root.path().join("session");
    fs::write(
        session_dir.join("session-00000000000000000001.bin"),
        b"corrupt external generation",
    )
    .unwrap();

    let completion = intent.execute();
    assert!(completion.result().is_err());
    runtime.complete_session_restore_save(completion).unwrap();
    assert!(runtime.pending_session_restore_save().is_none());
    assert!(runtime.session_restore_is_dirty(profile).unwrap());
    assert_eq!(runtime.active_session_restore(profile).unwrap().generation(), 0);
}
''')

write_new("tests/session_restore_profile_replacement_durability.rs", r'''use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use zorya::{PreparedProfile, ProfileRuntime, ProfileRuntimeError, SessionRestoreStore};

static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(1);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "zorya-session-replacement-{}-{label}-{id}",
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

fn load_profile(runtime: &mut ProfileRuntime, root: &Path) -> zorya::ProfileId {
    let selection = runtime.begin_selection(root).unwrap().into_intent();
    let prepared = PreparedProfile::load(&selection).unwrap();
    runtime.commit_selection(prepared).unwrap().active_profile()
}

#[test]
fn dirty_session_restore_blocks_replacement_until_exact_save_is_durable() {
    let first_root = TempRoot::new("first");
    let second_root = TempRoot::new("second");
    let mut runtime = ProfileRuntime::new();
    let first = load_profile(&mut runtime, first_root.path());
    runtime.add_session_window(first).unwrap();
    assert!(runtime.session_restore_is_dirty(first).unwrap());

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
    let prepared = rejection.into_parts().1;

    let intent = runtime
        .begin_session_restore_save_if_dirty(first)
        .unwrap()
        .unwrap();
    let save = intent.id();
    let rejection = runtime.commit_selection(prepared).unwrap_err();
    assert_eq!(runtime.pending_session_restore_save(), Some(save));
    let prepared = rejection.into_parts().1;

    runtime.complete_session_restore_save(intent.execute()).unwrap();
    assert!(!runtime.session_restore_is_dirty(first).unwrap());
    let persisted = SessionRestoreStore::open(first_root.path())
        .unwrap()
        .load()
        .unwrap()
        .into_snapshot();
    assert_eq!(persisted.generation(), 1);
    assert_eq!(persisted.len(), 1);

    let replacement = runtime.commit_selection(prepared).unwrap();
    assert_ne!(replacement.active_profile(), first);
}
''')

write_new("tests/session_restore_save_identity.rs", r'''use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use zorya::{PreparedProfile, ProfileRuntime};

static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(1);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "zorya-session-save-identity-{}-{label}-{id}",
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

fn load_profile(runtime: &mut ProfileRuntime, root: &Path) -> zorya::ProfileId {
    let selection = runtime.begin_selection(root).unwrap().into_intent();
    let prepared = PreparedProfile::load(&selection).unwrap();
    runtime.commit_selection(prepared).unwrap().active_profile()
}

#[test]
fn session_restore_save_identity_is_process_monotonic_across_profile_replacement() {
    let first_root = TempRoot::new("first");
    let second_root = TempRoot::new("second");
    let mut runtime = ProfileRuntime::new();

    let first = load_profile(&mut runtime, first_root.path());
    runtime.add_session_window(first).unwrap();
    let first_save = runtime.begin_session_restore_save(first).unwrap();
    let first_save_id = first_save.id();
    runtime
        .complete_session_restore_save(first_save.execute())
        .unwrap();

    let selection = runtime
        .begin_selection(second_root.path())
        .unwrap()
        .into_intent();
    let prepared = PreparedProfile::load(&selection).unwrap();
    let second = runtime.commit_selection(prepared).unwrap().active_profile();
    assert_ne!(second, first);

    runtime.add_session_window(second).unwrap();
    let second_save = runtime.begin_session_restore_save(second).unwrap();
    assert!(second_save.id().get() > first_save_id.get());
}
''')

write_new("tests/session_restore_worker_persistence.rs", r'''use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use zorya::{PreparedProfile, ProfileRuntime, ProfileWorker, ProfileWorkerCompletion};

static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(1);

struct TempRoot(PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "zorya-session-worker-{}-{label}-{id}",
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

fn dirty_session_runtime(
    root: &Path,
) -> (ProfileRuntime, zorya::ProfileSessionRestoreSaveIntent) {
    let mut runtime = ProfileRuntime::new();
    let selection = runtime.begin_selection(root).unwrap().into_intent();
    let prepared = PreparedProfile::load(&selection).unwrap();
    let profile = runtime.commit_selection(prepared).unwrap().active_profile();
    runtime.add_session_window(profile).unwrap();
    let intent = runtime
        .begin_session_restore_save_if_dirty(profile)
        .unwrap()
        .unwrap();
    (runtime, intent)
}

#[test]
fn session_restore_save_executes_on_profile_worker_and_reconciles_runtime() {
    let root = TempRoot::new("execute");
    let (mut runtime, intent) = dirty_session_runtime(root.path());
    let profile = intent.profile();
    let save = intent.id();
    let (tx, rx) = mpsc::sync_channel(1);
    let worker = ProfileWorker::spawn(move |completion| {
        tx.send((thread::current().name().map(str::to_owned), completion))
            .unwrap();
    })
    .unwrap();

    worker.save_session_restore(intent).unwrap();
    let (thread_name, completion) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(thread_name.as_deref(), Some("zorya-profile"));
    let ProfileWorkerCompletion::SessionRestoreSaved(completion) = completion else {
        panic!("expected session-restore-save completion");
    };
    assert_eq!(completion.id(), save);
    assert!(completion.result().is_ok());
    runtime.complete_session_restore_save(completion).unwrap();
    assert!(!runtime.session_restore_is_dirty(profile).unwrap());
    assert_eq!(runtime.active_session_restore(profile).unwrap().generation(), 1);
}

#[test]
fn queue_full_returns_exact_session_restore_save_for_runtime_cancellation() {
    let blocking_root = TempRoot::new("blocking");
    let queued_root = TempRoot::new("queued");
    let session_root = TempRoot::new("session");

    let mut queue_runtime = ProfileRuntime::new();
    let blocking = queue_runtime
        .begin_selection(blocking_root.path())
        .unwrap()
        .into_intent();
    let queued = queue_runtime
        .begin_selection(queued_root.path())
        .unwrap()
        .into_intent();
    let (mut runtime, intent) = dirty_session_runtime(session_root.path());
    let profile = intent.profile();
    let save = intent.id();

    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let mut block_first_completion = true;
    let worker = ProfileWorker::spawn(move |_| {
        if block_first_completion {
            block_first_completion = false;
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        }
    })
    .unwrap();

    worker.prepare(blocking).unwrap();
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    worker.prepare(queued).unwrap();
    let error = worker.save_session_restore(intent).unwrap_err();
    assert!(error.is_full());
    let intent = error.into_work();
    assert_eq!(intent.id(), save);
    assert_eq!(intent.profile(), profile);

    runtime.cancel_session_restore_save(save).unwrap();
    assert!(runtime.pending_session_restore_save().is_none());
    assert!(runtime.session_restore_is_dirty(profile).unwrap());

    release_tx.send(()).unwrap();
}

#[test]
fn disconnected_worker_returns_exact_unavailable_session_restore_save() {
    let blocking_root = TempRoot::new("disconnect-blocking");
    let session_root = TempRoot::new("disconnect-session");

    let mut queue_runtime = ProfileRuntime::new();
    let blocking = queue_runtime
        .begin_selection(blocking_root.path())
        .unwrap()
        .into_intent();
    let (mut runtime, intent) = dirty_session_runtime(session_root.path());
    let profile = intent.profile();
    let save = intent.id();

    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let worker = ProfileWorker::spawn(move |_| {
        entered_tx.send(()).unwrap();
        panic!("fixture terminates profile worker");
    })
    .unwrap();

    worker.prepare(blocking).unwrap();
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !worker.is_finished() && Instant::now() < deadline {
        thread::yield_now();
    }
    assert!(worker.is_finished());

    let error = worker.save_session_restore(intent).unwrap_err();
    assert!(!error.is_full());
    let intent = error.into_work();
    assert_eq!(intent.id(), save);
    assert_eq!(intent.profile(), profile);

    runtime.cancel_session_restore_save(save).unwrap();
    assert!(runtime.pending_session_restore_save().is_none());
    assert!(runtime.session_restore_is_dirty(profile).unwrap());
}
''')

# Documentation: this slice adds runtime save ownership, not scheduling/native restoration.
architecture = "docs/ARCHITECTURE.md"
replace_once(
    architecture,
    "Settings, history and bookmark save intents clone that exact token; all three stores verify the token/root before doing write work and again immediately before immutable-generation publication.",
    "Settings, history, bookmark and session-restore save intents clone that exact token; all four stores verify the token/root before doing write work and again immediately before immutable-generation publication.",
)
replace_once(
    architecture,
    "If an active profile still has exact settings/history persistence work or bookmark save ownership/mutations not yet durable, replacement is rejected with `ActiveProfileNotDurable` and returns the unchanged `PreparedProfile`; pending save ownership is never invalidated merely to switch profiles.",
    "If an active profile still has exact settings/history persistence work, bookmark save ownership/mutations or session-restore save ownership/mutations not yet durable, replacement is rejected with `ActiveProfileNotDurable` and returns the unchanged `PreparedProfile`; pending save ownership is never invalidated merely to switch profiles.",
)
replace_once(
    architecture,
    "`PreparedProfile::load` now opens the session store while the exact profile lock is held, preserves the exact `SessionRestoreSnapshot` plus optional `SessionRestoreRecovery`, and a successful exact selection commit transfers both unchanged into `ActiveProfile`; mutation revisions, save lifecycle/worker dispatch, scheduling and native startup restoration remain later Z3 work.",
    "`PreparedProfile::load` opens the session store while the exact profile lock is held, preserves the exact `SessionRestoreSnapshot` plus optional `SessionRestoreRecovery`, and a successful exact selection commit transfers both into `ActiveProfile`. The active profile wraps that snapshot in an exact-profile runtime state with monotonic mutation/durable revisions. Typed window/tab mutations advance the revision only after a real successful mutation. `ProfileSessionRestoreSaveId` binds one cloned snapshot, root, exact lock, base generation and captured mutation revision; exact successful completion advances only the represented storage generation/durable revision, preserving newer in-memory mutations for the next save, while failed or cancelled saves remain dirty. Only one session save may be in flight, stale completion/cancellation cannot clear newer ownership, profile replacement is blocked until session state is durable, and synchronous save execution is handed only to the bounded `ProfileWorker`. Scheduling and native startup restoration remain later Z3 work.",
)
replace_once(
    architecture,
    "and bookmark-save intents with `ProfileBookmarksSaveIntent::execute`, then invokes a caller-provided completion handler with exact typed work.",
    "bookmark-save intents with `ProfileBookmarksSaveIntent::execute`, and session-restore-save intents with `ProfileSessionRestoreSaveIntent::execute`, then invokes a caller-provided completion handler with exact typed work.",
)

roadmap = "docs/ROADMAP.md"
old_session_line = "- session restore now has a bounded, versioned and recoverable profile-local storage foundation with persisted window/tab identities distinct from runtime browser identities, ordered windows/tabs, exact active-tab membership and committed display-location-only records; immutable generations use exact profile-lock verification, no-overwrite publication, stale/concurrent writer rejection, explicit corruption fallback, fail-closed schema handling and bounded retention; locked worker-side profile preparation now loads the exact session snapshot plus recovery report and transfers both unchanged into the committed `ActiveProfile`; mutation/dirty/save lifecycle, worker save dispatch, scheduling and native startup restore remain later Z3 work;"
new_session_line = "- session restore now has bounded, versioned and recoverable profile-local storage plus locked preparation/active-profile attachment; persisted window/tab identities remain distinct from runtime browser identities and records remain committed display-location-only; an exact-profile runtime lifecycle now tracks monotonic mutation/durable revisions, one-in-flight save identity, captured generations and newer-mutation preservation, keeps failed/cancelled work dirty, blocks profile replacement until durable, and executes synchronous saves only through the bounded `ProfileWorker` with exact queue-failure recovery; scheduling and native startup restore remain later Z3 work;"
replace_once(roadmap, old_session_line, new_session_line)
