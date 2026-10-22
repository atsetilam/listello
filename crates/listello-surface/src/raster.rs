//! The one and only rasteriser: [`DrawCmd`]s onto a cairo context.
//!
//! Every backend shares this. That is not a convenience, it is the whole reason golden images
//! mean anything: if the simulator and the DRM panel each had their own interpretation of a
//! rounded rectangle, a golden image captured on macOS would prove nothing about what
//! the Touch Bar shows. One rasteriser, one answer.
//!
//! Cairo appearing here is not a D10 violation. D10 forbids *DRM* specifics above the Surface
//! line; cairo is the shared software rasteriser that lives *below* it, and no widget, layout
//! or config code ever names it.

use crate::draw::{Align, DrawCmd, TextStyle};
use crate::geom::Rect;
use anyhow::Result;
use cairo::{Context, Format, ImageSurface, Operator};

/// What a string will occupy when drawn in a given style.
///
/// `width`/`height` are ink extents — the pixels actually marked. `advance` is how far the pen
/// moves, which is what you want when placing something *after* the text. `ascent`/`descent`
/// come from the font rather than the string, so they do not change with the content.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct TextMetrics {
    pub width: f64,
    pub height: f64,
    pub advance: f64,
    pub ascent: f64,
    pub descent: f64,
}

thread_local! {
    /// A 1×1 scratch context, so measuring text does not need a real surface. The loop is
    /// single-threaded (D3), so one per thread is one, and it is created lazily because
    /// `listello-core` measures nothing until it has a config.
    static SCRATCH: Option<Context> = ImageSurface::create(Format::ARgb32, 1, 1)
        .ok()
        .and_then(|s| Context::new(&s).ok());
}

/// Decoded RGBA8 pixels — an [`crate::DrawCmd::Image`]'s payload — into a cairo surface.
///
/// This lives here, below the Surface line, because it is entirely a cairo detail: cairo wants
/// **premultiplied ARGB32 in native byte order** with a stride it chooses itself, and the wire
/// format deliberately does not encode any of that (`listello_protocol::Icon` is straight RGBA8,
/// and its doc comment says the conversion is the renderer's side of the boundary). Doing it
/// once, here, is what keeps `listello-ui` from learning what premultiplication is.
///
/// `None` rather than an error for every bad input — a wrong byte count, a zero dimension, a
/// size cairo refuses, a stride it will not lend us. Icons arrive from the user's disk by way of
/// another process and a daemon must not die because one of them was a lie.
pub fn image_from_rgba(width: u32, height: u32, rgba: &[u8]) -> Option<ImageSurface> {
    let (w, h) = (width as usize, height as usize);
    let expected = w.checked_mul(h)?.checked_mul(4)?;
    if expected == 0 || rgba.len() != expected {
        return None;
    }
    let mut surface = ImageSurface::create(Format::ARgb32, width as i32, height as i32).ok()?;
    let stride = surface.stride() as usize;
    {
        let mut data = surface.data().ok()?;
        for y in 0..h {
            let row = &mut data[y * stride..y * stride + w * 4];
            for x in 0..w {
                let src = &rgba[(y * w + x) * 4..][..4];
                let a = src[3] as u32;
                // Premultiply, rounding the way cairo's own reference conversion does.
                let mul = |c: u8| (((c as u32 * a) + 127) / 255) as u8;
                let (r, g, b) = (mul(src[0]), mul(src[1]), mul(src[2]));
                // ARgb32 is a native-endian u32 0xAARRGGBB, so the byte order in memory is
                // BGRA on little-endian and ARGB on big-endian. Write the u32, not the bytes.
                let px = (a << 24) | ((r as u32) << 16) | ((g as u32) << 8) | b as u32;
                row[x * 4..x * 4 + 4].copy_from_slice(&px.to_ne_bytes());
            }
        }
    }
    surface.mark_dirty();
    Some(surface)
}

/// Measure `text` in `style`.
///
/// Widgets need this to size themselves before they emit commands — that is why it lives above
/// the rasteriser rather than inside it. Returns zeroes rather than failing: a widget that
/// cannot measure should still draw something, and a daemon should not die because cairo could
/// not allocate a one-pixel surface.
pub fn measure_text(style: &TextStyle, text: &str) -> TextMetrics {
    SCRATCH.with(|c| {
        let Some(c) = c.as_ref() else {
            return TextMetrics::default();
        };
        c.set_font_face(&style.font);
        c.set_font_size(style.size);
        let (Ok(ink), Ok(font)) = (c.text_extents(text), c.font_extents()) else {
            return TextMetrics::default();
        };
        TextMetrics {
            width: ink.width(),
            height: ink.height(),
            advance: ink.x_advance(),
            ascent: font.ascent(),
            descent: font.descent(),
        }
    })
}

