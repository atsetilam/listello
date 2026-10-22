//! Draw commands in, pixels out.
//!
//! D10 makes rendering backend-agnostic: layout and widget code talks to a [`Surface`] and must
//! never be able to tell which implementation is underneath it. [`DrmSurface`] drives the real
//! Touch Bar; the simulator draws a Touch Bar-shaped window on a desktop, so the strip can be seen
//! without a Touch Bar; [`PixelSurface`] renders to memory, which is how the golden-image
//! harness — the gate for all rendering work — sees anything at all.
//!
//! # Where the line falls
//!
//! The Surface line is the **pixel** line, not the draw-command line:
//!
//! ```text
//!   widgets ──DrawCmd──► layout ──Frame──► Surface ──► DRM / window / PNG
//!                                            │
//!                                    one shared cairo rasteriser
//! ```
//!
//! Widgets emit [`DrawCmd`]s (D3: pure functions, state in, draw commands out). A [`Frame`] is
//! a complete command list plus the [`Damage`] regions it changed. Every backend rasterises it
//! with the *same* code ([`raster`], via [`Canvas`]) and differs only in where the finished
//! bytes go.
//!
//! That split is deliberate and it is what makes golden images worth committing: two
//! rasterisers would drift, and a PNG captured on macOS would then prove nothing about
//! what the panel shows. Cairo is named in this crate on purpose and named nowhere above it.
//!
//! # Rules
//!
//! - **No DRM anywhere above this crate.** That was tiny-dfr's shape — `FunctionLayer::draw()`
//!   took a `cairo::Surface` and returned `Vec<drm::control::ClipRect>` — and closing it is
//!   what M1 step 1 means.
//! - **Logical coordinates are landscape**, origin top-left, one unit per panel pixel. The
//!   panel's portrait DRM mode and its 90° software rotation live in [`Rotation`], below the
//!   line. Nothing above it may know they exist.
//! - **The buffer may be bigger than the panel.** The Touch Bar's dumb buffer is allocated 64 px
//!   wide while the visible mode is narrower. [`Canvas`] takes both sizes; only the logical one
//!   is ever visible through [`Surface::size`], so golden images are in logical space and stay
//!   hardware-independent.
//! - **Never hardcode the resolution.** [`DrmSurface`] reads it from the mode; every other
//!   backend is told it (from config, defaulting to the reference panel — a figure that is
//!   still unmeasured until M2).
//! - **Nothing here panics on bad input.** This crate is linked into a daemon.

pub mod canvas;
pub mod draw;
pub mod geom;
pub mod golden;
pub mod packaged;
pub mod pixel;
pub mod raster;

pub use canvas::{Canvas, DeviceRect, Rotation};
pub use draw::{Align, Color, Damage, DrawCmd, Frame, TextStyle};
pub use geom::{Point, Rect, Size};
pub use pixel::PixelSurface;
pub use raster::{image_from_rgba, measure_text, TextMetrics};

// The simulator's window stack, off by default. `listello-sim` turns it on; `listello-core`
// never does, and `cargo tree -p listello-core` is what proves it.
#[cfg(feature = "simulator")]
mod simulator;
#[cfg(feature = "simulator")]
pub use simulator::SimulatorSurface;

#[cfg(target_os = "linux")]
mod drm_backend;
#[cfg(target_os = "linux")]
mod drm_surface;
#[cfg(target_os = "linux")]
pub use drm_surface::DrmSurface;

use anyhow::Result;

/// Somewhere pixels can go.
///
/// Deliberately two methods. Everything a backend actually differs in — how big it is, and what
/// it does with a finished frame — is here; everything they share is in [`Canvas`]. Resist
/// adding a third: any method that only one implementation can answer usefully is a D10
/// violation waiting to be discovered by whoever ports this to a second machine.
pub trait Surface {
    /// The drawable area in logical pixels: landscape, origin top-left. This is the geometry
    /// layout divides up, and the only geometry a widget ever sees.
    fn size(&self) -> Size;

    /// Rasterise `frame` and put it where this surface puts things.
    ///
    /// The command list is executed in full and clipped to `frame.damage`; presenting a
    /// partial *command list* is always a bug (see [`Frame`]). A frame with empty damage is a
    /// no-op, not an error.
    fn present(&mut self, frame: &Frame) -> Result<()>;
}
