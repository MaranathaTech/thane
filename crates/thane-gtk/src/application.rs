use std::cell::OnceCell;

use gtk4::prelude::*;
use gtk4::gio;

use crate::css;
use crate::fonts;
use crate::window::AppWindow;

const APP_ID: &str = "com.thane.app";

/// Build and run the GTK application.
///
/// GTK ensures single-instance behavior via the application ID.
/// If a second instance is launched, it activates the existing window.
pub fn run_app() {
    let app = gtk4::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();

    app.connect_startup(|_| {
        fonts::load_bundled_fonts();
        css::load_css();
    });

    // Store the window so re-activation presents the existing one.
    let window_cell: std::rc::Rc<OnceCell<AppWindow>> = std::rc::Rc::new(OnceCell::new());

    {
        let wc = window_cell.clone();
        app.connect_activate(move |app| {
            if let Some(existing) = wc.get() {
                // Second activation — just present the existing window.
                existing.present();
            } else {
                let window = AppWindow::new(app);
                window.present();
                let _ = wc.set(window);
            }
        });
    }

    // Handle command-line arguments (for opening directories).
    app.connect_command_line(|app, _cmdline| {
        app.activate();
        0.into()
    });

    app.run();
}
