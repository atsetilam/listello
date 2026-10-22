//! The immutable snapshot a widget reads, the icons it draws with, and the colours it draws in.

use crate::hit::RegionId;
use cairo::ImageSurface;
use chrono::{DateTime, FixedOffset};
use listello_protocol::{
    AppId, BatteryState, FolderState, Icon, NetworkState, WindowId, WindowInfo,
};
use listello_surface::{image_from_rgba, Color, TextStyle};
use std::collections::HashMap;

/// Which icon, without saying where it came from.
///
/// **Never a path.** `listello-ui` does no I/O (D3) and the renderer runs as `nobody` and cannot
/// read the user's icon theme (D11), so a widget names an icon and the binary is the
/// one that already put the pixels in [`Icons`]. The two variants differ only in who filled it:
/// `Named` at startup from the packaged set, `App` at runtime from the agent.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum IconKey {
    /// A packaged icon named by config — `"volume-up"`, `"trash-full"`. `listello-core` loads
    /// these through librsvg, `listello-sim` through cairo's PNG loader; a *name* rather than a
    /// path is what lets those two differ without any widget noticing.
    Named(String),
    /// An application icon, pushed by the agent in reply to `CoreToAgent::RequestIcon`. Absent
    /// until it arrives, and absent again for as long as no agent is connected.
    App(AppId),
}

/// The pixels a widget may draw, keyed by name.
///
/// A widget only ever looks up. It cannot insert, cannot load, and cannot ask for one that has
/// not arrived yet — [`Icons::get`] returning `None` is a **normal state**, not an error: a
/// freshly started strip has no app icons at all until the agent answers. Every widget must
/// draw something sensible in that case (its label, or nothing), and none of them may
/// `.expect()`.
#[derive(Clone, Debug, Default)]
pub struct Icons {
    map: HashMap<IconKey, ImageSurface>,
}

impl Icons {
    pub fn new() -> Icons {
        Icons::default()
    }

    pub fn get(&self, key: &IconKey) -> Option<&ImageSurface> {
        self.map.get(key)
    }

    /// Store already-decoded pixels. The binary's entry point for the packaged set.
    pub fn insert(&mut self, key: IconKey, image: ImageSurface) {
        self.map.insert(key, image);
    }

    /// Store an icon that arrived over the socket. `false` when the message was malformed —
    /// [`Icon::is_valid`]'s case, plus anything cairo refuses — in which case nothing is stored
    /// and the widget falls back exactly as it would for an icon that never came.
    ///
    /// Deserialisation does not go through `Icon::new`, so this is one of the two places the
    /// protocol says every received icon has to be re-checked.
    pub fn insert_app(&mut self, app_id: AppId, icon: &Icon) -> bool {
        if !icon.is_valid() {
            return false;
        }
        match image_from_rgba(icon.width, icon.height, &icon.rgba) {
            Some(image) => {
                self.map.insert(IconKey::App(app_id), image);
                true
            }
            None => false,
        }
    }

