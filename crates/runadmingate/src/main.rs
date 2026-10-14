//! RunAdminGate service and launcher.
//!
//! One binary: `/install` and `/uninstall` manage the service, `/service` is
//! the service itself, and anything else is a request to start a program.
#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod win;

#[cfg(windows)]
fn main() {
    std::process::exit(win::run());
}

#[cfg(not(windows))]
fn main() {
    eprintln!("runadmingate is a Windows program; cross-build it with `cargo win`.");
    std::process::exit(1);
}
