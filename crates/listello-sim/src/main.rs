//! The simulator binary: a Touch Bar-shaped window on a desktop.
//!
//! This is M1's exit criterion — one command on a Mac that does not run Linux yet, a window
//! shaped like the panel, and clicking things in it does something. `listello-core`
//! links libinput, uinput, udev and DRM and does not compile on macOS, so the simulator is a
//! separate binary rather than a mode of the daemon. See DESIGN.md's D10 addendum.
//!
//! # Who owns the main thread
//!
//! winit's usual mode is `EventLoop::run`: it takes the thread and calls *you* back, which
//! would invert D3's single-threaded loop and put the window in charge of the program.
//! `pump_app_events(Some(timeout), &mut app)` polls and returns instead, so the loop below
//! keeps ownership and winit becomes one more source drained per iteration — exactly the shape
//! `listello-core`'s `EventPump` gives the libinput fds. Both binaries feed the same `Shell`
//! (`listello-ui`); this one just supplies winit events and a `SimulatorSurface` where
//! `listello-core` supplies libinput events and a `DrmSurface`.
//!
//! # Two measured facts, both of which contradict the obvious reading
//!
//! - **The window does not exist on the first pump.** `resumed` fires asynchronously; with a
//!   short timeout the loop returns from pump 0 with no window at all. That is normal — skip
//!   the frame and pump again — and it is why nothing here unwraps a window.
//! - **`inner_size()` is in physical pixels.** A nominally 800 × 600 window on a Retina Mac
//!   measures 1600 × 1200. Everything is physical for that reason and `scale_factor()` is
//!   consulted exactly once, to *ask* for a window, and never again in the pixel or input
//!   paths where a stray factor would hide a bug.
//!
//! # What a press does here
//!
//! `Shell::handle_input` returns the `Action`s a completed press asks for. Executing a `Keys`
//! action means uinput, and `System` means D11's session round trip — both `listello-core`'s
//! job, not this binary's, and neither exists on macOS. This binary only ever prints what it
//! got; that is enough to prove the routing without a fake uinput.

mod input;

use anyhow::{anyhow, Result};
use cairo::{FontFace, FontSlant, FontWeight};
use chrono::Local;
use input::InputTranslator;
use listello_protocol::{
    AgentToCore, AppId, BatteryState, Event, FolderState, NetworkState, WindowInfo,
};
use listello_surface::{Color, SimulatorSurface, Size, Surface, TextStyle};
use listello_ui::{Shell, Theme, UiConfig};
use std::rc::Rc;
use std::time::Duration;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::platform::pump_events::{EventLoopExtPumpEvents, PumpStatus};
use winit::window::{Window, WindowId};

/// The reference panel geometry, in logical pixels.
///
/// **Unmeasured.** RESEARCH §1 says never to hardcode the resolution, and `DrmSurface` does
/// not: it reads the mode. Every other backend has to be told a number, and this is the
/// project's placeholder until M2 puts Linux on the MacBook and brings back the real one.
/// Naming it once is what makes replacing it a one-line change.
const PANEL: Size = Size::new(2008, 60);

/// How long each pump is allowed to block. ~60 Hz; the frame clock proper (`listello-protocol`'s
/// `Event::Frame`) is M1's loop but this binary has no animating widget yet to justify wiring
/// it up, so the pump timeout alone stands in as the redraw cadence.
const PUMP_TIMEOUT: Duration = Duration::from_millis(16);

/// A generic demo strip, used when no config path is given on the command line. **Not a personal
/// layout** (that is M5's job, per D8) — just enough widgets, of enough different kinds, to
/// show that config, layout, widgets and input all actually meet.
const DEFAULT_CONFIG: &str = "\
[strip]
gap = 12.0

