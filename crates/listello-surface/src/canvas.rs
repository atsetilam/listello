//! The shared half of every backend: an in-memory buffer, the logical→device transform, and
//! damage bookkeeping.
//!
//! A [`Surface`](crate::Surface) implementation is then a thin thing: own a `Canvas`, call
//! [`Canvas::paint`], and get the resulting pixels wherever they need to go — a DRM dumb
//! buffer, a window, a PNG. Nothing about rotation or damage arithmetic is duplicated per
//! backend, because getting it subtly different per backend is exactly how golden images start
//! disagreeing with hardware.

use crate::draw::{Damage, Frame};
use crate::geom::{Rect, Size};
use crate::raster::rasterize;
use anyhow::Result;
use cairo::{Context, Format, ImageSurface, ImageSurfaceData};

/// How logical (landscape) coordinates sit in the device buffer.
///
/// This exists because the Touch Bar panel reports a **portrait** DRM mode — 60 × 2008 on the 16", 60 × 2170 on the 13"
/// — and tiny-dfr rotates into it in software. That rotation is a property of the hardware, so
/// D10 puts it strictly below the Surface line: layout and widgets only ever see a wide, short
/// strip.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Rotation {
    /// Device and logical coordinates coincide. The simulator and the golden-image harness.
    #[default]
    None,
    /// Logical landscape drawn into a portrait buffer, rotated a quarter turn clockwise:
    /// `translate(logical_height, 0)` then `rotate(90°)`, which is tiny-dfr's transform.
    /// Logical `(x, y)` lands at device `(logical_height - y, x)`.
    Cw90,
}

/// A rectangle in device (buffer) pixels, snapped outward, clamped to the buffer.
///
/// Above the Surface line nothing produces or consumes one of these; they exist so a backend
/// can hand hardware a damage list without re-deriving the transform.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DeviceRect {
    pub x1: u32,
    pub y1: u32,
    pub x2: u32,
    pub y2: u32,
}

impl DeviceRect {
    pub fn width(&self) -> u32 {
        self.x2.saturating_sub(self.x1)
    }
    pub fn height(&self) -> u32 {
        self.y2.saturating_sub(self.y1)
    }
    pub fn is_empty(&self) -> bool {
        self.width() == 0 || self.height() == 0
    }
}

/// An ARGB32 buffer plus everything needed to draw logical coordinates into it.
pub struct Canvas {
    image: ImageSurface,
    logical: Size,
    buffer: Size,
    rotation: Rotation,
}

impl Canvas {
    /// `logical` is what layout sees. `buffer` is the real allocation, which may be **larger**:
    /// on the Touch Bar the dumb buffer is 64 px wide while the visible panel is narrower, and
    /// the cairo surface has to match the framebuffer or the memcpy into it is misaligned. The
    /// extra pixels are never drawn to and stay black.
    pub fn new(logical: Size, buffer: Size, rotation: Rotation) -> Result<Canvas> {
        let image =
            ImageSurface::create(Format::ARgb32, buffer.width as i32, buffer.height as i32)?;
        Ok(Canvas {
            image,
            logical,
            buffer,
            rotation,
        })
    }

    /// A canvas whose buffer is exactly its logical size and needs no rotation.
    pub fn unrotated(logical: Size) -> Result<Canvas> {
        Canvas::new(logical, logical, Rotation::None)
    }

    pub fn logical_size(&self) -> Size {
        self.logical
    }

    pub fn buffer_size(&self) -> Size {
        self.buffer
    }

    pub fn rotation(&self) -> Rotation {
        self.rotation
    }

    /// Draw `frame`, returning the device-space regions that changed.
    ///
    /// The command list is executed in full and clipped to the damage; see [`Frame`]. An empty
    /// return means nothing was touched and the backend can skip presenting entirely.
    pub fn paint(&mut self, frame: &Frame) -> Result<Vec<DeviceRect>> {
        let logical_damage: Vec<Rect> = match &frame.damage {
            Damage::Full => vec![self.logical.to_rect()],
            Damage::Regions(rects) => rects
                .iter()
                .map(|r| r.snap_out().clamp_to(&self.logical.to_rect()))
                .filter(|r| !r.is_empty())
                .collect(),
        };
        if logical_damage.is_empty() {
            return Ok(Vec::new());
        }

        {
            let c = Context::new(&self.image)?;
            self.apply_transform(&c);
            for r in &logical_damage {
                c.rectangle(r.x, r.y, r.width, r.height);
            }
            c.clip();
            rasterize(&c, &frame.commands)?;
        }
        self.image.flush();

        Ok(logical_damage
            .iter()
            .map(|r| self.to_device(r))
            .filter(|r| !r.is_empty())
            .collect())
    }

