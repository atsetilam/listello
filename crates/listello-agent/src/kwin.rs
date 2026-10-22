//! The KDE backend: Plasma's `org_kde_plasma_window_management` protocol.
//!
//! KWin keeps this protocol on a blacklist and advertises it only to a client whose executable
//! has a `.desktop` file declaring it in `X-KDE-Wayland-Interfaces` (D11 amendment, RESEARCH
//! §3). An agent run from a build directory therefore sees an empty registry -- no error, no
//! refusal -- and that is reported as [`Limitation::NoWindowManagement`] instead of an empty
//! window list. The development escape hatch is starting KWin with
//! `KWIN_WAYLAND_NO_PERMISSION_CHECKS=1`.

use crate::windows::{Probe, StreamEvent, WindowSource};
use anyhow::{Context, Result};
use listello_protocol::{AppId, Limitation, WindowCommand, WindowId, WindowInfo};
use std::collections::HashMap;
use std::os::fd::BorrowedFd;
use wayland_client::{
    backend::{Backend, WaylandError},
    globals::{registry_queue_init, BindError, GlobalListContents},
    protocol::wl_registry::WlRegistry,
    Connection, Dispatch, EventQueue, QueueHandle,
};
use wayland_protocols_plasma::plasma_window_management::client::{
    org_kde_plasma_window::{self, OrgKdePlasmaWindow},
    org_kde_plasma_window_management::{self, OrgKdePlasmaWindowManagement},
};
use wayland_protocols_plasma::screencast::v1::client::{
    zkde_screencast_stream_unstable_v1::{self, ZkdeScreencastStreamUnstableV1},
    zkde_screencast_unstable_v1::{self, ZkdeScreencastUnstableV1},
};

const STATE_ACTIVE: u32 = 0x1;
const STATE_MINIMIZED: u32 = 0x2;
const STATE_MAXIMIZED: u32 = 0x4;
const STATE_SKIP_TASKBAR: u32 = 0x1000;

/// `window_with_uuid` arrived in version 13; older compositors only have the deprecated
/// numeric ids and are not worth a second code path.
const MIN_VERSION: u32 = 13;
const MAX_VERSION: u32 = 18;

pub fn probe() -> Probe {
    match connect() {
        Ok(source) => Probe::Ready(Box::new(source)),
        Err(limit) => Probe::Unavailable(limit),
    }
}

fn connect() -> std::result::Result<KwinSource, Limitation> {
    let conn = Connection::connect_to_env().map_err(|e| {
        eprintln!("listello-agent: cannot reach a Wayland compositor: {e}");
        Limitation::UnsupportedCompositor {
            name: "none (no Wayland session)".to_string(),
        }
    })?;
    let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(|e| {
        eprintln!("listello-agent: cannot read the Wayland registry: {e}");
        Limitation::UnsupportedCompositor {
            name: "unknown".to_string(),
        }
    })?;
    let qh = queue.handle();
    let manager = globals
        .bind::<OrgKdePlasmaWindowManagement, _, _>(&qh, MIN_VERSION..=MAX_VERSION, ())
        .map_err(|e| match e {
            BindError::NotPresent => {
                eprintln!(
                    "listello-agent: the compositor does not offer \
                     org_kde_plasma_window_management to this process. Either listello-agent \
                     is running from an uninstalled path, or its .desktop file is missing or \
                     does not declare X-KDE-Wayland-Interfaces. (To develop without installing, \
                     start KWin with KWIN_WAYLAND_NO_PERMISSION_CHECKS=1.)"
                );
                Limitation::NoWindowManagement
            }
            BindError::UnsupportedVersion => {
                eprintln!(
                    "listello-agent: the compositor's window management protocol is older \
                     than version {MIN_VERSION}"
                );
                Limitation::UnsupportedCompositor {
                    name: "KWin (protocol too old)".to_string(),
                }
            }
        })?;

    // Window streams for live previews: gated like the window list, and optional. Version 1
    // has `stream_window`, which is all this needs.
    let screencast = globals
        .bind::<ZkdeScreencastUnstableV1, _, _>(&qh, 1..=1, ())
        .inspect_err(|_| {
            eprintln!(
                "listello-agent: the compositor offers no window streams to this process \
                 (X-KDE-Wayland-Interfaces lacks zkde_screencast_unstable_v1?); live \
                 previews fall back to stills"
            )
        })
        .ok();

    let mut state = State::default();
    // One round trip delivers the existing windows' `window_with_uuid`; a second their state.
    for _ in 0..2 {
        if let Err(e) = queue.roundtrip(&mut state) {
            eprintln!("listello-agent: window management handshake failed: {e}");
            return Err(Limitation::NoWindowManagement);
        }
    }
    Ok(KwinSource {
        backend: conn.backend(),
        conn,
        queue,
        state,
        _manager: manager,
        screencast,
        streams: HashMap::new(),
    })
}

