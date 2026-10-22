//! The messages that make listello work.
//!
//! This crate exists from day one on purpose. D3 says every source of change sends a message
//! and the single-threaded loop owns all state; D11 says one of those sources lives in another
//! process. **The same enum is both** — the loop's internal message type and the wire format
//! between `listello-core` and `listello-agent`. Keeping it in a crate that neither binary
//! owns is what stops the D11 boundary quietly rotting back into a single process.
//!
//! Two directions, and they are not symmetric:
//!
//! - [`AgentToCore`] carries **state** — window lists, battery, wifi, decoded icon pixels.
//! - [`CoreToAgent`] carries **actions** — activate this window, launch this app, open this
//!   folder, lock the session.
//!
//! Two rules that are easy to violate and expensive to fix later:
//!
//! 1. **Icons cross as pixels, not paths.** The renderer runs as `nobody` and cannot read the
//!    user's icon theme under `~/.local/share`, so a path is not portable across the boundary.
//!    [`Icon`] is therefore raw bytes with a width and a height, and nothing else.
//! 2. **The renderer must be useful with no agent connected.** Nothing in [`CoreToAgent`] may
//!    be on the path to function keys, media keys or the Fn layer. See the note on
//!    [`SystemAction`], which is one variant long for exactly that reason.
//!
//! # The shape of the loop
//!
//! [`Event`] is what the single-threaded loop consumes, and everything that can change state
//! is one of its variants — input, the frame clock, the agent, the outside world. A source of
//! change is added by adding a variant, never by handing something a reference to the core.
//!
//! ```text
//!   libinput ──┐
//!   frame clock├──► Event ──► loop (owns all state) ──► snapshot ──► widgets ──► Frame
//!   agent ─────┤                    │
//!   signals ───┘                    └──► CoreToAgent ──► socket ──► listello-agent
//! ```
//!
//! M1 ships no agent: the simulator feeds [`AgentToCore`] values straight into the loop, which
//! is exactly what the socket will do in M4, and is why the types have to exist now.
//!
//! # Versioning
//!
//! Both binaries ship from this repository and are installed together, so the protocol is
//! versioned only enough to fail loudly on a mismatched pair — not to be compatible with one.
//! [`PROTOCOL_VERSION`] is exchanged in the two `Hello` messages; a mismatch closes the
//! socket. See [`wire`] for the framing.

#![deny(unsafe_code)]

pub mod link;
pub mod wire;

use serde::{Deserialize, Serialize};

/// Bumped whenever a message changes meaning, or a variant is added or removed. Adding one
/// counts: the codec is not self-describing, so an older peer cannot skip what it does not
/// know — it mis-parses it.
pub const PROTOCOL_VERSION: u32 = 9;

// ---------------------------------------------------------------------------------------
// Identifiers
// ---------------------------------------------------------------------------------------

/// A window, as the agent's `WindowSource` sees it.
///
/// Opaque on purpose: KWin's handle, a wlroots toplevel and an X11 `Window` have nothing in
/// common, so the agent mints an id and keeps the mapping. The renderer only ever hands one
/// back ([`CoreToAgent::ActivateWindow`]), and an id that no longer exists is a no-op — the
/// window list is a snapshot, and the user can always tap a stale one.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct WindowId(pub u64);

/// An application, keyed the way the desktop keys it: the `.desktop` file's basename without
/// the extension (`org.kde.dolphin`), which is also what Wayland's `app_id` carries.
///
/// Not a path. The renderer must never be handed something it would have to resolve on disk.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct AppId(pub String);

