//! Event-log source registration. The allow-list itself lives in
//! `runadmingate_win::config`.

use runadmingate_win::reg::{self, Key, RegResult, HKEY_LOCAL_MACHINE, KEY_WOW64_64KEY, KEY_WRITE};

const EVENT_SOURCE_KEY: &str =
    r"SYSTEM\CurrentControlSet\Services\EventLog\Application\RunAdminGate";

/// EventCreate.exe's message table maps IDs 1-1000 to "%1", so events show
/// their text in Event Viewer without a message DLL of our own.
pub fn register_event_source() -> RegResult<()> {
    let (key, _) = Key::create(
        HKEY_LOCAL_MACHINE,
        EVENT_SOURCE_KEY,
        KEY_WRITE | KEY_WOW64_64KEY,
    )?;
    key.set_expand_string("EventMessageFile", r"%SystemRoot%\System32\EventCreate.exe")?;
    key.set_dword("TypesSupported", 7)
}

pub fn unregister_event_source() -> RegResult<()> {
    reg::delete_tree(HKEY_LOCAL_MACHINE, EVENT_SOURCE_KEY)
}
