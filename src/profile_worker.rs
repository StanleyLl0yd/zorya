use crate::profile_lock::{ProfileLock, ProfileLockError, ProfileLockOwner};
use crate::profile_runtime::{
    PreparedProfile, ProfileHistorySaveCompletion, ProfileHistorySaveIntent,
    ProfilePreparationError, ProfileSelectionId, ProfileSelectionIntent,
    ProfileSettingsSaveCompletion, ProfileSettingsSaveIntent,
};
use std::fmt;
use std::io;
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::thread::{self, JoinHandle};

pub const PROFILE_WORKER_COMMAND_QUEUE_CAPACITY: usize = 1;

enum ProfileWorkerCommand {
    Prepare(ProfileSelectionIntent),
    SaveSettings(ProfileSettingsSaveIntent),
    SaveHistory(ProfileHistorySaveIntent),
    ReleaseLock(ProfileLock),
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProfileWorkerCompletion {
    Prepared {
        selection: ProfileSelectionId,
        result: Result<PreparedProfile, ProfilePreparationError>,
    },
    SettingsSaved(ProfileSettingsSaveCompletion),
    HistorySaved(ProfileHistorySaveCompletion),
    LockReleased {
        owner: ProfileLockOwner,
        result: Result<(), ProfileLockError>,
    },
}

#[derive(Debug)]
pub struct ProfileWorkerSpawnError {
    kind: io::ErrorKind,
    message: String,
}

impl ProfileWorkerSpawnError {
    pub const fn kind(&self) -> io::ErrorKind {
        self.kind
    }
}

impl fmt::Display for ProfileWorkerSpawnError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "failed to start profile worker: {}",
            self.message
        )
    }
}

impl std::error::Error for ProfileWorkerSpawnError {}

#[derive(Debug, PartialEq, Eq)]
pub enum ProfileWorkerSubmitError<T> {
    Full(T),
    Unavailable(T),
}

impl<T> ProfileWorkerSubmitError<T> {
    pub const fn is_full(&self) -> bool {
        matches!(self, Self::Full(_))
    }

    pub fn into_work(self) -> T {
        match self {
            Self::Full(work) | Self::Unavailable(work) => work,
        }
    }
}

impl<T> fmt::Display for ProfileWorkerSubmitError<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full(_) => formatter.write_str("profile worker command queue is full"),
            Self::Unavailable(_) => formatter.write_str("profile worker is unavailable"),
        }
    }
}

impl<T: fmt::Debug> std::error::Error for ProfileWorkerSubmitError<T> {}

pub struct ProfileWorker {
    sender: SyncSender<ProfileWorkerCommand>,
    thread: JoinHandle<()>,
}

impl fmt::Debug for ProfileWorker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProfileWorker")
            .field("finished", &self.thread.is_finished())
            .finish_non_exhaustive()
    }
}

impl ProfileWorker {
    pub fn spawn(
        completion_handler: impl FnMut(ProfileWorkerCompletion) + Send + 'static,
    ) -> Result<Self, ProfileWorkerSpawnError> {
        let (sender, receiver) = sync_channel(PROFILE_WORKER_COMMAND_QUEUE_CAPACITY);
        let thread = thread::Builder::new()
            .name("zorya-profile".into())
            .spawn(move || profile_worker_main(receiver, completion_handler))
            .map_err(|error| ProfileWorkerSpawnError {
                kind: error.kind(),
                message: error.to_string(),
            })?;
        Ok(Self { sender, thread })
    }

