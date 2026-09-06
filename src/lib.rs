mod app;
#[cfg(any(target_os = "windows", test))]
mod async_lifecycle;
pub mod engine;
mod navigation;
mod platform;

pub use app::{BrowserApp, BrowserModelError, BrowserWindow, BrowserWindowId, Tab, TabId};
pub use navigation::{
    HistoryEntry, HistoryEntryId, NavigationFailure, NavigationId, NavigationIntent,
    NavigationIntentKind, NavigationStart, TabNavigation,
};

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    platform::run(platform::RunMode::Interactive)
}

#[doc(hidden)]
pub fn run_native_smoke() -> Result<(), Box<dyn std::error::Error>> {
    platform::run(platform::RunMode::ExitAfterFirstPresentation)
}
