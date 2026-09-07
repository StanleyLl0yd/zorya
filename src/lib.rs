mod app;
#[cfg(any(target_os = "windows", test))]
mod async_lifecycle;
#[cfg(target_os = "windows")]
mod branding;
mod chrome;
mod commands;
pub mod engine;
mod navigation;
mod platform;
mod presentation_handoff;
mod tab_activation;
mod tab_close;
mod tab_strip;

pub use app::{
    BrowserApp, BrowserModelError, BrowserWindow, BrowserWindowId, Tab, TabCloseResult, TabId,
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
