from pathlib import Path


def replace_once(text: str, old: str, new: str, label: str) -> str:
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{label}: expected exactly one match, found {count}")
    return text.replace(old, new, 1)


windows_path = Path("src/platform/windows.rs")
windows = windows_path.read_text()

windows = replace_once(
    windows,
    "    ProfileLock, ProfileLockOwner, ProfileRuntime, ProfileRuntimeError, ProfileSelectionIntent,\n"
    "    ProfileSettingsSavePolicy, ProfileSettingsSaveScheduler, ProfileSettingsSaveUrgency,\n",
    "    ProfileLock, ProfileLockOwner, ProfileRuntime, ProfileRuntimeError, ProfileSelectionIntent,\n"
    "    ProfileSessionRestoreSavePolicy, ProfileSessionRestoreSaveScheduler,\n"
    "    ProfileSessionRestoreSaveUrgency, ProfileSettingsSavePolicy, ProfileSettingsSaveScheduler,\n"
    "    ProfileSettingsSaveUrgency,\n",
    "windows imports",
)

windows = replace_once(
    windows,
    'const BOOKMARKS_PERSISTENCE_SMOKE_LOCATION: &str =\n'
    '    "https://example.test/zorya-bookmark-persistence-smoke";\n',
    'const BOOKMARKS_PERSISTENCE_SMOKE_LOCATION: &str =\n'
    '    "https://example.test/zorya-bookmark-persistence-smoke";\n'
    'const SESSION_RESTORE_SAVE_DEBOUNCE_MILLIS: u64 = 1_000;\n'
    'const SESSION_RESTORE_SAVE_MAX_DIRTY_MILLIS: u64 = 10_000;\n'
    'const SESSION_RESTORE_SAVE_MUTATION_THRESHOLD: u64 = 8;\n'
    'const SESSION_RESTORE_PERSISTENCE_SMOKE_LOCATION: &str =\n'
    '    "https://example.test/zorya-session-restore-persistence-smoke";\n',
    "windows constants",
)

windows = replace_once(
    windows,
    'fn native_bookmarks_save_policy() -> ProfileBookmarksSavePolicy {\n'
    '    ProfileBookmarksSavePolicy::new(\n'
    '        BOOKMARKS_SAVE_DEBOUNCE_MILLIS,\n'
    '        BOOKMARKS_SAVE_MAX_DIRTY_MILLIS,\n'
    '        BOOKMARKS_SAVE_MUTATION_THRESHOLD,\n'
    '    )\n'
    '    .expect("native bookmarks save policy is valid")\n'
    '}\n',
    'fn native_bookmarks_save_policy() -> ProfileBookmarksSavePolicy {\n'
    '    ProfileBookmarksSavePolicy::new(\n'
    '        BOOKMARKS_SAVE_DEBOUNCE_MILLIS,\n'
    '        BOOKMARKS_SAVE_MAX_DIRTY_MILLIS,\n'
    '        BOOKMARKS_SAVE_MUTATION_THRESHOLD,\n'
    '    )\n'
    '    .expect("native bookmarks save policy is valid")\n'
    '}\n\n'
    'fn native_session_restore_save_policy() -> ProfileSessionRestoreSavePolicy {\n'
    '    ProfileSessionRestoreSavePolicy::new(\n'
    '        SESSION_RESTORE_SAVE_DEBOUNCE_MILLIS,\n'
    '        SESSION_RESTORE_SAVE_MAX_DIRTY_MILLIS,\n'
    '        SESSION_RESTORE_SAVE_MUTATION_THRESHOLD,\n'
    '    )\n'
    '    .expect("native session-restore save policy is valid")\n'
    '}\n',
    "native session policy",
)

windows = replace_once(
    windows,
    "    bookmarks_scheduler: ProfileBookmarksSaveScheduler,\n    profile_clock: NativeProfileClock,\n",
    "    bookmarks_scheduler: ProfileBookmarksSaveScheduler,\n"
    "    session_restore_scheduler: ProfileSessionRestoreSaveScheduler,\n"
    "    profile_clock: NativeProfileClock,\n",
    "native shell scheduler field",
)

