//! Layout of the manager window.

use iced::widget::text::Wrapping;
use iced::widget::{
    button, center, column, container, image, opaque, pick_list, radio, row, scrollable, space,
    stack, text, text_input, toggler, Column,
};
use iced::{Alignment, Color, Element, Font, Length, Padding};

use super::app::{App, Dialog, Message, Toast, PLACES};
use super::model::{Args, Item, Kind};
use super::platform::Service;
use super::style::{self, glyph, *};

type El<'a> = Element<'a, Message>;

const MONO: Font = Font::with_name("Consolas");

fn icon<'a>(code: char, size: u32, color: Color) -> El<'a> {
    text(code.to_string())
        .font(ICONS)
        .size(size)
        .color(color)
        .into()
}

/// Glyph + label, the usual button content.
fn label<'a>(code: char, caption: &'a str, color: Color) -> El<'a> {
    row![icon(code, 12, color), text(caption).size(13)]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
}

fn badge<'a>(caption: &'a str, fg: Color, bg: Color) -> El<'a> {
    container(text(caption).size(11).color(fg))
        .padding([2, 8])
        .style(style::tint(bg, fg))
        .into()
}

fn banner<'a>(code: char, message: String, fg: Color, bg: Color) -> El<'a> {
    container(
        row![
            icon(code, 14, fg),
            text(message).size(12).color(fg).width(Length::Fill)
        ]
        .spacing(10),
    )
    .padding([10, 12])
    .width(Length::Fill)
    .style(style::tint(bg, fg))
    .into()
}

fn section<'a>(title: &'a str, body: impl Into<El<'a>>) -> El<'a> {
    column![text(title).font(SEMIBOLD).size(13), body.into()]
        .spacing(8)
        .into()
}

fn item_icon<'a>(app: &App, item: &Item, size: f32) -> El<'a> {
    match (item.kind, app.cached_icon(&item.path)) {
        (Kind::Pattern, _) | (_, None) => {
            let code = match item.kind {
                Kind::Folder { .. } => glyph::FOLDER,
                Kind::Pattern => glyph::CODE,
                _ => glyph::APP,
            };
            center(icon(code, (size * 0.5) as u32, ACCENT))
                .width(size)
                .height(size)
                .style(style::tint(ACCENT_TINT, ACCENT))
                .into()
        }
        (_, Some(handle)) => image(handle).width(size).height(size).into(),
    }
}