impl AppId {
    pub fn new(id: impl Into<String>) -> AppId {
        AppId(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

// ---------------------------------------------------------------------------------------
// State: agent → core
// ---------------------------------------------------------------------------------------

/// Decoded icon pixels, ready to become a `cairo::ImageSurface` on the far side.
///
/// Straight (non-premultiplied) RGBA8, `width * height * 4` bytes, no row padding — the same
/// convention as `DrawCmd`'s colours, and the format every icon loader can produce. The
/// renderer converts to cairo's premultiplied ARGB32 once, at receive time; that conversion is
/// deliberately on the renderer's side so the wire format does not encode a cairo detail.
///
/// Sizes are the agent's choice — it is the side that knows the icon theme and can pick the
/// nearest available size — but it should send something close to what the Dock will draw.
/// This is the largest thing that ever crosses the socket, and it is why the codec is binary.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Icon {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes of straight RGBA8.
    pub rgba: Vec<u8>,
}

impl Icon {
    /// `None` when the byte count does not match the dimensions. Validating at construction
    /// rather than at the draw site is what keeps a corrupt or hostile message away from
    /// cairo — the renderer is a daemon and must not panic on external data.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Option<Icon> {
        let expected = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        (expected != 0 && rgba.len() == expected).then_some(Icon {
            width,
            height,
            rgba,
        })
    }

    /// The same check, for an [`Icon`] that arrived over the socket rather than through
    /// [`Icon::new`]. Deserialisation cannot go through the constructor, so every received
    /// icon must be run past this before it is used.
    pub fn is_valid(&self) -> bool {
        (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|n| n.checked_mul(4))
            .is_some_and(|n| n != 0 && n == self.rgba.len())
    }
}

/// One open window.
///
/// No picture: D7's row of windows is labelled entries, and a preview, where the user wants
/// one, arrives separately as [`AgentToCore::Preview`] while the row is open.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct WindowInfo {
    pub id: WindowId,
    pub app_id: AppId,
    /// The title, as the compositor reports it. Arbitrary user data: it can be empty,
    /// enormous, or hostile, and the widget that draws it must assume all three.
    pub title: String,
    pub active: bool,
    pub minimized: bool,
}

/// Battery, when there is one. A desktop reports none and the widget draws nothing.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct BatteryState {
    /// `0.0..=1.0`. M5 wants both the percentage *and* the bar, from this one number.
    pub charge: f64,
    /// On external power (the adapter is in), whether or not the battery is taking charge.
    pub charging: bool,
    /// `None` when UPower has no estimate yet, which is normal for a minute or two after a
    /// state change.
    pub time_remaining_secs: Option<u64>,
}

/// Wifi, as much of it as a symbol needs.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct NetworkState {
    pub connected: bool,
    /// `0.0..=1.0`, for choosing which signal-strength glyph to draw.
    pub signal: f64,
    pub ssid: Option<String>,
}

/// Whether a watched folder has anything in it. A Trash entry needs full/empty (M5),
/// and the same answer is worth having for Downloads and Screenshots.
///
/// `key` is whatever the renderer passed to [`CoreToAgent::WatchFolder`] — the config's own
/// identifier for that entry. The path does not come back: it went out once, and echoing it
/// would tempt the renderer into treating it as a filesystem path it can act on.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct FolderState {
    pub key: String,
    pub empty: bool,
}

/// Something the agent cannot supply, in words a human can act on.
///
/// This exists because of the D11 amendment: KWin does not advertise
/// `org_kde_plasma_window_management` to an uninstalled binary, so the agent sees an empty
/// registry with no error to report — and an empty window list looks exactly like our own bug.
/// The agent says *why* instead, and the renderer logs it once.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Limitation {
    /// No window-management protocol: the agent is running from an uninstalled path, or its
    /// `.desktop` file is missing or does not declare `X-KDE-Wayland-Interfaces`.
    NoWindowManagement,
    /// The compositor is not one any `WindowSource` backend supports.
    UnsupportedCompositor { name: String },
    /// A session service is absent — UPower, NetworkManager, logind.
    ServiceUnavailable { service: String },
    /// The compositor will not hand over window pictures, so the window row has no previews:
    /// on KDE, the agent's `.desktop` file is missing or lacks
    /// `X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2`.
    NoWindowCapture,
}

