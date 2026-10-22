//! Render a Touch Bar-shaped strip to a PNG, twice: once through the unrotated
//! [`PixelSurface`] the golden-image harness uses, and once through a `Canvas` shaped exactly
//! like the real panel's rotated, over-wide framebuffer.
//!
//! This is the crate's own smoke test and the first thing in listello anyone can actually look
//! at. It proves the two things that are expensive to get wrong: that the rasteriser draws what
//! the commands say, and that [`Rotation::Cw90`] puts it on the panel the right way up.
//!
//! ```sh
//! cargo run -p listello-surface --example strip
//! ```

use anyhow::Result;
use cairo::{FontFace, FontSlant, FontWeight};
use listello_surface::{
    canvas::Rotation, Align, Canvas, Color, DrawCmd, Frame, PixelSurface, Rect, Size, Surface,
    TextStyle,
};
use std::rc::Rc;

/// Provisional, and it must stay provisional: the real figures come back from M2, and
/// hardcoding a resolution anywhere that matters is forbidden (RESEARCH §1). This is an
/// example, so it is allowed to guess.
const PANEL: Size = Size::new(2008, 60);
/// The dumb buffer is allocated wider than the visible panel.
const PANEL_BUFFER: Size = Size::new(64, 2008);

fn demo_frame(size: Size, style: &TextStyle) -> Frame {
    let (w, h) = (size.width as f64, size.height as f64);
    let mut cmds = vec![DrawCmd::Clear(Color::BLACK)];

    // tiny-dfr's proportions: buttons occupy the middle 70% of the strip's height.
    let (top, bottom) = (h * 0.15, h * 0.85);
    let radius = 8.0;
    let labels = ["esc", "F1", "F2", "F3", "F4", "F5", "F6"];
    let spacing = 16.0;
    let slot = (w * 0.55 - spacing * (labels.len() - 1) as f64) / labels.len() as f64;

    for (i, label) in labels.iter().enumerate() {
        let x = i as f64 * (slot + spacing);
        let rect = Rect::new(x, top - radius, slot, bottom - top + radius * 2.0);
        cmds.push(DrawCmd::RoundedRect {
            rect,
            radius,
            // One button lit, to show the active state.
            color: Color::grey(if i == 2 { 0.400 } else { 0.200 }),
        });
        cmds.push(DrawCmd::Text {
            rect,
            text: label.to_string(),
            style: style.clone(),
            color: Color::WHITE,
            align: Align::Center,
        });
    }

    // A separator and a right-hand status group: M5's shape, drawn with nothing but the
    // primitives the IR already has.
    cmds.push(DrawCmd::Rect {
        rect: Rect::new(w * 0.60, h * 0.25, 2.0, h * 0.5),
        color: Color::grey(0.35),
    });
    cmds.push(DrawCmd::Text {
        rect: Rect::new(w * 0.62, 0.0, w * 0.36, h),
        text: "listello — 12:34  Mon 3 Aug".to_string(),
        style: style.clone(),
        color: Color::WHITE,
        align: Align::End,
    });

    // A clipped region, which is how the Dock's scrollable strip will be expressed (M4).
    cmds.push(DrawCmd::PushClip(Rect::new(w * 0.62, 0.0, 40.0, h)));
    cmds.push(DrawCmd::Rect {
        rect: Rect::new(w * 0.62, h * 0.2, 200.0, h * 0.6),
        color: Color::rgb(0.15, 0.45, 0.75),
    });
    cmds.push(DrawCmd::PopClip);

    Frame::full(cmds)
}

fn main() -> Result<()> {
    // `Rc::new`, never `FontFace::clone` — see TextStyle's documentation.
    let style = TextStyle::new(
        Rc::new(FontFace::toy_create(
            "sans-serif",
            FontSlant::Normal,
            FontWeight::Normal,
        )?),
        32.0,
    );

    let mut pixels = PixelSurface::new(PANEL)?;
    pixels.present(&demo_frame(PANEL, &style))?;
    pixels.write_png("strip.png")?;
    println!(
        "wrote strip.png  ({} × {}, logical)",
        PANEL.width, PANEL.height
    );

    // The same frame through the panel's real geometry. The output is portrait and 64 px wide,
    // which is what the Touch Bar's framebuffer actually is.
    let mut canvas = Canvas::new(PANEL, PANEL_BUFFER, Rotation::Cw90)?;
    let damage = canvas.paint(&demo_frame(PANEL, &style))?;
    let mut file = std::fs::File::create("strip-panel.png")?;
    canvas.image().write_to_png(&mut file)?;
    println!(
        "wrote strip-panel.png ({} × {}, as the panel sees it; {} damage rect(s))",
        PANEL_BUFFER.width,
        PANEL_BUFFER.height,
        damage.len()
    );
    Ok(())
}