fn header(app: &App) -> El<'_> {
    let (caption, fg, bg) = match app.service {
        Service::Running => ("Service running", GREEN, GREEN_TINT),
        Service::Busy => ("Service starting…", AMBER, AMBER_TINT),
        Service::Stopped => ("Service stopped", AMBER, AMBER_TINT),
        Service::NotInstalled => ("Service not installed", TEXT2, CHIP),
    };
    let pill = container(
        row![
            text("●").size(9).color(fg),
            text(caption).size(12).color(fg)
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding([5, 12])
    .style(style::tint(bg, fg));
    let action: El = if app.service == Service::NotInstalled {
        button(label(glyph::SHIELD, "Install service", Color::WHITE))
            .padding([8, 14])
            .style(accent_button)
            .on_press_maybe(app.launcher_bundled.then_some(Message::Install))
            .into()
    } else {
        button(text("Uninstall").size(13))
            .padding([8, 12])
            .style(ghost_button)
            .on_press(Message::AskUninstall)
            .into()
    };
    row![
        image(image::Handle::from_rgba(style::APP_ICON_SIZE, style::APP_ICON_SIZE, style::APP_ICON_RGBA)).width(44).height(44),
        column![
            text("RunAdminGate").font(SEMIBOLD).size(20),
            text("Let standard users run approved apps as administrator, without sharing an admin password.")
                .size(12)
                .color(TEXT2),
        ]
        .spacing(2),
        space::horizontal(),
        pill,
        action,
    ]
    .spacing(14)
    .align_y(Alignment::Center)
    .padding(Padding { top: 18.0, right: 24.0, bottom: 16.0, left: 24.0 })
    .into()
}

fn list_row<'a>(app: &'a App, item: &'a Item) -> El<'a> {
    let selected = app.selected.as_deref() == Some(item.raw.as_str());
    let mut badges = row![].spacing(6).align_y(Alignment::Center);
    if item.issue.is_some() {
        badges = badges.push(badge("Needs attention", RED, RED_TINT));
    }
    if let Kind::Folder { recursive: true } = item.kind {
        badges = badges.push(badge("Subfolders", TEXT2, CHIP));
    }
    if item.open_with.is_some() {
        badges = badges.push(badge("Open with", ACCENT, ACCENT_TINT));
    } else if item.args() == Args::File {
        badges = badges.push(badge("Opens files", TEXT2, CHIP));
    }
    if item.args() == Args::Custom {
        badges = badges.push(badge("Arguments", TEXT2, CHIP));
    }
    button(
        row![
            item_icon(app, item, 32.0),
            column![
                text(&item.name).font(SEMIBOLD).size(14),
                text(item.subtitle())
                    .size(12)
                    .color(TEXT2)
                    .wrapping(Wrapping::WordOrGlyph),
            ]
            .spacing(1)
            .width(Length::Fill),
            badges,
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .padding([8, 10])
    .width(Length::Fill)
    .style(style::row_button(selected))
    .on_press(Message::Select(item.raw.clone()))
    .into()
}

fn empty<'a>(code: char, title: &'a str, body: &'a str, extra: Option<El<'a>>) -> El<'a> {
    let mut content = column![
        icon(code, 36, TEXT3),
        text(title).font(SEMIBOLD).size(16),
        text(body).size(13).color(TEXT2)
    ]
    .spacing(8)
    .align_x(Alignment::Center);
    if let Some(extra) = extra {
        content = content.push(extra);
    }
    center(content).into()
}

fn list_panel(app: &App) -> El<'_> {
    let enabled = app.installed;
    let toolbar = row![
        text_input("Search apps", &app.search)
            .on_input(Message::Search)
            .padding([8, 10])
            .size(13)
            .style(style::input)
            .width(Length::Fill),
        button(label(glyph::ADD, "Add app", Color::WHITE))
            .padding([8, 14])
            .style(accent_button)
            .on_press_maybe(enabled.then_some(Message::AddApp)),
        button(label(glyph::FOLDER, "Add folder", TEXT))
            .padding([8, 12])
            .style(subtle_button)
            .on_press_maybe(enabled.then_some(Message::AddFolder)),
        button(label(glyph::CODE, "Add rule", TEXT))
            .padding([8, 12])
            .style(subtle_button)
            .on_press_maybe(enabled.then_some(Message::OpenRuleDialog(None))),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let visible: Vec<&Item> = app
        .items
        .iter()
        .filter(|i| i.matches(&app.search))
        .collect();
    let content: El = if !app.installed {
        let note: Option<El> = (!app.launcher_bundled).then(|| {
            text("runadmingate.exe must be in the same folder as this program.")
                .size(12)
                .color(RED)
                .into()
        });
        empty(
            glyph::SHIELD,
            "Install the RunAdminGate service to begin",
            "The service starts approved apps with administrator rights for standard users.",
            note,
        )
    } else if app.items.is_empty() {
        empty(
            glyph::ADD,
            "No apps yet",
            "Click Add app, or drag an app or folder onto this window.",
            None,
        )
    } else if visible.is_empty() {
        empty(
            glyph::SEARCH,
            "No matches",
            "No allowed app matches your search.",
            None,
        )
    } else {
        scrollable(
            Column::with_children(visible.into_iter().map(|i| list_row(app, i)))
                .spacing(2)
                .padding(Padding {
                    right: 10.0,
                    ..Padding::ZERO
                }),
        )
        .height(Length::Fill)
        .into()
    };

    container(column![toolbar, content].spacing(12))
        .padding(16)
        .style(style::card)
        .width(Length::FillPortion(3))
        .height(Length::Fill)
        .into()
}

fn args_section<'a>(app: &'a App, item: &'a Item) -> El<'a> {
    let choice = Some(app.args_choice(item));
    let option = |caption: &'a str, value: Args| {
        radio(caption, value, choice, Message::SetArgs)
            .size(16)
            .text_size(13)
    };
    let mut body = column![
        option("No arguments", Args::None),
        option("Let users open a file with it", Args::File),
        option("Only these arguments", Args::Custom),
    ]
    .spacing(10);
    if choice == Some(Args::Custom) {
        body = body.push(
            column![
                row![
                    text_input("/S /D=*", &app.draft.custom_args)
                        .on_input(Message::CustomArgs)
                        .on_submit(Message::SaveCustomArgs)
                        .padding([7, 10])
                        .size(13)
                        .style(style::input),
                    button(text("Save").size(13))
                        .padding([7, 14])
                        .style(subtle_button)
                        .on_press(Message::SaveCustomArgs),
                ]
                .spacing(8),
                text("Users must pass exactly this. * matches anything, ? one character.")
                    .size(11)
                    .color(TEXT3),
            ]
            .spacing(4)
            .padding(Padding {
                left: 26.0,
                ..Padding::ZERO
            }),
        );
    }
    section("Allowed arguments", body)
}

