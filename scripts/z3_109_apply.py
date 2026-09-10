from pathlib import Path


def read(path: str) -> str:
    return Path(path).read_text(encoding="utf-8")


def write(path: str, text: str) -> None:
    Path(path).write_text(text, encoding="utf-8")


def replace_once(path: str, old: str, new: str) -> None:
    text = read(path)
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one exact marker, found {count}: {old[:120]!r}")
    write(path, text.replace(old, new, 1))


def insert_before_once(path: str, marker: str, block: str) -> None:
    replace_once(path, marker, block + marker)


def append_inside_final_brace(path: str, block: str) -> None:
    text = read(path)
    pos = text.rfind("\n}")
    if pos < 0:
        raise SystemExit(f"{path}: final module brace not found")
    write(path, text[:pos] + block + text[pos:])


# Portable bookmark primitive: exact-location duplicate removal cannot fail after
# the runtime has reserved one mutation revision.
insert_before_once(
    "src/bookmarks.rs",
    "    pub fn clear(&mut self) {\n",
    """    pub(crate) fn remove_bookmarks_at_exact_location(&mut self, location: &str) -> usize {
        let previous_len = self.bookmarks.len();
        self.bookmarks
            .retain(|bookmark| bookmark.location() != location);
        previous_len - self.bookmarks.len()
    }

""",
)

# Typed portable outcome lives next to the bookmark save/runtime lifecycle.
insert_before_once(
    "src/profile_runtime/bookmarks_persistence.rs",
    "#[derive(Debug, PartialEq, Eq)]\npub struct ProfileBookmarksSaveIntent {\n",
    """#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileBookmarkToggleOutcome {
    Added { bookmark: BookmarkId },
    Removed { count: usize },
}

""",
)

insert_before_once(
    "src/profile_runtime/bookmarks_persistence.rs",
    "    pub fn add_bookmark(\n",
    """    pub fn toggle_bookmark(
        &mut self,
        profile: ProfileId,
        title: impl Into<String>,
        location: impl Into<String>,
    ) -> Result<ProfileBookmarkToggleOutcome, ProfileBookmarksRuntimeError> {
        let location = location.into();
        let state = self.active_bookmarks_state_mut(profile)?;
        let matching = state
            .snapshot
            .bookmarks()
            .iter()
            .filter(|bookmark| bookmark.location() == location.as_str())
            .count();
        let next_revision = state
            .mutation_revision
            .checked_add(1)
            .ok_or(ProfileBookmarksRuntimeError::MutationRevisionExhausted)?;

        let outcome = if matching == 0 {
            let bookmark = state
                .snapshot
                .add_bookmark(title, location)
                .map_err(ProfileBookmarksRuntimeError::Bookmarks)?;
            ProfileBookmarkToggleOutcome::Added { bookmark }
        } else {
            let count = state
                .snapshot
                .remove_bookmarks_at_exact_location(&location);
            debug_assert_eq!(count, matching);
            ProfileBookmarkToggleOutcome::Removed { count }
        };
        state.mutation_revision = next_revision;
        Ok(outcome)
    }

""",
)

