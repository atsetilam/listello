//! The system-tray icon (D8 amendment): `listello-settings --tray`.
//!
//! Off unless `[general] tray = true` in the user's file. Plasma starts it at login from
//! `/etc/xdg/autostart`, and it leaves at once when the tray is off; Apply starts it when the
//! user turns it on, and it leaves by itself when they turn it off, since it watches the file.
//! One per user: a second copy sees the lock taken and leaves.
//!
//! The menu is Open settings, Reload listello, Check for updates and Stop listello. Check for
//! updates asks the agent, which does the looking and shows the answer as a notification (D9).
//!
//! The agent starts the tray as its own child (`listello-agent`'s `tray_child`), so it lives and
//! dies with listello: Stop listello (`lifecycle`) stops the agent's unit and takes the tray
//! with it. A stopped listello has no tray, so there is no Start item. The same file also
//! holds the few calls the C++ side makes before the settings window opens.

use crate::lifecycle;
use crate::strings::{self, Lang};
use std::fs::File;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[cxx_qt::bridge]
pub mod ffi {
    extern "Rust" {
        fn tray_enabled() -> bool;
        fn take_tray_lock() -> bool;
        fn tray_text(key: &str) -> String;
        fn config_dir_text() -> String;
        fn open_settings();
        fn reload_listello();
        fn check_for_updates();
        fn stop_listello();

        // For the settings window's start-up (`cpp/app.cpp`).
        fn lifecycle_enabled() -> bool;
        fn first_start_needed() -> bool;
        fn answer_first_start(every_time: bool);
        fn ensure_started();
        fn listello_stopped() -> bool;
    }

    unsafe extern "C++" {
        include!("listello-settings/cpp/tray.h");
        fn run_tray() -> i32;
    }
}

fn user_dir() -> Option<PathBuf> {
    listello_agent::desktop::config_dir().map(|d| d.join("listello"))
}

/// `[general]` of the user's file, read loosely: the tray follows its own switch even while
/// the layout in the same file does not validate.
fn general(key: &str) -> Option<toml_edit::Item> {
    let text = std::fs::read_to_string(user_dir()?.join("config.toml")).ok()?;
    let doc: toml_edit::DocumentMut = text.parse().ok()?;
    doc.get("general")?.get(key).cloned()
}

fn tray_enabled() -> bool {
    general("tray").and_then(|i| i.as_bool()) == Some(true)
}

/// Held until the process ends; the kernel lets go of it then.
fn take_tray_lock() -> bool {
    let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") else {
        return true;
    };
    let Ok(file) = File::create(PathBuf::from(dir).join("listello-tray.lock")) else {
        return true;
    };
    if file.try_lock().is_err() {
        return false;
    }
    std::mem::forget(file);
    true
}

fn tray_text(key: &str) -> String {
    let setting = general("language")
        .and_then(|i| i.as_str().map(str::to_string))
        .unwrap_or_default();
    strings::tr(Lang::from_setting(&setting), key)
}

fn config_dir_text() -> String {
    user_dir().map_or_else(String::new, |d| d.to_string_lossy().into_owned())
}

fn spawn_self(args: &[&str]) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    // A tray started before an upgrade runs a file that has since been replaced, and Linux
    // names it "… (deleted)"; the new one is at the old path.
    let exe = match exe.to_str().and_then(|p| p.strip_suffix(" (deleted)")) {
        Some(path) => PathBuf::from(path),
        None => exe,
    };
    let started = Command::new(exe).args(args).stdin(Stdio::null()).spawn();
    if let Err(e) = started {
        eprintln!("listello-settings: could not start {args:?}: {e}");
    }
}

fn open_settings() {
    spawn_self(&[]);
}

fn lifecycle_enabled() -> bool {
    lifecycle::enabled()
}

fn first_start_needed() -> bool {
    lifecycle::first_start()
}

fn answer_first_start(every_time: bool) {
    lifecycle::answer_first_start(every_time);
}

fn ensure_started() {
    lifecycle::ensure_started();
}

fn listello_stopped() -> bool {
    lifecycle::is_stopped()
}

/// "Reload listello": the agent passes it on to the renderer (`listello-agent`'s `reload`).
/// Only the agent's main process, not the apps it started or this tray.
pub fn reload_listello() {
    if !signal_agent("SIGUSR1") {
        eprintln!("listello-settings: could not ask the agent to reload");
    }
}

/// "Check for updates": the agent looks now and answers with a notification, whatever the
/// setting for the automatic check (`listello-agent`'s `updates`).
pub fn check_for_updates() {
    if !signal_agent("SIGHUP") {
        eprintln!("listello-settings: could not ask the agent to check for updates");
    }
}

fn signal_agent(signal: &str) -> bool {
    Command::new("systemctl")
        .args([
            "--user",
            "kill",
            "--kill-whom=main",
            &format!("--signal={signal}"),
            "listello-agent.service",
        ])
        .status()
        .is_ok_and(|s| s.success())
}

pub fn stop_listello() {
    lifecycle::stop();
}
