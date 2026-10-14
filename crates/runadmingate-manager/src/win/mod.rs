mod app;
mod model;
mod platform;
mod shortcut;
mod style;
mod view;

use windows::Win32::System::Com::{
    CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};

pub fn run() -> i32 {
    // Shell links and icon extraction need COM on the UI thread.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
    }
    let result = iced::application(app::App::new, app::App::update, view::view)
        .title("RunAdminGate")
        .theme(|_: &app::App| style::theme())
        .subscription(app::App::subscription)
        .default_font(style::FONT)
        .window(iced::window::Settings {
            icon: iced::window::icon::from_rgba(
                style::APP_ICON_RGBA.to_vec(),
                style::APP_ICON_SIZE,
                style::APP_ICON_SIZE,
            )
            .ok(),
            size: iced::Size::new(1080.0, 700.0),
            min_size: Some(iced::Size::new(880.0, 560.0)),
            ..Default::default()
        })
        .centered()
        .run();
    i32::from(result.is_err())
}
