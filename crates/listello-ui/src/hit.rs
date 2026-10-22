//! Where presses land, and what they ask for.
//!
//! These types live here rather than in `listello-protocol` or `listello-surface`, and both
//! exclusions are deliberate:
//!
//! - **Not `listello-protocol`**, because nothing here crosses the D11 socket. Only
//!   `AgentToCore` and `CoreToAgent` do, and keeping that set small is what makes the
//!   boundary auditable.
//! - **Not `listello-surface`**, because that is the *pixel* line (D10). A `Surface` turns
//!   commands into pixels and has no concept of a press.

use listello_protocol::{AppId, SystemAction, WindowCommand, WindowId};
use listello_surface::Rect;

/// Which widget, and which part of it.
///
/// Stable across frames by construction: `widget` is the widget's index in the strip and
/// `part` is its own numbering of its sub-regions. That stability is what lets [`crate::UiState`]
/// name a region as "currently held down" and have the next frame agree — a per-frame counter
/// would silently repaint the wrong button whenever a widget's contents changed.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RegionId {
    pub widget: u16,
    pub part: u16,
}

impl RegionId {
    pub const fn new(widget: u16, part: u16) -> RegionId {
        RegionId { widget, part }
    }
}

/// The name of a key to emit, as written in config.
///
/// **Deliberately a name and not an `input_linux::Key`.** That crate does not build on macOS
/// (`input-linux-sys` cannot find the Linux uapi types; measured 2026-08-03), and naming it
/// here would drag this whole crate back into `listello-core` and out of reach of the Mac —
/// which is the one machine M1's exit criterion runs on.
///
/// Resolution to an evdev code therefore happens in `listello-core`, when it builds a layer.
/// **That is a deferral, not a design:** it means an unknown key name is caught later than it
/// should be. `listello-core` must report it as a config error and keep running — never
/// `.expect()` on it — and M3, which is where the Fn row comes back, is where a proper
/// name→code table earns its place.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct KeyName(String);

impl KeyName {
    /// `None` if the name is empty or is not plausibly a key name. Validation is deliberately
    /// weak here — phase 04's config parser owns the grammar.
    pub fn new(name: impl Into<String>) -> Option<KeyName> {
        let name = name.into();
        if name.is_empty() || name.chars().any(|c| c.is_whitespace()) {
            None
        } else {
            Some(KeyName(name))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a press on a region asks the shell to do.
///
/// Data, never a callback. A widget is a pure function and cannot be handed something to call;
/// it describes what it wants and the shell decides whether that is currently possible. The
/// variants past `Keys` and `System` are named now, before they have callers, because adding
/// one later means touching every `match` in the shell — and because naming them costs
/// nothing while guessing their shape mid-M4 costs a round.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub enum Action {
    /// Pressing this does nothing. Separators and decorations.
    #[default]
    None,
    /// Emit these keys via uinput, pressed in order and released in reverse. What a widget
    /// declares; the shell turns it into [`Action::KeysDown`] when the finger lands and
    /// [`Action::KeysUp`] when it lifts, so holding a media key repeats it the way a held
    /// keyboard key does (tiny-dfr's behaviour).
    Keys(Vec<KeyName>),
    /// Press these keys, in order, and leave them down. Only the shell produces this, on touch
    /// down, and always follows it with a matching [`Action::KeysUp`].
    KeysDown(Vec<KeyName>),
    /// Release keys a previous [`Action::KeysDown`] pressed, in reverse order.
    KeysUp(Vec<KeyName>),
    /// Something only the session can do (D11: exactly one variant, and that is the point).
    System(SystemAction),
    /// Raise and focus a window. M4.
    Activate(WindowId),
    /// Close, minimise or maximise a window: the Dock's window controls.
    Window(WindowId, WindowCommand),
    /// Make this the user's list of pinned apps: the Dock's pin, which computes the whole new
    /// list so nobody downstream has to know what was pinned before.
    SetPins(Vec<AppId>),
    /// Launch an application. M4/M6.
    Launch(AppId),
    /// Open or close the row of window entries for an app with several windows (D7). Consumed
    /// by the shell, which holds that state; it never reaches the binary.
    ToggleApp(AppId),
    /// Open a folder in the user's file manager, by the path config gave it. M5's folders
    /// widget.
    ///
    /// The renderer carries this string from the config file to `CoreToAgent::OpenPath` without
    /// ever looking at it: it does not check that the path exists, does not expand it, and
    /// certainly does not open it — as `nobody` it could not, and that is the point. Only the
    /// agent, running as the user, can act on it.
    OpenPath(String),
}

/// A rectangle a press can land in.
#[derive(Clone, PartialEq, Debug)]
pub struct HitRegion {
    pub id: RegionId,
    pub rect: Rect,
    pub action: Action,
}
