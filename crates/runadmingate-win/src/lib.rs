//! Windows integration shared by `runadmingate.exe` and the manager.
//! Empty on other platforms so the workspace still builds and tests there.

#[cfg(windows)]
pub mod config;
#[cfg(windows)]
pub mod fileinfo;
#[cfg(windows)]
pub mod icon;
#[cfg(windows)]
pub mod openwith;
#[cfg(windows)]
pub mod reg;
#[cfg(windows)]
pub mod shell;
#[cfg(windows)]
pub mod shortcuts;
