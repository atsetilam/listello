// Retained from tiny-dfr (https://github.com/AsahiLinux/tiny-dfr)
// Copyright The Asahi Linux Contributors
// SPDX-License-Identifier: MIT
//
// M1 phase 06: this is the file the header on the old version of this file said M1 would
// eventually dismantle. `FunctionLayer`, `Button`, icon/SVG/PNG loading, battery rendering and
// the Fn-triggered media layer are gone; `listello-core` now builds a `listello_ui::Shell` from
// `listello_ui::UiConfig` and drives it exactly the way `listello-sim`'s `main.rs` does — the
// same `Shell`, a `DrmSurface` and libinput events in place of a `SimulatorSurface` and winit's.
// What's left of tiny-dfr here is the outer crash-bitmap wrapper, uinput setup, and the
// privilege drop — the parts that were never about a fixed row of buttons to begin with.
//
// `config.rs`'s header lists what this dropped and why: icons, battery, the Fn layer,
// pixel-shift and adaptive-brightness *configurability* (brightness adaptation itself is kept,
// just no longer settable from the widget config file). None of that is a decision reversed —
// D8/D3/D10 all still hold — it is a milestone's worth of feature surface `UiConfig` does not
// describe yet, recorded here rather than quietly ported around.

use chrono::{Local, Timelike};
use input_linux::{uinput::UInputHandle, EventKind, Key, SynchronizeKind};
use input_linux_sys::{input_event, input_id, timeval, uinput_setup};
use libc::c_char;
use listello_protocol::{AgentToCore, AppId, CoreToAgent, Event, Limitation};
use nix::sys::signal::{SigSet, Signal};
use privdrop::PrivDrop;
use serde::de::value::{Error as DeError, StrDeserializer};
use serde::Deserialize;
use std::{
    collections::HashSet,
    fs::OpenOptions,
    os::fd::AsRawFd,
    panic::{self, AssertUnwindSafe},
    path::Path,
    time::{Duration, Instant},
};

mod agent_socket;
mod backlight;
mod clock;
mod config;
mod event_loop;
mod fonts;
mod panel_mode;

use agent_socket::AgentSocket;
use backlight::BacklightManager;
use config::{load_font, ConfigManager};
use event_loop::EventPump;
use listello_surface::{Color, DrmSurface, Surface, TextStyle};
use listello_ui::{Action, IconKey, KeyName, Shell, Theme, UiConfig};

const TIMEOUT_MS: i32 = 10 * 1000;
/// On a T2 MacBook the renderer leaves this long after the last agent has gone (or after it
/// started with none), which hands the Touch Bar back to the T2's own keys. Long enough for a
/// reload or an agent restart not to flash them.
const ALONE_GRACE: Duration = Duration::from_secs(8);
/// Pock's text size on the panel, measured from its screenshots: a regular weight, not bold.
const FONT_SIZE: f64 = 23.0;
const BUTTON_COLOR_INACTIVE: f64 = 0.200;
const BUTTON_COLOR_ACTIVE: f64 = 0.400;
const BUTTON_RADIUS: f64 = 8.0;
/// The size app icons are requested at, and the size the Dock draws them: icons are drawn at
/// their natural size, never scaled.
const APP_ICON_PX: u32 = 40;

/// The pattern `load_font` resolves through fontconfig. Was a config field (`FontTemplate`);
/// `UiConfig` has no rendering-choice fields (`config.rs`'s header), so it is a constant until
/// a milestone gives `Theme` a reason to be reloadable too.
const FONT_TEMPLATE: &str = "Inter";