/// Execute `commands` onto `c`, in order.
///
/// The caller sets up the transform and the damage clip first; this function only draws. It
/// leaves the context's clip and source as it found them.
pub fn rasterize(c: &Context, commands: &[DrawCmd]) -> Result<()> {
    // Depth of the caller-visible clip stack, so an unbalanced PopClip cannot restore past the
    // state we were handed. A widget bug should look wrong, not take the daemon down.
    let mut clip_depth = 0usize;

    for cmd in commands {
        match cmd {
            DrawCmd::Clear(color) => {
                c.save()?;
                // Source, not Over: clearing to a translucent colour must replace what is
                // underneath, not blend with it.
                c.set_operator(Operator::Source);
                c.set_source_rgba(color.r, color.g, color.b, color.a);
                c.paint()?;
                c.restore()?;
            }

            DrawCmd::Rect { rect, color } => {
                if rect.is_empty() {
                    continue;
                }
                c.set_source_rgba(color.r, color.g, color.b, color.a);
                c.rectangle(rect.x, rect.y, rect.width, rect.height);
                c.fill()?;
            }

            DrawCmd::RoundedRect {
                rect,
                radius,
                color,
            } => {
                if rect.is_empty() {
                    continue;
                }
                c.set_source_rgba(color.r, color.g, color.b, color.a);
                rounded_rect_path(c, rect, *radius);
                c.fill()?;
            }

            DrawCmd::Text {
                rect,
                text,
                style,
                color,
                align,
            } => {
                if text.is_empty() {
                    continue;
                }
                c.set_font_face(&style.font);
                c.set_font_size(style.size);
                let ink = c.text_extents(text)?;
                let font = c.font_extents()?;

                // Horizontal: on ink, discounting the bearing, so the glyphs look centred
                // rather than the pen positions being centred.
                let x = match align {
                    Align::Start => rect.left() - ink.x_bearing(),
                    Align::Center => rect.center_x() - ink.width() / 2.0 - ink.x_bearing(),
                    Align::End => rect.right() - ink.width() - ink.x_bearing(),
                };
                // Vertical: on the font's own metrics, so the baseline does not move when the
                // string does. See DrawCmd::Text.
                let y = rect.center_y() + (font.ascent() - font.descent()) / 2.0;

                c.set_source_rgba(color.r, color.g, color.b, color.a);
                c.move_to(x.round(), y.round());
                c.show_text(text)?;
            }

            DrawCmd::Image {
                rect,
                image,
                alpha,
                angle,
            } => {
                if rect.is_empty() || image.width() == 0 || image.height() == 0 {
                    continue;
                }
                c.save()?;
                if angle.is_finite() && *angle != 0.0 {
                    // Turned about its centre; the clip below turns with it.
                    c.translate(rect.center_x(), rect.center_y());
                    c.rotate(*angle);
                    c.translate(-rect.center_x(), -rect.center_y());
                }
                c.rectangle(rect.x, rect.y, rect.width, rect.height);
                c.clip();
                c.translate(rect.x, rect.y);
                c.scale(
                    rect.width / image.width() as f64,
                    rect.height / image.height() as f64,
                );
                c.set_source_surface(image, 0.0, 0.0)?;
                if *alpha >= 1.0 {
                    c.paint()?;
                } else {
                    c.paint_with_alpha(alpha.clamp(0.0, 1.0))?;
                }
                c.restore()?;
            }

            DrawCmd::PushClip(rect) => {
                c.save()?;
                clip_depth += 1;
                c.rectangle(rect.x, rect.y, rect.width, rect.height);
                c.clip();
            }

            DrawCmd::PopClip => {
                if clip_depth > 0 {
                    clip_depth -= 1;
                    c.restore()?;
                }
            }
        }
    }

    // A widget that pushed and forgot to pop must not leak a clip into the next frame.
    for _ in 0..clip_depth {
        c.restore()?;
    }
    Ok(())
}