    /// The raw ARGB32 buffer, for a backend to blit or encode. Borrows the canvas mutably
    /// because cairo hands out its pixel data exclusively.
    pub fn data(&mut self) -> Result<ImageSurfaceData<'_>> {
        Ok(self.image.data()?)
    }

    /// The underlying cairo surface, for backends that can hand it straight to a window
    /// system instead of copying bytes.
    pub fn image(&self) -> &ImageSurface {
        &self.image
    }

    fn apply_transform(&self, c: &Context) {
        match self.rotation {
            Rotation::None => {}
            Rotation::Cw90 => {
                c.translate(self.logical.height as f64, 0.0);
                c.rotate((90.0f64).to_radians());
            }
        }
    }

    /// Map a logical rect to device pixels, snapping outward and clamping to the buffer.
    fn to_device(&self, r: &Rect) -> DeviceRect {
        let r = r.snap_out();
        let (x1, y1, x2, y2) = match self.rotation {
            Rotation::None => (r.left(), r.top(), r.right(), r.bottom()),
            Rotation::Cw90 => {
                let h = self.logical.height as f64;
                (h - r.bottom(), r.left(), h - r.top(), r.right())
            }
        };
        let clamp = |v: f64, max: u32| v.max(0.0).min(max as f64) as u32;
        DeviceRect {
            x1: clamp(x1, self.buffer.width),
            y1: clamp(y1, self.buffer.height),
            x2: clamp(x2, self.buffer.width),
            y2: clamp(y2, self.buffer.height),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::{Color, DrawCmd};

    fn canvas_cw90() -> Canvas {
        // Shaped like the real panel: logical 2008 × 60, buffer 64 × 2008.
        Canvas::new(Size::new(2008, 60), Size::new(64, 2008), Rotation::Cw90).unwrap()
    }

    #[test]
    fn cw90_maps_the_whole_panel_onto_the_buffer() {
        let c = canvas_cw90();
        let d = c.to_device(&Size::new(2008, 60).to_rect());
        assert_eq!(
            d,
            DeviceRect {
                x1: 0,
                y1: 0,
                x2: 60,
                y2: 2008
            }
        );
    }

    /// tiny-dfr pushed damage as `(height - top - radius, left_edge, height - bot + radius,
    /// left_edge + width)`. Same numbers, derived rather than hand-written.
    #[test]
    fn cw90_matches_tiny_dfrs_damage_arithmetic() {
        let c = canvas_cw90();
        let (height, bot, top, radius) = (60.0, 9.0, 51.0, 8.0);
        let (left_edge, button_width) = (100.0, 120.0);
        let logical = Rect::new(
            left_edge,
            bot - radius,
            button_width,
            top - bot + radius * 2.0,
        );
        let d = c.to_device(&logical);
        assert_eq!(d.x1 as f64, height - top - radius);
        assert_eq!(d.x2 as f64, height - bot + radius);
        assert_eq!(d.y1 as f64, left_edge);
        assert_eq!(d.y2 as f64, left_edge + button_width);
    }

    #[test]
    fn no_damage_means_no_paint() {
        let mut c = canvas_cw90();
        let frame = Frame::new(
            vec![DrawCmd::Clear(Color::WHITE)],
            Damage::Regions(Vec::new()),
        );
        assert!(c.paint(&frame).unwrap().is_empty());
    }

    #[test]
    fn drawing_lands_where_the_rotation_says_it_does() {
        let mut c = Canvas::new(Size::new(8, 4), Size::new(4, 8), Rotation::Cw90).unwrap();
        let frame = Frame::full(vec![
            DrawCmd::Clear(Color::BLACK),
            // Logical top-left pixel -> device (logical_height - 1, 0) = (3, 0).
            DrawCmd::Rect {
                rect: Rect::new(0.0, 0.0, 1.0, 1.0),
                color: Color::WHITE,
            },
        ]);
        c.paint(&frame).unwrap();
        let stride = c.image().stride() as usize;
        let data = c.data().unwrap();
        let px = |x: usize, y: usize| -> u32 {
            let o = y * stride + x * 4;
            u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]])
        };
        assert_eq!(
            px(3, 0),
            0xFFFF_FFFF,
            "logical (0,0) should be device (3,0)"
        );
        assert_eq!(px(0, 0), 0xFF00_0000, "device (0,0) should still be black");
    }
}
