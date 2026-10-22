//! listello's session agent.
//!
//! It runs as the user inside their graphical session (`systemd --user`) and supplies
//! everything that needs session access: the window list (`WindowSource`, KDE first, D4),
//! `.desktop` and icon resolution, and launching things. It knows nothing about DRM, drawing
//! or layout.
//!
//! Whatever happens here, the Touch Bar keeps working: if this process dies, the renderer
//! falls back to function keys and media keys (D11).

mod actions;
mod agent;
mod capture;
mod folders;
mod kwin;
mod layout;
mod link;
mod live;
mod power;
mod previews;
mod reload;
mod tray_child;
mod wifi;
mod windows;

use listello_agent::{desktop, icons, pins};

use agent::{Agent, Sensors};
use desktop::{Catalog, Overrides};
use link::{LinkEvent, Reconnecting};
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use std::time::{Duration, Instant};
use windows::Probe;

/// Wake at least this often, to reap children and notice a stalled connection.
const LONGEST_WAIT: Duration = Duration::from_secs(30);

fn main() {
    if let Some(arg) = std::env::args().nth(1) {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("listello-agent {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--list" => return list_windows(),
            "--capture" => return capture_window(),
            _ => {
                println!(
                    "listello-agent {}\n\n\
                     The session half of listello. Run it as your own user inside your graphical \
                     session; it connects to listello-core by itself and keeps reconnecting.\n\n\
                     Usage: listello-agent [--version | --list | --capture ID FILE | --help]\n  --list      print the windows the agent can see, then exit\n  --capture   save a picture of one window as PNG, to check previews work",
                    env!("CARGO_PKG_VERSION")
                );
                return;
            }
        }
    }

    // First, before anything can start a thread.
    let mut reload = reload::Reload::new();
    let (source, limitation) = match windows::probe() {
        Probe::Ready(source) => {
            eprintln!("listello-agent: reading windows from KWin");
            (Some(source), None)
        }
        Probe::Unavailable(limit) => (None, Some(limit)),
    };
    let catalog = Catalog::scan(Overrides::load());
    let theme = icons::current_theme();
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute());
    let folders = folders::Folders::new(desktop::home(), data_home);
    let mut agent = Agent::new(source, limitation, catalog, theme)
        .with_sensors(Sensors::system(), folders)
        .with_pins(pins::default_path())
        .with_previews(
            previews::Previews::new(
                capture::Capturer::connect().map(|c| Box::new(c) as Box<dyn previews::Grab>),
            )
            .with_streams(live::Live::connect().map(|l| Box::new(l) as Box<dyn previews::Streams>)),
        );
    let mut user_layout = layout::UserLayout::new(
        desktop::config_dir().map(|d| d.join("listello")),
        layout::time_locale(),
    );
    let mut link = Reconnecting::default_path();
    let mut tray = tray_child::TrayChild::new();
    tray.sync(user_layout.tray_wanted());

    loop {
        // What to wait on: the renderer, the compositor, and the next reconnect attempt.
        let mut timeout = LONGEST_WAIT.min(agent.until_sensor_read(Instant::now()));
        if let Some(until) = link.until_retry() {
            timeout = timeout.min(until);
        }
        if let Some(until) = agent.until_previews(Instant::now()) {
            timeout = timeout.min(until);
        }
        let mut fds = Vec::new();
        if let Some(fd) = link.fd() {
            let mut flags = PollFlags::POLLIN;
            if link.wants_write() {
                flags |= PollFlags::POLLOUT;
            }
            fds.push(PollFd::new(fd, flags));
        }
        if let Some(source) = agent.source() {
            fds.push(PollFd::new(source.poll_fd(), PollFlags::POLLIN));
        }
        if let Some(fd) = agent.folders_fd() {
            fds.push(PollFd::new(fd, PollFlags::POLLIN));
        }
        if let Some(fd) = agent.live_fd() {
            fds.push(PollFd::new(fd, PollFlags::POLLIN));
        }
        if let Some(fd) = user_layout.fd() {
            fds.push(PollFd::new(fd, PollFlags::POLLIN));
        }
        if let Some(fd) = reload.fd() {
            fds.push(PollFd::new(fd, PollFlags::POLLIN));
        }
        let wait = PollTimeout::from(timeout.as_millis().min(u16::MAX as u128) as u16);
        let _ = poll(&mut fds, wait);

        agent.reap();
        tray.reap();

        // Compositor first: a changed window list goes out in the same turn.
        let mut outgoing = Vec::new();
        if let Some(source) = agent.source_mut() {
            match source.dispatch() {
                Ok(true) => outgoing = agent.windows_changed(),
                Ok(false) => {}
                Err(e) => {
                    eprintln!("listello-agent: lost the compositor: {e:#}");
                    // Without a compositor there is nothing to serve; exit so systemd
                    // restarts us into the next session.
                    std::process::exit(1);
                }
            }
        }

        // PipeWire before the previews, so frames that just arrived go out this turn. Run every
        // turn, not only when its descriptor woke us: it is cheap, and stream set-up and
        // tear-down queue work on its loop too.
        agent.dispatch_live();
        outgoing.extend(agent.folders_changed());
        outgoing.extend(agent.poll_sensors(Instant::now()));
        outgoing.extend(agent.poll_previews(Instant::now()));
        let (layout, pins_touched) = user_layout.changed();
        outgoing.extend(layout);
        if user_layout.take_touched() {
            tray.sync(user_layout.tray_wanted());
        }
        // The settings window writes the pins too; the Touch Bar's own saves come back here
        // as well, which only repeats what the renderer already has.
        if pins_touched {
            outgoing.push(listello_protocol::AgentToCore::Pins(agent.pins()));
        }
        let reload_asked = reload.asked(Instant::now());
        if reload.quit() {
            // Stopped: tell the renderer to give the panel back now, then go.
            eprintln!("listello-agent: stopping");
            let _ = link.send(&listello_protocol::AgentToCore::Quit);
            std::process::exit(0);
        }
        if reload_asked {
            if link.is_connected() {
                eprintln!("listello-agent: reloading the renderer");
                outgoing.push(listello_protocol::AgentToCore::Restart);
            } else {
                eprintln!("listello-agent: asked to reload, but the renderer is not connected");
            }
        }

        for event in link.poll() {
            match event {
                LinkEvent::Connected => {
                    eprintln!("listello-agent: connected to the renderer");
                    outgoing = agent.greeting();
                    outgoing.push(user_layout.greeting());
                }
                LinkEvent::Message(msg) => outgoing.extend(agent.handle(msg)),
                LinkEvent::Disconnected => {
                    eprintln!("listello-agent: renderer gone, waiting for it");
                    agent.disconnected();
                    outgoing.clear();
                }
            }
        }

        for msg in &outgoing {
            if let Some(LinkEvent::Disconnected) = link.send(msg) {
                break;
            }
        }
    }
}