    /// Drop every icon the agent supplied, keeping the packaged ones. Called on disconnect: a
    /// stale app icon outlives the app it belonged to, and the packaged set never goes stale.
    pub fn clear_app_icons(&mut self) {
        self.map.retain(|k, _| matches!(k, IconKey::Named(_)));
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Everything a widget is allowed to know about the world.
///
/// Immutable for the duration of a frame, and built by the shell from the messages the loop
/// delivered (D3). Growing this struct is how a new source of state reaches widgets. Handing a
/// widget anything it could *call* is how this design dies.
///
/// # Everything the agent supplies is optional, and that is D11
///
/// `battery`, `network`, `folders`, `windows` and the `App` half of `icons` are all filled from
/// `AgentToCore` messages, and all of them are empty whenever no agent is connected — which is
/// a supported, permanent state, not a startup transient. The renderer must stay useful with no
/// agent: function keys, media keys and the clock are `listello-core`'s own job and do not
/// depend on any of these fields. A widget that cannot draw without the agent draws *nothing*
/// rather than a placeholder that lies.
#[derive(Clone, Debug)]
pub struct UiState {
    /// Wall-clock time, already converted to the local offset by the shell.
    ///
    /// `FixedOffset` rather than `Local` on purpose: a widget must never consult the machine's
    /// timezone database, both because that is I/O and because a test that did would produce a
    /// different answer on every machine it ran on. The shell resolves the offset
    /// once; a test constructs one directly.
    pub now: DateTime<FixedOffset>,

    /// The region currently held down, if any — what a widget consults to draw itself pressed.
    /// Set on `TouchPhase::Down`, cleared on `Up` *and* `Cancel`.
    pub pressed: Option<RegionId>,

    /// Whether the Fn key is held (D6: holding Fn shows F1–F12).
    pub fn_held: bool,

    /// Whether an agent is connected at all. A widget rarely needs this — an empty `windows`
    /// says the same thing — but the Dock wants to tell "no windows open" from "nobody is
    /// telling me about windows", and those deserve different pixels.
    pub agent_connected: bool,

    /// `None` on a desktop, when UPower is absent, and whenever no agent is connected.
    pub battery: Option<BatteryState>,

    /// `None` when there is no wifi device, NetworkManager is absent, or no agent is connected.
    pub network: Option<NetworkState>,

    /// Watched folders, by the config key that was sent in `CoreToAgent::WatchFolder`. The path
    /// is not here and never comes back over the socket (`FolderState`'s doc comment says why).
    pub folders: HashMap<String, FolderState>,

    /// Every open window, in the compositor's stacking order. Grouping into apps is the Dock
    /// widget's job, not this struct's: the snapshot stays the shape the wire delivered it in.
    pub windows: Vec<WindowInfo>,

    /// The pixels widgets may draw. See [`Icons`].
    pub icons: Icons,

    /// The app whose windows the Dock is showing as a row of entries (D7), if any. Held here
    /// for the same reason scroll is: a widget cannot remember a tap.
    pub expanded: Option<AppId>,

    /// Horizontal scroll offset in logical pixels, by widget index.
    ///
    /// A widget is a pure function and cannot hold a scroll position across frames, so the
    /// shell holds it and the widget reads its own through [`crate::Ctx::scroll`]. Absent means
    /// zero. Clamping against the content width happens at draw time
    /// ([`crate::layout::scroll_offset_clamped`]), not here — this struct does not know how
    /// wide anything is.
    pub scroll: HashMap<u16, f64>,

    /// Apps the agent started and no window has answered yet, with how long each has run. The Dock bounces their icons. Held here because a widget cannot count time; the
    /// shell advances it from the frame clock and drops an entry when the app's window appears
    /// or [`LAUNCH_TIMEOUT_SECS`] runs out.
    pub launching: HashMap<AppId, Launch>,

    /// The window whose controls (close, minimise, maximise) the Dock is showing after a long
    /// press, and for how long they have been up. Its icon wiggles meanwhile.
    pub controls: Option<WindowControls>,

    /// The apps the user pinned from the Touch Bar, as the agent keeps them. `None` until they
    /// pin or unpin something; the Dock then shows its config's `pinned`.
    pub pins: Option<Vec<AppId>>,

    /// Whether the row of windows shows a picture of each, and how (`[strip] window_previews`,
    /// `preview_style`); `None` with previews off. Config rather than state, copied in because
    /// the Dock lays the row out differently with it, and a widget sees nothing but this
    /// snapshot.
    pub window_previews: Option<crate::config::PreviewStyle>,

    /// The pictures the agent sent for the open row's windows. A window with none yet, or
    /// none at all (minimised), gets a placeholder of the same size.
    pub previews: HashMap<WindowId, ImageSurface>,
}

/// Seconds for one hop of a launching app's icon.
pub const BOUNCE_PERIOD: f64 = 0.7;

/// One launch the Dock is bouncing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Launch {
    /// Seconds since the launch began.
    pub age: f64,
    /// Once the app's window has appeared: the age at which the bounce stops, so the icon ends
    /// on a last hop landing on the floor instead of freezing in mid-air.
    pub ends_at: Option<f64>,
}

impl Launch {
    pub fn started() -> Launch {
        Launch {
            age: 0.0,
            ends_at: None,
        }
    }

    /// The window appeared: finish the hop in progress, plus one more if less than half of it
    /// is left, so there is always a visible last hop. Does nothing the second time.
    pub fn finish(&mut self) {
        if self.ends_at.is_some() {
            return;
        }
        let left = BOUNCE_PERIOD - self.age % BOUNCE_PERIOD;
        let extra = if left < BOUNCE_PERIOD / 2.0 {
            BOUNCE_PERIOD
        } else {
            0.0
        };
        self.ends_at = Some(self.age + left + extra);
    }

    pub fn is_over(&self) -> bool {
        self.age >= self.ends_at.unwrap_or(LAUNCH_TIMEOUT_SECS)
    }
}

/// How long a launch may bounce without a window appearing before the shell gives up on it.
/// Some apps open nothing visible (a tray-only app, a launch that failed after starting).
pub const LAUNCH_TIMEOUT_SECS: f64 = 10.0;

impl UiState {
    /// A snapshot with nothing in it but the time: no agent, no icons, nothing pressed. What
    /// both binaries start from, and what every test that does not care about the agent wants.
    pub fn new(now: DateTime<FixedOffset>) -> UiState {
        UiState {
            now,
            pressed: None,
            fn_held: false,
            agent_connected: false,
            battery: None,
            network: None,
            folders: HashMap::new(),
            windows: Vec::new(),
            icons: Icons::new(),
            expanded: None,
            scroll: HashMap::new(),
            launching: HashMap::new(),
            controls: None,
            pins: None,
            window_previews: None,
            previews: HashMap::new(),
        }
    }

