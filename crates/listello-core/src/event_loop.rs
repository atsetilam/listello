// New in M1 step 3. Updated in M1 phase 06 for the `Shell`/`UiConfig` wiring.
//
// tiny-dfr's `main()` was one function that mixed reading raw libinput/udev/inotify fds with
// interpreting what they meant (RESEARCH §2.1). `EventPump` is the split: it owns every raw
// source of change and translates readiness into `listello_protocol::Event`s, and `real_main`
// goes back to only ever reacting to those — D3's "every source of change sends a message,"
// applied to the loop that was still the one place violating it.
//
// What stays out of here on purpose: anything that needs `Shell` or `uinput` state to decide
// what an event *means* (which widget a touch landed on, what a completed press does). Those
// are core state, not input-source plumbing, and D3 says the pump hands over a message, never
// a reference to that state. Config is the one exception with a wrinkle: `poll_events` still
// triggers the reload (it owns the inotify fd), but the *parsed result* comes back through
// `take_reload()` rather than an out-parameter, because `Shell` — unlike the old
// `Config`/`FunctionLayer` pair — has to be rebuilt, not mutated in place.

use crate::agent_socket::{AgentSocket, SocketEvent};
use crate::clock::FrameClock;
use crate::config::{ConfigManager, LoadedConfig};
use anyhow::Result;
use chrono::Timelike;
use input::{
    event::{
        device::DeviceEvent,
        keyboard::{KeyboardEvent, KeyboardEventTrait},
        touch::{TouchEvent, TouchEventPosition, TouchEventSlot},
        Event as LibinputEvent, EventTrait,
    },
    Device as InputDevice, Libinput, LibinputInterface,
};
use input_linux::Key;
use libc::{O_ACCMODE, O_RDONLY, O_RDWR, O_WRONLY};
use listello_protocol::{CoreToAgent, Event, InputEvent, Touch, TouchPhase};
use nix::{
    errno::Errno,
    sys::epoll::{Epoll, EpollCreateFlags, EpollEvent, EpollFlags},
};
use std::{
    collections::VecDeque,
    fs::OpenOptions,
    os::{
        fd::{AsFd, OwnedFd},
        unix::fs::OpenOptionsExt,
    },
    path::Path,
};
use udev::{MonitorBuilder, MonitorSocket};

use crate::backlight::BacklightManager;

struct Interface;

impl LibinputInterface for Interface {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> std::result::Result<OwnedFd, i32> {
        let mode = flags & O_ACCMODE;

        OpenOptions::new()
            .custom_flags(flags)
            .read(mode == O_RDONLY || mode == O_RDWR)
            .write(mode == O_WRONLY || mode == O_RDWR)
            .open(path)
            .map(|file| file.into())
            .map_err(|err| err.raw_os_error().unwrap_or(libc::EIO))
    }
    fn close_restricted(&mut self, fd: OwnedFd) {
        _ = std::fs::File::from(fd);
    }
}

/// Every raw source of change `listello-core` reads directly, translated into `Event`s.
///
/// The two libinput seats (`seat-touchbar` and `seat0`), the udev `power_supply` monitor, the
/// config file's inotify watch, the frame clock and the agent socket (D11) are all here.
pub struct EventPump {
    epoll: Epoll,
    input_tb: Libinput,
    input_main: Libinput,
    udev_monitor: MonitorSocket,
    cfg_mgr: ConfigManager,
    frame_clock: FrameClock,
    digitizer: Option<InputDevice>,
    width: u16,
    height: u16,
    last_minute: u32,
    queue: VecDeque<Event>,
    pending_reload: Option<LoadedConfig>,
    agent: AgentSocket,
    /// What epoll currently watches of the agent connection, so a change is noticed.
    agent_registered: Option<(u64, bool)>,
}

const TOKEN_AGENT_LISTENER: u64 = 4;
const TOKEN_AGENT_CONNECTION: u64 = 5;

