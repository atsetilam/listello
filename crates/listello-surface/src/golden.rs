//! The compare-and-report half of the golden-image harness: render into a [`PixelSurface`],
//! compare against a committed PNG, fail loudly on the first pixel that disagrees.
//!
//! This is the gate for all rendering work (D10, ROADMAP M1): a widget test is "render this
//! widget against a fixed state snapshot into a `PixelSurface`, call [`assert_matches`]", and
//! a pass here is evidence a human can still see, not just a green checkmark.
//!
//! # Recording a golden image
//!
//! Run the test once with `LISTELLO_GOLDEN_UPDATE=1` set. That writes (or overwrites) the PNG
//! at the given path instead of comparing against it — commit the result. There is
//! deliberately no separate "record" binary: the same test that checks a golden image is the
//! only thing that knows how to produce one, so the two can never drift apart.

use crate::pixel::PixelSurface;
use crate::Surface;
use anyhow::{bail, Context, Result};
use cairo::ImageSurface;
use std::fs::File;
use std::path::Path;

/// Compare `surface`'s current contents against the PNG at `path`.
///
/// Fails with a message naming the first differing pixel and how many differ in total, and
/// writes the actual render next to `path` (suffixed `.actual.png`) so a human can look at both
/// without re-running anything. Set `LISTELLO_GOLDEN_UPDATE=1` to (re)write `path` instead of
/// comparing — see the module docs.
pub fn assert_matches(path: impl AsRef<Path>, surface: &mut PixelSurface) -> Result<()> {
    let path = path.as_ref();

    if std::env::var_os("LISTELLO_GOLDEN_UPDATE").is_some() {
        surface
            .write_png(path)
            .with_context(|| format!("failed to record golden image at {}", path.display()))?;
        return Ok(());
    }

    let size = surface.size();
    let (actual, actual_stride) = surface.pixels()?;

    let mut file = File::open(path).with_context(|| {
        format!(
            "no golden image at {} -- rerun with LISTELLO_GOLDEN_UPDATE=1 to record one",
            path.display()
        )
    })?;
    let mut golden = ImageSurface::create_from_png(&mut file)
        .with_context(|| format!("{} is not a readable PNG", path.display()))?;

    if golden.width() != size.width as i32 || golden.height() != size.height as i32 {
        bail!(
            "{}: size mismatch -- golden is {}x{}, rendered is {}x{}",
            path.display(),
            golden.width(),
            golden.height(),
            size.width,
            size.height
        );
    }

    let golden_stride = golden.stride() as usize;
    let golden_data = golden
        .data()
        .with_context(|| format!("{}: could not read decoded PNG data", path.display()))?;

    let mut first_diff = None;
    let mut diff_count: u64 = 0;
    for y in 0..size.height as usize {
        for x in 0..size.width as usize {
            let a = pixel_at(&actual, actual_stride, x, y);
            let g = pixel_at(&golden_data, golden_stride, x, y);
            if a != g {
                diff_count += 1;
                first_diff.get_or_insert((x, y, a, g));
            }
        }
    }
    drop(golden_data);

    if diff_count == 0 {
        return Ok(());
    }

    let (x, y, a, g) = first_diff.expect("diff_count > 0 implies first_diff was set");
    let actual_path = path.with_extension("actual.png");
    // Best-effort: a failed write here must not hide the real failure, which is the mismatch.
    let _ = surface.write_png(&actual_path);
    bail!(
        "{}: {diff_count} pixel(s) differ from the golden image out of {total} (first at \
         ({x}, {y}): got {a:#010x}, want {g:#010x}). Rendered output written to {} -- if this \
         change is intentional, rerun with LISTELLO_GOLDEN_UPDATE=1 and commit the new PNG.",
        path.display(),
        actual_path.display(),
        total = size.width as u64 * size.height as u64,
    );
}

/// One ARGB32 pixel at `(x, y)` in a buffer with the given `stride` (bytes per row, not
/// necessarily `4 * width` -- cairo aligns rows, and both golden and rendered pixels go through
/// this to avoid comparing padding as if it were image data).
fn pixel_at(data: &[u8], stride: usize, x: usize, y: usize) -> u32 {
    let o = y * stride + x * 4;
    u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::{Color, DrawCmd, Frame};
    use crate::geom::Size;
    use std::sync::Mutex;

    // `LISTELLO_GOLDEN_UPDATE` is process-global; every test in this module goes through this
    // lock, including ones that never touch it, so none of them can observe another test's
    // `set_var` mid-flight.
    static ENV_GUARD: Mutex<()> = Mutex::new(());

    fn render(color: Color) -> PixelSurface {
        let mut s = PixelSurface::new(Size::new(4, 4)).unwrap();
        s.present(&Frame::full(vec![DrawCmd::Clear(color)]))
            .unwrap();
        s
    }

    #[test]
    fn identical_pixels_pass() {
        let _guard = ENV_GUARD.lock().unwrap();
        let dir = std::env::temp_dir().join("listello-golden-test-identical");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("white.png");

        let mut recorded = render(Color::WHITE);
        recorded.write_png(&path).unwrap();

        let mut rendered = render(Color::WHITE);
        assert_matches(&path, &mut rendered).unwrap();
    }

    #[test]
    fn a_changed_pixel_is_reported_with_its_coordinates() {
        let _guard = ENV_GUARD.lock().unwrap();
        let dir = std::env::temp_dir().join("listello-golden-test-mismatch");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("white.png");

        let mut recorded = render(Color::WHITE);
        recorded.write_png(&path).unwrap();

        let mut rendered = render(Color::BLACK);
        let err = assert_matches(&path, &mut rendered)
            .unwrap_err()
            .to_string();
        assert!(err.contains("16 pixel(s) differ"), "{err}");
        assert!(err.contains("(0, 0)"), "{err}");
    }

    #[test]
    fn a_missing_golden_names_the_update_env_var() {
        let _guard = ENV_GUARD.lock().unwrap();
        let path = std::env::temp_dir().join("listello-golden-test-does-not-exist.png");
        let _ = std::fs::remove_file(&path);

        let mut rendered = render(Color::WHITE);
        let err = assert_matches(&path, &mut rendered)
            .unwrap_err()
            .to_string();
        assert!(err.contains("LISTELLO_GOLDEN_UPDATE"), "{err}");
    }

    #[test]
    fn update_mode_records_instead_of_comparing() {
        let _guard = ENV_GUARD.lock().unwrap();
        let dir = std::env::temp_dir().join("listello-golden-test-update");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("recorded.png");
        let _ = std::fs::remove_file(&path);

        // SAFETY: this crate's tests do not run this in parallel with anything else that reads
        // this env var, and it's restored before the function returns.
        unsafe { std::env::set_var("LISTELLO_GOLDEN_UPDATE", "1") };
        let mut rendered = render(Color::WHITE);
        let result = assert_matches(&path, &mut rendered);
        unsafe { std::env::remove_var("LISTELLO_GOLDEN_UPDATE") };

        result.unwrap();
        assert!(path.exists());
    }
}