windows = replace_once(
    windows,
    "            bookmarks_scheduler: ProfileBookmarksSaveScheduler::new(native_bookmarks_save_policy()),\n"
    "            profile_clock: NativeProfileClock::new(),\n",
    "            bookmarks_scheduler: ProfileBookmarksSaveScheduler::new(native_bookmarks_save_policy()),\n"
    "            session_restore_scheduler: ProfileSessionRestoreSaveScheduler::new(\n"
    "                native_session_restore_save_policy(),\n"
    "            ),\n"
    "            profile_clock: NativeProfileClock::new(),\n",
    "native shell scheduler init",
)

session_smoke = '''    fn run_session_restore_persistence_smoke(
        &mut self,
        event_loop: &ActiveEventLoop,
    ) -> Result<(), String> {
        if self.run_mode != RunMode::ExitAfterSessionRestorePersistence {
            return Err("session-restore persistence smoke started outside its run mode".into());
        }
        let profile = self.active_profile_id()?;
        let marker_count = self
            .profile_runtime
            .active_session_restore(profile)
            .map_err(|error| format!("failed to inspect active session restore: {error}"))?
            .windows()
            .iter()
            .flat_map(|window| window.tabs().iter())
            .filter(|tab| tab.location() == SESSION_RESTORE_PERSISTENCE_SMOKE_LOCATION)
            .count();
        if marker_count > 1 {
            return Err("session-restore persistence smoke found duplicate persisted markers".into());
        }
        if marker_count == 0 {
            let window = self
                .profile_runtime
                .add_session_window(profile)
                .map_err(|error| format!("failed to create session smoke window: {error}"))?;
            self.profile_runtime
                .add_session_tab(profile, window, SESSION_RESTORE_PERSISTENCE_SMOKE_LOCATION)
                .map_err(|error| format!("failed to create session smoke tab: {error}"))?;
        }
        self.shutdown(event_loop);
        Ok(())
    }

'''
windows = replace_once(
    windows,
    "    fn begin_profile_cycle_smoke_create(&mut self) -> Result<(), String> {\n",
    session_smoke + "    fn begin_profile_cycle_smoke_create(&mut self) -> Result<(), String> {\n",
    "session smoke method",
)

session_drive = '''    fn drive_session_restore_save(
        &mut self,
        urgency: ProfileSessionRestoreSaveUrgency,
    ) -> Result<(), String> {
        let Some(profile) = self
            .profile_runtime
            .active_profile()
            .map(|profile| profile.id())
        else {
            return Ok(());
        };
        let now_millis = self.profile_clock.now_millis();
        let Some(intent) = self
            .session_restore_scheduler
            .poll(&mut self.profile_runtime, profile, now_millis, urgency)
            .map_err(|error| format!("failed to schedule session-restore save: {error}"))?
        else {
            return Ok(());
        };
        let save = intent.id();
        let worker = self
            .profile_worker
            .as_ref()
            .ok_or_else(|| "profile worker is unavailable for session-restore save".to_string())?;

        match worker.save_session_restore(intent) {
            Ok(()) => Ok(()),
            Err(error) => {
                let queue_full = error.is_full();
                let message = error.to_string();
                let intent = error.into_work();
                debug_assert_eq!(intent.id(), save);
                self.profile_runtime
                    .cancel_session_restore_save(save)
                    .map_err(|cancel| {
                        format!(
                            "failed to submit session-restore save {}: {message}; failed to cancel pending save: {cancel}",
                            save.get()
                        )
                    })?;
                if queue_full {
                    Ok(())
                } else {
                    Err(format!(
                        "failed to submit session-restore save {}: {message}",
                        save.get()
                    ))
                }
            }
        }
    }

'''
windows = replace_once(
    windows,
    "    fn settings_flush_complete(&self) -> Result<bool, String> {\n",
    session_drive + "    fn settings_flush_complete(&self) -> Result<bool, String> {\n",
    "session save driver",
)