fn main() {
    match std::env::args().nth(1).as_deref() {
        // Run by systemd around the renderer, as root; see `panel_mode`.
        Some("--panel-mode") => {
            match std::env::args().nth(2).as_deref() {
                Some("2") => panel_mode::start(),
                Some("after-stop") => panel_mode::after_stop(),
                _ => {
                    eprintln!("usage: listello-core --panel-mode 2|after-stop");
                    std::process::exit(2);
                }
            }
            return;
        }
        Some("--version" | "-V") => {
            println!("listello-core {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        _ => {}
    }
    let mut surface = DrmSurface::open().unwrap();
    // `&mut dyn Surface`, not `&mut DrmSurface`: the whole program above this line is meant to
    // be unable to tell which backend it got, and passing the concrete type would let that rot
    // back in unnoticed (D10).
    let _ = panic::catch_unwind(AssertUnwindSafe(|| real_main(&mut surface)));

    // Everything above has panicked, so the crash bitmap goes in through the one path that
    // does not depend on any of it. It is stored in device order and bypasses the draw list
    // deliberately — see DrmSurface::blit_device_argb.
    let crash_bitmap = include_bytes!("crash_bitmap.raw");
    let mut data = Vec::with_capacity(crash_bitmap.len() * 8 * 4);
    for byte in crash_bitmap {
        for i in 0..8 {
            let color = if ((byte >> i) & 0x1) == 0 { 0xFF } else { 0x0 };
            data.extend_from_slice(&[color, color, color, color]);
        }
    }
    let _ = surface.blit_device_argb(&data);

    let mut sigset = SigSet::empty();
    sigset.add(Signal::SIGTERM);
    sigset.wait().unwrap();
}

fn build_theme() -> Theme {
    Theme {
        text: TextStyle::new(load_font(FONT_TEMPLATE), FONT_SIZE),
        background: Color::BLACK,
        foreground: Color::WHITE,
        button: Color::grey(BUTTON_COLOR_INACTIVE),
        button_pressed: Color::grey(BUTTON_COLOR_ACTIVE),
        // Same placeholder listello-ui's own tests and listello-sim use (W1) — M5's actual
        // strip (W8) is what gives this a real value, not this binary's own default.
        // "On / charging": green, where grey 0.8 was indistinguishable from the white bar.
        accent: Color::rgb(0.3, 0.85, 0.4),
        corner_radius: BUTTON_RADIUS,
    }
}

/// A config-supplied key name resolved to the evdev code it names. `hit.rs`'s doc comment on
/// `KeyName` calls this resolution out as `listello-core`'s job, deferred from `listello-ui`
/// because `input_linux::Key` does not build on macOS. Uses `Key`'s own `Deserialize` impl —
/// the same one the pre-M1 `ButtonConfig::action` field used — so accepted names are exactly
/// the ones documented at https://docs.rs/input-linux/latest/input_linux/enum.Key.html (e.g.
/// `"F5"`, `"LeftCtrl"` — not the `KEY_`-prefixed evdev spelling).
fn resolve_key(name: &str) -> Option<Key> {
    Key::deserialize(StrDeserializer::<DeError>::new(name)).ok()
}

fn emit<F>(uinput: &mut UInputHandle<F>, ty: EventKind, code: u16, value: i32)
where
    F: AsRawFd,
{
    uinput
        .write(&[input_event {
            value,
            type_: ty as u16,
            code,
            time: timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
        }])
        .unwrap();
}

/// The evdev keys a list of config names stands for; unknown names are reported and skipped.
fn resolve_keys(names: &[KeyName]) -> Vec<Key> {
    names
        .iter()
        .filter_map(|name| {
            let key = resolve_key(name.as_str());
            if key.is_none() {
                eprintln!("listello: unknown key name {:?}, ignoring", name.as_str());
            }
            key
        })
        .collect()
}

fn sync<F>(uinput: &mut UInputHandle<F>)
where
    F: AsRawFd,
{
    emit(
        uinput,
        EventKind::Synchronize,
        SynchronizeKind::Report as u16,
        0,
    );
}

/// Press the keys in order and leave them down. The compositor repeats a key that stays down,
/// which is how holding a media button behaves like holding a keyboard key (tiny-dfr's
/// `Button::set_active`, true on touch).
fn press_keys<F>(uinput: &mut UInputHandle<F>, names: &[KeyName])
where
    F: AsRawFd,
{
    let keys = resolve_keys(names);
    if keys.is_empty() {
        return;
    }
    for key in &keys {
        emit(uinput, EventKind::Key, *key as u16, 1);
    }
    sync(uinput);
}

/// Release the keys in reverse order — the other half of [`press_keys`].
fn release_keys<F>(uinput: &mut UInputHandle<F>, names: &[KeyName])
where
    F: AsRawFd,
{
    let keys = resolve_keys(names);
    if keys.is_empty() {
        return;
    }
    for key in keys.iter().rev() {
        emit(uinput, EventKind::Key, *key as u16, 0);
    }
    sync(uinput);
}

/// What a completed press asks for, executed. `Shell` only ever describes the action (D3); this
/// is the one place in the binary that turns `Keys` into uinput writes and agent requests into
/// socket messages.
fn execute<F>(uinput: &mut UInputHandle<F>, pump: &mut EventPump, action: Action)
where
    F: AsRawFd,
{
    match action {
        Action::None => {}
        // The shell's own business (which app's windows are showing); it never gets out.
        Action::ToggleApp(_) => {}
        Action::KeysDown(names) => press_keys(uinput, &names),
        Action::KeysUp(names) => release_keys(uinput, &names),
        // A one-shot combo; `Shell` itself turns `Keys` into down/up, so this is only reached
        // by a caller that builds the action by hand.
        Action::Keys(names) => {
            press_keys(uinput, &names);
            release_keys(uinput, &names);
        }
        // Everything below needs the user's session, so it goes to the agent (D11). With no
        // agent connected the press is dropped, which is the documented degraded behaviour.
        Action::System(action) => pump.send_to_agent(&CoreToAgent::SystemAction(action)),
        Action::Activate(id) => pump.send_to_agent(&CoreToAgent::ActivateWindow(id)),
        Action::SetPins(pins) => pump.send_to_agent(&CoreToAgent::SavePins(pins)),
        Action::Window(id, command) => {
            pump.send_to_agent(&CoreToAgent::WindowCommand { id, command })
        }
        Action::Launch(app_id) => pump.send_to_agent(&CoreToAgent::Launch { app_id }),
        Action::OpenPath(path) => pump.send_to_agent(&CoreToAgent::OpenPath { path }),
    }
}

fn describe(limit: &Limitation) -> String {
    match limit {
        Limitation::NoWindowManagement => "no window list: it is running from an uninstalled \
            path, or its .desktop file is missing or does not declare X-KDE-Wayland-Interfaces"
            .to_string(),
        Limitation::UnsupportedCompositor { name } => {
            format!("the compositor {name:?} is not supported")
        }
        Limitation::ServiceUnavailable { service } => format!("{service} is not available"),
        Limitation::NoWindowCapture => "no window previews: KWin refuses window pictures to \
            the agent; its .desktop file must declare \
            X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2"
            .to_string(),
    }
}

/// Ask the agent to watch every folder the strip shows. The agent answers with each one's
/// current state and then only changes; the paths never come back (D11).
fn watch_folders(ui: &listello_ui::UiConfig, pump: &mut EventPump) {
    for (key, path) in ui.watched_folders() {
        pump.send_to_agent(&CoreToAgent::WatchFolder { key, path });
    }
}

/// Ask the agent for the icon of every app the strip could draw and does not have yet.
/// `requested` remembers what was asked so an app with no icon is asked about once, not on
/// every window-list update.
fn request_missing_icons(shell: &Shell, pump: &mut EventPump, requested: &mut HashSet<AppId>) {
    for app_id in shell.wanted_app_ids() {
        if shell
            .state()
            .icons
            .get(&IconKey::App(app_id.clone()))
            .is_none()
            && requested.insert(app_id.clone())
        {
            pump.send_to_agent(&CoreToAgent::RequestIcon {
                app_id,
                size_px: APP_ICON_PX,
            });
        }
    }
}

/// Where the packaged SVG icons are installed.
const PACKAGED_ICON_DIR: &str = "/usr/share/listello";

/// Put every packaged icon the config's widgets name into the shell, so the strip has its
/// icons even with no agent running (D11). One that fails to load just means its widget draws
/// the label instead.
fn load_packaged_icons(shell: &mut Shell, ui: &listello_ui::UiConfig) {
    let names = ui.icon_names();
    let loaded = listello_surface::packaged::load_icons(Path::new(PACKAGED_ICON_DIR), &names);
    for missing in names
        .iter()
        .filter(|n| !loaded.iter().any(|(l, _)| l == *n))
    {
        eprintln!("listello: packaged icon {missing:?} not found in {PACKAGED_ICON_DIR}");
    }
    for (name, image) in loaded {
        shell.state_mut().icons.insert(IconKey::Named(name), image);
    }
}

/// What "follow the system" means with nobody's session to ask: the machine's `LC_TIME`, else
/// its `LANG`, from `/etc/locale.conf`. The daemon's own environment has neither.
fn machine_time_locale() -> Option<String> {
    let text = std::fs::read_to_string("/etc/locale.conf").ok()?;
    let value = |key: &str| {
        text.lines()
            .filter_map(|l| l.trim().strip_prefix(key)?.strip_prefix('='))
            .map(|v| v.trim().trim_matches('"').to_string())
            .find(|v| !v.is_empty())
    };
    value("LC_TIME").or_else(|| value("LANG"))
}

/// The layout the strip shows (D8 amendment): the connected user's own when they have one,
/// the machine's otherwise, with `[general] language` applied. "The system" is the user's
/// session while their agent is connected, the machine's when not.
fn effective_layout(
    machine: &UiConfig,
    user: Option<&UiConfig>,
    user_time_locale: Option<&str>,
    machine_time_locale: Option<&str>,
) -> UiConfig {
    let mut ui = user.unwrap_or(machine).clone();
    ui.resolve_language(user_time_locale.or(machine_time_locale));
    ui
}

/// Replace the strip with `ui`, keeping everything the agent has already said.
fn swap_layout<F>(
    shell: &mut Shell,
    ui: &UiConfig,
    theme: &Theme,
    uinput: &mut UInputHandle<F>,
    pump: &mut EventPump,
    requested_icons: &mut HashSet<AppId>,
) where
    F: AsRawFd,
{
    // Whatever is held down belongs to the strip about to be dropped.
    for action in shell.release_held() {
        execute(uinput, pump, action);
    }
    let mut old = std::mem::replace(
        shell,
        Shell::new(ui, theme.clone(), Local::now().fixed_offset()),
    );
    shell.carry_over_agent_state(&mut old);
    load_packaged_icons(shell, ui);
    if shell.state().agent_connected {
        watch_folders(ui, pump);
        request_missing_icons(shell, pump, requested_icons);
    }
}

fn real_main(surface: &mut dyn Surface) {
    let panel = surface.size();
    let mut uinput = UInputHandle::new(OpenOptions::new().write(true).open("/dev/uinput").unwrap());
    let mut backlight = BacklightManager::new();
    let cfg_mgr = ConfigManager::new();
    let loaded = cfg_mgr
        .load_config()
        .unwrap_or_else(|e| panic!("failed to load config: {e:#}"));
    let theme = build_theme();
    let mut settings = loaded.settings;
    let machine_locale = machine_time_locale();
    // The machine's layout, and the connected user's own and their `LC_TIME` while there is
    // one: what the strip shows is always `effective_layout` of these.
    let mut machine_ui = loaded.ui;
    let mut user_ui: Option<UiConfig> = None;
    let mut user_locale: Option<String> = None;
    let mut ui_config = effective_layout(&machine_ui, None, None, machine_locale.as_deref());
    let mut shell = Shell::new(&ui_config, theme.clone(), Local::now().fixed_offset());
    load_packaged_icons(&mut shell, &ui_config);

    // The agent socket is opened (or inherited from systemd) while still root: run by hand it
    // has to create /run/listello.
    let agent_socket = AgentSocket::open().unwrap_or_else(|e| panic!("agent socket: {e:#}"));

    // drop privileges to input and video group
    let groups = ["input", "video"];

    PrivDrop::default()
        .user("nobody")
        .group_list(&groups)
        .apply()
        .unwrap_or_else(|e| panic!("Failed to drop privileges: {}", e));

    // Everything that reads a raw fd (libinput's two seats, the udev power_supply monitor, the
    // config file's inotify watch) now lives behind `EventPump`, which is what turns readiness
    // into `listello_protocol::Event`s (M1 step 3). `cfg_mgr` is created above, before the
    // privilege drop, because arming the inotify watch on `/etc/listello/config.toml` needs
    // root; reading from the fd afterwards does not.
    let mut event_pump = EventPump::new(
        panel.width as u16,
        panel.height as u16,
        cfg_mgr,
        agent_socket,
    )
    .unwrap();

    uinput.set_evbit(EventKind::Key).unwrap();
    for k in Key::iter() {
        uinput.set_keybit(k).unwrap();
    }
    let mut dev_name_c = [0 as c_char; 80];
    let dev_name = "Dynamic Function Row Virtual Input Device".as_bytes();
    for i in 0..dev_name.len() {
        dev_name_c[i] = dev_name[i] as c_char;
    }
    uinput
        .dev_setup(&uinput_setup {
            id: input_id {
                bustype: 0x19,
                vendor: 0x1209,
                product: 0x316E,
                version: 1,
            },
            ff_effects_max: 0,
            name: dev_name_c,
        })
        .unwrap();
    uinput.dev_create().unwrap();

    // Forces the first iteration to present regardless of what `poll_events` returns.
    let mut needs_redraw = true;
    let mut requested_icons: HashSet<AppId> = HashSet::new();
    let mut layout_changed = false;
    // A T2 MacBook has its own keys to fall back on, so with nobody to serve the renderer goes
    // away and leaves the panel to them. Elsewhere there is no such fallback: it stays.
    let leaves_when_alone = panel_mode::is_t2();
    let mut alone_since = Some(Instant::now());

    loop {
        let ms_left = ((60 - Local::now().second()) * 1000) as i32;
        let mut next_timeout_ms = ms_left.min(TIMEOUT_MS);
        if let (true, Some(since)) = (leaves_when_alone, alone_since) {
            let left = ALONE_GRACE.saturating_sub(since.elapsed());
            if left.is_zero() {
                eprintln!("listello: no agent is running listello any more, leaving the Touch Bar to the T2");
                std::process::exit(0);
            }
            next_timeout_ms = next_timeout_ms.min(left.as_millis() as i32 + 1);
        }

        // The only thing that moves is the Dock's launch bounce; it asks for frames only while
        // an app is launching.
        event_pump.set_animating(shell.is_animating());

        // Blocks for at most `next_timeout_ms`, then hands back everything that happened as
        // `Event`s — D3's whole answer to "what can change state."
        for event in event_pump.poll_events(next_timeout_ms, &mut backlight) {
            match event {
                Event::ConfigReloaded => {
                    if let Some(reload) = event_pump.take_reload() {
                        // `[core]` is the machine's alone, whoever's layout is up.
                        settings = reload.settings;
                        machine_ui = reload.ui;
                        layout_changed = true;
                    }
                    needs_redraw = true;
                }
                Event::Input(input) => {
                    for action in shell.handle_input(input, panel) {
                        execute(&mut uinput, &mut event_pump, action);
                    }
                    needs_redraw = true;
                }
                Event::TimeChanged => needs_redraw = true,
                Event::Frame { elapsed_secs } => {
                    shell.advance(elapsed_secs);
                    needs_redraw = true;
                }
                Event::AgentConnected => {
                    alone_since = None;
                    shell.set_agent_connected(true);
                    eprintln!("listello: agent connected");
                    watch_folders(&ui_config, &mut event_pump);
                    request_missing_icons(&shell, &mut event_pump, &mut requested_icons);
                    needs_redraw = true;
                }
                Event::AgentDisconnected => {
                    alone_since = Some(Instant::now());
                    // Everything the agent told us is stale the moment it goes (D11); the
                    // function keys and media keys never depended on it.
                    shell.set_agent_connected(false);
                    requested_icons.clear();
                    // The user's layout leaves with them; the machine's comes back.
                    user_ui = None;
                    user_locale = None;
                    layout_changed = true;
                    eprintln!("listello: agent disconnected");
                    needs_redraw = true;
                }
                Event::Agent(AgentToCore::Restart) => {
                    // systemd starts us again (`RestartForceExitStatus`) with the panel left
                    // as it is; the agent reconnects.
                    eprintln!("listello: reloading, as the user asked");
                    std::process::exit(panel_mode::RELOAD_EXIT);
                }
                Event::Agent(AgentToCore::Quit) => {
                    if leaves_when_alone {
                        eprintln!(
                            "listello: listello was stopped, leaving the Touch Bar to the T2"
                        );
                        std::process::exit(0);
                    }
                }
                Event::Agent(msg) => {
                    // The agent sends each limitation once per cause, so logging every one
                    // here is logging it once.
                    if let AgentToCore::Limited(limit) = &msg {
                        eprintln!("listello: the agent reports: {}", describe(limit));
                    }
                    if let AgentToCore::UserConfig {
                        layout,
                        time_locale,
                    } = &msg
                    {
                        // The agent checked it already; it is checked again because nothing
                        // from the socket is trusted, and a bad one keeps the machine's up.
                        user_ui = layout.as_deref().and_then(|text| {
                            UiConfig::parse(text)
                                .inspect_err(|e| {
                                    eprintln!("listello: the user's layout is not valid: {e}")
                                })
                                .ok()
                        });
                        user_locale = time_locale.clone();
                        layout_changed = true;
                    }
                    shell.apply_agent(&msg);
                    request_missing_icons(&shell, &mut event_pump, &mut requested_icons);
                    needs_redraw = true;
                }
                // No DRM reset/shutdown handling yet (M3).
                Event::DisplayReset | Event::Shutdown => {}
            }
        }

        if std::mem::take(&mut layout_changed) {
            let ui = effective_layout(
                &machine_ui,
                user_ui.as_ref(),
                user_locale.as_deref(),
                machine_locale.as_deref(),
            );
            if ui != ui_config {
                swap_layout(
                    &mut shell,
                    &ui,
                    &theme,
                    &mut uinput,
                    &mut event_pump,
                    &mut requested_icons,
                );
                ui_config = ui;
                needs_redraw = true;
            }
        }

        // The row of windows opened, closed or changed: tell the agent what to capture.
        if let Some(msg) = shell.preview_request(panel) {
            event_pump.send_to_agent(&msg);
        }

        if needs_redraw {
            shell.set_now(Local::now().fixed_offset());
            // A failed present is not fatal. The panel can go away under us — suspend/resume
            // and appletbdrm rebinds are both expected conditions (RESEARCH §1) — and taking
            // the daemon down over one dropped frame is worse than skipping it.
            if let Err(e) = surface.present(&shell.frame(panel)) {
                eprintln!("listello: failed to present frame: {e:#}");
            }
            needs_redraw = false;
        }

        backlight.update_backlight(settings.adaptive_brightness, settings.active_brightness);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use listello_ui::WidgetConfig;

    #[test]
    fn the_users_layout_wins_while_connected_and_system_means_their_time_format() {
        let clock = |ui: &UiConfig| match &ui.widgets[0] {
            WidgetConfig::Clock { locale, .. } => locale.clone(),
            _ => panic!("not a clock"),
        };
        let machine = UiConfig::parse("[[widget]]\nkind = \"clock\"\n").unwrap();
        let user = UiConfig::parse(
            "[general]\nlanguage = \"english\"\n[[widget]]\nkind = \"clock\"\n[[widget]]\nkind = \"battery\"\n",
        )
        .unwrap();
        // Nobody connected: the machine's layout, in the machine's language.
        let ui = effective_layout(&machine, None, None, Some("de_DE.UTF-8"));
        assert_eq!(ui.widgets.len(), 1);
        assert_eq!(clock(&ui).as_deref(), Some("de_DE.UTF-8"));
        // A user with no layout of their own: the machine's, in the user's time format.
        let ui = effective_layout(&machine, None, Some("it_IT.UTF-8"), Some("de_DE.UTF-8"));
        assert_eq!(clock(&ui).as_deref(), Some("it_IT.UTF-8"));
        // A user with their own: theirs, in the language it names.
        let ui = effective_layout(&machine, Some(&user), Some("it_IT.UTF-8"), None);
        assert_eq!(ui.widgets.len(), 2);
        assert_eq!(clock(&ui).as_deref(), Some("en_US"));
    }

    /// A key name the UI crate accepts is only a well-formed string; the evdev name it stands
    /// for is resolved here, and an unresolvable one is silently a dead button. Every key in
    /// the shipped layouts must resolve.
    #[test]
    fn every_key_in_the_shipped_layouts_resolves_to_an_evdev_key() {
        for (name, text) in [
            (
                "config.toml",
                include_str!("../../../share/listello/config.toml"),
            ),
            (
                "config.example-dock.toml",
                include_str!("../../../share/listello/config.example-dock.toml"),
            ),
        ] {
            let cfg = UiConfig::parse(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            for widget in cfg.widgets.iter().chain(&cfg.fn_widgets) {
                let keys: Vec<&String> = match widget {
                    WidgetConfig::Button { keys, .. } => keys.iter().collect(),
                    WidgetConfig::ControlCenter { buttons, .. } => {
                        buttons.iter().flat_map(|b| &b.keys).collect()
                    }
                    _ => Vec::new(),
                };
                for key in keys {
                    assert!(resolve_key(key).is_some(), "{name}: {key:?} is not a key");
                }
            }
        }
    }
}
