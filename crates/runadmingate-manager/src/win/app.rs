//! State and update logic of the manager window.

use std::collections::HashMap;
use std::path::PathBuf;

use iced::widget::image;
use iced::{event, window, Event, Subscription, Task};
use runadmingate_core::policy::{Policy, FILE_ARGUMENT};
use runadmingate_core::{folder_entry, AllowedPath, LogonMode};
use runadmingate_win::{config, icon, openwith, shell};

use super::model::{self, Args, Item};
use super::platform::{self, LauncherAction, Service};
use super::shortcut::{self, Place};

#[derive(Debug, Clone)]
pub enum Message {
    Refresh,
    Select(String),
    Search(String),
    AddApp,
    AddFolder,
    Picked(Option<PathBuf>, Pick),
    FileHovered,
    HoverLeft,
    FileDropped(PathBuf),
    OpenRuleDialog(Option<String>),
    RuleText(String),
    SaveRule,
    CloseDialog,
    SetMode(LogonMode),
    SetArgs(Args),
    CustomArgs(String),
    SaveCustomArgs,
    SetRecursive(bool),
    SetOpenWith(bool),
    OpenWithTypes(String),
    SuggestTypes,
    SaveOpenWith,
    ShortcutPlace(PlaceChoice),
    CreateShortcut,
    AskRemove,
    Remove,
    Install,
    AskUninstall,
    Uninstall,
    LauncherDone(Result<String, String>),
    CopyPath,
    DismissToast,
}

/// What a file picker was opened for.
#[derive(Debug, Clone)]
pub enum Pick {
    App,
    Folder,
    ShortcutTarget,
    ShortcutFolder(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceChoice {
    PublicDesktop,
    StartMenu,
    Folder,
}

impl std::fmt::Display for PlaceChoice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PlaceChoice::PublicDesktop => "Desktop (all users)",
            PlaceChoice::StartMenu => "Start menu (all users)",
            PlaceChoice::Folder => "Choose a folder…",
        })
    }
}

pub const PLACES: [PlaceChoice; 3] = [
    PlaceChoice::PublicDesktop,
    PlaceChoice::StartMenu,
    PlaceChoice::Folder,
];

pub enum Dialog {
    Rule {
        original: Option<String>,
        text: String,
        error: Option<String>,
    },
    ConfirmRemove,
    ConfirmUninstall,
}

pub struct Toast {
    pub ok: bool,
    pub text: String,
}

/// Inputs in the details panel; reset when the selection changes.
#[derive(Default)]
pub struct Draft {
    pub custom_args: String,
    /// The user chose "these arguments" but hasn't saved a pattern yet.
    pub custom_open: bool,
    pub open_with_types: String,
    pub place: Option<PlaceChoice>,
}

pub struct App {
    pub service: Service,
    pub installed: bool,
    pub launcher_bundled: bool,
    pub items: Vec<Item>,
    pub default_mode: Option<LogonMode>,
    pub selected: Option<String>,
    pub search: String,
    pub dialog: Option<Dialog>,
    pub toast: Option<Toast>,
    pub busy: Option<&'static str>,
    pub hovering: bool,
    pub draft: Draft,
    icons: HashMap<String, Option<image::Handle>>,
}

impl App {
    pub fn new() -> (App, Task<Message>) {
        let mut app = App {
            service: Service::NotInstalled,
            installed: false,
            launcher_bundled: false,
            items: Vec::new(),
            default_mode: None,
            selected: None,
            search: String::new(),
            dialog: None,
            toast: None,
            busy: None,
            hovering: false,
            draft: Draft::default(),
            icons: HashMap::new(),
        };
        app.reload();
        (app, Task::none())
    }

    pub fn selected_item(&self) -> Option<&Item> {
        let raw = self.selected.as_ref()?;
        self.items.iter().find(|item| &item.raw == raw)
    }