fn open_with_section<'a>(app: &'a App, item: &'a Item) -> El<'a> {
    let on = item.open_with.is_some();
    let name = item
        .open_with
        .as_ref()
        .map(|a| a.name.clone())
        .unwrap_or_else(|| format!("{} (admin)", item.name));
    let body = column![
        toggler(on)
            .label("Show in the “Open with” menu")
            .on_toggle(Message::SetOpenWith)
            .size(18)
            .text_size(13),
        text(format!("Right-click a file › Open with › Choose another app › “{name}”. Tick “Always use this app” to make it the default; after that it is listed under Open with directly."))
        .size(12)
        .color(TEXT2),
        row![
            text_input("File types, e.g. .txt, .log", &app.draft.open_with_types)
                .on_input(Message::OpenWithTypes)
                .on_submit(Message::SaveOpenWith)
                .padding([7, 10])
                .size(13)
                .style(style::input),
            button(label(glyph::SPARKLE, "Suggest", TEXT))
                .padding([7, 10])
                .style(subtle_button)
                .on_press(Message::SuggestTypes),
            button(text("Save").size(13))
                .padding([7, 14])
                .style(subtle_button)
                .on_press(Message::SaveOpenWith),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    ]
    .spacing(8);
    section("Open with", body)
}

fn shortcut_section<'a>(app: &'a App, item: &'a Item) -> El<'a> {
    let hint = if item.is_program() {
        "Users double-click it to start the app as administrator. It uses the app's own icon."
    } else {
        "You'll pick the program inside the folder."
    };
    let body = column![
        row![
            pick_list(
                &PLACES[..],
                Some(app.draft.place.unwrap_or(PLACES[0])),
                Message::ShortcutPlace
            )
            .text_size(13)
            .padding([7, 10])
            .style(style::select)
            .menu_style(style::select_menu),
            button(label(glyph::LINK, "Create shortcut", TEXT))
                .padding([8, 12])
                .style(subtle_button)
                .on_press(Message::CreateShortcut),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        text(hint).size(12).color(TEXT2),
    ]
    .spacing(6);
    section("Shortcut", body)
}

fn panel(content: El<'_>) -> El<'_> {
    container(content)
        .padding(20)
        .style(style::card)
        .width(Length::FillPortion(2))
        .height(Length::Fill)
        .into()
}

fn details(app: &App) -> El<'_> {
    let Some(item) = app.selected_item() else {
        return panel(empty(
            glyph::APP,
            "Nothing selected",
            "Select an app to see and change its settings.",
            None,
        ));
    };

    let head = row![
        item_icon(app, item, 48.0),
        column![
            text(&item.name).font(SEMIBOLD).size(20),
            row![
                text(item.subtitle())
                    .size(12)
                    .color(TEXT2)
                    .wrapping(Wrapping::WordOrGlyph)
                    .width(Length::Fill),
                button(icon(glyph::COPY, 12, TEXT2))
                    .padding(5)
                    .style(ghost_button)
                    .on_press(Message::CopyPath),
            ]
            .spacing(4)
            .align_y(Alignment::Center),
        ]
        .spacing(2)
        .width(Length::Fill),
    ]
    .spacing(14)
    .align_y(Alignment::Center);

    let mut content = column![head].spacing(18);
    if let Some(issue) = &item.issue {
        content = content.push(banner(glyph::WARNING, issue.clone(), RED, RED_TINT));
    }
    content = content.push(banner(
        glyph::SHIELD,
        "Runs as SYSTEM: full control of this computer. Only allow apps you'd trust every user with.".to_string(),
        TEXT2,
        CHIP,
    ));
    match item.kind {
        Kind::Program => {
            content = content.push(args_section(app, item));
            content = content.push(open_with_section(app, item));
        }
        Kind::Script => content = content.push(args_section(app, item)),
        Kind::Folder { recursive } => {
            content = content.push(section(
                "Folder",
                column![
                    toggler(recursive)
                        .label("Include subfolders")
                        .on_toggle(Message::SetRecursive)
                        .size(18)
                        .text_size(13),
                    text("Every program in this folder can be started as administrator.")
                        .size(12)
                        .color(TEXT2),
                ]
                .spacing(8),
            ));
            content = content.push(banner(
                glyph::WARNING,
                "Make sure standard users can't add or change files in this folder.".to_string(),
                AMBER,
                AMBER_TINT,
            ));
        }
        Kind::Pattern => {
            content = content.push(section(
                "Rule",
                row![
                    text(&item.raw)
                        .font(MONO)
                        .size(12)
                        .width(Length::Fill)
                        .wrapping(Wrapping::WordOrGlyph),
                    button(label(glyph::EDIT, "Edit", TEXT))
                        .padding([7, 12])
                        .style(subtle_button)
                        .on_press(Message::OpenRuleDialog(Some(item.raw.clone()))),
                ]
                .spacing(8)
                .align_y(Alignment::Center),
            ));
        }
    }
    content = content.push(shortcut_section(app, item));
    content = content.push(row![
        space::horizontal(),
        button(label(glyph::DELETE, "Remove", RED))
            .padding([8, 12])
            .style(danger_button)
            .on_press(Message::AskRemove),
    ]);
    panel(
        scrollable(content.padding(Padding {
            right: 12.0,
            ..Padding::ZERO
        }))
        .height(Length::Fill)
        .into(),
    )
}

fn toast(t: &Toast) -> El<'_> {
    let (fg, bg, code) = if t.ok {
        (GREEN, GREEN_TINT, glyph::CHECK)
    } else {
        (RED, RED_TINT, glyph::WARNING)
    };
    container(
        row![
            icon(code, 14, fg),
            text(&t.text).size(13).color(fg).width(Length::Fill),
            button(icon(glyph::CLOSE, 10, fg))
                .padding(6)
                .style(ghost_button)
                .on_press(Message::DismissToast),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .padding([8, 14])
    .width(Length::Fill)
    .style(style::tint(bg, fg))
    .into()
}

fn buttons<'a>(primary: El<'a>) -> El<'a> {
    row![
        space::horizontal(),
        button(text("Cancel").size(13))
            .padding([8, 16])
            .style(subtle_button)
            .on_press(Message::CloseDialog),
        primary,
    ]
    .spacing(8)
    .into()
}

fn dialog_view<'a>(app: &'a App, dialog: &'a Dialog) -> El<'a> {
    match dialog {
        Dialog::Rule { original, text: value, error } => {
            let examples = [
                (r"C:\Tools\app.exe", "one program"),
                (r"C:\Tools\", "programs in this folder (C:\\Tools\\** adds subfolders)"),
                (r"\\server\share\setup*.exe", "* = any name, ** = any folders, ? = one character"),
                (r#""C:\App\setup.exe" /S"#, "exactly these arguments"),
                (r#""C:\App\viewer.exe" %1"#, "may open one file"),
            ];
            let help = Column::with_children(examples.into_iter().map(|(rule, meaning)| {
                row![text(rule).font(MONO).size(12).width(Length::FillPortion(1)), text(meaning).size(12).color(TEXT2).width(Length::FillPortion(1))]
                    .spacing(12)
                    .into()
            }))
            .spacing(6);
            let mut content = column![
                text(if original.is_some() { "Edit rule" } else { "Add a rule" }).font(SEMIBOLD).size(18),
                text_input(r"C:\Tools\app.exe", value)
                    .on_input(Message::RuleText)
                    .on_submit(Message::SaveRule)
                    .padding(10)
                    .size(13)
                    .font(MONO)
                    .style(style::input),
            ]
            .spacing(14);
            if let Some(error) = error {
                content = content.push(text(error).size(12).color(RED));
            }
            content = content.push(container(help).padding(12).width(Length::Fill).style(style::tint(CHIP, TEXT)));
            content.push(buttons(
                button(text("Save rule").size(13)).padding([8, 16]).style(accent_button).on_press(Message::SaveRule).into(),
            ))
            .into()
        }
        Dialog::ConfirmRemove => {
            let name = app.selected_item().map(|i| i.name.as_str()).unwrap_or("this entry");
            column![
                text(format!("Remove {name}?")).font(SEMIBOLD).size(18),
                text("Users will no longer be able to start it as administrator. Its shortcuts and Open with entry stop working.")
                    .size(13)
                    .color(TEXT2),
                buttons(button(text("Remove").size(13)).padding([8, 16]).style(danger_button).on_press(Message::Remove).into()),
            ]
            .spacing(14)
            .into()
        }
        Dialog::ConfirmUninstall => column![
            text("Uninstall RunAdminGate?").font(SEMIBOLD).size(18),
            text("This stops the service and removes the allow-list, all Open with entries and the installed program files.")
                .size(13)
                .color(TEXT2),
            buttons(button(text("Uninstall").size(13)).padding([8, 16]).style(danger_button).on_press(Message::Uninstall).into()),
        ]
        .spacing(14)
        .into(),
    }
}

fn modal<'a>(content: El<'a>, width: f32) -> El<'a> {
    opaque(
        center(
            container(content)
                .padding(24)
                .width(width)
                .style(style::dialog),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(style::backdrop),
    )
}

pub fn view(app: &App) -> El<'_> {
    let body = row![list_panel(app), details(app)]
        .spacing(16)
        .padding(Padding {
            top: 0.0,
            right: 24.0,
            bottom: 16.0,
            left: 24.0,
        })
        .height(Length::Fill);
    let mut root = column![header(app), body];
    if let Some(t) = &app.toast {
        root = root.push(container(toast(t)).padding(Padding {
            top: 0.0,
            right: 24.0,
            bottom: 16.0,
            left: 24.0,
        }));
    }
    let base = container(root)
        .style(style::page)
        .width(Length::Fill)
        .height(Length::Fill);

    let overlay: Option<El> = if let Some(caption) = app.busy {
        Some(modal(
            row![icon(glyph::SHIELD, 18, ACCENT), text(caption).size(14)]
                .spacing(12)
                .align_y(Alignment::Center)
                .into(),
            360.0,
        ))
    } else if let Some(dialog) = &app.dialog {
        Some(modal(dialog_view(app, dialog), 560.0))
    } else if app.hovering {
        Some(modal(
            column![
                icon(glyph::UPLOAD, 32, ACCENT),
                text("Drop to allow this app or folder")
                    .font(SEMIBOLD)
                    .size(15)
            ]
            .spacing(10)
            .align_x(Alignment::Center)
            .into(),
            360.0,
        ))
    } else {
        None
    };
    match overlay {
        Some(overlay) => stack![base, overlay].into(),
        None => base.into(),
    }
}