/// State, flowing from the session into the renderer.
///
/// Every payload-bearing variant is a **complete replacement, never a delta**. The agent may
/// die and reconnect at any moment, and an incremental protocol would need a resync path that
/// is only ever exercised on the day it is already broken. Window lists are small; icons are
/// requested one at a time and cached.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum AgentToCore {
    /// First message on every connection. A [`PROTOCOL_VERSION`] mismatch closes the socket.
    Hello { protocol_version: u32 },

    /// The complete set of open windows, in the order each first appeared -- not stacking
    /// order, which reshuffles on every focus change and would make a Dock's tiles jump
    /// around whenever one is tapped. Each `app_id` is already resolved to its desktop
    /// entry's id where there is one.
    Windows(Vec<WindowInfo>),

    /// Pixels for an app, in reply to [`CoreToAgent::RequestIcon`]. `None` means the agent
    /// looked and found nothing: a normal answer that gets the fallback glyph, not an error,
    /// and not something to retry in a loop.
    Icon { app_id: AppId, icon: Option<Icon> },

    /// `None` when the machine has no battery, or UPower is not there.
    Battery(Option<BatteryState>),

    /// `None` when there is no wifi device, or NetworkManager is not there.
    Network(Option<NetworkState>),

    /// One watched folder changed.
    Folder(FolderState),

    /// The apps the user has pinned to the Dock from the Touch Bar, in order. `None` when they
    /// never have: the Dock then shows the config's `pinned`. Sent on connecting and after every
    /// [`CoreToAgent::SavePins`].
    Pins(Option<Vec<AppId>>),

    /// A launch the agent started is believed to be under way. M6's bounce is driven by this
    /// and by the window that eventually appears; RESEARCH §6.4 is why it has to be a message
    /// rather than something the renderer can infer for itself.
    LaunchStarted { app_id: AppId },

    /// A picture of one window, for the window row, in reply to
    /// [`CoreToAgent::WatchPreviews`] and repeated while the row stays open whenever the window
    /// looks different. Already scaled to fit the size asked for. `None` means no picture —
    /// minimised, gone, or refused — and the row draws a placeholder.
    Preview { id: WindowId, image: Option<Icon> },

    /// Something the agent cannot do, and why. Sent once per cause, not once per poll.
    Limited(Limitation),

    /// The user's own layout (D8 amendment): the text of `~/.config/listello/config.toml`, in
    /// the same format as the machine's, or `None` when there is none or it does not validate —
    /// the machine's layout then stays up. Text, not a parsed config: the renderer validates it
    /// again, since nothing that arrives here is trusted. `time_locale` is the user's `LC_TIME`
    /// (else `LANG`), what "follow the system" means while this user is connected. Sent on
    /// connecting and whenever the file changes; the renderer drops both when the agent goes.
    UserConfig {
        layout: Option<String>,
        time_locale: Option<String>,
    },

    /// "Reload listello" (D8 amendment), from the tray or the settings window: the renderer
    /// exits and systemd starts it again (`RestartForceExitStatus`), so the Touch Bar starts
    /// over from nothing. The agent reconnects by itself and sends everything again.
    Restart,

    /// "Stop listello" (D8, second amendment): the user stopped listello, or logged out. The
    /// renderer leaves at once and the Touch Bar goes back to the T2's own keys, instead of
    /// waiting out the grace period it gives an agent that merely blipped. A machine with no
    /// such fallback (Apple Silicon) ignores it and treats it as the agent leaving.
    Quit,
}

// ---------------------------------------------------------------------------------------
// Actions: core → agent
// ---------------------------------------------------------------------------------------

/// Session-level actions that genuinely need the user's session bus.
///
/// One variant, and it should stay close to one. It is tempting to put M5's control centre
/// here — mute, volume, brightness — and it would be wrong: those are key events, emitted
/// through uinput by `listello-core` itself, exactly as tiny-dfr already emits media keys.
/// Routing them through the agent would make half the control centre stop working when the
/// user logs out, which is the failure D11 exists to prevent. Locking is the exception that
/// proves it: there is no key event for it, and logind is a session service.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum SystemAction {
    LockSession,
}

