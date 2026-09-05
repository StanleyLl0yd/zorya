mod app;
pub mod engine;
mod platform;

pub use app::{BrowserApp, BrowserModelError, BrowserWindow, BrowserWindowId, Tab, TabId};

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    platform::run()
}