    /// Icon for a file or folder, rendered at 2x for sharpness on high-DPI screens.
    pub fn icon(&mut self, path: &str) -> Option<image::Handle> {
        let key = path.to_lowercase();
        self.icons
            .entry(key)
            .or_insert_with(|| {
                icon::rgba(path, 64).map(|i| image::Handle::from_rgba(i.width, i.height, i.pixels))
            })
            .clone()
    }

    pub fn cached_icon(&self, path: &str) -> Option<image::Handle> {
        self.icons.get(&path.to_lowercase()).cloned().flatten()
    }

    fn reload(&mut self) {
        self.service = platform::service();
        self.installed = config::is_installed();
        self.launcher_bundled = platform::bundled_launcher().is_some();
        let (allowed, mode) = if self.installed {
            config::load().unwrap_or_default()
        } else {
            Default::default()
        };
        self.default_mode = mode;
        self.items = model::build(&allowed, mode);
        let paths: Vec<String> = self.items.iter().map(|i| i.path.clone()).collect();
        for path in paths {
            self.icon(&path);
        }
        if self.selected_item().is_none() {
            self.selected = None;
        }
    }

    fn select(&mut self, raw: Option<String>) {
        self.selected = raw;
        self.draft = Draft::default();
        if let Some(item) = self.selected_item() {
            let custom = if item.args() == Args::Custom {
                item.parsed.args.clone()
            } else {
                String::new()
            };
            let types = item
                .open_with
                .as_ref()
                .map(|a| a.extensions.join(", "))
                .unwrap_or_default();
            self.draft.custom_args = custom;
            self.draft.open_with_types = types;
        }
    }

