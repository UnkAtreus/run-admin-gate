//! `.lnk` files that start a program through RunAdminGate, with the
//! program's own icon.

use std::path::{Path, PathBuf};

use runadmingate_win::{fileinfo, shell};
use windows::core::{Interface, HSTRING};
use windows::Win32::System::Com::{CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    PublicDesktop,
    StartMenu,
    Folder(PathBuf),
}

impl Place {
    pub fn folder(&self) -> Option<PathBuf> {
        match self {
            Place::PublicDesktop => shell::public_desktop(),
            Place::StartMenu => shell::common_start_menu(),
            Place::Folder(path) => Some(path.clone()),
        }
    }
}

/// Characters Windows doesn't allow in file names.
fn file_name_safe(name: &str) -> String {
    name.chars()
        .map(|c| {
            if "<>:\"/\\|?*".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect()
}

/// Creates "<Name> (admin).lnk" in `place` that runs `program args` via the
/// installed launcher. Returns the shortcut path.
pub fn create(program: &str, args: &str, place: &Place) -> Result<PathBuf, String> {
    let launcher = shell::installed_launcher().filter(|p| p.exists()).ok_or(
        "The RunAdminGate service isn't installed, so there is no launcher for the shortcut to call.",
    )?;
    let folder = place
        .folder()
        .ok_or("Could not find the destination folder.")?;
    let name = fileinfo::display_name(program);
    let lnk = folder.join(format!("{} (admin).lnk", file_name_safe(&name)));
    let arguments = if args.is_empty() {
        format!("\"{program}\"")
    } else {
        format!("\"{program}\" {args}")
    };
    let (icon_file, icon_index) =
        fileinfo::icon_location(program).unwrap_or((program.to_string(), 0));
    let work_dir = Path::new(program)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();

    unsafe {
        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(|e| e.message())?;
        link.SetPath(&HSTRING::from(launcher.as_os_str()))
            .map_err(|e| e.message())?;
        link.SetArguments(&HSTRING::from(arguments))
            .map_err(|e| e.message())?;
        link.SetWorkingDirectory(&HSTRING::from(work_dir))
            .map_err(|e| e.message())?;
        link.SetDescription(&HSTRING::from(format!(
            "Run {name} with administrator rights (RunAdminGate)"
        )))
        .map_err(|e| e.message())?;
        link.SetIconLocation(&HSTRING::from(icon_file), icon_index)
            .map_err(|e| e.message())?;
        link.cast::<IPersistFile>()
            .and_then(|file| file.Save(&HSTRING::from(lnk.as_os_str()), true))
            .map_err(|e| format!("Could not save {}: {}", lnk.display(), e.message()))?;
    }
    runadmingate_win::shortcuts::record(&lnk.to_string_lossy());
    Ok(lnk)
}
