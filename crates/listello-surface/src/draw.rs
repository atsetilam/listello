//! The draw-command IR: what a widget returns, and the only thing a [`Surface`] accepts.
//!
//! [`Surface`]: crate::Surface
//!
//! D3 says widgets are pure functions — immutable state and geometry in, draw commands and hit
//! regions out. This module is the "draw commands" half. It is deliberately small and boring:
//! every command is plain data in logical coordinates, so a widget can be tested by asserting
//! on the list it produced without rasterising anything, and a new widget can be written
//! without ever touching cairo.
//!
//! Two rules that keep it that way:
//!
//! 1. **No paths, no handles to live resources, no I/O.** [`DrawCmd::Image`] carries decoded
//!    pixels, never a filename — the same rule D11 imposes on the IPC boundary, for the same
//!    reason. SVGs are rasterised to an image *above* this line, at load time.
//! 2. **No backend can be distinguished from here.** If a command only makes sense on DRM, or
//!    only in the simulator, it does not belong in this enum.
//!
//! The list is flat and executed in order, with [`DrawCmd::PushClip`]/[`DrawCmd::PopClip`]
//! bracketing nested regions. A tree would be prettier and would buy nothing.

use crate::geom::Rect;
use cairo::{FontFace, ImageSurface};
use std::rc::Rc;

/// Straight non-premultiplied RGBA, each channel in `0.0..=1.0`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Color {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

impl Color {
    pub const BLACK: Color = Color::rgb(0.0, 0.0, 0.0);
    pub const WHITE: Color = Color::rgb(1.0, 1.0, 1.0);
    pub const TRANSPARENT: Color = Color::rgba(0.0, 0.0, 0.0, 0.0);

    pub const fn rgb(r: f64, g: f64, b: f64) -> Color {
        Color { r, g, b, a: 1.0 }
    }

    pub const fn rgba(r: f64, g: f64, b: f64, a: f64) -> Color {
        Color { r, g, b, a }
    }

    /// The grey ramp the Touch Bar is mostly made of.
    pub const fn grey(v: f64) -> Color {
        Color::rgb(v, v, v)
    }

    pub fn with_alpha(self, a: f64) -> Color {
        Color { a, ..self }
    }
}

/// How text is placed horizontally within the rectangle it is given.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Align {
    Start,
    #[default]
    Center,
    End,
}

/// A font and a size.
///
/// The face comes from fontconfig (see `listello-core`'s `fonts` module); this crate neither
/// resolves nor owns fonts, it only draws with the one it is handed.
///
/// # The `Rc` is load-bearing. Do not remove it.
///
/// `cairo::FontFace` looks like a reference-counted handle and is not one, in the feature set
/// this project uses. Built without cairo-rs's `use_glib` feature — which is how tiny-dfr
/// configured it and what we inherited — `impl Clone for FontFace` is
/// `FontFace::from_raw_none(self.to_raw_none())`, which copies the pointer **without** calling
/// `cairo_font_face_reference`, while `Drop` still calls `cairo_font_face_destroy`. Cloning a
/// face and dropping both halves therefore over-releases it, and cairo aborts the process:
///
/// ```text
/// Assertion failed: (CAIRO_REFERENCE_COUNT_HAS_REFERENCE (&font_face->ref_count)),
/// function cairo_font_face_destroy, file cairo-font-face.c, line 176
/// ```
///
/// `[verified — reproduced on cairo-rs 0.22.0, macOS, 2026-08-03]`. tiny-dfr never hit it
/// because it stored exactly one face and only ever passed `&FontFace`. Draw commands are
/// owned data, so listello has to clone; wrapping in `Rc` means the clone is ours and cairo
/// only ever sees one release. The loop is single-threaded (D3), so `Rc` and not `Arc`.
#[derive(Clone, Debug)]
pub struct TextStyle {
    pub font: Rc<FontFace>,
    pub size: f64,
}

impl TextStyle {
    pub fn new(font: Rc<FontFace>, size: f64) -> TextStyle {
        TextStyle { font, size }
    }
}