/// What a window's title-bar buttons do, offered by the Dock's window controls.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum WindowCommand {
    /// What the window's close button does: the app decides, so some apps only hide to the
    /// system tray. Never a kill.
    Close,
    Minimize,
    /// Maximise, or restore a window that already is.
    ToggleMaximize,
}

/// How a window's picture is made to the size the row asks for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub enum PreviewFit {
    /// The whole window, shrunk to fit inside, keeping its shape. Smaller than asked for in one
    /// direction.
    #[default]
    Whole,
    /// Exactly the size asked for: a band through the middle of the window, as wide as the
    /// window and as tall as the box's shape allows.
    Crop,
    /// Exactly the size asked for: the whole window, squashed or stretched into it.
    Stretch,
}

/// Actions, flowing from the renderer into the session.
///
/// Fire-and-forget. Nothing here has a reply except [`CoreToAgent::RequestIcon`], and even
/// that answer is an ordinary [`AgentToCore::Icon`] rather than a correlated response — there
/// is no request/response machinery in this protocol, and there should not be one. A loop that
/// waits for an answer is a loop that has stopped drawing.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum CoreToAgent {
    /// First message on every connection. A [`PROTOCOL_VERSION`] mismatch closes the socket.
    Hello {
        protocol_version: u32,
    },

    /// Raise and focus a window. A stale id is a no-op.
    ActivateWindow(WindowId),

    /// Close, minimise or maximise a window, as its title-bar buttons would. A stale id is a
    /// no-op.
    WindowCommand {
        id: WindowId,
        command: WindowCommand,
    },

    /// Keep this as the user's pinned apps, in this order, replacing whatever was kept before.
    /// The agent stores it in the user's own config directory, because the renderer can write
    /// nowhere, and answers with [`AgentToCore::Pins`].
    SavePins(Vec<AppId>),

    /// Start an app that is pinned but not running.
    Launch {
        app_id: AppId,
    },

    /// Open a folder in the user's file manager. The renderer holds the configured path as an
    /// opaque string and never touches the filesystem itself — it could not, as `nobody`.
    OpenPath {
        path: String,
    },

    /// Ask for an app's icon at roughly this size. The renderer caches by [`AppId`]; the agent
    /// answers once and may later push a fresh [`AgentToCore::Icon`] unprompted if the icon
    /// theme changes.
    RequestIcon {
        app_id: AppId,
        size_px: u32,
    },

    /// The window row is showing these windows: send an [`AgentToCore::Preview`] of each at
    /// once and keep them fresh until told otherwise. Each picture is made to `height_px` ×
    /// `max_width_px` as `fit` says. An empty list means the row closed: stop capturing. Never
    /// sent when the user has previews turned off, so then nothing is ever captured (D7).
    /// `live` asks for a picture whenever a window changes, at a capped rate, rather than a
    /// still now and then; an agent that cannot stream falls back to stills.
    WatchPreviews {
        windows: Vec<WindowId>,
        height_px: u32,
        max_width_px: u32,
        fit: PreviewFit,
        live: bool,
    },

    /// Watch a folder and report [`AgentToCore::Folder`] whenever its emptiness changes.
    /// `key` comes back unchanged in the reply.
    WatchFolder {
        key: String,
        path: String,
    },

    SystemAction(SystemAction),
}

// ---------------------------------------------------------------------------------------
// Input, and the loop's own event type
// ---------------------------------------------------------------------------------------

/// What a touch is doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum TouchPhase {
    Down,
    Motion,
    Up,
    /// The driver or the compositor took the touch away mid-gesture. Not the same as `Up`: a
    /// cancelled touch must not activate whatever it was over.
    Cancel,
}