    pub fn prepare(
        &self,
        intent: ProfileSelectionIntent,
    ) -> Result<(), ProfileWorkerSubmitError<ProfileSelectionIntent>> {
        match self.sender.try_send(ProfileWorkerCommand::Prepare(intent)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(ProfileWorkerCommand::Prepare(intent))) => {
                Err(ProfileWorkerSubmitError::Full(intent))
            }
            Err(TrySendError::Disconnected(ProfileWorkerCommand::Prepare(intent))) => {
                Err(ProfileWorkerSubmitError::Unavailable(intent))
            }
            Err(TrySendError::Full(ProfileWorkerCommand::SaveSettings(_)))
            | Err(TrySendError::Disconnected(ProfileWorkerCommand::SaveSettings(_)))
            | Err(TrySendError::Full(ProfileWorkerCommand::SaveHistory(_)))
            | Err(TrySendError::Disconnected(ProfileWorkerCommand::SaveHistory(_)))
            | Err(TrySendError::Full(ProfileWorkerCommand::ReleaseLock(_)))
            | Err(TrySendError::Disconnected(ProfileWorkerCommand::ReleaseLock(_))) => {
                unreachable!("prepare submission preserves its command variant")
            }
        }
    }

    pub fn save_settings(
        &self,
        intent: ProfileSettingsSaveIntent,
    ) -> Result<(), ProfileWorkerSubmitError<ProfileSettingsSaveIntent>> {
        match self
            .sender
            .try_send(ProfileWorkerCommand::SaveSettings(intent))
        {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(ProfileWorkerCommand::SaveSettings(intent))) => {
                Err(ProfileWorkerSubmitError::Full(intent))
            }
            Err(TrySendError::Disconnected(ProfileWorkerCommand::SaveSettings(intent))) => {
                Err(ProfileWorkerSubmitError::Unavailable(intent))
            }
            Err(TrySendError::Full(ProfileWorkerCommand::Prepare(_)))
            | Err(TrySendError::Disconnected(ProfileWorkerCommand::Prepare(_)))
            | Err(TrySendError::Full(ProfileWorkerCommand::SaveHistory(_)))
            | Err(TrySendError::Disconnected(ProfileWorkerCommand::SaveHistory(_)))
            | Err(TrySendError::Full(ProfileWorkerCommand::ReleaseLock(_)))
            | Err(TrySendError::Disconnected(ProfileWorkerCommand::ReleaseLock(_))) => {
                unreachable!("settings-save submission preserves its command variant")
            }
        }
    }

    pub fn save_history(
        &self,
        intent: ProfileHistorySaveIntent,
    ) -> Result<(), ProfileWorkerSubmitError<ProfileHistorySaveIntent>> {
        match self
            .sender
            .try_send(ProfileWorkerCommand::SaveHistory(intent))
        {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(ProfileWorkerCommand::SaveHistory(intent))) => {
                Err(ProfileWorkerSubmitError::Full(intent))
            }
            Err(TrySendError::Disconnected(ProfileWorkerCommand::SaveHistory(intent))) => {
                Err(ProfileWorkerSubmitError::Unavailable(intent))
            }
            Err(TrySendError::Full(ProfileWorkerCommand::Prepare(_)))
            | Err(TrySendError::Disconnected(ProfileWorkerCommand::Prepare(_)))
            | Err(TrySendError::Full(ProfileWorkerCommand::SaveSettings(_)))
            | Err(TrySendError::Disconnected(ProfileWorkerCommand::SaveSettings(_)))
            | Err(TrySendError::Full(ProfileWorkerCommand::ReleaseLock(_)))
            | Err(TrySendError::Disconnected(ProfileWorkerCommand::ReleaseLock(_))) => {
                unreachable!("history-save submission preserves its command variant")
            }
        }
    }

    pub fn release_lock(
        &self,
        lock: ProfileLock,
    ) -> Result<(), ProfileWorkerSubmitError<ProfileLock>> {
        match self.sender.try_send(ProfileWorkerCommand::ReleaseLock(lock)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(ProfileWorkerCommand::ReleaseLock(lock))) => {
                Err(ProfileWorkerSubmitError::Full(lock))
            }
            Err(TrySendError::Disconnected(ProfileWorkerCommand::ReleaseLock(lock))) => {
                Err(ProfileWorkerSubmitError::Unavailable(lock))
            }
            Err(TrySendError::Full(ProfileWorkerCommand::Prepare(_)))
            | Err(TrySendError::Disconnected(ProfileWorkerCommand::Prepare(_)))
            | Err(TrySendError::Full(ProfileWorkerCommand::SaveSettings(_)))
            | Err(TrySendError::Disconnected(ProfileWorkerCommand::SaveSettings(_)))
            | Err(TrySendError::Full(ProfileWorkerCommand::SaveHistory(_)))
            | Err(TrySendError::Disconnected(ProfileWorkerCommand::SaveHistory(_))) => {
                unreachable!("lock-release submission preserves its command variant")
            }
        }
    }

    pub fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }
}

