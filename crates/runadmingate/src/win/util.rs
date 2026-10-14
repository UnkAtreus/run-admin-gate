//! Small shared Win32 helpers.

use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, MB_SETFOREGROUND, MB_TOPMOST, MESSAGEBOX_STYLE,
};

/// Owned kernel handle, closed on drop. Kernel handles may be used from any thread.
pub struct Handle(pub HANDLE);

unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Handle {
    pub fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_invalid() && !self.0 .0.is_null() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

pub fn from_wide(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// System message for a Win32 error code.
pub fn error_text(code: u32) -> String {
    std::io::Error::from_raw_os_error(code as i32).to_string()
}

/// Shows `text` in a message box, or prints it to stdout in quiet mode so a
/// caller (the manager) can capture it. Without a stdout handle printing is a no-op.
pub fn report(text: &str, style: MESSAGEBOX_STYLE, quiet: bool) {
    if quiet {
        println!("{text}");
    } else {
        message(text, style);
    }
}

pub fn message(text: &str, style: MESSAGEBOX_STYLE) {
    unsafe {
        MessageBoxW(
            HWND::default(),
            &HSTRING::from(text),
            &HSTRING::from(runadmingate_core::PRODUCT_NAME),
            style | MB_SETFOREGROUND | MB_TOPMOST,
        );
    }
}
