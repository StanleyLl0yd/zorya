mod app;
#[cfg(any(target_os = "windows", test))]
mod async_lifecycle;
mod chrome;
pub mod engine;
mod navigation;
mod platform;
mod tab_activation;

pub use app::{
    BrowserApp, BrowserModelError, BrowserWindow, BrowserWindowId, Tab, TabCloseResult, TabId,
};
pub use chrome::{AddressBarEdit, AddressBarState, AddressBarSubmission};
pub use navigation::{
    HistoryEntry, HistoryEntryId, NavigationControls, NavigationFailure, NavigationId,
    NavigationIntent, NavigationIntentKind, NavigationStart, ReloadControl, TabNavigation,
};
pub use tab_activation::{TabActivationId, TabActivationIntent, TabActivationStart};

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    platform::run(platform::RunMode::Interactive)
}

#[doc(hidden)]
pub fn run_native_smoke() -> Result<(), Box<dyn std::error::Error>> {
    platform::run(platform::RunMode::ExitAfterFirstPresentation)
}
