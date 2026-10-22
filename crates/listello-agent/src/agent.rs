//! What the agent does with a message, apart from the sockets it arrives on.
//!
//! [`Agent`] holds the session-side state -- the window source, the desktop-entry catalogue --
//! and turns each [`CoreToAgent`] into side effects plus the [`AgentToCore`] messages to send
//! back. Keeping the sockets out of it is what lets the tests drive it with a
//! [`crate::windows::FakeWindowSource`].

use crate::actions::Spawner;
use crate::desktop::Catalog;
use crate::folders::Folders;
use crate::icons;
use crate::power::read_battery;
use crate::previews::Previews;
use crate::wifi::read_network;
use crate::windows::WindowSource;
use listello_protocol::{
    AgentToCore, AppId, BatteryState, CoreToAgent, Limitation, NetworkState, SystemAction,
    WindowInfo,
};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// How often the battery and wifi files are re-read. Nothing announces a change in them, and
/// neither moves fast enough to want more.
const SENSOR_EVERY: Duration = Duration::from_secs(10);

/// Where the battery and wifi state come from. `None` roots read nothing, which is how the
/// tests keep the real machine's battery out of their expectations.
#[derive(Default)]
pub struct Sensors {
    pub power_root: Option<PathBuf>,
    pub net_root: Option<PathBuf>,
    pub proc_wireless: Option<PathBuf>,
}

impl Sensors {
    pub fn system() -> Sensors {
        Sensors {
            power_root: Some("/sys/class/power_supply".into()),
            net_root: Some("/sys/class/net".into()),
            proc_wireless: Some("/proc/net/wireless".into()),
        }
    }

    fn battery(&self) -> Option<BatteryState> {
        read_battery(self.power_root.as_ref()?)
    }

    fn network(&self) -> Option<NetworkState> {
        let wireless = self
            .proc_wireless
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_default();
        read_network(self.net_root.as_ref()?, &wireless)
    }
}

pub struct Agent {
    source: Option<Box<dyn WindowSource>>,
    /// Why there is no source. Sent with the greeting on every connection, so a renderer that
    /// reconnects still learns it.
    limitation: Option<Limitation>,
    catalog: Catalog,
    theme: String,
    spawner: Spawner,
    sensors: Sensors,
    /// What the renderer was last told, so an unchanged reading is never sent twice. The outer
    /// `None` is "never told".
    sent_battery: Option<Option<BatteryState>>,
    sent_network: Option<Option<NetworkState>>,
    next_sensor_read: Instant,
    folders: Option<Folders>,
    /// Where the user's Dock pins are kept; `None` with no home directory.
    pins_path: Option<PathBuf>,
    previews: Previews,
}

impl Agent {
    pub fn new(
        source: Option<Box<dyn WindowSource>>,
        limitation: Option<Limitation>,
        catalog: Catalog,
        theme: String,
    ) -> Agent {
        Agent {
            source,
            limitation,
            catalog,
            theme,
            spawner: Spawner::default(),
            sensors: Sensors::default(),
            sent_battery: None,
            sent_network: None,
            next_sensor_read: Instant::now(),
            folders: None,
            pins_path: None,
            previews: Previews::default(),
        }
    }

    /// Supply the window row's previews from `previews`.
    pub fn with_previews(mut self, previews: Previews) -> Agent {
        self.previews = previews;
        self
    }

    /// How long until the open window row's previews are due, if a row is open.
    pub fn until_previews(&self, now: Instant) -> Option<Duration> {
        self.previews.until_due(now)
    }

    /// Fresh previews for the open window row, when due; only the changed ones.
    pub fn poll_previews(&mut self, now: Instant) -> Vec<AgentToCore> {
        self.previews.poll(self.source.as_deref_mut(), now)
    }

