//! The arithmetic that divides the Touch Bar's strip: fixed and flexible widths, gaps, and a
//! scroll region that clips (D3, M4).
//!
//! The whole engine is [`solve_widths`]: one rectangle's worth of width split among widgets
//! that either want an exact number of pixels or a share of whatever is left. Everything else
//! here is bookkeeping around it — placing the solved widths ([`strip_rects`]), rendering a
//! strip end to end ([`render_strip`]), and the Dock's shape, a scrollable region whose
//! children sit at their natural width and clip to a viewport ([`render_scroll`]).
//!
//! Pure arithmetic over [`Rect`]s — no I/O, no cairo, no state. Every rule below is a
//! unit-testable fact about numbers, which is why this module can be delegated and gated by
//! tests that run on any machine.

use crate::state::{Theme, UiState};
use crate::widget::{Ctx, Output, Sizing, Widget};
use listello_surface::{DrawCmd, Rect};

/// How many widgets a strip renders. `Ctx::index` is a `u16`; anything past this index has no
/// id that fits the id space and is dropped rather than wrapped to an id it does not own.
pub(crate) const MAX_WIDGETS: usize = u16::MAX as usize;

/// A single width or weight, clamped to what layout may use: rule 1 (NaN and infinite are
/// `0.0`) and rule 2 (negative and zero are `0.0`) collapse into one check.
fn usable(v: f64) -> f64 {
    if v.is_finite() && v > 0.0 {
        v
    } else {
        0.0
    }
}

/// Solve one strip's widths: exact widths for [`Sizing::Fixed`], a share of the leftover for
/// [`Sizing::Flex`]. One width per input, in order, same length as the input.
///
/// The rules, in order:
///
/// 1. A non-finite value is `0.0` — a NaN reaching cairo is a corrupted frame in a daemon, and
///    the value can come from a user's TOML.
/// 2. A negative value is `0.0`; a flex weight of zero or less contributes no weight and
///    receives no width.
/// 3. An empty input is an empty result. No panic, no division.
/// 4. Gaps are reserved before anything else: one `gap.max(0.0)` between adjacent items and
///    none at the ends, so `0.0` for a single item.
/// 5. Fixed widths are honoured in full and never shrunk.
/// 6. Leftover is `available - gaps - fixed`, clamped to `0.0` if negative.
/// 7. Flex widths are `leftover * weight / total_weight`; a `total_weight <= 0.0` leaves the
///    leftover unused and the strip ends short, left-aligned.
///
/// **Over-subscription overflows; it does not shrink.** If the fixed widths and gaps exceed
/// `available`, every fixed widget keeps its width and the strip runs off the right-hand edge.
/// That is deliberate: shrinking silently hides a config mistake, whereas an obviously clipped
/// strip is a bug the user can see and report.
///
/// No rounding anywhere — sub-pixel coordinates are expected (`geom.rs`), and rounding here is
/// what makes a strip of nine buttons end two pixels short of the edge.
pub fn solve_widths(sizings: &[Sizing], available: f64, gap: f64) -> Vec<f64> {
    let n = sizings.len();
    if n == 0 {
        return Vec::new();
    }
    let gap = gap.max(0.0);
    let reserved_gaps = if n <= 1 { 0.0 } else { gap * (n - 1) as f64 };

    let mut fixed_sum = 0.0;
    let mut total_weight = 0.0;
    for &s in sizings {
        match s {
            Sizing::Fixed(v) => fixed_sum += usable(v),
            Sizing::Flex(v) => total_weight += usable(v),
        }
    }

    let leftover = (available - reserved_gaps - fixed_sum).max(0.0);

    let mut widths = Vec::with_capacity(n);
    for &s in sizings {
        let w = match s {
            Sizing::Fixed(v) => usable(v),
            Sizing::Flex(v) => {
                if total_weight > 0.0 {
                    leftover * usable(v) / total_weight
                } else {
                    0.0
                }
            }
        };
        widths.push(w);
    }
    widths
}

