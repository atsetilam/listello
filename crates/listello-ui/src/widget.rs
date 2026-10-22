//! The `Widget` trait: an immutable snapshot and a rectangle in, draw commands and hit regions
//! out (D3).

use crate::hit::{Action, HitRegion, RegionId};
use crate::state::{IconKey, Theme, UiState};
use cairo::ImageSurface;
use listello_surface::{DrawCmd, Rect};

/// How much of the strip a widget wants.
///
/// Two variants and no `Natural`/intrinsic size: measuring intrinsic width means measuring
/// text, which means a font, which means the widget reaching for something. A widget that
/// wants to be as wide as its label states a `Fixed` width its config gave it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Sizing {
    /// Exactly this many logical pixels. Negative values are clamped to zero by layout.
    Fixed(f64),
    /// A share of whatever is left after the fixed widgets, proportional to this weight.
    /// A weight of zero or less means the widget gets no space, not that it gets an error.
    Flex(f64),
}

/// What a widget is given.
pub struct Ctx<'a> {
    pub state: &'a UiState,
    pub theme: &'a Theme,
    /// This widget's index in the strip — the `widget` half of every [`RegionId`] it emits.
    /// Use [`Ctx::region`] rather than constructing ids by hand.
    pub index: u16,
}

impl Ctx<'_> {
    /// An id for this widget's `part`-th region.
    pub fn region(&self, part: u16) -> RegionId {
        RegionId::new(self.index, part)
    }

    /// Whether `part` of this widget is the region currently held down.
    pub fn is_pressed(&self, part: u16) -> bool {
        self.state.pressed == Some(self.region(part))
    }

    /// The pixels for an icon, if the binary has them.
    ///
    /// `None` is a **normal answer**, not an error: the packaged set may not contain the name
    /// config asked for, and an app icon does not exist until the agent replies. Draw the label
    /// instead, or nothing. Never `.expect()` here — this is external data twice over.
    pub fn icon(&self, key: &IconKey) -> Option<&ImageSurface> {
        self.state.icons.get(key)
    }

    /// This widget's horizontal scroll offset in logical pixels; `0.0` if it has never been
    /// scrolled.
    ///
    /// A widget cannot hold this itself (D3: pure function, no `&mut self`), so the shell holds
    /// it and hands it back here. The value is unclamped — it is whatever the shell has
    /// accumulated — and the widget clamps it against its own content width with
    /// [`crate::layout::scroll_offset_clamped`], because only the widget knows how wide its
    /// content is.
    pub fn scroll(&self) -> f64 {
        self.state
            .scroll
            .get(&self.index)
            .copied()
            .filter(|v| v.is_finite())
            .unwrap_or(0.0)
    }
}

/// What a widget produces.
#[derive(Clone, Debug, Default)]
pub struct Output {
    commands: Vec<DrawCmd>,
    hits: Vec<HitRegion>,
}

impl Output {
    pub fn new() -> Output {
        Output::default()
    }

    pub fn draw(&mut self, cmd: DrawCmd) {
        self.commands.push(cmd);
    }

    pub fn hit(&mut self, id: RegionId, rect: Rect, action: Action) {
        self.hits.push(HitRegion { id, rect, action });
    }

    pub fn commands(&self) -> &[DrawCmd] {
        &self.commands
    }

    pub fn hits(&self) -> &[HitRegion] {
        &self.hits
    }

    pub fn into_parts(self) -> (Vec<DrawCmd>, Vec<HitRegion>) {
        (self.commands, self.hits)
    }
}

/// A pure function from state and geometry to pixels-to-be and presses-to-come.
///
/// # What an implementation may not do
///
/// No I/O, no clock, no environment, no `&mut self`, no handle to anything. If a widget needs
/// to know something, it arrives in [`UiState`]; if it wants something to happen, it emits an
/// [`Action`] and the shell decides. Both `sizing` and `render` must be deterministic given
/// their inputs — that is what makes a widget testable by asserting on the command list, which
/// is how phases 05a and 05b gate.
///
/// # Why `render` appends rather than returns
///
/// Layout renders every widget in the strip into one `Output` per frame, because a frame's
/// command list is always complete (D10 / `draw.rs`) — partial redraws are expressed as
/// damage, never by trimming commands. Appending keeps that a single allocation.
pub trait Widget {
    /// How much horizontal space this widget wants. Called before `render`, and must not
    /// depend on anything `render` will be told.
    fn sizing(&self) -> Sizing;

    /// The width this widget's content needs right now, for a widget configured to fit it
    /// (a clock with no `width` or `flex`). Layout uses it in place of `sizing`'s answer, so
    /// the room the content does not need goes to the flexible widgets instead of sitting
    /// empty inside this one. `None`, the default, means `sizing` stands.
    fn fitted_width(&self, _ctx: &Ctx<'_>) -> Option<f64> {
        None
    }

    /// Whether dragging a finger across this widget scrolls it. Only the Dock does.
    fn scrollable(&self) -> bool {
        false
    }

    /// Apps this widget would like icons for even when no window is open for them -- the
    /// Dock's pinned list. The binary asks the agent for any it does not have.
    fn wanted_apps(&self) -> Vec<listello_protocol::AppId> {
        Vec::new()
    }

    /// Whether this widget wants the whole strip for itself right now, in place of every other
    /// widget. The Dock does while it shows one app's windows. The first widget that says so
    /// gets the strip, keeping its own index, so its regions stay the same ones.
    fn takes_whole_strip(&self, _state: &UiState) -> bool {
        false
    }

    /// Draw into `rect` and declare where presses land.
    ///
    /// `rect` is in logical coordinates (landscape, origin top-left, one unit per panel
    /// pixel) and is the widget's whole world: drawing outside it is a bug that layout will
    /// not clip for you unless the widget is inside a scroll region.
    fn render(&self, ctx: &Ctx<'_>, rect: Rect, out: &mut Output);
}
