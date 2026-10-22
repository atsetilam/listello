//! `listello-settings`: the window that edits a user's Touch Bar (D8 amendment).
//!
//! The editing, checking and drawing are Rust (`model`, `picture`); the window is QML with
//! Kirigami, so it looks like the rest of Plasma. Nothing reaches the Touch Bar until Apply,
//! which writes `~/.config/listello/config.toml` for the agent to pick up.
//!
//! `--tray` runs the system-tray icon instead (`tray`).

mod bridge;
mod keys;
mod lifecycle;
mod model;
mod picture;
mod strings;
mod tray;

fn main() {
    let arg = std::env::args().nth(1);
    if let Some("--version" | "-V") = arg.as_deref() {
        println!("listello-settings {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    cxx_qt::init_crate!(listello_settings);
    if arg.as_deref() == Some("--tray") {
        std::process::exit(tray::ffi::run_tray());
    }
    cxx_qt::init_qml_module!("org.listello.settings");
    std::process::exit(bridge::qobject::run_settings_window());
}