/// Position solved widths inside `bounds`: each rect gets `bounds.y` and `bounds.height`, and
/// `x` walks right by each width plus one gap.
///
/// The running `x` is accumulated by adding each returned width — never recomputed as
/// `bounds.x + i * (w + gap)`. With sub-pixel flex widths the second form drifts, and the drift
/// shows up as a one-pixel seam between two adjacent buttons a hundred frames later.
pub fn strip_rects(sizings: &[Sizing], bounds: Rect, gap: f64) -> Vec<Rect> {
    let gap = gap.max(0.0);
    let widths = solve_widths(sizings, bounds.width, gap);
    let mut rects = Vec::with_capacity(widths.len());
    let mut x = bounds.x;
    for w in widths {
        rects.push(Rect::new(x, bounds.y, w, bounds.height));
        x += w + gap;
    }
    rects
}

/// Where [`render_strip`] puts each widget: one rect per widget, in order.
pub fn strip_layout(
    widgets: &[&dyn Widget],
    state: &UiState,
    theme: &Theme,
    bounds: Rect,
    gap: f64,
) -> Vec<Rect> {
    let sizings: Vec<Sizing> = widgets
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let ctx = Ctx {
                state,
                theme,
                index: i.min(MAX_WIDGETS) as u16,
            };
            w.fitted_width(&ctx).map_or(w.sizing(), Sizing::Fixed)
        })
        .collect();
    strip_rects(&sizings, bounds, gap)
}

/// Render every widget in a strip, left to right, into one [`Output`].
///
/// `sizing()` is asked on each widget first, then one shared solve, then `render` with the
/// widget's rect and a [`Ctx`] whose `index` is its position in the slice. A widget whose rect
/// is empty is still rendered: skipping it here would make widget indices depend on geometry,
/// breaking `RegionId` stability across frames.
///
/// Past `MAX_WIDGETS` widgets there is no index that fits the id space; the rest are dropped
/// rather than wrapped to an id they do not own.
pub fn render_strip(
    widgets: &[&dyn Widget],
    state: &UiState,
    theme: &Theme,
    bounds: Rect,
    gap: f64,
) -> Output {
    let rects = strip_layout(widgets, state, theme, bounds, gap);
    let mut out = Output::new();
    let rendered = widgets.len().min(MAX_WIDGETS);
    for (i, w) in widgets.iter().enumerate().take(rendered) {
        let ctx = Ctx {
            state,
            theme,
            index: i as u16,
        };
        // Nothing a widget draws may leave its own slot: a fixed slot narrower than its content
        // clips instead of spilling over a neighbour or off the panel's edge. Widgets that can
        // shrink to fit (the battery) still should; this is the guarantee behind them.
        out.draw(DrawCmd::PushClip(rects[i]));
        w.render(&ctx, rects[i], &mut out);
        out.draw(DrawCmd::PopClip);
    }
    out
}

/// The scroll offset actually usable for content `content_width` wide inside a
/// `viewport_width` viewport: `0.0` for a non-finite or negative offset, `0.0` when the
/// content fits, otherwise the offset clamped so the content's right edge never passes the
/// viewport's right edge.
///
/// A non-finite content or viewport width also yields `0.0` — the offset is a shift fed
/// straight into drawing, and a NaN there is a corrupted frame.
pub fn scroll_offset_clamped(offset: f64, content_width: f64, viewport_width: f64) -> f64 {
    if !offset.is_finite() || offset <= 0.0 {
        return 0.0;
    }
    if !content_width.is_finite() || !viewport_width.is_finite() {
        return 0.0;
    }
    if content_width <= viewport_width {
        return 0.0;
    }
    offset.min(content_width - viewport_width)
}

