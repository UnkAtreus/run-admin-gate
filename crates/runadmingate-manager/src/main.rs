//! RunAdminGate Manager: choose which programs standard users may run as
//! administrator, add them to "Open with", and create shortcuts.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod win;

#[cfg(windows)]
fn main() {
    std::process::exit(win::run());
}

#[cfg(not(windows))]
fn main() {
    eprintln!("runadmingate-manager is a Windows program; cross-build it with `cargo win`.");
    std::process::exit(1);
}
