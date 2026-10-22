//! A [`Surface`] that renders to memory and nothing else.
//!
//! This is the backend the golden-image harness runs on, and the only one that builds and runs
//! everywhere: macOS, a Linux desktop, and a headless CI box with no display attached. A widget
//! test is "render this widget against a fixed state snapshot into a `PixelSurface`, compare to
//! a committed PNG", and that has to work headless or rendering work cannot be gated at all
//! (D10).
//!
//! It is also the honest test of whether the abstraction holds: a third implementation that
//! shares no code with the other two except [`Canvas`].

use crate::canvas::Canvas;
use crate::draw::Frame;
use crate::geom::Size;
use crate::Surface;
use anyhow::Result;
use std::fs::File;
use std::path::Path;

/// An off-screen surface holding the last frame presented to it.
pub struct PixelSurface {
    canvas: Canvas,
}

impl PixelSurface {
    /// Unrotated, buffer size equal to logical size — golden images are in logical space, so
    /// the panel's portrait framebuffer is not reproduced here and does not need to be.
    pub fn new(size: Size) -> Result<PixelSurface> {
        Ok(PixelSurface {
            canvas: Canvas::unrotated(size)?,
        })
    }

    /// The accumulated pixels, for a caller that wants to compare them itself.
    pub fn canvas(&mut self) -> &mut Canvas {
        &mut self.canvas
    }

    /// Write the current contents as a PNG. Used to *record* a golden image; comparisons
    /// should be done on pixels, not on encoded bytes.
    pub fn write_png(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let mut file = File::create(path)?;
        self.canvas.image().write_to_png(&mut file)?;
        Ok(())
    }

    /// The current contents as PNG bytes, for a caller that shows them rather than stores
    /// them (the settings window's picture of the strip).
    pub fn png_bytes(&mut self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.canvas.image().write_to_png(&mut out)?;
        Ok(out)
    }

    /// The buffer as ARGB32 rows, `stride` bytes apart. Note `stride` is not necessarily
    /// `4 * width`: cairo aligns rows, and comparing whole buffers without accounting for that
    /// is a test that passes on one machine and fails on another.
    pub fn pixels(&mut self) -> Result<(Vec<u8>, usize)> {
        let stride = self.canvas.image().stride() as usize;
        let data = self.canvas.data()?;
        Ok((data.to_vec(), stride))
    }
}

impl Surface for PixelSurface {
    fn size(&self) -> Size {
        self.canvas.logical_size()
    }

    fn present(&mut self, frame: &Frame) -> Result<()> {
        self.canvas.paint(frame)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::{Color, DrawCmd};
    use crate::geom::Rect;

    #[test]
    fn clip_confines_a_clear_to_the_damaged_region() {
        // The property partial redraws rely on: a full command list, clipped to damage,
        // repaints the background under the changed widgets and touches nothing else.
        let mut s = PixelSurface::new(Size::new(8, 8)).unwrap();
        s.present(&Frame::full(vec![DrawCmd::Clear(Color::WHITE)]))
            .unwrap();
        s.present(&Frame::new(
            vec![DrawCmd::Clear(Color::BLACK)],
            crate::Damage::Regions(vec![Rect::new(0.0, 0.0, 4.0, 8.0)]),
        ))
        .unwrap();

        let stride = s.canvas().image().stride() as usize;
        let (data, _) = s.pixels().unwrap();
        let px = |x: usize, y: usize| -> u32 {
            let o = y * stride + x * 4;
            u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]])
        };
        assert_eq!(px(0, 0), 0xFF00_0000, "inside damage: repainted black");
        assert_eq!(px(7, 0), 0xFFFF_FFFF, "outside damage: untouched");
    }

    #[test]
    fn unbalanced_clip_does_not_leak_into_the_next_frame() {
        let mut s = PixelSurface::new(Size::new(8, 8)).unwrap();
        s.present(&Frame::full(vec![
            DrawCmd::PushClip(Rect::new(0.0, 0.0, 1.0, 1.0)),
            DrawCmd::Clear(Color::WHITE),
            // deliberately no PopClip
        ]))
        .unwrap();
        s.present(&Frame::full(vec![DrawCmd::Clear(Color::WHITE)]))
            .unwrap();

        let stride = s.canvas().image().stride() as usize;
        let (data, _) = s.pixels().unwrap();
        let o = 7 * stride + 7 * 4;
        assert_eq!(
            u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]),
            0xFFFF_FFFF
        );
    }
}
