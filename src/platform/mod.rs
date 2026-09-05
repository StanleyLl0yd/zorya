#[cfg(not(target_os = "windows"))]
use std::error::Error;
#[cfg(not(target_os = "windows"))]
use std::fmt;

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "windows")]
pub(crate) use windows::run;

#[cfg(not(target_os = "windows"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UnsupportedPlatform;

#[cfg(not(target_os = "windows"))]
impl fmt::Display for UnsupportedPlatform {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the Zorya native developer shell is currently available only on Windows")
    }
}

#[cfg(not(target_os = "windows"))]
impl Error for UnsupportedPlatform {}

#[cfg(not(target_os = "windows"))]
pub(crate) fn run() -> Result<(), Box<dyn Error>> {
    Err(Box::new(UnsupportedPlatform))
}
