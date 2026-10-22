//! Proves the golden-image harness end to end: a fixed draw-command list, rendered into a
//! `PixelSurface`, compared against a PNG committed at `tests/golden/strip.png`.
//!
//! This is the harness rendering work is gated on (ROADMAP M1) — not a widget test, since the `Widget` trait doesn't exist yet. It exists so that the first
//! real widget test only has to add a draw-command list and a PNG, not reinvent comparison.
//!
//! To regenerate `strip.png` after a deliberate rendering change:
//!
//! ```text
//! LISTELLO_GOLDEN_UPDATE=1 cargo test -p listello-surface --test golden_harness
//! ```

use listello_surface::draw::{Color, DrawCmd, Frame};
use listello_surface::geom::{Rect, Size};
use listello_surface::golden::assert_matches;
use listello_surface::pixel::PixelSurface;
use listello_surface::Surface;

fn strip_frame() -> Frame {
    Frame::full(vec![
        DrawCmd::Clear(Color::BLACK),
        DrawCmd::RoundedRect {
            rect: Rect::new(4.0, 4.0, 24.0, 24.0),
            radius: 4.0,
            color: Color::grey(0.5),
        },
        DrawCmd::Rect {
            rect: Rect::new(30.0, 0.0, 2.0, 32.0),
            color: Color::WHITE,
        },
    ])
}

#[test]
fn strip_matches_the_committed_golden_image() {
    let mut surface = PixelSurface::new(Size::new(64, 32)).unwrap();
    surface.present(&strip_frame()).unwrap();

    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/strip.png");
    assert_matches(path, &mut surface).unwrap();
}