/// One drawing operation, in logical coordinates.
#[derive(Clone, Debug)]
pub enum DrawCmd {
    /// Fill the entire current clip region. The first command of a frame is normally
    /// `Clear(Color::BLACK)`: under a damage clip it repaints exactly the background that the
    /// damaged widgets sat on, which is why partial redraws need no special case anywhere.
    Clear(Color),

    /// A filled rectangle. Separators and dividers are this, not a line primitive.
    Rect { rect: Rect, color: Color },

    /// A filled rectangle with rounded corners — the Touch Bar's button shape.
    /// `radius` is clamped to half the shorter side.
    RoundedRect {
        rect: Rect,
        radius: f64,
        color: Color,
    },

    /// Text, placed inside `rect`: horizontally per `align`, and vertically centred on the
    /// font's own ascent/descent rather than on the ink extents of this particular string.
    ///
    /// That last detail is deliberate and differs from tiny-dfr. Centring on ink extents makes
    /// a clock jump vertically between `11:11` and `12:00`, because the two strings have
    /// different ink heights. Font extents do not depend on the content, so the baseline holds
    /// still across a redraw.
    Text {
        rect: Rect,
        text: String,
        style: TextStyle,
        color: Color,
        align: Align,
    },

    /// Decoded pixels, scaled to fill `rect`. To draw an icon at its natural size, pass
    /// `Rect::center(w, h)` of the slot rather than adding a fit mode here.
    ///
    /// `alpha` multiplies the image's own alpha; M6's launch bounce wants it. `angle` turns the
    /// image about the centre of `rect`, in radians, clockwise; the Dock's wiggle wants it, and
    /// everything else passes `0.0`.
    Image {
        rect: Rect,
        image: ImageSurface,
        alpha: f64,
        angle: f64,
    },

    /// Intersect the clip with `rect` until the matching [`DrawCmd::PopClip`]. The Dock's
    /// scrollable strip (M4/M5) is the reason this exists.
    PushClip(Rect),

    /// Undo the most recent [`DrawCmd::PushClip`]. An unmatched pop is ignored rather than
    /// fatal — a widget bug must not take the daemon down.
    PopClip,
}

/// Which part of the surface a frame actually changed.
#[derive(Clone, Debug, Default)]
pub enum Damage {
    /// Everything. The correct answer whenever it is not obviously wrong: the panel is about
    /// half a megapixel and a full repaint is cheap.
    #[default]
    Full,
    /// Only these regions changed. Anything outside them keeps the previous frame's pixels,
    /// so the command list must still be complete — it is clipped, not truncated.
    Regions(Vec<Rect>),
}

impl Damage {
    /// Build damage from an iterator of changed rectangles, collapsing to [`Damage::Full`]
    /// when there is nothing to gain from being precise.
    pub fn from_rects(rects: impl IntoIterator<Item = Rect>) -> Damage {
        let rects: Vec<Rect> = rects.into_iter().filter(|r| !r.is_empty()).collect();
        if rects.is_empty() {
            // No change at all is expressible as an empty region list, and callers that mean
            // "nothing happened" should simply not present a frame.
            Damage::Regions(Vec::new())
        } else {
            Damage::Regions(rects)
        }
    }

    pub fn is_nothing(&self) -> bool {
        matches!(self, Damage::Regions(r) if r.is_empty())
    }
}

/// One frame: the complete command list, plus which regions of it are new.
///
/// The command list is always complete. Presenting a partial frame by trimming commands is a
/// bug that shows up as stale pixels weeks later; trimming *damage* is safe and is how partial
/// redraws are expressed.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub commands: Vec<DrawCmd>,
    pub damage: Damage,
}

impl Frame {
    pub fn new(commands: Vec<DrawCmd>, damage: Damage) -> Frame {
        Frame { commands, damage }
    }

    /// A frame that repaints everything.
    pub fn full(commands: Vec<DrawCmd>) -> Frame {
        Frame {
            commands,
            damage: Damage::Full,
        }
    }
}
