mod app;
#[cfg(any(target_os = "windows", test))]
mod async_lifecycle;
#[cfg(target_os = "windows")]
mod branding;
mod bookmarks;
mod browsing_history;
mod chrome;
mod commands;
pub mod engine;
mod http_transport;
mod navigation;
mod platform;
mod presentation_handoff;
mod profile;
mod profile_catalog;
mod profile_history_scheduler;
mod profile_lock;
mod profile_metadata;
mod profile_runtime;
mod profile_settings_scheduler;
mod profile_worker;
mod tab_activation;
mod tab_close;
mod tab_strip;

pub use app::{
    BrowserApp, BrowserModelError, BrowserNavigationCommit, BrowserWindow, BrowserWindowId, Tab,
    TabCloseResult, TabId,
};
pub use bookmarks::{
    BOOKMARKS_SCHEMA_VERSION, Bookmark, BookmarkId, BookmarksCleanupWarning, BookmarksError,
    BookmarksLoad, BookmarksRecovery, BookmarksSave, BookmarksSnapshot, BookmarksStore,
    MAX_BOOKMARK_LOCATION_BYTES, MAX_BOOKMARK_TITLE_BYTES, MAX_BOOKMARKS,
    MAX_BOOKMARKS_RECORD_BYTES,
};
pub use browsing_history::{
    BROWSING_HISTORY_SCHEMA_VERSION, BrowsingHistoryCleanupWarning, BrowsingHistoryError,
    BrowsingHistoryLoad, BrowsingHistoryRecord, BrowsingHistoryRecovery, BrowsingHistorySave,
    BrowsingHistorySnapshot, BrowsingHistoryStore, BrowsingHistoryVisit, BrowsingHistoryVisitId,
    MAX_BROWSING_HISTORY_RECORD_BYTES, MAX_BROWSING_HISTORY_VISITS, MAX_HISTORY_LOCATION_BYTES,
};
pub use chrome::{AddressBarEdit, AddressBarState, AddressBarSubmission, WindowChromeSnapshot};
pub use commands::{BrowserCommand, BrowserCommandEffect};
pub use navigation::{
    HistoryEntry, HistoryEntryId, NavigationControls, NavigationFailure, NavigationId,
    NavigationIntent, NavigationIntentKind, NavigationStart, ReloadControl, TabNavigation,
};
pub use presentation_handoff::{
    CurrentFramePermit, PresentationFramePermit, PresentationGeneration, PresentationHandoffError,
    PresentationTransitionStart, TabPresentationHandoff, TargetFramePermit, WebContentPresentation,
};
pub use profile::{
    MAX_SETTING_KEY_BYTES, MAX_SETTING_VALUE_BYTES, MAX_SETTINGS_ENTRIES,
    MAX_SETTINGS_RECORD_BYTES, ProfileStorageError, ProfileStore, SETTINGS_SCHEMA_VERSION,
    SettingsCleanupWarning, SettingsLoad, SettingsRecovery, SettingsSave, SettingsSnapshot,
};
pub use profile_catalog::{
    MAX_DISCOVERED_PROFILES, MAX_PROFILE_CATALOG_DIRECTORY_ENTRIES,
    MAX_PROFILE_IDENTITY_DIRECTORY_ENTRIES, PROFILE_IDENTITY_DIRECTORY_NAME,
    PROFILE_IDENTITY_SCHEMA_VERSION, PROFILE_READY_DIRECTORY_NAME, ProfileCatalog,
    ProfileCatalogCreateError, ProfileCatalogCreateIntent, ProfileCatalogDiscoverIntent,
    ProfileCatalogEntry, ProfileCatalogError, ProfileCatalogRenameError,
    ProfileCatalogRenameIntent, ProfileIdentityError, ProfileStorageId, load_profile_storage_id,
};
pub use profile_history_scheduler::{
    ProfileHistorySavePolicy, ProfileHistorySavePolicyError, ProfileHistorySaveScheduler,
    ProfileHistorySaveUrgency,
};
pub use profile_lock::{
    MAX_PROFILE_LOCK_BYTES, PROFILE_LOCK_FILE_NAME, ProfileLock, ProfileLockError, ProfileLockOwner,
};
pub use profile_metadata::{
    MAX_PROFILE_DISPLAY_NAME_BYTES, MAX_PROFILE_METADATA_DIRECTORY_ENTRIES,
    PROFILE_METADATA_DIRECTORY_NAME, PROFILE_METADATA_SCHEMA_VERSION, ProfileDisplayName,
    ProfileDisplayNameError, ProfileMetadata, ProfileMetadataError, load_profile_metadata,
};
pub use profile_runtime::{
    ActiveProfile, ColorSchemePreference, PreparedProfile, ProductSettings,
    ProfileHistorySaveCompletion, ProfileHistorySaveId, ProfileHistorySaveIntent, ProfileId,
    ProfilePreparationError, ProfileRuntime, ProfileRuntimeError, ProfileSelectionCommit,
    ProfileSelectionCommitError, ProfileSelectionId, ProfileSelectionIntent, ProfileSelectionStart,
    ProfileSettingsError, ProfileSettingsSaveCompletion, ProfileSettingsSaveId,
    ProfileSettingsSaveIntent,
};
pub use profile_settings_scheduler::{
    ProfileSettingsSavePolicy, ProfileSettingsSavePolicyError, ProfileSettingsSaveScheduler,
    ProfileSettingsSaveUrgency,
};
pub use profile_worker::{
    PROFILE_WORKER_COMMAND_QUEUE_CAPACITY, ProfileWorker, ProfileWorkerCompletion,
    ProfileWorkerSpawnError, ProfileWorkerSubmitError,
};
pub use tab_activation::{
    TabActivationId, TabActivationIntent, TabActivationStart, TabCycleDirection,
};
pub use tab_close::{
    ActiveTabCloseHandoff, LastTabCloseHandoff, TabCloseCommitError, TabCloseStart,
};
pub use tab_strip::{TabStripItemSnapshot, TabStripSnapshot};

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    platform::run(platform::RunMode::Interactive)
}

#[doc(hidden)]
pub fn run_native_smoke() -> Result<(), Box<dyn std::error::Error>> {
    platform::run(platform::RunMode::ExitAfterFirstPresentation)
}

#[doc(hidden)]
pub fn run_native_tab_activation_smoke() -> Result<(), Box<dyn std::error::Error>> {
    platform::run(platform::RunMode::ExitAfterTabActivation)
}

#[doc(hidden)]
pub fn run_native_tab_close_smoke() -> Result<(), Box<dyn std::error::Error>> {
    platform::run(platform::RunMode::ExitAfterTabClose)
}

#[doc(hidden)]
pub fn run_native_tab_supersession_smoke() -> Result<(), Box<dyn std::error::Error>> {
    platform::run(platform::RunMode::ExitAfterRapidTabActivation)
}

#[doc(hidden)]
pub fn run_native_http_navigation_smoke() -> Result<(), Box<dyn std::error::Error>> {
    platform::run(platform::RunMode::ExitAfterRealHttpNavigation)
}

#[doc(hidden)]
pub fn run_native_profile_cycle_smoke() -> Result<(), Box<dyn std::error::Error>> {
    platform::run(platform::RunMode::ExitAfterProfileCycle)
}

#[doc(hidden)]
pub fn run_native_color_scheme_smoke() -> Result<(), Box<dyn std::error::Error>> {
    platform::run(platform::RunMode::ExitAfterColorSchemeCycle)
}