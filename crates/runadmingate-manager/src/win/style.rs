//! Look and feel: Windows 11 light palette, Segoe UI, Fluent-style controls.

use iced::font::Weight;
use iced::theme::Palette;
use iced::widget::{button, container, text_input};
use iced::{Background, Border, Color, Font, Shadow, Theme};

pub const FONT: Font = Font::with_name("Segoe UI");
pub const SEMIBOLD: Font = Font {
    weight: Weight::Semibold,
    ..FONT
};
/// Icon glyphs ship with Windows 10 and 11.
pub const ICONS: Font = Font::with_name("Segoe MDL2 Assets");

pub const BG: Color = Color::from_rgb8(0xF3, 0xF3, 0xF3);
pub const CARD: Color = Color::WHITE;
pub const BORDER: Color = Color::from_rgb8(0xE5, 0xE5, 0xE5);
pub const TEXT: Color = Color::from_rgb8(0x1B, 0x1B, 0x1B);
pub const TEXT2: Color = Color::from_rgb8(0x5F, 0x5F, 0x5F);
pub const TEXT3: Color = Color::from_rgb8(0x8A, 0x8A, 0x8A);
pub const ACCENT: Color = Color::from_rgb8(0x00, 0x5F, 0xB8);
pub const ACCENT_HOVER: Color = Color::from_rgb8(0x19, 0x6E, 0xBF);
pub const ACCENT_PRESSED: Color = Color::from_rgb8(0x00, 0x4C, 0x93);
pub const ACCENT_TINT: Color = Color::from_rgb8(0xE5, 0xF0, 0xFA);
pub const HOVER: Color = Color::from_rgb8(0xF5, 0xF5, 0xF5);
pub const GREEN: Color = Color::from_rgb8(0x0F, 0x7B, 0x0F);
pub const GREEN_TINT: Color = Color::from_rgb8(0xDF, 0xF6, 0xDD);
pub const AMBER: Color = Color::from_rgb8(0x9D, 0x5D, 0x00);
pub const AMBER_TINT: Color = Color::from_rgb8(0xFF, 0xF4, 0xCE);
pub const RED: Color = Color::from_rgb8(0xC4, 0x2B, 0x1C);
pub const RED_TINT: Color = Color::from_rgb8(0xFD, 0xE7, 0xE9);
pub const CHIP: Color = Color::from_rgb8(0xF0, 0xF0, 0xF0);

/// Segoe MDL2 Assets code points.
pub mod glyph {
    pub const SHIELD: char = '\u{EA18}';
    pub const ADD: char = '\u{E710}';
    pub const FOLDER: char = '\u{E8B7}';
    pub const CODE: char = '\u{E943}';
    pub const SEARCH: char = '\u{E721}';
    pub const DELETE: char = '\u{E74D}';
    pub const LINK: char = '\u{E71B}';
    pub const WARNING: char = '\u{E7BA}';
    pub const CHECK: char = '\u{E73E}';
    pub const CLOSE: char = '\u{E711}';
    pub const COPY: char = '\u{E8C8}';
    pub const APP: char = '\u{ECAA}';
    pub const EDIT: char = '\u{E70F}';
    pub const SPARKLE: char = '\u{E945}';
    pub const UPLOAD: char = '\u{E898}';
}

pub fn theme() -> Theme {
    Theme::custom(
        "RunAdminGate",
        Palette {
            background: BG,
            text: TEXT,
            primary: ACCENT,
            success: GREEN,
            warning: AMBER,
            danger: RED,
        },
    )
}

fn border(color: Color, radius: f32) -> Border {
    Border {
        color,
        width: 1.0,
        radius: radius.into(),
    }
}

pub fn page(_: &Theme) -> container::Style {
    container::Style {
        background: Some(BG.into()),
        text_color: Some(TEXT),
        ..Default::default()
    }
}

// No shadows anywhere: tiny-skia accumulates them in partially redrawn regions.
pub fn card(_: &Theme) -> container::Style {
    container::Style {
        background: Some(CARD.into()),
        border: border(BORDER, 8.0),
        ..Default::default()
    }
}

/// The app icon as 128x128 RGBA (same artwork as res/app.ico).
pub const APP_ICON_RGBA: &[u8] = include_bytes!("../../res/icon128.rgba");
pub const APP_ICON_SIZE: u32 = 128;