append_inside_final_brace(
    "src/profile_runtime/bookmarks_persistence.rs",
    """

    #[test]
    fn exact_location_toggle_adds_and_removes_duplicates_in_one_revision() {
        let root = TempRoot::new();
        let mut runtime = ProfileRuntime::new();
        let profile = load_profile(&mut runtime, root.path());
        let location = "https://example.test/toggle";

        let first = runtime.toggle_bookmark(profile, "", location).unwrap();
        let first_id = match first {
            ProfileBookmarkToggleOutcome::Added { bookmark } => bookmark,
            other => panic!("unexpected first toggle outcome: {other:?}"),
        };
        assert_eq!(first_id.get(), 1);
        assert_eq!(runtime.bookmarks_mutation_revision(profile).unwrap(), 1);

        let survivor = runtime
            .add_bookmark(profile, "Keep", "https://example.test/keep")
            .unwrap();
        let duplicate = runtime.add_bookmark(profile, "Duplicate", location).unwrap();
        assert_eq!(survivor.get(), 2);
        assert_eq!(duplicate.get(), 3);
        assert_eq!(runtime.bookmarks_mutation_revision(profile).unwrap(), 3);

        assert_eq!(
            runtime.toggle_bookmark(profile, "ignored on removal", location),
            Ok(ProfileBookmarkToggleOutcome::Removed { count: 2 })
        );
        assert_eq!(runtime.bookmarks_mutation_revision(profile).unwrap(), 4);
        let remaining = runtime.active_bookmarks(profile).unwrap().bookmarks();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id(), survivor);
        assert_eq!(remaining[0].location(), "https://example.test/keep");

        let added = runtime.toggle_bookmark(profile, "", location).unwrap();
        let added_id = match added {
            ProfileBookmarkToggleOutcome::Added { bookmark } => bookmark,
            other => panic!("unexpected re-add outcome: {other:?}"),
        };
        assert_eq!(added_id.get(), 4);
        assert_eq!(runtime.bookmarks_mutation_revision(profile).unwrap(), 5);
        assert_eq!(runtime.active_bookmarks(profile).unwrap().len(), 2);
    }

    #[test]
    fn toggle_rejects_stale_profile_without_retargeting_replacement() {
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
            runtime.toggle_bookmark(first, "", "https://example.test/stale-toggle"),
            Err(ProfileBookmarksRuntimeError::StaleProfile {
                expected: Some(expected),
                actual,
            }) if expected == second && actual == first
        ));
        assert!(runtime.active_bookmarks(second).unwrap().is_empty());
        assert_eq!(runtime.bookmarks_mutation_revision(second).unwrap(), 0);
    }

    #[test]
    fn toggle_revision_exhaustion_is_failure_atomic_for_add_and_duplicate_removal() {
        let add_root = TempRoot::new();
        let mut add_runtime = ProfileRuntime::new();
        let add_profile = load_profile(&mut add_runtime, add_root.path());
        add_runtime.active.as_mut().unwrap().bookmarks.mutation_revision = u64::MAX;

        assert_eq!(
            add_runtime.toggle_bookmark(add_profile, "", "https://example.test/exhausted-add"),
            Err(ProfileBookmarksRuntimeError::MutationRevisionExhausted)
        );
        assert!(add_runtime.active_bookmarks(add_profile).unwrap().is_empty());

        let remove_root = TempRoot::new();
        let mut remove_runtime = ProfileRuntime::new();
        let remove_profile = load_profile(&mut remove_runtime, remove_root.path());
        let location = "https://example.test/exhausted-remove";
        remove_runtime
            .active
            .as_mut()
            .unwrap()
            .bookmarks
            .snapshot
            .add_bookmark("First", location)
            .unwrap();
        remove_runtime
            .active
            .as_mut()
            .unwrap()
            .bookmarks
            .snapshot
            .add_bookmark("Second", location)
            .unwrap();
        remove_runtime.active.as_mut().unwrap().bookmarks.mutation_revision = u64::MAX;
        let before = remove_runtime.active_bookmarks(remove_profile).unwrap().clone();

        assert_eq!(
            remove_runtime.toggle_bookmark(remove_profile, "", location),
            Err(ProfileBookmarksRuntimeError::MutationRevisionExhausted)
        );
        assert_eq!(remove_runtime.active_bookmarks(remove_profile).unwrap(), &before);
    }
""",
)

replace_once(
    "src/profile_runtime.rs",
    """pub use bookmarks_persistence::{
    ProfileBookmarksRuntimeError, ProfileBookmarksSaveCompletion, ProfileBookmarksSaveId,
    ProfileBookmarksSaveIntent,
};""",
    """pub use bookmarks_persistence::{
    ProfileBookmarkToggleOutcome, ProfileBookmarksRuntimeError, ProfileBookmarksSaveCompletion,
    ProfileBookmarksSaveId, ProfileBookmarksSaveIntent,
};""",
)

replace_once(
    "src/lib.rs",
    """    ActiveProfile, ColorSchemePreference, PreparedProfile, ProductSettings,
    ProfileBookmarksRuntimeError, ProfileBookmarksSaveCompletion, ProfileBookmarksSaveId,
    ProfileBookmarksSaveIntent, ProfileHistorySaveCompletion,""",
    """    ActiveProfile, ColorSchemePreference, PreparedProfile, ProductSettings,
    ProfileBookmarkToggleOutcome, ProfileBookmarksRuntimeError, ProfileBookmarksSaveCompletion,
    ProfileBookmarksSaveId, ProfileBookmarksSaveIntent, ProfileHistorySaveCompletion,""",
)