/// tiny-dfr's button outline, generalised: four corner arcs joined into one sub-path.
fn rounded_rect_path(c: &Context, rect: &Rect, radius: f64) {
    let r = radius.min(rect.width / 2.0).min(rect.height / 2.0).max(0.0);
    if r <= 0.0 {
        c.rectangle(rect.x, rect.y, rect.width, rect.height);
        return;
    }
    let (left, right) = (rect.left() + r, rect.right() - r);
    let (top, bottom) = (rect.top() + r, rect.bottom() - r);
    c.new_sub_path();
    c.arc(right, top, r, (-90.0f64).to_radians(), 0.0);
    c.arc(right, bottom, r, 0.0, (90.0f64).to_radians());
    c.arc(
        left,
        bottom,
        r,
        (90.0f64).to_radians(),
        (180.0f64).to_radians(),
    );
    c.arc(
        left,
        top,
        r,
        (180.0f64).to_radians(),
        (270.0f64).to_radians(),
    );
    c.close_path();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pixel at `(x, y)`, as cairo stores it: a native-endian `0xAARRGGBB`.
    fn pixel_at(surface: &mut ImageSurface, x: usize, y: usize) -> u32 {
        let stride = surface.stride() as usize;
        let data = surface.data().expect("surface data should be borrowable");
        let mut bytes = [0u8; 4];
        bytes.copy_from_slice(&data[y * stride + x * 4..][..4]);
        u32::from_ne_bytes(bytes)
    }

    #[test]
    fn opaque_pixels_survive_the_conversion_unchanged() {
        // Opaque red, green, blue, white — alpha 255 means premultiplication is the identity,
        // so any channel that comes back wrong is a byte-order bug, not a rounding one.
        let rgba = vec![
            255, 0, 0, 255, //
            0, 255, 0, 255, //
            0, 0, 255, 255, //
            255, 255, 255, 255,
        ];
        let mut image = image_from_rgba(4, 1, &rgba).expect("4x1 should convert");
        assert_eq!(pixel_at(&mut image, 0, 0), 0xffff0000);
        assert_eq!(pixel_at(&mut image, 1, 0), 0xff00ff00);
        assert_eq!(pixel_at(&mut image, 2, 0), 0xff0000ff);
        assert_eq!(pixel_at(&mut image, 3, 0), 0xffffffff);
    }

    #[test]
    fn alpha_is_premultiplied_because_cairo_requires_it() {
        // Half-transparent white: every colour channel must come back scaled by the alpha, or
        // cairo draws it far too bright.
        let mut image = image_from_rgba(1, 1, &[255, 255, 255, 128]).expect("1x1 should convert");
        assert_eq!(pixel_at(&mut image, 0, 0), 0x80808080);
    }

    #[test]
    fn a_fully_transparent_pixel_premultiplies_to_nothing() {
        let mut image = image_from_rgba(1, 1, &[255, 200, 100, 0]).expect("1x1 should convert");
        assert_eq!(pixel_at(&mut image, 0, 0), 0x00000000);
    }

    #[test]
    fn rows_land_on_their_own_stride() {
        // A 3px-wide surface has a 12-byte row but a stride cairo pads to its own alignment;
        // writing at `y * width * 4` instead of `y * stride` would shear the image.
        let mut rgba = vec![0u8; 3 * 2 * 4];
        rgba[3 * 4..][..4].copy_from_slice(&[255, 0, 0, 255]); // (0, 1) = red
        let mut image = image_from_rgba(3, 2, &rgba).expect("3x2 should convert");
        assert_eq!(pixel_at(&mut image, 0, 1), 0xffff0000);
        assert_eq!(pixel_at(&mut image, 0, 0), 0x00000000);
    }

    #[test]
    fn a_byte_count_that_contradicts_the_dimensions_is_refused() {
        assert!(image_from_rgba(64, 64, &[0xff; 16]).is_none());
        assert!(image_from_rgba(0, 0, &[]).is_none());
        assert!(image_from_rgba(1, 1, &[0xff; 3]).is_none());
    }

    #[test]
    fn an_absurd_size_is_refused_rather_than_attempted() {
        // u32::MAX squared overflows the byte count; a daemon must not turn that into an
        // allocation attempt, and certainly not into a panic.
        assert!(image_from_rgba(u32::MAX, u32::MAX, &[0xff; 4]).is_none());
    }
}