    /// Forget everything the agent told us, keeping what is ours: the time, the packaged icons,
    /// the pressed region, the Fn key.
    ///
    /// Called when the agent disconnects. **A stale window list is worse than no window list** —
    /// tapping an app that closed while the agent was down would activate a window id that no
    /// longer exists, and the strip would go on claiming a battery level that stopped updating.
    pub fn clear_agent_state(&mut self) {
        self.agent_connected = false;
        self.battery = None;
        self.network = None;
        self.folders.clear();
        self.windows.clear();
        self.expanded = None;
        self.launching.clear();
        self.controls = None;
        self.pins = None;
        self.previews.clear();
        self.icons.clear_app_icons();
    }
}

/// The controls a long press brought up: for which app, for which of its windows (none when
/// the app is pinned but not running, and only its pin is offered), and seconds since they
/// appeared.
#[derive(Clone, PartialEq, Debug)]
pub struct WindowControls {
    pub app: AppId,
    pub window: Option<WindowId>,
    pub age: f64,
}

/// Colours and text style. Supplied by the binary, never resolved here.
///
/// The font in particular: `listello-core` gets it from fontconfig (`fonts.rs`), while
/// `listello-sim` uses cairo's toy font API, which needs no fontconfig and therefore works on
/// the Mac. This crate only ever draws with the face it is handed.
///
/// Icons are deliberately **not** here even though they too are supplied by the binary: half of
/// them arrive at runtime from the agent, so they live in [`UiState`] with everything else that
/// changes while the strip is up.
#[derive(Clone, Debug)]
pub struct Theme {
    pub text: TextStyle,
    pub background: Color,
    pub foreground: Color,
    pub button: Color,
    pub button_pressed: Color,
    /// The one colour that means "this is on / charging / active" rather than "this is a
    /// button". The battery widget's charging fill is its first caller.
    pub accent: Color,
    pub corner_radius: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-08-04T09:00:00+02:00").expect("valid timestamp")
    }

    fn icon(width: u32, height: u32) -> Icon {
        Icon::new(width, height, vec![0xff; (width * height * 4) as usize])
            .expect("well-formed icon")
    }

    #[test]
    fn a_new_state_has_nothing_from_the_agent() {
        let state = UiState::new(now());
        assert!(!state.agent_connected);
        assert!(state.battery.is_none());
        assert!(state.network.is_none());
        assert!(state.windows.is_empty());
        assert!(state.folders.is_empty());
        assert!(state.icons.is_empty());
    }

    #[test]
    fn an_app_icon_round_trips_into_the_store() {
        let mut icons = Icons::new();
        assert!(icons.insert_app(AppId::new("org.kde.dolphin"), &icon(4, 4)));
        assert!(icons
            .get(&IconKey::App(AppId::new("org.kde.dolphin")))
            .is_some());
    }

    #[test]
    fn a_lying_icon_is_refused_rather_than_drawn() {
        // Dimensions that do not match the byte count: what a hostile or buggy agent sends, and
        // what deserialisation will hand us unchecked because it skips `Icon::new`.
        let liar = Icon {
            width: 64,
            height: 64,
            rgba: vec![0xff; 16],
        };
        let mut icons = Icons::new();
        assert!(!icons.insert_app(AppId::new("liar"), &liar));
        assert!(icons.get(&IconKey::App(AppId::new("liar"))).is_none());
    }

    #[test]
    fn a_zero_sized_icon_is_refused() {
        let empty = Icon {
            width: 0,
            height: 0,
            rgba: Vec::new(),
        };
        let mut icons = Icons::new();
        assert!(!icons.insert_app(AppId::new("empty"), &empty));
    }

    #[test]
    fn clearing_agent_state_keeps_packaged_icons_and_drops_app_icons() {
        let mut state = UiState::new(now());
        let packaged = image_from_rgba(2, 2, &[0xff; 16]).expect("cairo should make a 2x2 surface");
        state
            .icons
            .insert(IconKey::Named("volume-up".into()), packaged);
        state.icons.insert_app(AppId::new("app"), &icon(2, 2));
        assert_eq!(state.icons.len(), 2);

        state.clear_agent_state();
        assert_eq!(state.icons.len(), 1);
        assert!(state
            .icons
            .get(&IconKey::Named("volume-up".into()))
            .is_some());
        assert!(state.icons.get(&IconKey::App(AppId::new("app"))).is_none());
    }

    #[test]
    fn clearing_agent_state_keeps_the_time_and_the_pressed_region() {
        let mut state = UiState::new(now());
        state.pressed = Some(RegionId::new(3, 1));
        state.fn_held = true;
        state.battery = Some(BatteryState {
            charge: 0.5,
            charging: false,
            time_remaining_secs: None,
        });

        state.clear_agent_state();
        assert_eq!(state.now, now());
        assert_eq!(state.pressed, Some(RegionId::new(3, 1)));
        assert!(state.fn_held);
        assert!(state.battery.is_none());
    }
}