# Portable browser boundary for the native shortcut. Deliberately bypasses both
# display_location() (which can expose pending navigation) and address-bar edit text.
insert_before_once(
    "src/app.rs",
    "    pub const fn address_bar(&self) -> &AddressBarState {\n",
    """    pub fn active_committed_location(&self) -> Option<&str> {
        self.active_tab()?
            .navigation()
            .current_entry()
            .map(HistoryEntry::location)
    }

""",
)

append_inside_final_brace(
    "src/app.rs",
    """

    #[test]
    fn active_committed_location_ignores_pending_navigation_and_address_edit_text() {
        let mut app = BrowserApp::new();
        let window = app.create_window().expect("window");
        let tab = app.create_tab(window).expect("tab");
        let first = app
            .begin_navigation(window, tab, "https://example.test/committed")
            .expect("begin first")
            .intent()
            .id();
        app.commit_navigation(window, tab, first, "https://example.test/committed")
            .expect("commit first");

        assert_eq!(
            app.window(window)
                .and_then(BrowserWindow::active_committed_location),
            Some("https://example.test/committed")
        );

        app.begin_address_bar_edit(window)
            .expect("begin edit")
            .expect("active tab");
        app.set_address_bar_text(window, "https://typed.invalid/not-a-target")
            .expect("set edit");
        let pending = app
            .begin_navigation(window, tab, "https://example.test/pending")
            .expect("begin pending")
            .intent()
            .id();

        let browser_window = app.window(window).expect("window");
        assert_eq!(
            browser_window.active_committed_location(),
            Some("https://example.test/committed")
        );
        assert_eq!(
            browser_window.address_bar_text(),
            "https://typed.invalid/not-a-target"
        );
        assert_eq!(
            browser_window
                .active_tab()
                .and_then(|tab| tab.navigation().display_location()),
            Some("https://example.test/pending")
        );

        app.commit_navigation(window, tab, pending, "https://example.test/pending-final")
            .expect("commit pending");
        let browser_window = app.window(window).expect("window");
        assert_eq!(
            browser_window.active_committed_location(),
            Some("https://example.test/pending-final")
        );
        assert_eq!(
            browser_window.address_bar_text(),
            "https://typed.invalid/not-a-target"
        );
    }
""",
)

replace_once(
    "src/platform/mod.rs",
    """    ExitAfterColorSchemeCycle,
    ExitAfterBookmarksPersistence,
}""",
    """    ExitAfterColorSchemeCycle,
    ExitAfterBookmarksPersistence,
    ExitAfterBookmarkToggleAdd,
    ExitAfterBookmarkToggleRemove,
}""",
)

insert_before_once(
    "src/lib.rs",
    "pub fn run_native_bookmarks_persistence_smoke() -> Result<(), Box<dyn std::error::Error>> {\n",
    "",
)
# Add new smoke entry points after the existing persistence smoke function.
replace_once(
    "src/lib.rs",
    """pub fn run_native_bookmarks_persistence_smoke() -> Result<(), Box<dyn std::error::Error>> {
    platform::run(platform::RunMode::ExitAfterBookmarksPersistence)
}""",
    """pub fn run_native_bookmarks_persistence_smoke() -> Result<(), Box<dyn std::error::Error>> {
    platform::run(platform::RunMode::ExitAfterBookmarksPersistence)
}

#[doc(hidden)]
pub fn run_native_bookmark_toggle_add_smoke() -> Result<(), Box<dyn std::error::Error>> {
    platform::run(platform::RunMode::ExitAfterBookmarkToggleAdd)
}

#[doc(hidden)]
pub fn run_native_bookmark_toggle_remove_smoke() -> Result<(), Box<dyn std::error::Error>> {
    platform::run(platform::RunMode::ExitAfterBookmarkToggleRemove)
}""",
)