/// `--list`: what the agent sees right now. The first thing to run when the Dock is empty.
/// `--capture ID FILE`: one window, by the number `--list` prints, at full size.
fn capture_window() {
    let args: Vec<String> = std::env::args().skip(2).collect();
    let [id, file] = args.as_slice() else {
        eprintln!("usage: listello-agent --capture ID FILE");
        std::process::exit(2);
    };
    let Ok(id) = id.parse::<u64>() else {
        eprintln!("not a window number: {id}");
        std::process::exit(2);
    };
    let source = match windows::probe() {
        Probe::Ready(source) => source,
        Probe::Unavailable(limit) => {
            eprintln!("no window list: {limit:?}");
            std::process::exit(1);
        }
    };
    let Some(handle) = source.capture_handle(listello_protocol::WindowId(id)) else {
        eprintln!("no window {id}; see --list");
        std::process::exit(1);
    };
    let Some(capturer) = capture::Capturer::connect() else {
        std::process::exit(1);
    };
    match capturer.capture(&handle, 4096, 8192, listello_protocol::PreviewFit::Whole) {
        Ok(icon) => {
            if let Err(e) = capture::save_png(&icon, std::path::Path::new(file)) {
                eprintln!("could not write {file}: {e}");
                std::process::exit(1);
            }
            println!("{}x{} -> {file}", icon.width, icon.height);
        }
        Err(e) => {
            eprintln!("capture failed: {e:?}");
            std::process::exit(1);
        }
    }
}

fn list_windows() {
    match windows::probe() {
        Probe::Ready(source) => {
            for w in source.windows() {
                println!(
                    "{:>4}  {:<28} {}{}{}",
                    w.id.0,
                    w.app_id.as_str(),
                    w.title,
                    if w.active { "  [active]" } else { "" },
                    if w.minimized { "  [minimised]" } else { "" },
                );
            }
        }
        Probe::Unavailable(limit) => {
            eprintln!("no window list: {limit:?}");
            std::process::exit(1);
        }
    }
}