struct Tracked {
    id: WindowId,
    proxy: OrgKdePlasmaWindow,
    /// KWin's own id, which its screenshot interface takes.
    uuid: String,
    title: String,
    app_id: String,
    flags: u32,
    /// `initial_state` has arrived: the title, app id and flags above are real.
    ready: bool,
}

#[derive(Default)]
struct State {
    /// In the order the windows first appeared.
    windows: Vec<Tracked>,
    next_id: u64,
    dirty: bool,
    stream_events: Vec<(WindowId, StreamEvent)>,
}

pub struct KwinSource {
    conn: Connection,
    /// Kept so `poll_fd` can lend its descriptor for as long as `self` lives.
    backend: Backend,
    queue: EventQueue<State>,
    state: State,
    _manager: OrgKdePlasmaWindowManagement,
    screencast: Option<ZkdeScreencastUnstableV1>,
    streams: HashMap<WindowId, ZkdeScreencastStreamUnstableV1>,
}

impl KwinSource {
    fn flush(&self, what: &str) {
        if let Err(e) = self.conn.flush() {
            eprintln!("listello-agent: could not send the {what}: {e}");
        }
    }
}

impl WindowSource for KwinSource {
    fn poll_fd(&self) -> BorrowedFd<'_> {
        self.backend.poll_fd()
    }

    fn dispatch(&mut self) -> Result<bool> {
        loop {
            self.queue
                .dispatch_pending(&mut self.state)
                .context("dispatching Wayland events")?;
            if let Some(guard) = self.queue.prepare_read() {
                match guard.read() {
                    Ok(_) => {}
                    Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e).context("reading from the compositor"),
                }
                break;
            }
        }
        self.queue
            .dispatch_pending(&mut self.state)
            .context("dispatching Wayland events")?;
        match self.conn.flush() {
            Ok(()) => {}
            Err(WaylandError::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e).context("writing to the compositor"),
        }
        Ok(std::mem::take(&mut self.state.dirty))
    }

    fn windows(&self) -> Vec<WindowInfo> {
        self.state
            .windows
            .iter()
            .filter(|w| w.ready && w.flags & STATE_SKIP_TASKBAR == 0 && !w.app_id.is_empty())
            .map(|w| WindowInfo {
                id: w.id,
                app_id: AppId::new(w.app_id.clone()),
                title: w.title.clone(),
                active: w.flags & STATE_ACTIVE != 0,
                minimized: w.flags & STATE_MINIMIZED != 0,
            })
            .collect()
    }

    fn activate(&mut self, id: WindowId) {
        let Some(window) = self.state.windows.iter().find(|w| w.id == id) else {
            return;
        };
        // Set "active" and clear "minimized" in one request.
        window
            .proxy
            .set_state(STATE_ACTIVE | STATE_MINIMIZED, STATE_ACTIVE);
        if let Err(e) = self.conn.flush() {
            eprintln!("listello-agent: could not send the activation: {e}");
        }
    }

    fn command(&mut self, id: WindowId, command: WindowCommand) {
        let Some(window) = self.state.windows.iter().find(|w| w.id == id) else {
            return;
        };
        match command {
            // KWin's own close: the same request as the title bar's button, so the app decides
            // whether that means quitting or hiding to the tray.
            WindowCommand::Close => window.proxy.close(),
            WindowCommand::Minimize => window.proxy.set_state(STATE_MINIMIZED, STATE_MINIMIZED),
            WindowCommand::ToggleMaximize => {
                let maximized = window.flags & STATE_MAXIMIZED != 0;
                let value = if maximized { 0 } else { STATE_MAXIMIZED };
                // Restoring a minimised window too, so a maximise is seen.
                window
                    .proxy
                    .set_state(STATE_MAXIMIZED | STATE_MINIMIZED, value);
            }
        }
        if let Err(e) = self.conn.flush() {
            eprintln!("listello-agent: could not send the window command: {e}");
        }
    }

    fn capture_handle(&self, id: WindowId) -> Option<String> {
        let window = self.state.windows.iter().find(|w| w.id == id)?;
        Some(window.uuid.clone())
    }

    fn start_stream(&mut self, id: WindowId) -> bool {
        let (Some(screencast), Some(uuid)) = (&self.screencast, self.capture_handle(id)) else {
            return false;
        };
        let qh = self.queue.handle();
        let stream = screencast.stream_window(
            uuid,
            zkde_screencast_unstable_v1::Pointer::Hidden.into(),
            &qh,
            id,
        );
        if let Some(old) = self.streams.insert(id, stream) {
            old.close();
        }
        self.flush("stream request");
        true
    }

    fn end_stream(&mut self, id: WindowId) {
        if let Some(stream) = self.streams.remove(&id) {
            stream.close();
            self.flush("stream close");
        }
    }

    fn stream_events(&mut self) -> Vec<(WindowId, StreamEvent)> {
        let events = std::mem::take(&mut self.state.stream_events);
        for (id, event) in &events {
            // A stream that ended is closed on our side too, as the protocol asks.
            if matches!(event, StreamEvent::Ended(_)) {
                self.end_stream(*id);
            }
        }
        events
    }
}

