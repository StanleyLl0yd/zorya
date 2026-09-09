use crate::profile_runtime::{
    PreparedProfile, ProfileHistorySaveCompletion, ProfileHistorySaveIntent, ProfilePreparationError,
    ProfileSelectionId, ProfileSelectionIntent,
};
use std::fmt;
use std::io;
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::thread::{self, JoinHandle};

pub const PROFILE_WORKER_COMMAND_QUEUE_CAPACITY: usize = 1;

enum ProfileWorkerCommand {
    Prepare(ProfileSelectionIntent),
    SaveHistory(ProfileHistorySaveIntent),
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProfileWorkerCompletion {
    Prepared {
        selection: ProfileSelectionId,
        result: Result<PreparedProfile, ProfilePreparationError>,
    },
    HistorySaved(ProfileHistorySaveCompletion),
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
        write!(formatter, "failed to start profile worker: {}", self.message)
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
            Err(TrySendError::Full(ProfileWorkerCommand::SaveHistory(_)))
            | Err(TrySendError::Disconnected(ProfileWorkerCommand::SaveHistory(_))) => {
                unreachable!("prepare submission preserves its command variant")
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
            | Err(TrySendError::Disconnected(ProfileWorkerCommand::Prepare(_))) => {
                unreachable!("history-save submission preserves its command variant")
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
            ProfileWorkerCommand::SaveHistory(intent) => {
                ProfileWorkerCompletion::HistorySaved(intent.execute())
            }
        };
        completion_handler(completion);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ProfileRuntime, ProfileRuntimeError};
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

    fn worker_channel() -> (ProfileWorker, mpsc::Receiver<(String, ProfileWorkerCompletion)>) {
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
        assert_eq!(runtime.active_browsing_history(profile).unwrap().generation(), 1);
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