impl EventPump {
    pub fn new(
        width: u16,
        height: u16,
        cfg_mgr: ConfigManager,
        agent: AgentSocket,
    ) -> Result<EventPump> {
        let mut input_tb = Libinput::new_with_udev(Interface);
        let mut input_main = Libinput::new_with_udev(Interface);
        input_tb
            .udev_assign_seat("seat-touchbar")
            .map_err(|()| anyhow::anyhow!("failed to assign libinput seat \"seat-touchbar\""))?;
        input_main
            .udev_assign_seat("seat0")
            .map_err(|()| anyhow::anyhow!("failed to assign libinput seat \"seat0\""))?;
        let udev_monitor = MonitorBuilder::new()?
            .match_subsystem("power_supply")?
            .listen()?;

        let epoll = Epoll::new(EpollCreateFlags::empty())?;
        epoll.add(input_main.as_fd(), EpollEvent::new(EpollFlags::EPOLLIN, 0))?;
        epoll.add(input_tb.as_fd(), EpollEvent::new(EpollFlags::EPOLLIN, 1))?;
        epoll.add(cfg_mgr.fd(), EpollEvent::new(EpollFlags::EPOLLIN, 2))?;
        epoll.add(&udev_monitor, EpollEvent::new(EpollFlags::EPOLLIN, 3))?;
        epoll.add(
            agent.listener_fd(),
            EpollEvent::new(EpollFlags::EPOLLIN, TOKEN_AGENT_LISTENER),
        )?;

        Ok(EventPump {
            epoll,
            input_tb,
            input_main,
            udev_monitor,
            cfg_mgr,
            frame_clock: FrameClock::new(),
            digitizer: None,
            width,
            height,
            last_minute: chrono::Local::now().minute(),
            queue: VecDeque::new(),
            pending_reload: None,
            agent,
            agent_registered: None,
        })
    }

    /// Hand an action to the agent. Dropped when none is connected: the renderer never
    /// depends on one (D11).
    pub fn send_to_agent(&mut self, msg: &CoreToAgent) {
        self.agent.send(msg);
        self.sync_agent_registration();
    }

    /// Keep epoll watching exactly the agent connection that exists now, for writability only
    /// while bytes are queued. A dropped connection closes its descriptor, and epoll forgets a
    /// closed descriptor by itself, so only additions and changes need handling.
    fn sync_agent_registration(&mut self) {
        let wanted = self
            .agent
            .connection_id()
            .map(|id| (id, self.agent.wants_write()));
        if wanted == self.agent_registered {
            return;
        }
        let (Some((id, writable)), Some(fd)) = (wanted, self.agent.connection_fd()) else {
            self.agent_registered = None;
            return;
        };
        let mut flags = EpollFlags::EPOLLIN;
        if writable {
            flags |= EpollFlags::EPOLLOUT;
        }
        let mut event = EpollEvent::new(flags, TOKEN_AGENT_CONNECTION);
        let result = match self.agent_registered {
            Some((old, _)) if old == id => self.epoll.modify(fd, &mut event),
            _ => self.epoll.add(fd, event),
        };
        match result {
            Ok(()) => self.agent_registered = Some((id, writable)),
            Err(e) => eprintln!("listello: cannot watch the agent connection: {e}"),
        }
    }

    /// Whether *anything* wants to be woken every frame right now. Level-triggered: call it
    /// every time the caller recomputes the answer (`clock`'s doc comment says why).
    pub fn set_animating(&mut self, want: bool) {
        self.frame_clock.set_animating(want);
    }

    /// The config a `ConfigReloaded` event this call just queued was parsed into, if any.
    /// `None` on any call that didn't just see a *successful* reload — including one that saw
    /// a failed reload, which `ConfigManager` already logged and chose not to surface here.
    pub fn take_reload(&mut self) -> Option<LoadedConfig> {
        self.pending_reload.take()
    }