session_flush = '''    fn session_restore_flush_complete(&self) -> Result<bool, String> {
        let Some(profile) = self
            .profile_runtime
            .active_profile()
            .map(|profile| profile.id())
        else {
            return Ok(true);
        };
        if self.profile_runtime.pending_session_restore_save().is_some() {
            return Ok(false);
        }
        self.profile_runtime
            .session_restore_is_dirty(profile)
            .map(|dirty| !dirty)
            .map_err(|error| format!("failed to inspect session-restore dirty state: {error}"))
    }

'''
windows = replace_once(
    windows,
    "    fn profile_flush_complete(&self) -> Result<bool, String> {\n",
    session_flush + "    fn profile_flush_complete(&self) -> Result<bool, String> {\n",
    "session flush completion",
)

windows = replace_once(
    windows,
    "        Ok(self.settings_flush_complete()?\n"
    "            && self.history_flush_complete()?\n"
    "            && self.bookmarks_flush_complete()?)\n",
    "        Ok(self.settings_flush_complete()?\n"
    "            && self.history_flush_complete()?\n"
    "            && self.bookmarks_flush_complete()?\n"
    "            && self.session_restore_flush_complete()?)\n",
    "profile flush aggregate",
)

windows = replace_once(
    windows,
    "        let bookmarks_urgency = if flush {\n"
    "            ProfileBookmarksSaveUrgency::Flush\n"
    "        } else {\n"
    "            ProfileBookmarksSaveUrgency::Normal\n"
    "        };\n"
    "        self.drive_settings_save(settings_urgency)?;\n"
    "        self.drive_history_save(history_urgency)?;\n"
    "        self.drive_bookmarks_save(bookmarks_urgency)\n",
    "        let bookmarks_urgency = if flush {\n"
    "            ProfileBookmarksSaveUrgency::Flush\n"
    "        } else {\n"
    "            ProfileBookmarksSaveUrgency::Normal\n"
    "        };\n"
    "        let session_restore_urgency = if flush {\n"
    "            ProfileSessionRestoreSaveUrgency::Flush\n"
    "        } else {\n"
    "            ProfileSessionRestoreSaveUrgency::Normal\n"
    "        };\n"
    "        self.drive_settings_save(settings_urgency)?;\n"
    "        self.drive_history_save(history_urgency)?;\n"
    "        self.drive_bookmarks_save(bookmarks_urgency)?;\n"
    "        self.drive_session_restore_save(session_restore_urgency)\n",
    "profile save aggregate",
)

windows = replace_once(
    windows,
    "            self.bookmarks_scheduler.next_save_due_millis(),\n",
    "            self.bookmarks_scheduler.next_save_due_millis(),\n"
    "            self.session_restore_scheduler.next_save_due_millis(),\n",
    "profile save deadline",
)

session_completion = '''            ProfileWorkerCompletion::SessionRestoreSaved(completion) => {
                let save = completion.id();
                let storage_error = completion.result().as_ref().err().map(ToString::to_string);
                match self.profile_runtime.complete_session_restore_save(completion) {
                    Ok(_) => {
                        if let Some(error) = storage_error {
                            if self.fatal_error.is_none() {
                                self.fatal_error = Some(format!(
                                    "session-restore save {} failed on profile worker: {error}",
                                    save.get()
                                ));
                            }
                            self.begin_shutdown();
                            self.finish_shutdown(event_loop);
                            return;
                        }

                        let flush = self.shutdown_requested || self.replacement_flush_requested();
                        if let Err(error) = self.drive_profile_saves(flush) {
                            self.fail(event_loop, error);
                            return;
                        }
                        if self.shutdown_requested {
                            match self.profile_flush_complete() {
                                Ok(true) => self.continue_shutdown_after_profile_flush(event_loop),
                                Ok(false) => {}
                                Err(error) => self.fail(event_loop, error),
                            }
                        } else if self.pending_profile_replacement.is_some() {
                            match self.profile_flush_complete() {
                                Ok(true) => self.continue_profile_replacement(event_loop),
                                Ok(false) => {}
                                Err(error) => self.fail(event_loop, error),
                            }
                        }
                    }
                    Err(error) => self.fail(
                        event_loop,
                        format!(
                            "failed to reconcile session-restore save {}: {error}",
                            save.get()
                        ),
                    ),
                }
            }
'''
windows = replace_once(
    windows,
    '            ProfileWorkerCompletion::SessionRestoreSaved(_) => self.fail(\n'
    '                event_loop,\n'
    '                "unexpected session-restore-save completion arrived without native session persistence wiring",\n'
    '            ),\n',
    session_completion,
    "session completion reconciliation",
)