replace_once(
    "src/main.rs",
    """        (Some(argument), None) if argument == "--native-bookmarks-persistence-smoke" => {
            finish(zorya::run_native_bookmarks_persistence_smoke())
        }
        (None, None) => finish(zorya::run()),""",
    """        (Some(argument), None) if argument == "--native-bookmarks-persistence-smoke" => {
            finish(zorya::run_native_bookmarks_persistence_smoke())
        }
        (Some(argument), None) if argument == "--native-bookmark-toggle-add-smoke" => {
            finish(zorya::run_native_bookmark_toggle_add_smoke())
        }
        (Some(argument), None) if argument == "--native-bookmark-toggle-remove-smoke" => {
            finish(zorya::run_native_bookmark_toggle_remove_smoke())
        }
        (None, None) => finish(zorya::run()),""",
)

replace_once(
    "src/platform/windows.rs",
    """    PresentationGeneration, PresentationHandoffError, ProfileBookmarksSavePolicy,
    ProfileBookmarksSaveScheduler, ProfileBookmarksSaveUrgency, ProfileCatalogCreateIntent,""",
    """    PresentationGeneration, PresentationHandoffError, ProfileBookmarkToggleOutcome,
    ProfileBookmarksSavePolicy, ProfileBookmarksSaveScheduler, ProfileBookmarksSaveUrgency,
    ProfileCatalogCreateIntent,""",
)

insert_before_once(
    "src/platform/windows.rs",
    "    fn run_bookmarks_persistence_smoke(\n",
    """    fn toggle_active_committed_bookmark(
        &mut self,
    ) -> Result<Option<ProfileBookmarkToggleOutcome>, String> {
        let location = self
            .browser
            .window(self.browser_window)
            .and_then(|window| window.active_committed_location())
            .map(str::to_owned);
        let Some(location) = location else {
            return Ok(None);
        };
        let profile = self.active_profile_id()?;
        let outcome = self
            .profile_runtime
            .toggle_bookmark(profile, "", location)
            .map_err(|error| format!("failed to toggle active committed bookmark: {error}"))?;
        self.drive_bookmarks_save(ProfileBookmarksSaveUrgency::Normal)?;
        Ok(Some(outcome))
    }

    fn run_bookmark_toggle_smoke(
        &mut self,
        event_loop: &ActiveEventLoop,
    ) -> Result<(), String> {
        let expects_existing = match self.run_mode {
            RunMode::ExitAfterBookmarkToggleAdd => false,
            RunMode::ExitAfterBookmarkToggleRemove => true,
            _ => return Err("bookmark toggle smoke started outside its run mode".into()),
        };
        let location = self
            .browser
            .window(self.browser_window)
            .and_then(|window| window.active_committed_location())
            .ok_or_else(|| "bookmark toggle smoke requires a committed active page".to_string())?
            .to_owned();
        if location != START_LOCATION {
            return Err(format!(
                "bookmark toggle smoke expected committed {START_LOCATION}, found {location}"
            ));
        }

        let profile = self.active_profile_id()?;
        let before_count = self
            .profile_runtime
            .active_bookmarks(profile)
            .map_err(|error| format!("failed to inspect bookmark toggle smoke state: {error}"))?
            .bookmarks()
            .iter()
            .filter(|bookmark| bookmark.location() == location.as_str())
            .count();
        let expected_before = usize::from(expects_existing);
        if before_count != expected_before {
            return Err(format!(
                "bookmark toggle smoke expected {expected_before} persisted matches before toggle, found {before_count}"
            ));
        }
        let before_revision = self
            .profile_runtime
            .bookmarks_mutation_revision(profile)
            .map_err(|error| format!("failed to inspect bookmark toggle revision: {error}"))?;

        let outcome = self
            .toggle_active_committed_bookmark()?
            .ok_or_else(|| "bookmark toggle smoke lost its committed target".to_string())?;
        match (expects_existing, outcome) {
            (false, ProfileBookmarkToggleOutcome::Added { bookmark }) => {
                let created = self
                    .profile_runtime
                    .active_bookmarks(profile)
                    .map_err(|error| format!("failed to inspect added bookmark: {error}"))?
                    .bookmark(bookmark)
                    .ok_or_else(|| "bookmark toggle smoke lost newly added bookmark".to_string())?;
                if !created.title().is_empty() || created.location() != location {
                    return Err(
                        "bookmark toggle smoke synthesized title or changed committed target".into(),
                    );
                }
            }
            (true, ProfileBookmarkToggleOutcome::Removed { count: 1 }) => {}
            (_, outcome) => {
                return Err(format!(
                    "bookmark toggle smoke returned unexpected outcome: {outcome:?}"
                ));
            }
        }

        let expected_revision = before_revision
            .checked_add(1)
            .ok_or_else(|| "bookmark toggle smoke revision overflowed".to_string())?;
        let actual_revision = self
            .profile_runtime
            .bookmarks_mutation_revision(profile)
            .map_err(|error| format!("failed to inspect bookmark toggle revision: {error}"))?;
        if actual_revision != expected_revision {
            return Err(format!(
                "bookmark toggle smoke advanced revision to {actual_revision}; expected {expected_revision}"
            ));
        }
        let after_count = self
            .profile_runtime
            .active_bookmarks(profile)
            .map_err(|error| format!("failed to inspect bookmark toggle result: {error}"))?
            .bookmarks()
            .iter()
            .filter(|bookmark| bookmark.location() == location.as_str())
            .count();
        let expected_after = usize::from(!expects_existing);
        if after_count != expected_after {
            return Err(format!(
                "bookmark toggle smoke expected {expected_after} matches after toggle, found {after_count}"
            ));
        }

        self.shutdown(event_loop);
        Ok(())
    }

""",
)