    /// PipeWire's descriptor, while a live row wants it watched.
    pub fn live_fd(&self) -> Option<std::os::fd::BorrowedFd<'_>> {
        self.previews
            .streaming()
            .then(|| self.previews.live_fd())
            .flatten()
    }

    /// PipeWire's descriptor is readable.
    pub fn dispatch_live(&self) {
        self.previews.dispatch_live();
    }

    /// The renderer is gone: whatever row it had open is closed.
    pub fn disconnected(&mut self) {
        self.previews.stop();
    }

    /// Keep the user's Dock pins at `path`.
    pub fn with_pins(mut self, path: Option<PathBuf>) -> Agent {
        self.pins_path = path;
        self
    }

    pub fn pins(&self) -> Option<Vec<AppId>> {
        self.pins_path.as_deref().and_then(crate::pins::load)
    }

    /// Supply battery and wifi from `sensors` and folder watching from `folders`.
    pub fn with_sensors(mut self, sensors: Sensors, folders: Option<Folders>) -> Agent {
        self.sensors = sensors;
        self.folders = folders;
        self
    }

    /// The inotify descriptor to poll, when folder watching is available.
    pub fn folders_fd(&self) -> Option<std::os::fd::BorrowedFd<'_>> {
        self.folders.as_ref().map(Folders::fd)
    }

    /// Folders whose emptiness changed since the last call.
    pub fn folders_changed(&mut self) -> Vec<AgentToCore> {
        match &mut self.folders {
            Some(folders) => folders
                .dispatch()
                .into_iter()
                .map(AgentToCore::Folder)
                .collect(),
            None => Vec::new(),
        }
    }

    /// How long until the battery and wifi are due to be read again.
    pub fn until_sensor_read(&self, now: Instant) -> Duration {
        self.next_sensor_read.saturating_duration_since(now)
    }

    /// Re-read the battery and wifi if it is time, and return only what changed.
    pub fn poll_sensors(&mut self, now: Instant) -> Vec<AgentToCore> {
        if now < self.next_sensor_read {
            return Vec::new();
        }
        self.next_sensor_read = now + SENSOR_EVERY;
        let mut out = Vec::new();
        // A source with no root configured is not there at all: say nothing about it, rather
        // than announcing "no battery" for a reading nobody took.
        if self.sensors.power_root.is_some() {
            let battery = self.sensors.battery();
            if self.sent_battery != Some(battery) {
                self.sent_battery = Some(battery);
                out.push(AgentToCore::Battery(battery));
            }
        }
        if self.sensors.net_root.is_some() {
            let network = self.sensors.network();
            if self.sent_network.as_ref() != Some(&network) {
                self.sent_network = Some(network.clone());
                out.push(AgentToCore::Network(network));
            }
        }
        out
    }

    pub fn source(&self) -> Option<&dyn WindowSource> {
        self.source.as_deref()
    }

    pub fn source_mut(&mut self) -> Option<&mut (dyn WindowSource + 'static)> {
        self.source.as_deref_mut()
    }

    pub fn reap(&mut self) {
        self.spawner.reap();
    }

    /// Everything the renderer needs to know right after connecting. Complete replacements,
    /// never deltas: the renderer may have been restarted and remembers nothing.
    pub fn greeting(&mut self) -> Vec<AgentToCore> {
        let mut out = Vec::new();
        self.previews.stop();
        if let Some(limit) = &self.limitation {
            out.push(AgentToCore::Limited(limit.clone()));
        }
        if self.source.is_some() {
            out.push(AgentToCore::Windows(self.windows()));
        }
        // A renderer that just connected knows nothing: forget what was sent and send it all.
        self.sent_battery = None;
        self.sent_network = None;
        self.next_sensor_read = Instant::now();
        out.extend(self.poll_sensors(Instant::now()));
        if let Some(folders) = &mut self.folders {
            out.extend(folders.all().into_iter().map(AgentToCore::Folder));
        }
        out.push(AgentToCore::Pins(self.pins()));
        out
    }

    /// The source's windows with each `app_id` resolved to the id of its desktop entry, so
    /// that a window of `firefox` and a pinned `firefox-esr` are the same app to the Dock. An
    /// app with no desktop entry keeps the id its window reported.
    fn windows(&self) -> Vec<WindowInfo> {
        let Some(source) = &self.source else {
            return Vec::new();
        };
        source
            .windows()
            .into_iter()
            .map(|mut w| {
                if let Some(entry) = self.catalog.find(w.app_id.as_str()) {
                    w.app_id = AppId::new(entry.id.clone());
                }
                w
            })
            .collect()
    }

    /// The window list after the source reported a change.
    pub fn windows_changed(&self) -> Vec<AgentToCore> {
        match &self.source {
            Some(_) => vec![AgentToCore::Windows(self.windows())],
            None => Vec::new(),
        }
    }

    pub fn handle(&mut self, msg: CoreToAgent) -> Vec<AgentToCore> {
        match msg {
            // The link consumes the Hello; it never reaches here.
            CoreToAgent::Hello { .. } => Vec::new(),
            CoreToAgent::ActivateWindow(id) => {
                if let Some(source) = self.source.as_mut() {
                    source.activate(id);
                }
                Vec::new()
            }
            CoreToAgent::SavePins(pins) => {
                let Some(path) = &self.pins_path else {
                    eprintln!("listello-agent: no home directory to keep Dock pins in");
                    return vec![AgentToCore::Pins(None)];
                };
                match crate::pins::save(path, &pins) {
                    Ok(saved) => vec![AgentToCore::Pins(Some(saved))],
                    Err(e) => {
                        eprintln!(
                            "listello-agent: could not save Dock pins to {}: {e}",
                            path.display()
                        );
                        // Tell the renderer what is really kept, so it does not show a pin
                        // that will be gone on the next start.
                        vec![AgentToCore::Pins(self.pins())]
                    }
                }
            }
            CoreToAgent::WindowCommand { id, command } => {
                if let Some(source) = self.source.as_mut() {
                    source.command(id, command);
                }
                Vec::new()
            }
            CoreToAgent::Launch { app_id } => {
                if self.spawner.launch(&self.catalog, &app_id) {
                    vec![AgentToCore::LaunchStarted { app_id }]
                } else {
                    Vec::new()
                }
            }
            CoreToAgent::OpenPath { path } => {
                self.spawner.open_path(&path);
                Vec::new()
            }
            CoreToAgent::RequestIcon { app_id, size_px } => {
                let icon = icons::icon_for(&self.catalog, &self.theme, app_id.as_str(), size_px);
                vec![AgentToCore::Icon { app_id, icon }]
            }
            CoreToAgent::SystemAction(SystemAction::LockSession) => {
                self.spawner.lock_session();
                Vec::new()
            }
            CoreToAgent::WatchPreviews {
                windows,
                height_px,
                max_width_px,
                fit,
                live,
            } => {
                self.previews
                    .watch(windows, height_px, max_width_px, fit, live, Instant::now());
                Vec::new()
            }
            CoreToAgent::WatchFolder { key, path } => self
                .folders
                .as_mut()
                .and_then(|folders| folders.watch(key, &path))
                .map(AgentToCore::Folder)
                .into_iter()
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop::Overrides;
    use crate::windows::FakeWindowSource;
    use listello_protocol::WindowId;

    fn window(id: u64, app: &str) -> WindowInfo {
        WindowInfo {
            id: WindowId(id),
            app_id: AppId::new(app),
            title: format!("{app} {id}"),
            active: false,
            minimized: false,
        }
    }

    fn agent_with(windows: Vec<WindowInfo>) -> Agent {
        Agent::new(
            Some(Box::new(FakeWindowSource::new(windows))),
            None,
            Catalog::new(Vec::new(), Overrides::default()),
            "hicolor".to_string(),
        )
    }

    #[test]
    fn the_greeting_is_the_whole_window_list() {
        let mut agent = agent_with(vec![window(1, "a"), window(2, "b")]);
        let greeting = agent.greeting();
        // The windows, then the pins (none: this agent keeps no pin file).
        assert_eq!(greeting.len(), 2);
        assert!(matches!(&greeting[0], AgentToCore::Windows(w) if w.len() == 2));
        assert_eq!(greeting[1], AgentToCore::Pins(None));
    }

    #[test]
    fn a_windows_app_id_is_resolved_to_its_desktop_entry() {
        use crate::desktop::parse_entry;
        let entry = parse_entry(
            "firefox-esr",
            "[Desktop Entry]\nType=Application\nName=Firefox\nExec=firefox\nStartupWMClass=firefox\n",
        )
        .expect("an entry");
        let mut agent = Agent::new(
            Some(Box::new(FakeWindowSource::new(vec![
                window(1, "firefox"),
                window(2, "no-desktop-file"),
            ]))),
            None,
            Catalog::new(vec![entry], Overrides::default()),
            "hicolor".to_string(),
        );
        let AgentToCore::Windows(windows) = &agent.greeting()[0] else {
            panic!("the greeting starts with the window list");
        };
        assert_eq!(windows[0].app_id, AppId::new("firefox-esr"));
        assert_eq!(windows[1].app_id, AppId::new("no-desktop-file"));
    }

    #[test]
    fn without_a_source_the_greeting_says_why() {
        let mut agent = Agent::new(
            None,
            Some(Limitation::NoWindowManagement),
            Catalog::new(Vec::new(), Overrides::default()),
            "hicolor".to_string(),
        );
        assert_eq!(
            agent.greeting(),
            vec![
                AgentToCore::Limited(Limitation::NoWindowManagement),
                AgentToCore::Pins(None)
            ]
        );
        assert!(agent.windows_changed().is_empty());
    }

    #[test]
    fn a_stale_activation_does_nothing_and_a_real_one_reaches_the_source() {
        let mut agent = agent_with(vec![window(1, "a")]);
        assert!(agent
            .handle(CoreToAgent::ActivateWindow(WindowId(1)))
            .is_empty());
        assert!(agent
            .handle(CoreToAgent::ActivateWindow(WindowId(99)))
            .is_empty());
    }

    #[test]
    fn pins_are_greeted_saved_and_answered_with_what_was_kept() {
        let dir = std::env::temp_dir().join(format!("listello-agent-pins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("pins.toml");
        let mut agent = agent_with(Vec::new()).with_pins(Some(path.clone()));
        assert!(agent.greeting().contains(&AgentToCore::Pins(None)));
        let pins = vec![AppId::new("a"), AppId::new("b"), AppId::new("a")];
        assert_eq!(
            agent.handle(CoreToAgent::SavePins(pins)),
            vec![AgentToCore::Pins(Some(vec![
                AppId::new("a"),
                AppId::new("b")
            ]))]
        );
        assert!(agent.greeting().contains(&AgentToCore::Pins(Some(vec![
            AppId::new("a"),
            AppId::new("b")
        ]))));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_window_command_answers_nothing_whether_or_not_the_window_exists() {
        let mut agent = agent_with(vec![window(1, "a")]);
        for id in [WindowId(1), WindowId(99)] {
            assert!(agent
                .handle(CoreToAgent::WindowCommand {
                    id,
                    command: listello_protocol::WindowCommand::Close,
                })
                .is_empty());
        }
    }

    #[test]
    fn an_icon_nobody_has_is_a_none_answer_not_an_error() {
        let mut agent = agent_with(Vec::new());
        let reply = agent.handle(CoreToAgent::RequestIcon {
            app_id: AppId::new("no-such-app-anywhere"),
            size_px: 64,
        });
        assert_eq!(
            reply,
            vec![AgentToCore::Icon {
                app_id: AppId::new("no-such-app-anywhere"),
                icon: None
            }]
        );
    }

    #[test]
    fn launching_an_unknown_app_is_silent() {
        let mut agent = agent_with(Vec::new());
        assert!(agent
            .handle(CoreToAgent::Launch {
                app_id: AppId::new("no-such-app-anywhere")
            })
            .is_empty());
    }

    fn sensor_agent(power: &crate::power::fixture::TempDir) -> Agent {
        agent_with(Vec::new()).with_sensors(
            Sensors {
                power_root: Some(power.path().to_path_buf()),
                net_root: None,
                proc_wireless: None,
            },
            None,
        )
    }

    fn write_battery(power: &crate::power::fixture::TempDir, percent: u32) {
        power.write("BAT0/type", "Battery\n");
        power.write("BAT0/capacity", &format!("{percent}\n"));
        power.write("BAT0/status", "Discharging\n");
    }

    #[test]
    fn an_unchanged_battery_is_never_sent_twice_and_a_change_is() {
        let power = crate::power::fixture::TempDir::new("agent-power");
        write_battery(&power, 80);
        let mut agent = sensor_agent(&power);
        let start = Instant::now();

        let first = agent.poll_sensors(start);
        assert!(matches!(
            first.as_slice(),
            [AgentToCore::Battery(Some(b))] if b.charge == 0.8
        ));
        // Not due yet, and then due but identical: silence either way.
        assert!(agent
            .poll_sensors(start + Duration::from_secs(1))
            .is_empty());
        assert!(agent.poll_sensors(start + SENSOR_EVERY).is_empty());

        write_battery(&power, 79);
        let changed = agent.poll_sensors(start + SENSOR_EVERY * 2);
        assert!(matches!(
            changed.as_slice(),
            [AgentToCore::Battery(Some(b))] if b.charge == 0.79
        ));
    }

    #[test]
    fn a_reconnecting_renderer_is_told_everything_again() {
        let power = crate::power::fixture::TempDir::new("agent-greet");
        write_battery(&power, 50);
        let mut agent = sensor_agent(&power);
        let first = agent.greeting();
        assert!(first
            .iter()
            .any(|m| matches!(m, AgentToCore::Battery(Some(_)))));
        // Same reading, but a new renderer: it must be sent again.
        let again = agent.greeting();
        assert!(again
            .iter()
            .any(|m| matches!(m, AgentToCore::Battery(Some(_)))));
    }

    #[test]
    fn a_machine_without_a_battery_says_so_once() {
        let power = crate::power::fixture::TempDir::new("agent-none");
        let mut agent = sensor_agent(&power);
        let start = Instant::now();
        assert!(agent
            .poll_sensors(start)
            .contains(&AgentToCore::Battery(None)));
        assert!(agent.poll_sensors(start + SENSOR_EVERY).is_empty());
    }

    #[test]
    fn watching_a_folder_answers_with_its_key_and_never_its_path() {
        let home = crate::power::fixture::TempDir::new("agent-home");
        home.write("Downloads/file", "x");
        let folders = Folders::new(Some(home.path().to_path_buf()), None);
        let mut agent = agent_with(Vec::new()).with_sensors(Sensors::default(), folders);
        let reply = agent.handle(CoreToAgent::WatchFolder {
            key: "down".into(),
            path: "~/Downloads".into(),
        });
        assert_eq!(
            reply,
            vec![AgentToCore::Folder(listello_protocol::FolderState {
                key: "down".into(),
                empty: false
            })]
        );
        // And the greeting repeats it for a renderer that reconnects.
        assert!(agent
            .greeting()
            .iter()
            .any(|m| matches!(m, AgentToCore::Folder(f) if f.key == "down")));
    }

    #[test]
    fn watch_requests_are_ignored_where_folder_watching_is_unavailable() {
        let mut agent = agent_with(Vec::new());
        assert!(agent
            .handle(CoreToAgent::WatchFolder {
                key: "k".into(),
                path: "/tmp".into()
            })
            .is_empty());
    }
}
