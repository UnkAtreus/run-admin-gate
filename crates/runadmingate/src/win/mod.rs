mod client;
mod eventlog;
mod handler;
mod install;
mod launch;
mod registry;
mod server;
mod service;
mod util;

use runadmingate_core::cmdline::split_arg;
use windows::Win32::System::Environment::GetCommandLineW;
use windows::Win32::UI::WindowsAndMessaging::{MB_ICONINFORMATION, MB_OK};

const USAGE: &str = "RunAdminGate\n\n\
    Start an allowed program with elevated rights:\n    runadmingate.exe [/quiet] <program> [arguments]\n\n\
    Install or update the service (administrator):\n    runadmingate.exe /install [/allowedpath:<entries>] [/asadmin | /assystem] [/quiet]\n\n\
    Remove the service and its allow-list (administrator):\n    runadmingate.exe /uninstall [/quiet]";

/// Splits the remaining command line into arguments.
fn rest_args(mut rest: &str) -> Vec<String> {
    let mut out = Vec::new();
    while let Some((arg, tail)) = split_arg(rest, false) {
        out.push(arg);
        rest = tail;
    }
    out
}

/// An Open-with stub (`apps\<id>.exe`, a copy of this binary) starts the
/// program registered for its name with the file it was given.
fn stub_target() -> Option<Result<String, String>> {
    let exe = std::env::current_exe().ok()?;
    let stem = exe.file_stem()?.to_string_lossy().into_owned();
    if stem.eq_ignore_ascii_case("runadmingate") {
        return None;
    }
    Some(
        runadmingate_win::openwith::lookup(&stem)
            .map(|app| app.program)
            .ok_or_else(|| format!("{stem} is not a registered RunAdminGate Open with entry.")),
    )
}

pub fn run() -> i32 {
    let line = unsafe { GetCommandLineW().to_string() }.unwrap_or_default();
    let Some((_, mut rest)) = split_arg(&line, true) else {
        return 87;
    };
    match stub_target() {
        Some(Ok(program)) => return client::run(&program, rest, false),
        Some(Err(text)) => {
            util::message(
                &text,
                MB_OK | windows::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
            );
            return 2;
        }
        None => {}
    }
    let mut quiet = false;
    loop {
        match split_arg(rest, false) {
            Some((arg, tail)) if arg.eq_ignore_ascii_case("/quiet") => {
                quiet = true;
                rest = tail;
            }
            Some((arg, tail)) => {
                return match arg.to_ascii_lowercase().as_str() {
                    "/install" => {
                        let mut args = rest_args(tail);
                        if quiet {
                            args.push("/quiet".into());
                        }
                        install::install(&args)
                    }
                    "/uninstall" => {
                        let mut args = rest_args(tail);
                        if quiet {
                            args.push("/quiet".into());
                        }
                        install::uninstall(&args)
                    }
                    "/service" => service::run(),
                    "/?" | "/help" => {
                        util::message(USAGE, MB_OK | MB_ICONINFORMATION);
                        0
                    }
                    _ => client::run(&arg, tail, quiet),
                };
            }
            None => {
                if !quiet {
                    util::message(USAGE, MB_OK | MB_ICONINFORMATION);
                }
                return 87; // ERROR_INVALID_PARAMETER
            }
        }
    }
}