[[widget]]
kind = \"button\"
label = \"F5\"
keys = [\"KEY_F5\"]
width = 220.0

[[widget]]
kind = \"separator\"
width = 2.0

[[widget]]
kind = \"clock\"
format = \"%H:%M:%S\"
flex = 1.0

[[widget]]
kind = \"separator\"
width = 2.0

[[widget]]
kind = \"button\"
label = \"F6\"
keys = [\"KEY_F6\"]
width = 220.0
";

fn main() -> Result<()> {
    let config = load_config()?;
    let mut event_loop = EventLoop::new()?;
    let mut app = SimApp::new(PANEL, &config)?;

    // The packaged icons, from the source tree: the simulator has no install directory.
    let share = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../share/listello"));
    for (name, image) in listello_surface::packaged::load_icons(share, &config.icon_names()) {
        app.shell
            .state_mut()
            .icons
            .insert(listello_ui::IconKey::Named(name), image);
    }

    if std::env::args().any(|a| a == "--fake-state") {
        apply_fake_state(&mut app.shell);
        println!(
            "--fake-state: pretending an agent is connected (battery, wifi, folders, windows)"
        );
    }

    loop {
        if let PumpStatus::Exit(code) = event_loop.pump_app_events(Some(PUMP_TIMEOUT), &mut app) {
            return match code {
                0 => Ok(()),
                c => Err(anyhow!("winit exited with status {c}")),
            };
        }

        for event in app.drain_events() {
            match event {
                Event::Shutdown => return Ok(()),
                Event::Input(input) => {
                    for action in app.shell.handle_input(input, PANEL) {
                        println!("action: {action:?}");
                    }
                }
                // Nothing in this binary sends the other variants yet (no agent, no config
                // reload, no frame clock) — matched explicitly so a new `Event` variant is a
                // compile error here, not a silently ignored one.
                Event::Frame { .. }
                | Event::Agent(_)
                | Event::AgentConnected
                | Event::AgentDisconnected
                | Event::TimeChanged
                | Event::ConfigReloaded
                | Event::DisplayReset => {}
            }
        }
        app.shell.set_now(Local::now().fixed_offset());
        app.present()?;

        if let Some(err) = app.take_error() {
            return Err(err);
        }
    }
}

/// Reads a config path from the first non-flag argument, or falls back to [`DEFAULT_CONFIG`]. A
/// bad path or a config that fails validation is an error the user sees on the terminal they
/// launched this from, not a panic — the same "reject, never crash" rule `UiConfig::parse`
/// enforces one layer down.
fn load_config() -> Result<UiConfig> {
    let mut config = parse_config()?;
    // "Follow the system" is this terminal's: `LC_ALL`, else `LC_TIME`, else `LANG`.
    let time_locale = ["LC_ALL", "LC_TIME", "LANG"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty());
    config.resolve_language(time_locale.as_deref());
    Ok(config)
}

fn parse_config() -> Result<UiConfig> {
    match std::env::args().skip(1).find(|a| !a.starts_with("--")) {
        Some(path) => {
            let src = std::fs::read_to_string(&path).map_err(|e| anyhow!("reading {path}: {e}"))?;
            UiConfig::parse(&src).map_err(|e| anyhow!("{path}: {e}"))
        }
        None => {
            UiConfig::parse(DEFAULT_CONFIG).map_err(|e| anyhow!("built-in default config: {e}"))
        }
    }
}

/// Feed the shell a plausible session: a battery, a wifi connection, three watched folders and
/// a handful of windows.
///
/// **A development switch, not a feature.** The Mac has no agent and never will — there is no
/// KWin here to list windows and no UPower to ask — so without this, every widget that draws
/// agent-supplied state renders its empty case forever and cannot be looked at. It goes through
/// `Shell::apply_agent` rather than writing `UiState` directly, so what it exercises is the
/// real path a real agent takes, message for message.
///
/// The one thing it cannot fake is an app icon: those are pixels the agent decodes from the
/// user's icon theme, and inventing them here would be inventing the wrong thing. The Dock's
/// no-icon fallback is what shows instead, which is worth seeing anyway.
fn apply_fake_state(shell: &mut Shell) {
    shell.set_agent_connected(true);
    shell.apply_agent(&AgentToCore::Battery(Some(BatteryState {
        charge: 0.62,
        charging: false,
        time_remaining_secs: Some(2 * 3600 + 40 * 60),
    })));
    shell.apply_agent(&AgentToCore::Network(Some(NetworkState {
        connected: true,
        signal: 0.75,
        ssid: Some("fake-network".into()),
    })));
    for (key, empty) in [
        ("screenshots", false),
        ("downloads", false),
        ("trash", true),
    ] {
        shell.apply_agent(&AgentToCore::Folder(FolderState {
            key: key.into(),
            empty,
        }));
    }
    let window = |id: u64, app: &str, title: &str, active: bool| WindowInfo {
        // Fully qualified: `WindowId` in this file is winit's, and these are the protocol's.
        id: listello_protocol::WindowId(id),
        app_id: AppId::new(app),
        title: title.into(),
        active,
        minimized: false,
    };
    shell.apply_agent(&AgentToCore::Windows(vec![
        window(1, "org.kde.dolphin", "Home", false),
        window(2, "firefox", "listello — GitHub", true),
        // Two windows of one app: D7's expanding row is the case that needs looking at.
        window(3, "firefox", "Asahi Linux", false),
        window(4, "org.kde.konsole", "user@laptop:~", false),
    ]));
}

/// winit's side of the loop: it owns the window and the surface, and it only ever *collects*
/// events for `main` to feed the `Shell`. Reacting to them is the `Shell`'s job.
struct SimApp {
    panel: Size,
    surface: Option<SimulatorSurface>,
    translator: InputTranslator,
    events: Vec<Event>,
    /// `ApplicationHandler`'s methods cannot return a `Result`, so a failure inside one is
    /// parked here and picked up by the loop. Losing a window is not a panic.
    error: Option<anyhow::Error>,
    shell: Shell,
}

impl SimApp {
    fn new(panel: Size, config: &UiConfig) -> Result<SimApp> {
        // Cairo's toy API: no fontconfig, therefore it works on the Mac. Wrapped in an `Rc`
        // immediately and never cloned — cloning a `FontFace` aborts the process in this
        // crate's feature set (RESEARCH §2.2).
        let font = FontFace::toy_create("sans-serif", FontSlant::Normal, FontWeight::Normal)?;
        let theme = Theme {
            // listello-core's theme, so the simulator looks like the panel.
            text: TextStyle::new(Rc::new(font), 23.0),
            background: Color::BLACK,
            foreground: Color::WHITE,
            button: Color::grey(0.2),
            button_pressed: Color::grey(0.4),
            accent: Color::rgb(0.3, 0.85, 0.4),
            corner_radius: 8.0,
        };
        let now = Local::now().fixed_offset();
        Ok(SimApp {
            panel,
            surface: None,
            translator: InputTranslator::default(),
            events: Vec::new(),
            error: None,
            shell: Shell::new(config, theme, now),
        })
    }

    fn drain_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    fn take_error(&mut self) -> Option<anyhow::Error> {
        self.error.take()
    }

    fn present(&mut self) -> Result<()> {
        let Some(surface) = self.surface.as_mut() else {
            return Ok(()); // no window yet — normal on the first pumps
        };
        surface.present(&self.shell.frame(self.panel))
    }
}

impl ApplicationHandler for SimApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.surface.is_some() {
            return;
        }
        // Ask for a window whose *physical* size lands near the panel's. A hint, not a
        // guarantee — an OS that clamps a 2008-point-wide window is a non-event, because the
        // blit already handles any window size.
        let scale = event_loop
            .primary_monitor()
            .map(|m| m.scale_factor())
            .unwrap_or(1.0)
            .max(1.0);
        let attrs = Window::default_attributes()
            .with_title("listello — simulator")
            .with_inner_size(winit::dpi::LogicalSize::new(
                self.panel.width as f64 / scale,
                self.panel.height as f64 / scale,
            ));

        match event_loop
            .create_window(attrs)
            .map_err(anyhow::Error::from)
            .and_then(|w| SimulatorSurface::new(Rc::new(w), self.panel))
        {
            Ok(s) => self.surface = Some(s),
            Err(e) => self.error = Some(e),
        }
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let window = match self.surface.as_ref() {
            Some(s) => s.window_size(),
            // No window means no coordinate frame to map into, and the only event worth
            // keeping without one is the one that ends the program.
            None => Size::new(0, 0),
        };
        if let Some(e) = self.translator.translate(&event, window, self.panel) {
            self.events.push(e);
        }
    }
}