windows = replace_once(
    windows,
    "                        } else if self.run_mode == RunMode::ExitAfterBookmarksPersistence {\n"
    "                            if let Err(error) = self.run_bookmarks_persistence_smoke(event_loop) {\n"
    "                                self.fail(event_loop, error);\n"
    "                            }\n"
    "                        } else if matches!(\n",
    "                        } else if self.run_mode == RunMode::ExitAfterBookmarksPersistence {\n"
    "                            if let Err(error) = self.run_bookmarks_persistence_smoke(event_loop) {\n"
    "                                self.fail(event_loop, error);\n"
    "                            }\n"
    "                        } else if self.run_mode == RunMode::ExitAfterSessionRestorePersistence {\n"
    "                            if let Err(error) = self.run_session_restore_persistence_smoke(event_loop) {\n"
    "                                self.fail(event_loop, error);\n"
    "                            }\n"
    "                        } else if matches!(\n",
    "session smoke trigger",
)

windows = replace_once(
    windows,
    "            || self.profile_runtime.pending_bookmarks_save().is_some();\n",
    "            || self.profile_runtime.pending_bookmarks_save().is_some()\n"
    "            || self.profile_runtime.pending_session_restore_save().is_some();\n",
    "pending session save wait",
)

windows_path.write_text(windows)

# CI: run the native persistence smoke twice against one isolated profile root.
ci_path = Path(".github/workflows/ci.yml")
ci = ci_path.read_text()
ci_anchor = '''      - name: Smoke test native committed-page bookmark toggle persistence and reload
'''
ci_block = '''      - name: Smoke test native session restore persistence and reload
        shell: pwsh
        run: |
          $smokeRoot = Join-Path $env:RUNNER_TEMP "zorya-session-restore-persistence-debug"
          if (Test-Path -LiteralPath $smokeRoot) {
            Remove-Item -LiteralPath $smokeRoot -Recurse -Force
          }
          $previousLocalAppData = $env:LOCALAPPDATA
          $env:LOCALAPPDATA = $smokeRoot
          try {
            foreach ($attempt in 1..2) {
              $process = Start-Process -FilePath ".\\target\\debug\\zorya.exe" -ArgumentList "--native-session-restore-persistence-smoke" -PassThru -NoNewWindow
              if (-not $process.WaitForExit(60000)) {
                $process.Kill()
                throw "native session restore persistence smoke attempt $attempt timed out"
              }
              if ($process.ExitCode -ne 0) {
                throw "native session restore persistence smoke attempt $attempt failed with exit code $($process.ExitCode)"
              }
            }
          } finally {
            $env:LOCALAPPDATA = $previousLocalAppData
          }
'''
ci = replace_once(ci, ci_anchor, ci_block + ci_anchor, "CI session smoke")
ci_path.write_text(ci)

