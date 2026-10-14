//! Known folders and shell notifications.

use std::path::PathBuf;

use windows::core::GUID;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{
    FOLDERID_CommonPrograms, FOLDERID_ProgramFiles, FOLDERID_PublicDesktop, SHChangeNotify,
    SHGetKnownFolderPath, KF_FLAG_DEFAULT, SHCNE_ASSOCCHANGED, SHCNF_IDLIST,
};

fn known_folder(id: &GUID) -> Option<PathBuf> {
    unsafe {
        let raw = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let path = raw.to_string().ok();
        CoTaskMemFree(Some(raw.0 as *const _));
        path.map(PathBuf::from)
    }
}

pub fn program_files() -> Option<PathBuf> {
    known_folder(&FOLDERID_ProgramFiles)
}

/// `C:\Users\Public\Desktop`: shortcuts every user sees.
pub fn public_desktop() -> Option<PathBuf> {
    known_folder(&FOLDERID_PublicDesktop)
}

/// The all-users Start menu `Programs` folder.
pub fn common_start_menu() -> Option<PathBuf> {
    known_folder(&FOLDERID_CommonPrograms)
}

/// `%ProgramFiles%\RunAdminGate\runadmingate.exe`.
pub fn installed_launcher() -> Option<PathBuf> {
    Some(program_files()?.join(runadmingate_core::INSTALLED_LAUNCHER))
}

/// Tells Explorer that file associations changed.
pub fn notify_associations_changed() {
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
}