fn profile_worker_main(
    receiver: std::sync::mpsc::Receiver<ProfileWorkerCommand>,
    mut completion_handler: impl FnMut(ProfileWorkerCompletion),
) {
    while let Ok(command) = receiver.recv() {
        let completion = match command {
            ProfileWorkerCommand::Prepare(intent) => {
                let selection = intent.id();
                let result = PreparedProfile::load(&intent);
                ProfileWorkerCompletion::Prepared { selection, result }
            }
            ProfileWorkerCommand::SaveSettings(intent) => {
                ProfileWorkerCompletion::SettingsSaved(intent.execute())
            }
            ProfileWorkerCommand::SaveHistory(intent) => {
                ProfileWorkerCompletion::HistorySaved(intent.execute())
            }
            ProfileWorkerCommand::ReleaseLock(lock) => {
                let owner = lock.owner();
                let result = lock.release();
                ProfileWorkerCompletion::LockReleased { owner, result }
            }
        };
        completion_handler(completion);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColorSchemePreference, ProfileRuntime, ProfileRuntimeError};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "zorya-profile-worker-{}-{name}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&root);
            let _ = fs::remove_file(&root);
            Self(root)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
            let _ = fs::remove_file(&self.0);
        }
    }

    fn worker_channel() -> (
        ProfileWorker,
        mpsc::Receiver<(String, ProfileWorkerCompletion)>,
    ) {
        let (sender, receiver) = mpsc::channel();
        let worker = ProfileWorker::spawn(move |completion| {
            let name = thread::current().name().unwrap_or_default().to_owned();
            sender.send((name, completion)).unwrap();
        })
        .unwrap();
        (worker, receiver)
    }

    fn receive(
        receiver: &mpsc::Receiver<(String, ProfileWorkerCompletion)>,
    ) -> (String, ProfileWorkerCompletion) {
        receiver.recv_timeout(Duration::from_secs(5)).unwrap()
    }

    #[test]
    fn lock_release_executes_on_named_worker() {
        let root = TempRoot::new("lock-release");
        let lock = ProfileLock::acquire(root.path()).unwrap();
        let owner = lock.owner();
        let (worker, receiver) = worker_channel();

        worker.release_lock(lock).unwrap();
        let (thread_name, completion) = receive(&receiver);
        assert_eq!(thread_name, "zorya-profile");
        let ProfileWorkerCompletion::LockReleased {
            owner: completed_owner,
            result,
        } = completion
        else {
            panic!("expected profile-lock release completion");
        };
        assert_eq!(completed_owner, owner);
        assert!(result.is_ok());

        let reacquired = ProfileLock::acquire(root.path()).unwrap();
        reacquired.release().unwrap();
    }

    #[test]
    fn preparation_runs_on_named_worker_and_preserves_selection_identity() {
        let root = TempRoot::new("prepare");
        let mut runtime = ProfileRuntime::new();
        let intent = runtime.begin_selection(root.path()).unwrap().into_intent();
        let selection = intent.id();
        let (worker, receiver) = worker_channel();

        worker.prepare(intent).unwrap();
        let (thread_name, completion) = receive(&receiver);
        assert_eq!(thread_name, "zorya-profile");

        let ProfileWorkerCompletion::Prepared {
            selection: completed_selection,
            result,
        } = completion
        else {
            panic!("expected prepared profile completion");
        };
        assert_eq!(completed_selection, selection);
        let prepared = result.unwrap();
        assert_eq!(prepared.selection(), selection);
        runtime.commit_selection(prepared).unwrap();
        assert_eq!(runtime.active_profile().unwrap().id().get(), 1);
    }

    #[test]
    fn preparation_error_keeps_exact_selection_identity() {
        let root = TempRoot::new("prepare-error");
        fs::write(root.path(), b"not a directory").unwrap();
        let mut runtime = ProfileRuntime::new();
        let intent = runtime.begin_selection(root.path()).unwrap().into_intent();
        let selection = intent.id();
        let (worker, receiver) = worker_channel();

        worker.prepare(intent).unwrap();
        let (_, completion) = receive(&receiver);
        let ProfileWorkerCompletion::Prepared {
            selection: completed_selection,
            result,
        } = completion
        else {
            panic!("expected prepared profile completion");
        };
        assert_eq!(completed_selection, selection);
        assert!(result.is_err());
        assert!(runtime.pending_selection().is_some());
    }

    #[test]
    fn settings_save_executes_on_worker_and_reconciles_runtime() {
        let root = TempRoot::new("settings-save");
        let mut runtime = ProfileRuntime::new();
        let selection = runtime.begin_selection(root.path()).unwrap().into_intent();
        let prepared = PreparedProfile::load(&selection).unwrap();
        let profile = runtime.commit_selection(prepared).unwrap().active_profile();
        runtime
            .active_settings_mut(profile)
            .unwrap()
            .set_color_scheme(ColorSchemePreference::Dark)
            .unwrap();
        let intent = runtime
            .begin_settings_save_if_dirty(profile)
            .unwrap()
            .unwrap();
        let save = intent.id();
        let (worker, receiver) = worker_channel();

        worker.save_settings(intent).unwrap();
        let (thread_name, completion) = receive(&receiver);
        assert_eq!(thread_name, "zorya-profile");
        let ProfileWorkerCompletion::SettingsSaved(completion) = completion else {
            panic!("expected settings-save completion");
        };
        assert_eq!(completion.id(), save);
        assert!(completion.result().is_ok());
        runtime.complete_settings_save(completion).unwrap();
        assert!(!runtime.settings_is_dirty(profile).unwrap());
        assert_eq!(runtime.active_profile().unwrap().settings().generation(), 1);
    }

    #[test]
    fn history_save_executes_on_worker_and_reconciles_runtime() {
        let root = TempRoot::new("history-save");
        let mut runtime = ProfileRuntime::new();
        let selection = runtime.begin_selection(root.path()).unwrap().into_intent();
        let prepared = PreparedProfile::load(&selection).unwrap();
        let profile = runtime.commit_selection(prepared).unwrap().active_profile();
        runtime
            .active_browsing_history_mut(profile)
            .unwrap()
            .record_visit(1, "https://example.test/")
            .unwrap();
        let intent = runtime
            .begin_browsing_history_save_if_dirty(profile)
            .unwrap()
            .unwrap();
        let save = intent.id();
        let (worker, receiver) = worker_channel();

        worker.save_history(intent).unwrap();
        let (thread_name, completion) = receive(&receiver);
        assert_eq!(thread_name, "zorya-profile");
        let ProfileWorkerCompletion::HistorySaved(completion) = completion else {
            panic!("expected history-save completion");
        };
        assert_eq!(completion.id(), save);
        assert!(completion.result().is_ok());
        runtime.complete_browsing_history_save(completion).unwrap();
        assert!(!runtime.browsing_history_is_dirty(profile).unwrap());
        assert_eq!(
            runtime
                .active_browsing_history(profile)
                .unwrap()
                .generation(),
            1
        );
    }

    #[test]
    fn bounded_queue_returns_full_work_without_blocking_or_losing_identity() {
        let first_root = TempRoot::new("queue-first");
        let second_root = TempRoot::new("queue-second");
        let third_root = TempRoot::new("queue-third");
        let mut runtime = ProfileRuntime::new();
        let first = runtime
            .begin_selection(first_root.path())
            .unwrap()
            .into_intent();
        let second = runtime
            .begin_selection(second_root.path())
            .unwrap()
            .into_intent();
        let third = runtime
            .begin_selection(third_root.path())
            .unwrap()
            .into_intent();
        let third_id = third.id();
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

        worker.prepare(first).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        worker.prepare(second).unwrap();
        let error = worker.prepare(third).unwrap_err();
        assert!(error.is_full());
        assert_eq!(error.into_work().id(), third_id);

        release_tx.send(()).unwrap();
    }

    #[test]
    fn settings_queue_failure_preserves_exact_work_for_runtime_cancellation() {
        let blocking_root = TempRoot::new("settings-queue-blocking");
        let queued_root = TempRoot::new("settings-queue-queued");
        let settings_root = TempRoot::new("settings-queue-save");
        let mut queue_runtime = ProfileRuntime::new();
        let blocking = queue_runtime
            .begin_selection(blocking_root.path())
            .unwrap()
            .into_intent();
        let queued = queue_runtime
            .begin_selection(queued_root.path())
            .unwrap()
            .into_intent();

        let mut settings_runtime = ProfileRuntime::new();
        let selection = settings_runtime
            .begin_selection(settings_root.path())
            .unwrap()
            .into_intent();
        let prepared = PreparedProfile::load(&selection).unwrap();
        let profile = settings_runtime
            .commit_selection(prepared)
            .unwrap()
            .active_profile();
        settings_runtime
            .active_settings_mut(profile)
            .unwrap()
            .set_color_scheme(ColorSchemePreference::Dark)
            .unwrap();
        let intent = settings_runtime
            .begin_settings_save_if_dirty(profile)
            .unwrap()
            .unwrap();
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
        let error = worker.save_settings(intent).unwrap_err();
        assert!(error.is_full());
        let intent = error.into_work();
        assert_eq!(intent.id(), save);

        settings_runtime.cancel_settings_save(save).unwrap();
        assert_eq!(settings_runtime.pending_settings_save(), None);
        assert!(settings_runtime.settings_is_dirty(profile).unwrap());

        release_tx.send(()).unwrap();
    }

    #[test]
    fn disconnected_worker_returns_unavailable_work() {
        let first_root = TempRoot::new("disconnect-first");
        let second_root = TempRoot::new("disconnect-second");
        let mut runtime = ProfileRuntime::new();
        let first = runtime
            .begin_selection(first_root.path())
            .unwrap()
            .into_intent();
        let second = runtime
            .begin_selection(second_root.path())
            .unwrap()
            .into_intent();
        let second_id = second.id();
        let (entered_tx, entered_rx) = mpsc::sync_channel(1);
        let worker = ProfileWorker::spawn(move |_| {
            entered_tx.send(()).unwrap();
            panic!("fixture terminates profile worker");
        })
        .unwrap();

        worker.prepare(first).unwrap();
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !worker.is_finished() && Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(worker.is_finished());

        let error = worker.prepare(second).unwrap_err();
        assert!(!error.is_full());
        assert_eq!(error.into_work().id(), second_id);
    }

    #[test]
    fn stale_preparation_completion_is_rejected_by_profile_runtime() {
        let first_root = TempRoot::new("stale-first");
        let second_root = TempRoot::new("stale-second");
        let mut runtime = ProfileRuntime::new();
        let first = runtime
            .begin_selection(first_root.path())
            .unwrap()
            .into_intent();
        let first_id = first.id();
        let second = runtime
            .begin_selection(second_root.path())
            .unwrap()
            .into_intent();
        let second_id = second.id();
        let (worker, receiver) = worker_channel();

        worker.prepare(first).unwrap();
        let (_, completion) = receive(&receiver);
        let ProfileWorkerCompletion::Prepared { selection, result } = completion else {
            panic!("expected prepared profile completion");
        };
        assert_eq!(selection, first_id);
        assert!(matches!(
            runtime.commit_selection(result.unwrap()),
            Err(ProfileRuntimeError::StaleSelection {
                expected: Some(expected),
                actual,
            }) if expected == second_id && actual == first_id
        ));

        worker.prepare(second).unwrap();
        let (_, completion) = receive(&receiver);
        let ProfileWorkerCompletion::Prepared { result, .. } = completion else {
            panic!("expected prepared profile completion");
        };
        runtime.commit_selection(result.unwrap()).unwrap();
    }
}
