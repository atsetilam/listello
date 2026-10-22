//! [`Surface`] on the real Touch Bar.
//!
//! New code. The DRM mechanics underneath it are tiny-dfr's, retained unchanged in
//! [`crate::drm_backend`]; this is the wrapper D10 asks for and RESEARCH §2.1 predicted would
//! be "a trait wrapper, nothing more".
//!
//! The two geometry facts it exists to contain:
//!
//! - the panel reports a **portrait** mode, so logical landscape is the mode transposed and
//!   drawn through [`Rotation::Cw90`];
//! - the dumb buffer is **wider than the panel** (64 px against a narrower visible mode), so
//!   the cairo buffer must match `fb_info()`, not the mode, or the copy into the dumb buffer
//!   is misaligned and every golden image built on mode geometry drifts.

use crate::canvas::{Canvas, Rotation};
use crate::draw::Frame;
use crate::drm_backend::DrmBackend;
use crate::geom::Size;
use crate::Surface;
use anyhow::Result;
use drm::control::ClipRect;

pub struct DrmSurface {
    backend: DrmBackend,
    canvas: Canvas,
}

impl DrmSurface {
    /// Find the Touch Bar, take it over, and set up a buffer that matches it.
    ///
    /// Both sizes come from the hardware. Nothing here is a constant — the geometry differs
    /// between the T2 and Apple Silicon panels, and between models (RESEARCH §1).
    pub fn open() -> Result<DrmSurface> {
        let backend = DrmBackend::open_card()?;

        // DRM reports the panel portrait: mode `(width, height)` is `(short, long)`. Logical
        // coordinates are landscape, so the two are transposed.
        let (mode_w, mode_h) = backend.mode().size();
        let logical = Size::new(mode_h as u32, mode_w as u32);

        // The framebuffer, on the other hand, is whatever was actually allocated — over-wide.
        let (fb_w, fb_h) = backend.fb_info()?.size();
        let buffer = Size::new(fb_w, fb_h);

        let canvas = Canvas::new(logical, buffer, Rotation::Cw90)?;
        Ok(DrmSurface { backend, canvas })
    }

    /// Paint raw ARGB32 bytes straight into the dumb buffer, in **device** order, and flip the
    /// whole panel.
    ///
    /// This deliberately bypasses everything above it, because it exists for the case where
    /// everything above it has panicked: tiny-dfr's crash bitmap. Do not reach for it for
    /// anything else — it is the one place in the project that knows the buffer's real
    /// orientation, and it is not on the [`Surface`] trait for that reason.
    pub fn blit_device_argb(&mut self, data: &[u8]) -> Result<()> {
        {
            let mut map = self.backend.map()?;
            let dst = map.as_mut();
            let n = data.len().min(dst.len());
            dst[..n].copy_from_slice(&data[..n]);
        }
        let buffer = self.canvas.buffer_size();
        self.backend.dirty(&[ClipRect::new(
            0,
            0,
            buffer.width as u16,
            buffer.height as u16,
        )])?;
        Ok(())
    }
}

impl Surface for DrmSurface {
    fn size(&self) -> Size {
        self.canvas.logical_size()
    }

    fn present(&mut self, frame: &Frame) -> Result<()> {
        let damage = self.canvas.paint(frame)?;
        if damage.is_empty() {
            return Ok(());
        }

        // Copy the whole buffer regardless of damage, as tiny-dfr does. It is roughly half a
        // megabyte and the damage rectangles are what actually limit the panel update; copying
        // only the damaged rows would mean walking them under the rotation for no measurable
        // gain.
        {
            let src = self.canvas.data()?;
            let mut map = self.backend.map()?;
            let dst = map.as_mut();
            let n = src.len().min(dst.len());
            dst[..n].copy_from_slice(&src[..n]);
        }

        let clips: Vec<ClipRect> = damage
            .iter()
            .map(|r| ClipRect::new(r.x1 as u16, r.y1 as u16, r.x2 as u16, r.y2 as u16))
            .collect();
        self.backend.dirty(&clips)?;
        Ok(())
    }
}
