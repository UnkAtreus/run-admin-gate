//! Windows Application event log, source "RunAdminGate".

use runadmingate_core::PRODUCT_NAME;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Security::PSID;
use windows::Win32::System::EventLog::{
    DeregisterEventSource, RegisterEventSourceW, ReportEventW, EVENTLOG_ERROR_TYPE,
    EVENTLOG_INFORMATION_TYPE, EVENTLOG_WARNING_TYPE,
};

/// Event IDs (1-1000, see `registry::register_event_source`).
pub mod id {
    pub const STARTED: u32 = 1;
    pub const REFUSED: u32 = 2;
    pub const FAILED: u32 = 3;
    pub const SERVICE_RUNNING: u32 = 10;
    pub const SERVICE_STOPPED: u32 = 11;
    pub const SERVICE_ERROR: u32 = 12;
    pub const CONFIG_WARNING: u32 = 20;
}

pub enum Level {
    Info,
    Warning,
    Error,
}

pub fn report(level: Level, event_id: u32, text: &str) {
    let kind = match level {
        Level::Info => EVENTLOG_INFORMATION_TYPE,
        Level::Warning => EVENTLOG_WARNING_TYPE,
        Level::Error => EVENTLOG_ERROR_TYPE,
    };
    unsafe {
        let Ok(source) = RegisterEventSourceW(PCWSTR::null(), &HSTRING::from(PRODUCT_NAME)) else {
            return;
        };
        let text = HSTRING::from(text);
        let strings = [PCWSTR(text.as_ptr())];
        let _ = ReportEventW(
            source,
            kind,
            0,
            event_id,
            PSID::default(),
            0,
            Some(&strings),
            None,
        );
        let _ = DeregisterEventSource(source);
    }
}