    /// Block for at most `timeout_ms` (the caller's own reasons to wake sooner go in here too,
    /// `min`'d against the frame clock), then drain every ready source and return what it
    /// meant.
    pub fn poll_events(&mut self, timeout_ms: i32, backlight: &mut BacklightManager) -> Vec<Event> {
        let timeout_ms = match self.frame_clock.next_timeout_ms() {
            Some(clock_ms) => timeout_ms.min(clock_ms as i32),
            None => timeout_ms,
        };
        // Wake in time to drop a connection that never says Hello.
        let timeout_ms = match self.agent.next_deadline() {
            Some(left) => timeout_ms.min(left.as_millis().min(i32::MAX as u128) as i32 + 1),
            None => timeout_ms,
        };

        let timeout_ms = timeout_ms.clamp(0, u16::MAX as i32) as u16;
        // The result only ever gates whether there's anything to drain, and every source below
        // is drained unconditionally regardless (the comment below explains why) — so a kernel-
        // level epoll error is exactly as actionable as an empty result: none. Log it and treat
        // it as "nothing ready yet" rather than crash the daemon over it.
        if let Err(e) = self
            .epoll
            .wait(&mut [EpollEvent::new(EpollFlags::EPOLLIN, 0)], timeout_ms)
        {
            if e != Errno::EINTR {
                eprintln!("listello: epoll_wait failed: {e}");
            }
        }

        // tiny-dfr's original shape, kept: which fd fired is not distinguished, everything
        // ready gets drained every wake. All of these reads are non-blocking, so a spurious
        // drain of an idle source costs nothing.
        _ = self.udev_monitor.iter().last();

        if let Some(reload) = self.cfg_mgr.poll_reload() {
            self.pending_reload = Some(reload);
            self.queue.push_back(Event::ConfigReloaded);
        }

        if let Err(e) = self.input_tb.dispatch() {
            eprintln!("listello: failed to dispatch touchbar libinput events: {e}");
        }
        if let Err(e) = self.input_main.dispatch() {
            eprintln!("listello: failed to dispatch main-seat libinput events: {e}");
        }
        for event in &mut self.input_tb.clone().chain(self.input_main.clone()) {
            backlight.process_event(&event);
            self.translate(event, backlight.current_bl());
        }

        for event in self.agent.poll() {
            self.queue.push_back(match event {
                SocketEvent::Connected => Event::AgentConnected,
                SocketEvent::Message(msg) => Event::Agent(msg),
                SocketEvent::Disconnected => Event::AgentDisconnected,
            });
        }
        self.sync_agent_registration();

        if let Some(elapsed_secs) = self.frame_clock.maybe_tick() {
            self.queue.push_back(Event::Frame { elapsed_secs });
        }

        let current_minute = chrono::Local::now().minute();
        if current_minute != self.last_minute {
            self.last_minute = current_minute;
            self.queue.push_back(Event::TimeChanged);
        }

        self.queue.drain(..).collect()
    }

    fn translate(&mut self, event: LibinputEvent, backlight_on: u32) {
        match event {
            LibinputEvent::Device(DeviceEvent::Added(evt)) => {
                let dev = evt.device();
                if dev.name().contains(" Touch Bar") {
                    self.digitizer = Some(dev);
                }
            }
            LibinputEvent::Keyboard(KeyboardEvent::Key(key)) => {
                if key.key() == Key::Fn as u32 {
                    self.queue.push_back(
                        InputEvent::FnKey {
                            pressed: key.key_state() == input::event::keyboard::KeyState::Pressed,
                        }
                        .into(),
                    );
                }
            }
            LibinputEvent::Touch(te) => {
                if Some(te.device()) != self.digitizer || backlight_on == 0 {
                    return;
                }
                let (width, height) = (self.width as u32, self.height as u32);
                let touch = match te {
                    TouchEvent::Down(dn) => Some(Touch {
                        slot: dn.seat_slot(),
                        phase: TouchPhase::Down,
                        x: dn.x_transformed(width),
                        y: dn.y_transformed(height),
                    }),
                    TouchEvent::Motion(mtn) => Some(Touch {
                        slot: mtn.seat_slot(),
                        phase: TouchPhase::Motion,
                        x: mtn.x_transformed(width),
                        y: mtn.y_transformed(height),
                    }),
                    TouchEvent::Up(up) => Some(Touch {
                        slot: up.seat_slot(),
                        phase: TouchPhase::Up,
                        // `Up` carries no position in libinput; the last known one is stale by
                        // definition, so the touch's owner must key off `slot`, not `x`/`y`.
                        x: 0.0,
                        y: 0.0,
                    }),
                    _ => None,
                };
                if let Some(touch) = touch {
                    self.queue.push_back(InputEvent::Touch(touch).into());
                }
            }
            _ => {}
        }
    }
}
