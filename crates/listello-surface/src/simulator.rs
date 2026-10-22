//! A [`Surface`] that puts the panel's pixels in a desktop window.
//!
//! Before Linux is on the Mac (M2), this is the only way anything this project draws can be
//! looked at — which is why ROADMAP made the simulator M1 rather than a
//! convenience. It is also the third independent implementation of the trait, and the one that
//! keeps [`crate::DrmSurface`] honest: nothing above `listello-surface` may be able to tell the
//! two apart (D10).
//!
//! # The canvas is panel-sized, never window-sized
//!
//! One canvas pixel is one panel pixel, always, whatever the desktop decided the window should
//! be. That is the property that makes what the window shows evidence about what the panel will
//! show, and it is the same reason D10 puts the Surface line at the pixel line. The window is
//! reconciled with the canvas at blit time by nearest-neighbour scaling — blocky on purpose:
//! a filtered simulator would hide exactly the single-pixel differences it exists to reveal.
//!
//! # Physical pixels throughout
//!
//! `Window::inner_size()` is in *physical* pixels — on a Retina Mac a nominally 800 × 600
//! window measures 1600 × 1200. Everything here is physical for that reason, and
//! `scale_factor()` is deliberately never consulted: used consistently, it cancels out, and
//! the one place it would have to appear is the place a bug would hide.

use crate::canvas::Canvas;
use crate::draw::Frame;
use crate::geom::Size;
use crate::Surface;
use anyhow::{anyhow, Result};
use std::num::NonZeroU32;
use std::rc::Rc;
use winit::window::Window;

/// A Touch Bar-shaped window.
pub struct SimulatorSurface {
    canvas: Canvas,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    window: Rc<Window>,
}

impl SimulatorSurface {
    /// `panel` is the logical Touch Bar geometry — what [`crate::DrmSurface`] would report.
    /// The window is whatever size the desktop gives it; the two are reconciled at blit time,
    /// so a compositor that clamps the requested width is a non-event.
    ///
    /// The window must already exist: winit creates it asynchronously in
    /// `ApplicationHandler::resumed`, so a caller that builds this before the first pump has
    /// returned has nothing to pass.
    pub fn new(window: Rc<Window>, panel: Size) -> Result<SimulatorSurface> {
        let context = softbuffer::Context::new(Rc::clone(&window))
            .map_err(|e| anyhow!("softbuffer context: {e}"))?;
        let surface = softbuffer::Surface::new(&context, Rc::clone(&window))
            .map_err(|e| anyhow!("softbuffer surface: {e}"))?;
        Ok(SimulatorSurface {
            canvas: Canvas::unrotated(panel)?,
            surface,
            window,
        })
    }

    /// The window's current size in physical pixels, which is what
    /// `listello-sim`'s input translation needs to map a cursor position onto the panel.
    pub fn window_size(&self) -> Size {
        let s = self.window.inner_size();
        Size::new(s.width, s.height)
    }

    /// Nearest-neighbour blit of the panel-sized canvas into a `dst` window buffer.
    ///
    /// Integer arithmetic, no filtering, every destination pixel clamped to a real source
    /// pixel. Split out from [`Surface::present`] so it can be reasoned about — and, later,
    /// tested — without a window.
    fn blit(src: &[u8], src_stride: usize, panel: Size, dst: &mut [u32], win: Size) {
        for dy in 0..win.height as usize {
            let sy = (dy as u64 * panel.height as u64 / win.height.max(1) as u64) as usize;
            let sy = sy.min(panel.height.saturating_sub(1) as usize);
            let src_row = sy * src_stride;
            let dst_row = dy * win.width as usize;
            for dx in 0..win.width as usize {
                let sx = (dx as u64 * panel.width as u64 / win.width.max(1) as u64) as usize;
                let sx = sx.min(panel.width.saturating_sub(1) as usize);
                let o = src_row + sx * 4;
                // Cairo's ARGB32 is host-order premultiplied; softbuffer wants 0x00RRGGBB.
                // The canvas is opaque — every frame starts with a Clear — so masking the
                // alpha away is enough and there is nothing to unpremultiply.
                let px = u32::from_ne_bytes([src[o], src[o + 1], src[o + 2], src[o + 3]]);
                dst[dst_row + dx] = px & 0x00FF_FFFF;
            }
        }
    }
}

impl Surface for SimulatorSurface {
    fn size(&self) -> Size {
        self.canvas.logical_size()
    }

    fn present(&mut self, frame: &Frame) -> Result<()> {
        self.canvas.paint(frame)?;

        // A zero-sized window is normal mid-resize, and `NonZeroU32` is not optional here.
        let inner = self.window.inner_size();
        let (w, h) = (inner.width.max(1), inner.height.max(1));
        let (nz_w, nz_h) = match (NonZeroU32::new(w), NonZeroU32::new(h)) {
            (Some(w), Some(h)) => (w, h),
            _ => return Ok(()),
        };
        self.surface
            .resize(nz_w, nz_h)
            .map_err(|e| anyhow!("softbuffer resize: {e}"))?;

        let panel = self.canvas.logical_size();
        let stride = self.canvas.image().stride() as usize;
        let mut buffer = self
            .surface
            .buffer_mut()
            .map_err(|e| anyhow!("softbuffer buffer: {e}"))?;
        {
            let data = self.canvas.data()?;
            SimulatorSurface::blit(&data, stride, panel, &mut buffer, Size::new(w, h));
        }
        buffer
            .present()
            .map_err(|e| anyhow!("softbuffer present: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a `panel`-sized ARGB32 buffer where each pixel encodes its own coordinates, so a
    /// misindexed blit produces a wrong *value* rather than merely a wrong-looking picture.
    fn source(panel: Size, stride: usize) -> Vec<u8> {
        let mut v = vec![0u8; stride * panel.height as usize];
        for y in 0..panel.height as usize {
            for x in 0..panel.width as usize {
                let px: u32 = 0xFF00_0000 | ((x as u32) << 8) | (y as u32);
                v[y * stride + x * 4..y * stride + x * 4 + 4].copy_from_slice(&px.to_ne_bytes());
            }
        }
        v
    }

    #[test]
    fn blit_is_identity_at_one_to_one() {
        let panel = Size::new(4, 3);
        // Deliberately not `width * 4`: cairo aligns rows and this is where that gets missed.
        let stride = 32;
        let src = source(panel, stride);
        let mut dst = vec![0u32; 12];
        SimulatorSurface::blit(&src, stride, panel, &mut dst, panel);
        assert_eq!(dst[0], 0x0000_0000);
        assert_eq!(dst[3], 0x0000_0300, "top-right is x=3, y=0");
        assert_eq!(dst[8], 0x0000_0002, "bottom-left is x=0, y=2");
        assert!(dst.iter().all(|p| p & 0xFF00_0000 == 0), "alpha masked off");
    }

    #[test]
    fn blit_upscales_by_repeating_and_never_reads_past_the_source() {
        let panel = Size::new(2, 2);
        let stride = 8;
        let src = source(panel, stride);
        let win = Size::new(5, 5); // not a multiple: the clamp is what stops an overrun
        let mut dst = vec![0u32; 25];
        SimulatorSurface::blit(&src, stride, panel, &mut dst, win);
        assert_eq!(dst[0], 0x0000_0000, "(0,0) -> src (0,0)");
        assert_eq!(dst[4], 0x0000_0100, "(4,0) -> clamped to src x=1");
        assert_eq!(dst[24], 0x0000_0101, "(4,4) -> clamped to src (1,1)");
    }
}