    fn ok(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast {
            ok: true,
            text: text.into(),
        });
    }

    fn fail(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast {
            ok: false,
            text: text.into(),
        });
    }

    /// Applies an edit to `AllowedPath`, then reloads and selects `select`.
    fn edit(&mut self, select: Option<String>, change: impl FnOnce(&mut AllowedPath)) -> bool {
        match config::update_allowed(change) {
            Ok(()) => {
                self.reload();
                self.select(select);
                true
            }
            Err(err) => {
                self.fail(format!(
                    "Could not save the allow-list: {}",
                    std::io::Error::from_raw_os_error(err.0 as i32)
                ));
                false
            }
        }
    }

    fn replace_selected(&mut self, new_text: String) -> bool {
        let Some(old) = self.selected.clone() else {
            return false;
        };
        let target = new_text.clone();
        self.edit(Some(target), move |allowed| {
            if let Some(index) = allowed.position(&old) {
                allowed.replace(index, &new_text);
            }
        })
    }

    fn add_entry(&mut self, entry: String, label: &str) {
        if entry.contains(';') {
            self.fail("Paths containing ';' can't be stored in the allow-list.");
            return;
        }
        if self
            .items
            .iter()
            .any(|i| i.raw.eq_ignore_ascii_case(&entry))
        {
            self.select(Some(entry));
            self.ok(format!("{label} is already allowed."));
            return;
        }
        let added = entry.clone();
        if self.edit(Some(added), move |allowed| {
            allowed.push(&entry);
        }) {
            let name = self
                .selected_item()
                .map(|i| i.name.clone())
                .unwrap_or_default();
            self.ok(format!("{name} can now be started as administrator."));
        }
    }

    fn add_path(&mut self, path: PathBuf) {
        if !self.installed {
            self.fail("Install the service first.");
            return;
        }
        let text = path.to_string_lossy().into_owned();
        if path.is_dir() {
            self.add_entry(folder_entry(&text), "This folder");
        } else {
            self.add_entry(text, "This app");
        }
    }

    fn set_open_with(&mut self, item: &Item, types: Vec<String>) {
        let Some(launcher) = shell::installed_launcher().filter(|p| p.exists()) else {
            self.fail("Install the service first.");
            return;
        };
        if types.is_empty() {
            self.fail("Enter at least one file type, for example .txt");
            return;
        }
        // Open with passes the file, so the entry must allow one file argument.
        let raw = if item.args() == Args::None {
            let new_text = model::with_args(item, FILE_ARGUMENT);
            if !self.replace_selected(new_text.clone()) {
                return;
            }
            new_text
        } else {
            item.raw.clone()
        };
        match openwith::register(&launcher, &item.path, &types) {
            Ok(app) => {
                self.reload();
                self.select(Some(raw));
                self.ok(format!(
                    "“{}” now appears in Open with for {}.",
                    app.name,
                    app.extensions.join(", ")
                ));
            }
            Err(err) => self.fail(err),
        }
    }

    fn make_shortcut(&mut self, program: &str, args: &str, place: Place) {
        match shortcut::create(program, args, &place) {
            Ok(lnk) => self.ok(format!("Shortcut created: {}", lnk.display())),
            Err(err) => self.fail(err),
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Refresh => self.reload(),
            Message::Select(raw) => self.select(Some(raw)),
            Message::Search(text) => self.search = text,
            Message::AddApp => return pick(Pick::App, None),
            Message::AddFolder => return pick(Pick::Folder, None),
            Message::Picked(None, _) => {}
            Message::Picked(Some(path), what) => match what {
                Pick::App | Pick::Folder => self.add_path(path),
                Pick::ShortcutTarget => {
                    let place = match self.draft.place.unwrap_or(PlaceChoice::PublicDesktop) {
                        PlaceChoice::PublicDesktop => Place::PublicDesktop,
                        PlaceChoice::StartMenu => Place::StartMenu,
                        PlaceChoice::Folder => return pick(Pick::ShortcutFolder(path), None),
                    };
                    self.make_shortcut(&path.to_string_lossy(), "", place);
                }
                Pick::ShortcutFolder(program) => {
                    let program = program.to_string_lossy().into_owned();
                    let args = self.shortcut_args(&program);
                    self.make_shortcut(&program, &args, Place::Folder(path));
                }
            },
            Message::FileHovered => self.hovering = true,
            Message::HoverLeft => self.hovering = false,
            Message::FileDropped(path) => {
                self.hovering = false;
                self.add_path(path);
            }
            Message::OpenRuleDialog(original) => {
                let text = original.clone().unwrap_or_default();
                self.dialog = Some(Dialog::Rule {
                    original,
                    text,
                    error: None,
                });
            }
            Message::RuleText(value) => {
                if let Some(Dialog::Rule { text, error, .. }) = &mut self.dialog {
                    *text = value;
                    *error = None;
                }
            }
            Message::SaveRule => {
                let Some(Dialog::Rule { original, text, .. }) = &self.dialog else {
                    return Task::none();
                };
                let (original, text) = (original.clone(), text.trim().to_string());
                let check = AllowedPath::parse(&text);
                let (_, errors) = Policy::compile(&check, None, platform::expand);
                let problem = if check.is_empty() {
                    Some("Enter a rule.".to_string())
                } else {
                    errors
                        .first()
                        .map(|e| format!("“{}”: {}", e.entry, e.reason))
                };
                if let Some(problem) = problem {
                    if let Some(Dialog::Rule { error, .. }) = &mut self.dialog {
                        *error = Some(problem);
                    }
                    return Task::none();
                }
                self.dialog = None;
                let first = check.entries()[0].clone();
                match original {
                    Some(old) => {
                        self.selected = Some(old);
                        if self.replace_selected(text) {
                            self.ok("Rule saved.");
                        }
                    }
                    None => self.add_entry(first, "This rule"),
                }
            }
            Message::CloseDialog => self.dialog = None,
            Message::SetMode(mode) => {
                if let Some(item) = self.selected_item().cloned() {
                    // Write a switch only when it differs from the default (LogonFlag).
                    let default = self.default_mode.unwrap_or(LogonMode::System);
                    let switch = (mode != default).then_some(mode);
                    if self.replace_selected(runadmingate_core::with_override(&item.raw, switch)) {
                        self.ok(match mode {
                            LogonMode::System => "Runs as SYSTEM.",
                            LogonMode::Admin => "Runs as the user, with administrator rights.",
                        });
                    }
                }
            }
            Message::SetArgs(args) => {
                if let Some(item) = self.selected_item().cloned() {
                    let text = match args {
                        Args::None => String::new(),
                        Args::File => FILE_ARGUMENT.to_string(),
                        Args::Custom => {
                            // Saved with SaveCustomArgs once a pattern is typed.
                            self.draft.custom_open = true;
                            return Task::none();
                        }
                    };
                    if args == Args::None && item.open_with.is_some() {
                        self.fail("Turn off Open with first: it needs “Let users open a file”.");
                        return Task::none();
                    }
                    if self.replace_selected(model::with_args(&item, &text)) {
                        self.ok("Arguments updated.");
                    }
                }
            }
            Message::CustomArgs(text) => self.draft.custom_args = text,
            Message::SaveCustomArgs => {
                if let Some(item) = self.selected_item().cloned() {
                    let args = self.draft.custom_args.trim().to_string();
                    if args.contains(';') {
                        self.fail("Arguments can't contain ';'.");
                    } else if self.replace_selected(model::with_args(&item, &args)) {
                        self.ok("Arguments updated.");
                    }
                }
            }
            Message::SetRecursive(recursive) => {
                if let Some(item) = self.selected_item().cloned() {
                    if self.replace_selected(model::with_recursive(&item, recursive)) {
                        self.ok(if recursive {
                            "Subfolders are included."
                        } else {
                            "Only this folder is included."
                        });
                    }
                }
            }
            Message::SetOpenWith(on) => {
                if let Some(item) = self.selected_item().cloned() {
                    if on {
                        let mut types = openwith::normalize_extensions(&self.draft.open_with_types);
                        if types.is_empty() {
                            types = openwith::suggest_extensions(&item.path);
                        }
                        if types.is_empty() {
                            self.draft.open_with_types.clear();
                            self.fail("Enter the file types to show it for (for example .txt, .log), then press Save.");
                            return Task::none();
                        }
                        self.set_open_with(&item, types);
                    } else if let Some(app) = &item.open_with {
                        openwith::unregister(&app.id);
                        let raw = item.raw.clone();
                        self.reload();
                        self.select(Some(raw));
                        self.ok("Removed from Open with.");
                    }
                }
            }
            Message::OpenWithTypes(text) => self.draft.open_with_types = text,
            Message::SuggestTypes => {
                if let Some(item) = self.selected_item() {
                    let types = openwith::suggest_extensions(&item.path);
                    if types.is_empty() {
                        self.fail("No file types are registered for this app. Type them in, e.g. .txt, .log");
                    } else {
                        self.draft.open_with_types = types.join(", ");
                    }
                }
            }
            Message::SaveOpenWith => {
                if let Some(item) = self.selected_item().cloned() {
                    let types = openwith::normalize_extensions(&self.draft.open_with_types);
                    self.set_open_with(&item, types);
                }
            }
            Message::ShortcutPlace(place) => self.draft.place = Some(place),
            Message::CreateShortcut => {
                let Some(item) = self.selected_item().cloned() else {
                    return Task::none();
                };
                if !item.is_program() {
                    // Folders and patterns: let the user pick the program inside.
                    let start = PathBuf::from(item.path.trim_end_matches('\\'));
                    return pick(Pick::ShortcutTarget, start.is_dir().then_some(start));
                }
                let args = self.shortcut_args(&item.path);
                match self.draft.place.unwrap_or(PlaceChoice::PublicDesktop) {
                    PlaceChoice::PublicDesktop => {
                        self.make_shortcut(&item.path, &args, Place::PublicDesktop)
                    }
                    PlaceChoice::StartMenu => {
                        self.make_shortcut(&item.path, &args, Place::StartMenu)
                    }
                    PlaceChoice::Folder => {
                        return pick(Pick::ShortcutFolder(PathBuf::from(&item.path)), None)
                    }
                }
            }
            Message::AskRemove => self.dialog = Some(Dialog::ConfirmRemove),
            Message::Remove => {
                self.dialog = None;
                if let Some(item) = self.selected_item().cloned() {
                    if let Some(app) = &item.open_with {
                        openwith::unregister(&app.id);
                    }
                    let raw = item.raw.clone();
                    if self.edit(None, move |allowed| {
                        if let Some(index) = allowed.position(&raw) {
                            allowed.remove(index);
                        }
                    }) {
                        self.ok(format!("{} was removed.", item.name));
                    }
                }
            }
            Message::Install => {
                return self.launch(LauncherAction::Install, "Installing the service…")
            }
            Message::AskUninstall => self.dialog = Some(Dialog::ConfirmUninstall),
            Message::Uninstall => {
                self.dialog = None;
                return self.launch(LauncherAction::Uninstall, "Uninstalling…");
            }
            Message::LauncherDone(result) => {
                self.busy = None;
                self.reload();
                match result {
                    Ok(text) => self.ok(text.lines().next().unwrap_or("Done.").to_string()),
                    Err(text) => self.fail(text),
                }
            }
            Message::CopyPath => {
                if let Some(item) = self.selected_item() {
                    let path = item.path.clone();
                    self.ok("Path copied.");
                    return iced::clipboard::write(path);
                }
            }
            Message::DismissToast => self.toast = None,
        }
        Task::none()
    }

    /// Fixed arguments a shortcut can carry: literal ones, never patterns or `%1`.
    fn shortcut_args(&self, program: &str) -> String {
        self.items
            .iter()
            .find(|i| i.path.eq_ignore_ascii_case(program))
            .filter(|i| i.args() == Args::Custom && !i.parsed.args.contains(['*', '?']))
            .map(|i| i.parsed.args.clone())
            .unwrap_or_default()
    }

    fn launch(&mut self, action: LauncherAction, label: &'static str) -> Task<Message> {
        self.busy = Some(label);
        Task::perform(
            async move { platform::run_launcher(action) },
            Message::LauncherDone,
        )
    }

    pub fn subscription(&self) -> Subscription<Message> {
        event::listen_with(|event, _, _| match event {
            Event::Window(window::Event::FileHovered(_)) => Some(Message::FileHovered),
            Event::Window(window::Event::FilesHoveredLeft) => Some(Message::HoverLeft),
            Event::Window(window::Event::FileDropped(path)) => Some(Message::FileDropped(path)),
            Event::Window(window::Event::Focused) => Some(Message::Refresh),
            _ => None,
        })
    }

    /// How the selected entry starts: its own switch, else `LogonFlag`.
    pub fn mode_choice(&self, item: &Item) -> LogonMode {
        item.parsed
            .mode
            .or(self.default_mode)
            .unwrap_or(LogonMode::System)
    }

    /// The arguments choice shown in the details panel.
    pub fn args_choice(&self, item: &Item) -> Args {
        if self.draft.custom_open {
            Args::Custom
        } else {
            item.args()
        }
    }
}

/// Opens a native picker on a worker thread.
fn pick(what: Pick, start: Option<PathBuf>) -> Task<Message> {
    let folder = matches!(what, Pick::Folder | Pick::ShortcutFolder(_));
    Task::perform(
        async move {
            let mut dialog = rfd::FileDialog::new();
            if let Some(start) = start {
                dialog = dialog.set_directory(start);
            }
            if folder {
                dialog.set_title("Choose a folder").pick_folder()
            } else {
                dialog
                    .set_title("Choose an app")
                    .add_filter(
                        "Apps and scripts",
                        &["exe", "com", "bat", "cmd", "ps1", "vbs", "msi", "msp"],
                    )
                    .add_filter("All files", &["*"])
                    .pick_file()
            }
        },
        move |path| Message::Picked(path, what.clone()),
    )
}