/// Render a scrollable region — the Dock's shape (M4): children laid out at their natural
/// width and clipped to `bounds`.
///
/// Inside a scroll region a [`Sizing::Flex`] is treated as `Fixed(0.0)`: "a share of the
/// leftover" is meaningless when the content defines the width. Widths are solved against an
/// unbounded `available`, so `content_width` is simply the sum of the widths plus the gaps
/// between them, and every rect is shifted left by
/// [`scroll_offset_clamped`](scroll_offset_clamped). The output is one clip around the whole
/// shifted strip:
///
/// ```text
/// PushClip(bounds)
///   ... every child's commands, at shifted x ...
/// PopClip
/// ```
///
/// Hit regions are emitted at the shifted, unclipped rect, exactly as drawn. A child scrolled
/// entirely out of view still contributes a hit region that lies outside `bounds`; the shell
/// rejects it because the press was never inside `bounds` in the first place. Filtering here
/// would mean layout deciding what is pressable, and layout does not own that.
pub fn render_scroll(
    widgets: &[&dyn Widget],
    state: &UiState,
    theme: &Theme,
    bounds: Rect,
    gap: f64,
    offset: f64,
) -> Output {
    let gap = gap.max(0.0);
    let sizings: Vec<Sizing> = widgets
        .iter()
        .map(|w| match w.sizing() {
            Sizing::Flex(_) => Sizing::Fixed(0.0),
            s => s,
        })
        .collect();
    let widths = solve_widths(&sizings, f64::INFINITY, gap);
    let n = widths.len();
    let content_width = widths.iter().sum::<f64>() + gap * n.saturating_sub(1) as f64;
    let shift = -scroll_offset_clamped(offset, content_width, bounds.width);

    let mut out = Output::new();
    out.draw(DrawCmd::PushClip(bounds));
    let mut x = bounds.x + shift;
    let rendered = n.min(MAX_WIDGETS);
    for (i, w) in widgets.iter().enumerate().take(rendered) {
        let ctx = Ctx {
            state,
            theme,
            index: i as u16,
        };
        let rect = Rect::new(x, bounds.y, widths[i], bounds.height);
        w.render(&ctx, rect, &mut out);
        x += widths[i] + gap;
    }
    out.draw(DrawCmd::PopClip);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hit::Action;
    use listello_surface::{Color, TextStyle};
    use std::rc::Rc;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    /// A minimal `Widget` fixture: one filled rect and one hit region.
    struct Stub {
        sizing: Sizing,
    }

    impl Widget for Stub {
        fn sizing(&self) -> Sizing {
            self.sizing
        }

        fn render(&self, ctx: &Ctx<'_>, rect: Rect, out: &mut Output) {
            out.draw(DrawCmd::Rect {
                rect,
                color: Color::WHITE,
            });
            out.hit(ctx.region(0), rect, Action::None);
        }
    }

    /// A `UiState` and `Theme` the stub ignores but `render_*` requires. Building a `Theme`
    /// needs a cairo face; the toy API needs no fontconfig and works on the Mac, but can still
    /// fail, in which case the caller skips rather than unwraps.
    fn fixture() -> Option<(UiState, Theme)> {
        let font = cairo::FontFace::toy_create(
            "sans-serif",
            cairo::FontSlant::Normal,
            cairo::FontWeight::Normal,
        )
        .ok()?;
        let theme = Theme {
            text: TextStyle::new(Rc::new(font), 14.0),
            background: Color::BLACK,
            foreground: Color::WHITE,
            button: Color::grey(0.3),
            button_pressed: Color::grey(0.5),
            accent: Color::grey(0.8),
            corner_radius: 4.0,
        };
        let now = chrono::DateTime::parse_from_rfc3339("2026-08-03T14:05:00+02:00").ok()?;
        let state = UiState::new(now);
        Some((state, theme))
    }

    #[test]
    fn solve_widths_table() {
        struct Case {
            sizings: Vec<Sizing>,
            available: f64,
            gap: f64,
            expected: Vec<f64>,
        }
        let cases = [
            Case {
                sizings: vec![],
                available: 100.0,
                gap: 4.0,
                expected: vec![],
            },
            Case {
                sizings: vec![Sizing::Flex(1.0)],
                available: 100.0,
                gap: 8.0,
                expected: vec![100.0],
            },
            Case {
                sizings: vec![Sizing::Flex(1.0), Sizing::Flex(1.0)],
                available: 100.0,
                gap: 8.0,
                expected: vec![46.0, 46.0],
            },
            Case {
                sizings: vec![Sizing::Flex(1.0), Sizing::Flex(3.0)],
                available: 100.0,
                gap: 0.0,
                expected: vec![25.0, 75.0],
            },
            Case {
                sizings: vec![Sizing::Fixed(30.0), Sizing::Flex(1.0)],
                available: 100.0,
                gap: 10.0,
                expected: vec![30.0, 60.0],
            },
            Case {
                sizings: vec![Sizing::Fixed(-5.0), Sizing::Flex(1.0)],
                available: 100.0,
                gap: 0.0,
                expected: vec![0.0, 100.0],
            },
            Case {
                sizings: vec![Sizing::Flex(0.0), Sizing::Flex(1.0)],
                available: 100.0,
                gap: 0.0,
                expected: vec![0.0, 100.0],
            },
            Case {
                sizings: vec![Sizing::Flex(-2.0), Sizing::Flex(-1.0)],
                available: 100.0,
                gap: 0.0,
                expected: vec![0.0, 0.0],
            },
            Case {
                sizings: vec![Sizing::Fixed(f64::NAN), Sizing::Flex(1.0)],
                available: 100.0,
                gap: 0.0,
                expected: vec![0.0, 100.0],
            },
            Case {
                sizings: vec![Sizing::Flex(f64::INFINITY), Sizing::Flex(1.0)],
                available: 100.0,
                gap: 0.0,
                expected: vec![0.0, 100.0],
            },
            Case {
                sizings: vec![Sizing::Fixed(80.0), Sizing::Fixed(80.0)],
                available: 100.0,
                gap: 0.0,
                expected: vec![80.0, 80.0],
            },
            Case {
                sizings: vec![Sizing::Fixed(10.0)],
                available: -50.0,
                gap: 0.0,
                expected: vec![10.0],
            },
        ];
        for (i, c) in cases.iter().enumerate() {
            let got = solve_widths(&c.sizings, c.available, c.gap);
            assert_eq!(got.len(), c.expected.len(), "case {i}: wrong length");
            for (g, e) in got.iter().zip(&c.expected) {
                assert!(approx(*g, *e), "case {i}: got {g}, expected {e}");
            }
        }
    }

    #[test]
    fn solve_widths_flex_strip_fills_available() {
        let widths = solve_widths(&[Sizing::Flex(1.0); 3], 100.0, 7.0);
        assert_eq!(widths.len(), 3);
        let sum: f64 = widths.iter().sum();
        assert!(
            approx(sum + 14.0, 100.0),
            "widths {widths:?} sum + 14.0 = {sum} + 14.0 != 100.0"
        );
    }

    #[test]
    fn strip_rects_ends_at_the_edge() {
        let sizings = [Sizing::Flex(1.0), Sizing::Flex(1.0), Sizing::Flex(1.0)];
        let bounds = Rect::new(0.0, 0.0, 100.0, 30.0);
        let rects = strip_rects(&sizings, bounds, 7.0);
        assert_eq!(rects.len(), 3);
        assert!(
            approx(rects[2].right(), 100.0),
            "third rect right = {}",
            rects[2].right()
        );
        for r in &rects {
            assert!(approx(r.y, 0.0), "y = {}", r.y);
            assert!(approx(r.height, 30.0), "height = {}", r.height);
        }
    }

    #[test]
    fn scroll_offset_clamped_cases() {
        assert!(approx(scroll_offset_clamped(50.0, 200.0, 100.0), 50.0));
        assert!(approx(scroll_offset_clamped(150.0, 200.0, 100.0), 100.0));
        assert!(approx(scroll_offset_clamped(-10.0, 200.0, 100.0), 0.0));
        assert!(approx(scroll_offset_clamped(50.0, 80.0, 100.0), 0.0));
        assert!(approx(scroll_offset_clamped(f64::NAN, 200.0, 100.0), 0.0));
    }

    #[test]
    fn render_strip_places_fixed_then_flex() {
        let Some((state, theme)) = fixture() else {
            eprintln!("skipping: cairo could not create the toy font");
            return;
        };
        let widgets = [
            Stub {
                sizing: Sizing::Fixed(30.0),
            },
            Stub {
                sizing: Sizing::Flex(1.0),
            },
        ];
        let refs: Vec<&dyn Widget> = widgets.iter().map(|w| w as &dyn Widget).collect();
        let bounds = Rect::new(0.0, 0.0, 100.0, 30.0);
        let out = render_strip(&refs, &state, &theme, bounds, 0.0);
        let rects: Vec<Rect> = out
            .commands()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Rect { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect();
        assert_eq!(rects.len(), 2);
        assert!(
            approx(rects[0].left(), 0.0),
            "fixed left = {}",
            rects[0].left()
        );
        assert!(
            approx(rects[0].width, 30.0),
            "fixed width = {}",
            rects[0].width
        );
        assert!(
            approx(rects[1].left(), 30.0),
            "flex left = {}",
            rects[1].left()
        );
        assert!(
            approx(rects[1].width, 70.0),
            "flex width = {}",
            rects[1].width
        );
        let hits = out.hits();
        assert_eq!(hits[0].id.widget, 0);
        assert_eq!(hits[1].id.widget, 1);
    }

    #[test]
    fn render_strip_drops_widgets_past_u16_max() {
        let Some((state, theme)) = fixture() else {
            eprintln!("skipping: cairo could not create the toy font");
            return;
        };
        let count = u16::MAX as usize + 2;
        let widgets: Vec<Stub> = (0..count)
            .map(|_| Stub {
                sizing: Sizing::Flex(1.0),
            })
            .collect();
        let refs: Vec<&dyn Widget> = widgets.iter().map(|w| w as &dyn Widget).collect();
        let bounds = Rect::new(0.0, 0.0, 100.0, 30.0);
        let out = render_strip(&refs, &state, &theme, bounds, 0.0);
        // Each rendered widget is one command inside its clip pair.
        let drawn = out
            .commands()
            .iter()
            .filter(|c| !matches!(c, DrawCmd::PushClip(_) | DrawCmd::PopClip))
            .count();
        assert_eq!(drawn, u16::MAX as usize);
        assert_eq!(out.hits().len(), u16::MAX as usize);
    }

    #[test]
    fn render_scroll_clips_to_bounds() {
        let Some((state, theme)) = fixture() else {
            eprintln!("skipping: cairo could not create the toy font");
            return;
        };
        let widgets = [
            Stub {
                sizing: Sizing::Fixed(60.0),
            },
            Stub {
                sizing: Sizing::Fixed(60.0),
            },
        ];
        let refs: Vec<&dyn Widget> = widgets.iter().map(|w| w as &dyn Widget).collect();
        let bounds = Rect::new(0.0, 0.0, 100.0, 30.0);
        let out = render_scroll(&refs, &state, &theme, bounds, 0.0, 0.0);
        let cmds = out.commands();
        assert!(matches!(cmds.first(), Some(DrawCmd::PushClip(r)) if *r == bounds));
        assert!(matches!(cmds.last(), Some(DrawCmd::PopClip)));
        assert_eq!(
            cmds.iter()
                .filter(|c| matches!(c, DrawCmd::PushClip(_)))
                .count(),
            1
        );
        assert_eq!(
            cmds.iter()
                .filter(|c| matches!(c, DrawCmd::PopClip))
                .count(),
            1
        );
        assert_eq!(cmds.len(), 4, "push, rect, rect, pop");
        assert_eq!(out.hits().len(), 2);
    }

    #[test]
    fn render_scroll_fits_matches_strip_rects() {
        let Some((state, theme)) = fixture() else {
            eprintln!("skipping: cairo could not create the toy font");
            return;
        };
        let widgets = [
            Stub {
                sizing: Sizing::Fixed(30.0),
            },
            Stub {
                sizing: Sizing::Fixed(30.0),
            },
        ];
        let refs: Vec<&dyn Widget> = widgets.iter().map(|w| w as &dyn Widget).collect();
        let bounds = Rect::new(0.0, 0.0, 100.0, 30.0);
        let gap = 8.0;
        let out = render_scroll(&refs, &state, &theme, bounds, gap, 0.0);
        let expected = strip_rects(&[Sizing::Fixed(30.0), Sizing::Fixed(30.0)], bounds, gap);
        let rects: Vec<Rect> = out
            .commands()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Rect { rect, .. } => Some(*rect),
                _ => None,
            })
            .collect();
        assert_eq!(rects, expected);
    }
}