release_path = Path(".github/workflows/release-candidate.yml")
release = release_path.read_text()
release_anchor = '''      - name: Smoke test release committed-page bookmark toggle persistence and reload
'''
release_block = '''      - name: Smoke test release native session restore persistence and reload
        shell: pwsh
        run: |
          $executable = Join-Path "target/release" "zorya.exe"
          $smokeRoot = Join-Path $env:RUNNER_TEMP "zorya-session-restore-persistence-release"
          if (Test-Path -LiteralPath $smokeRoot) {
            Remove-Item -LiteralPath $smokeRoot -Recurse -Force
          }
          $previousLocalAppData = $env:LOCALAPPDATA
          $env:LOCALAPPDATA = $smokeRoot
          try {
            foreach ($attempt in 1..2) {
              $process = Start-Process -FilePath $executable -ArgumentList "--native-session-restore-persistence-smoke" -PassThru -NoNewWindow
              if (-not $process.WaitForExit(60000)) {
                $process.Kill()
                throw "release session restore persistence smoke attempt $attempt timed out"
              }
              if ($process.ExitCode -ne 0) {
                throw "release session restore persistence smoke attempt $attempt failed with exit code $($process.ExitCode)"
              }
            }
          } finally {
            $env:LOCALAPPDATA = $previousLocalAppData
          }

'''
release = replace_once(release, release_anchor, release_block + release_anchor, "RC session smoke")

package_anchor = '''          & ".github/scripts/smoke-bookmark-toggle.ps1" `
'''
package_block = '''          $sessionSmokeRoot = Join-Path $env:RUNNER_TEMP "zorya-session-restore-persistence-package"
          if (Test-Path -LiteralPath $sessionSmokeRoot) {
            Remove-Item -LiteralPath $sessionSmokeRoot -Recurse -Force
          }
          $previousLocalAppData = $env:LOCALAPPDATA
          $env:LOCALAPPDATA = $sessionSmokeRoot
          try {
            foreach ($attempt in 1..2) {
              $process = Start-Process -FilePath $packagedExecutable -ArgumentList "--native-session-restore-persistence-smoke" -PassThru -NoNewWindow
              if (-not $process.WaitForExit(60000)) {
                $process.Kill()
                throw "packaged session restore persistence smoke attempt $attempt timed out"
              }
              if ($process.ExitCode -ne 0) {
                throw "packaged session restore persistence smoke attempt $attempt failed with exit code $($process.ExitCode)"
              }
            }
          } finally {
            $env:LOCALAPPDATA = $previousLocalAppData
          }

'''
release = replace_once(release, package_anchor, package_block + package_anchor, "packaged session smoke")
release_path.write_text(release)