replace_once(
    "src/platform/windows.rs",
    """            Key::Character(character)
                if !self.modifiers.shift_key() && character.as_str().eq_ignore_ascii_case("r") =>
            {
                self.handle_browser_command(BrowserCommand::ReloadOrStop)
            }
            Key::Character(character)
                if self.modifiers.shift_key() && character.as_str().eq_ignore_ascii_case("m") =>""",
    """            Key::Character(character)
                if !self.modifiers.shift_key() && character.as_str().eq_ignore_ascii_case("r") =>
            {
                self.handle_browser_command(BrowserCommand::ReloadOrStop)
            }
            Key::Character(character)
                if !self.modifiers.shift_key() && character.as_str().eq_ignore_ascii_case("d") =>
            {
                self.toggle_active_committed_bookmark().map(|_| ())
            }
            Key::Character(character)
                if self.modifiers.shift_key() && character.as_str().eq_ignore_ascii_case("m") =>""",
)

replace_once(
    "src/platform/windows.rs",
    """                        } else if self.run_mode == RunMode::ExitAfterBookmarksPersistence {
                            if let Err(error) = self.run_bookmarks_persistence_smoke(event_loop) {
                                self.fail(event_loop, error);
                            }
                        } else if self.run_mode == RunMode::ExitAfterFirstPresentation {""",
    """                        } else if self.run_mode == RunMode::ExitAfterBookmarksPersistence {
                            if let Err(error) = self.run_bookmarks_persistence_smoke(event_loop) {
                                self.fail(event_loop, error);
                            }
                        } else if matches!(
                            self.run_mode,
                            RunMode::ExitAfterBookmarkToggleAdd
                                | RunMode::ExitAfterBookmarkToggleRemove
                        ) {
                            if let Err(error) = self.run_bookmark_toggle_smoke(event_loop) {
                                self.fail(event_loop, error);
                            }
                        } else if self.run_mode == RunMode::ExitAfterFirstPresentation {""",
)

# Shared permanent two-process smoke helper avoids copy/paste drift among debug,
# release-candidate, publication, and packaged verification.
smoke_script = r'''param(
    [Parameter(Mandatory = $true)]
    [string]$Executable,
    [Parameter(Mandatory = $true)]
    [string]$ScratchRoot
)

$ErrorActionPreference = "Stop"
$resolvedExecutable = (Resolve-Path -LiteralPath $Executable).Path
if (Test-Path -LiteralPath $ScratchRoot) {
    Remove-Item -LiteralPath $ScratchRoot -Recurse -Force
}

$previousLocalAppData = $env:LOCALAPPDATA
$env:LOCALAPPDATA = $ScratchRoot
try {
    foreach ($argument in @(
        "--native-bookmark-toggle-add-smoke",
        "--native-bookmark-toggle-remove-smoke"
    )) {
        $process = Start-Process -FilePath $resolvedExecutable -ArgumentList $argument -PassThru -NoNewWindow
        if (-not $process.WaitForExit(60000)) {
            $process.Kill()
            throw "bookmark toggle smoke $argument timed out"
        }
        if ($process.ExitCode -ne 0) {
            throw "bookmark toggle smoke $argument failed with exit code $($process.ExitCode)"
        }
    }
} finally {
    $env:LOCALAPPDATA = $previousLocalAppData
}
'''
smoke_path = Path(".github/scripts/smoke-bookmark-toggle.ps1")
if smoke_path.exists():
    raise SystemExit(f"{smoke_path}: unexpected pre-existing path")