impl Dispatch<ZkdeScreencastUnstableV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZkdeScreencastUnstableV1,
        _: zkde_screencast_unstable_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZkdeScreencastStreamUnstableV1, WindowId> for State {
    fn event(
        state: &mut Self,
        _: &ZkdeScreencastStreamUnstableV1,
        event: zkde_screencast_stream_unstable_v1::Event,
        id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zkde_screencast_stream_unstable_v1::Event;
        let event = match event {
            Event::Created { node } => StreamEvent::Ready(node),
            Event::Failed { error } => StreamEvent::Ended(error),
            Event::Closed => StreamEvent::Ended("closed by the compositor".to_string()),
            _ => return,
        };
        state.stream_events.push((*id, event));
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as wayland_client::Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<OrgKdePlasmaWindowManagement, ()> for State {
    fn event(
        state: &mut Self,
        manager: &OrgKdePlasmaWindowManagement,
        event: org_kde_plasma_window_management::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let org_kde_plasma_window_management::Event::WindowWithUuid { uuid, .. } = event {
            let id = WindowId(state.next_id);
            state.next_id += 1;
            let proxy = manager.get_window_by_uuid(uuid.clone(), qh, id);
            state.windows.push(Tracked {
                id,
                proxy,
                uuid,
                title: String::new(),
                app_id: String::new(),
                flags: 0,
                ready: false,
            });
        }
    }
}

impl Dispatch<OrgKdePlasmaWindow, WindowId> for State {
    fn event(
        state: &mut Self,
        _: &OrgKdePlasmaWindow,
        event: org_kde_plasma_window::Event,
        id: &WindowId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use org_kde_plasma_window::Event;
        let Some(index) = state.windows.iter().position(|w| w.id == *id) else {
            return;
        };
        match event {
            Event::TitleChanged { title } => {
                state.windows[index].title = title;
                state.dirty = true;
            }
            Event::AppIdChanged { app_id } => {
                state.windows[index].app_id = app_id;
                state.dirty = true;
            }
            Event::StateChanged { flags } => {
                state.windows[index].flags = flags;
                state.dirty = true;
            }
            Event::InitialState => {
                state.windows[index].ready = true;
                state.dirty = true;
            }
            Event::Unmapped => {
                let gone = state.windows.remove(index);
                gone.proxy.destroy();
                state.dirty = true;
            }
            _ => {}
        }
    }
}