/// One touch point, in the same logical landscape coordinates a widget lays out in
/// (`listello-surface`'s `Rect`). The libinput backend and the simulator's mouse backend both
/// produce these, and nothing downstream can tell which one it was — the input equivalent of
/// what `Surface` does for pixels.
///
/// The Touch Bar is multi-touch; `slot` distinguishes fingers, and the Dock's horizontal
/// scroll (M4) is why the intermediate `Motion` events are carried rather than collapsed.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Touch {
    pub slot: u32,
    pub phase: TouchPhase,
    pub x: f64,
    pub y: f64,
}

/// Everything the input layer can say.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub enum InputEvent {
    Touch(Touch),
    /// The Fn key changed state (D6: holding Fn shows F1–F12). It is a keyboard modifier, not
    /// a touch, and the renderer owns it with or without an agent.
    FnKey {
        pressed: bool,
    },
}

/// Everything the single-threaded loop consumes.
///
/// This list is the whole answer to "what can change state" (D3). A new source of change means
/// a new variant here; it never means handing something a reference to the core.
///
/// Not `Serialize`: this type never crosses the socket. Only [`AgentToCore`] and
/// [`CoreToAgent`] do, and keeping the derive off is what makes that hard to forget.
#[derive(Clone, PartialEq, Debug)]
pub enum Event {
    Input(InputEvent),

    /// The frame clock fired: build a frame. This variant is the reason M1 builds a clock at
    /// all — tiny-dfr's loop wakes only on input or a 10 s timer and cannot drive an animation
    /// (RESEARCH §2.1), while the Dock's scrolling (M4) and the launch bounce (M6) both need
    /// one. `elapsed` is the time since the previous frame, so animation state advances by
    /// measured time rather than by counting frames that may not have been delivered.
    Frame {
        elapsed_secs: f64,
    },

    /// A message arrived from the agent.
    Agent(AgentToCore),

    /// The agent connected, or went away. Losing it is **not** an error: the renderer falls
    /// back to tiny-dfr's behaviour — function keys, media keys, the Fn layer — and keeps
    /// drawing. It never goes black.
    AgentConnected,
    AgentDisconnected,

    /// The wall clock moved in a way a clock widget must notice: a minute boundary, a timezone
    /// change, a resume. Separate from [`Event::Frame`] so that a clock does not have to read
    /// the time on every animation frame to discover that nothing changed.
    TimeChanged,

    /// `/etc/listello/config.toml` changed on disk and has already been reloaded. Config-local
    /// to `listello-core` — the agent has no config of its own to reload — but it is still a
    /// source of change per D3, so it gets a variant rather than a special case in the loop.
    ConfigReloaded,

    /// The panel came back from suspend and may need its mode set again (RESEARCH §1).
    DisplayReset,

    /// SIGTERM, SIGINT, or the simulator's window closing. Run the shutdown path once.
    Shutdown,
}

impl From<InputEvent> for Event {
    fn from(e: InputEvent) -> Event {
        Event::Input(e)
    }
}

impl From<AgentToCore> for Event {
    fn from(m: AgentToCore) -> Event {
        Event::Agent(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_rejects_a_mismatched_byte_count() {
        assert!(Icon::new(2, 2, vec![0; 16]).is_some());
        assert!(Icon::new(2, 2, vec![0; 15]).is_none());
        assert!(Icon::new(0, 4, Vec::new()).is_none());
        // The multiplication that would overflow must answer None, not abort the daemon.
        assert!(Icon::new(u32::MAX, u32::MAX, Vec::new()).is_none());
    }

    #[test]
    fn is_valid_catches_what_the_constructor_never_saw() {
        let icon = Icon::new(3, 5, vec![7; 60]).expect("well-formed");
        assert!(icon.is_valid());
        // Deserialisation does not go through the constructor, so this is reachable.
        let forged = Icon {
            width: 3,
            height: 5,
            rgba: vec![7; 59],
        };
        assert!(!forged.is_valid());
        let overflowing = Icon {
            width: u32::MAX,
            height: u32::MAX,
            rgba: Vec::new(),
        };
        assert!(!overflowing.is_valid());
    }
}