/// A rounded tinted area (badges, banners).
pub fn tint(background: Color, foreground: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(background.into()),
        text_color: Some(foreground),
        border: Border {
            radius: 6.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

pub fn backdrop(_: &Theme) -> container::Style {
    container::Style {
        background: Some(
            Color {
                a: 0.35,
                ..Color::BLACK
            }
            .into(),
        ),
        ..Default::default()
    }
}

pub fn dialog(_: &Theme) -> container::Style {
    container::Style {
        background: Some(CARD.into()),
        border: border(Color::from_rgb8(0xCC, 0xCC, 0xCC), 10.0),
        ..Default::default()
    }
}

pub fn accent_button(_: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => ACCENT_HOVER,
        button::Status::Pressed => ACCENT_PRESSED,
        button::Status::Disabled => Color::from_rgb8(0xBF, 0xBF, 0xBF),
        button::Status::Active => ACCENT,
    };
    button::Style {
        background: Some(background.into()),
        text_color: Color::WHITE,
        border: Border {
            radius: 4.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

pub fn subtle_button(_: &Theme, status: button::Status) -> button::Style {
    let (background, text_color) = match status {
        button::Status::Hovered => (HOVER, TEXT),
        button::Status::Pressed => (Color::from_rgb8(0xEB, 0xEB, 0xEB), TEXT2),
        button::Status::Disabled => (CARD, TEXT3),
        button::Status::Active => (CARD, TEXT),
    };
    button::Style {
        background: Some(background.into()),
        text_color,
        border: border(BORDER, 4.0),
        ..Default::default()
    }
}

pub fn ghost_button(_: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => Some(Background::Color(HOVER)),
        button::Status::Pressed => Some(Background::Color(Color::from_rgb8(0xEB, 0xEB, 0xEB))),
        _ => None,
    };
    button::Style {
        background,
        text_color: if status == button::Status::Disabled {
            TEXT3
        } else {
            TEXT2
        },
        border: Border {
            radius: 4.0.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

pub fn danger_button(_: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => Some(Background::Color(RED_TINT)),
        _ => None,
    };
    button::Style {
        background,
        text_color: RED,
        border: border(Color::from_rgb8(0xF1, 0xBB, 0xBC), 4.0),
        ..Default::default()
    }
}

pub fn row_button(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let background = match (selected, status) {
            (true, _) => Some(Background::Color(ACCENT_TINT)),
            (false, button::Status::Hovered | button::Status::Pressed) => {
                Some(Background::Color(HOVER))
            }
            _ => None,
        };
        button::Style {
            background,
            text_color: TEXT,
            border: Border {
                color: if selected {
                    Color::from_rgb8(0xB3, 0xD3, 0xF0)
                } else {
                    Color::TRANSPARENT
                },
                width: 1.0,
                radius: 6.0.into(),
            },
            ..Default::default()
        }
    }
}

pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: CARD.into(),
        border: Border {
            color: if focused { ACCENT } else { BORDER },
            width: if focused { 1.5 } else { 1.0 },
            radius: 4.0.into(),
        },
        icon: TEXT3,
        placeholder: TEXT3,
        value: TEXT,
        selection: Color { a: 0.3, ..ACCENT },
    }
}

pub fn select(
    _: &Theme,
    status: iced::widget::pick_list::Status,
) -> iced::widget::pick_list::Style {
    use iced::widget::pick_list::Status;
    let hovered = matches!(
        status,
        Status::Hovered | Status::Opened { is_hovered: true }
    );
    iced::widget::pick_list::Style {
        text_color: TEXT,
        placeholder_color: TEXT3,
        handle_color: TEXT2,
        background: (if hovered { HOVER } else { CARD }).into(),
        border: border(BORDER, 4.0),
    }
}

pub fn select_menu(_: &Theme) -> iced::overlay::menu::Style {
    iced::overlay::menu::Style {
        background: CARD.into(),
        border: border(BORDER, 6.0),
        text_color: TEXT,
        selected_text_color: TEXT,
        selected_background: ACCENT_TINT.into(),
        shadow: Shadow::default(),
    }
}