architecture_path = Path("docs/ARCHITECTURE.md")
architecture = architecture_path.read_text()
architecture = replace_once(
    architecture,
    "Normal native shutdown first flushes settings/history/bookmarks, then submits exact lock release",
    "Normal native shutdown first flushes settings/history/bookmarks/session restore, then submits exact lock release",
    "architecture lock flush",
)
architecture = replace_once(
    architecture,
    "Only one session save may be in flight, stale completion/cancellation cannot clear newer ownership, profile replacement is blocked until session state is durable, and synchronous save execution is handed only to the bounded `ProfileWorker`. Scheduling and native startup restoration remain later Z3 work.",
    "Only one session save may be in flight, stale completion/cancellation cannot clear newer ownership, profile replacement is blocked until session state is durable, and synchronous save execution is handed only to the bounded `ProfileWorker`. `ProfileSessionRestoreSaveScheduler` adds the same clock-injected debounce / maximum-dirty-age / mutation-threshold / explicit-flush contract used by the other profile stores. Windows owns a 1,000 ms / 10,000 ms / 8-mutation session policy, submits exact session save intents non-blockingly through `ProfileWorker`, cancels only exact in-flight ownership on queue failure, reconciles typed completions, includes session durability in profile replacement and graceful shutdown before lock release, and verifies persistence/reload through a deterministic native smoke without UI-thread filesystem I/O. Native browser-state synchronization and startup restoration remain later Z3 work.",
    "architecture session lifecycle",
)
architecture = replace_once(
    architecture,
    "Windows owns concrete bounded persistence policies for profile settings, browsing history and bookmarks. Settings and bookmarks use a 1,000 ms debounce, 10,000 ms maximum dirty age and 8-mutation threshold; history uses 5,000 ms / 30,000 ms / 32 mutations. All three schedulers share one process-local `Instant` origin, and `ApplicationHandler::about_to_wait` waits until the earliest observed deadline with `ControlFlow::WaitUntil`; any in-flight profile save uses `ControlFlow::Wait`, and `ControlFlow::Poll` is never used for persistence. Settings, history and bookmark intents are submitted non-blockingly through the same bounded `ProfileWorker`; queue failure cancels only the exact runtime ownership so dirty mutations remain retryable. Graceful shutdown requests explicit flush urgency for all three stores and exits only after all three exact lifecycles are clean or a persistence failure is reported. Navigation and future settings/bookmark callbacks mutate only in-memory product state and never execute profile filesystem work.",
    "Windows owns concrete bounded persistence policies for profile settings, browsing history, bookmarks and session restore. Settings, bookmarks and session restore use a 1,000 ms debounce, 10,000 ms maximum dirty age and 8-mutation threshold; history uses 5,000 ms / 30,000 ms / 32 mutations. All four schedulers share one process-local `Instant` origin, and `ApplicationHandler::about_to_wait` waits until the earliest observed deadline with `ControlFlow::WaitUntil`; any in-flight profile save uses `ControlFlow::Wait`, and `ControlFlow::Poll` is never used for persistence. Settings, history, bookmark and session intents are submitted non-blockingly through the same bounded `ProfileWorker`; queue failure cancels only the exact runtime ownership so dirty mutations remain retryable. Graceful shutdown requests explicit flush urgency for all four stores and exits only after all four exact lifecycles are clean or a persistence failure is reported. Navigation and future browser-state/settings/bookmark callbacks mutate only in-memory product state and never execute profile filesystem work.",
    "architecture native policies",
)
architecture = replace_once(
    architecture,
    "requests scheduler `Flush` urgency and keeps the event loop waiting for exact typed settings/history/bookmark completions.",
    "requests scheduler `Flush` urgency and keeps the event loop waiting for exact typed settings/history/bookmark/session completions.",
    "architecture shutdown completions",
)
architecture = replace_once(
    architecture,
    "Native profile creation/rename UX, profile deletion, private-browsing history policy, native bookmark mutation UX, session stores, broader settings wiring and migrations beyond schema v1 remain separate Z3 slices.",
    "Native profile creation/rename UX, profile deletion, private-browsing history policy, broader bookmark/session UX, native browser-state session synchronization, startup session restoration, broader settings wiring and migrations beyond schema v1 remain separate Z3 slices.",
    "architecture remaining work",
)
architecture_path.write_text(architecture)

roadmap_path = Path("docs/ROADMAP.md")
roadmap = roadmap_path.read_text()
roadmap = replace_once(
    roadmap,
    "an exact-profile runtime lifecycle now tracks monotonic mutation/durable revisions, one-in-flight save identity, captured generations and newer-mutation preservation, keeps failed/cancelled work dirty, blocks profile replacement until durable, and executes synchronous saves only through the bounded `ProfileWorker` with exact queue-failure recovery; scheduling and native startup restore remain later Z3 work;",
    "an exact-profile runtime lifecycle now tracks monotonic mutation/durable revisions, one-in-flight save identity, captured generations and newer-mutation preservation, keeps failed/cancelled work dirty, blocks profile replacement until durable, and executes synchronous saves only through the bounded `ProfileWorker` with exact queue-failure recovery; a clock-injected scheduler now coalesces session saves by debounce/max-age/mutation threshold plus explicit flush, Windows drives its deadline together with the other profile stores through `WaitUntil`, submits and reconciles exact worker saves, cancels exact ownership on submission failure, includes session durability in replacement and graceful shutdown, and proves native persistence/reload without UI-thread filesystem I/O; native browser-state synchronization and startup restore remain later Z3 work;",
    "roadmap session status",
)
roadmap_path.write_text(roadmap)