smoke_path.write_text(smoke_script, encoding="utf-8")

ci_step = '''      - name: Smoke test native committed-page bookmark toggle persistence and reload
        shell: pwsh
        run: |
          & ".github/scripts/smoke-bookmark-toggle.ps1" `
            -Executable ".\\target\\debug\\zorya.exe" `
            -ScratchRoot (Join-Path $env:RUNNER_TEMP "zorya-bookmark-toggle-debug")

'''
insert_before_once(
    ".github/workflows/ci.yml",
    "      - name: Upload developer executable\n",
    ci_step,
)

release_step = '''      - name: Smoke test release committed-page bookmark toggle persistence and reload
        shell: pwsh
        run: |
          & ".github/scripts/smoke-bookmark-toggle.ps1" `
            -Executable (Join-Path "target/release" "zorya.exe") `
            -ScratchRoot (Join-Path $env:RUNNER_TEMP "zorya-bookmark-toggle-release")

'''
insert_before_once(
    ".github/workflows/release-candidate.yml",
    "      - name: Package release candidate\n",
    release_step,
)

packaged_step = '''          & ".github/scripts/smoke-bookmark-toggle.ps1" `
            -Executable $packagedExecutable `
            -ScratchRoot (Join-Path $env:RUNNER_TEMP "zorya-bookmark-toggle-package")

'''
insert_before_once(
    ".github/workflows/release-candidate.yml",
    '          $buildInfoPath = Join-Path $packageRoot "BUILD-INFO.txt"\n',
    packaged_step,
)

insert_before_once(
    ".github/workflows/release.yml",
    "      - name: Package Technical Preview\n",
    release_step.replace("zorya-bookmark-toggle-release", "zorya-bookmark-toggle-publish"),
)
insert_before_once(
    ".github/workflows/release.yml",
    '          $buildInfoPath = Join-Path $packageRoot "BUILD-INFO.txt"\n',
    packaged_step.replace(
        "zorya-bookmark-toggle-package", "zorya-bookmark-toggle-publish-package"
    ),
)

replace_once(
    "docs/ROADMAP.md",
    "and includes bookmark durability in replacement and graceful shutdown; native bookmark mutation UX remains;",
    "and includes bookmark durability in replacement and graceful shutdown; `Ctrl+D` now toggles only the active tab's exact committed history location against the exact active `ProfileId`, never pending navigation or editable address-bar text, creates an empty-title bookmark until Rarog exposes a supported page-title contract, removes all exact-location duplicates as one failure-atomic mutation revision while preserving unrelated bookmark IDs, and persists only through the existing scheduler → bounded `ProfileWorker` path; broader bookmark-management UX remains;",
)

replace_once(
    "docs/ARCHITECTURE.md",
    "Native bookmark mutation UX remains later Z3 work.",
    "Windows now exposes the first native bookmark mutation through `Ctrl+D`. The shortcut derives its target only from the active tab's committed `HistoryEntry`; pending navigation display text and editable address-bar text are deliberately excluded. The exact active `ProfileId` is validated before mutation. If no bookmark has the exact committed location, the runtime adds one with an empty title until Rarog exposes a supported page-title contract; if matches exist, all exact-location duplicates are removed in one failure-atomic mutation revision while unrelated bookmark IDs and order remain stable. The callback mutates only in-memory product state and hands durability to the existing bookmark scheduler and bounded `ProfileWorker`; it performs no filesystem work and introduces no polling.",
)
