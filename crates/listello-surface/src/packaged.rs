//! The packaged icon set: SVG files in `share/listello/`, rasterised once at startup.
//!
//! This sits below the pixel line next to the shared rasteriser on purpose (D10): the
//! widgets above it only ever ask for an icon by name and get pixels, and both binaries that
//! draw — the daemon and the simulator — load the same files the same way, so a golden image
//! built from them means the same thing on the panel.
//!
//! A config names an icon with dashes (`trash-full`); the file is the same name with
//! underscores (`trash_full.svg`). Names come from a config file, so anything that is not
//! plain letters, digits, dashes and underscores is refused rather than joined onto a path.

use cairo::{Context, Format, ImageSurface};
use librsvg_rebind::prelude::*;
use std::fs;
use std::path::Path;

/// Side of the square every packaged icon is rasterised at, in panel pixels.
pub const ICON_PX: u32 = 40;
/// Larger files are not icons.
const MAX_SVG_BYTES: u64 = 2 * 1024 * 1024;

/// Rasterise `<dir>/<name with underscores>.svg` at [`ICON_PX`]. `None` for a bad name, a
/// missing or oversized file, or an SVG librsvg cannot read — a normal answer, never a panic.
pub fn load_icon(dir: &Path, name: &str) -> Option<ImageSurface> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    let path = dir.join(format!("{}.svg", name.replace('-', "_")));
    if fs::metadata(&path).ok()?.len() > MAX_SVG_BYTES {
        return None;
    }
    let handle = librsvg_rebind::Handle::from_file(path.to_str()?).ok()?;
    let target = ImageSurface::create(Format::ARgb32, ICON_PX as i32, ICON_PX as i32).ok()?;
    let cr = Context::new(&target).ok()?;
    let viewport = librsvg_rebind::Rectangle::new(0.0, 0.0, ICON_PX as f64, ICON_PX as f64);
    handle.render_document(&cr, &viewport).ok()?;
    drop(cr);
    Some(target)
}

/// Every name in `names` that loads, with its pixels. A name that does not load is simply
/// absent, and the widget that asked for it draws its label instead.
pub fn load_icons<'a>(
    dir: &Path,
    names: impl IntoIterator<Item = &'a String>,
) -> Vec<(String, ImageSurface)> {
    names
        .into_iter()
        .filter_map(|name| Some((name.clone(), load_icon(dir, name)?)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn packaged_dir() -> PathBuf {
        PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../share/listello"))
    }

    #[test]
    fn a_dashed_name_finds_its_underscored_file_at_the_icon_size() {
        let icon = load_icon(&packaged_dir(), "volume-up").expect("volume_up.svg loads");
        assert_eq!(
            (icon.width(), icon.height()),
            (ICON_PX as i32, ICON_PX as i32)
        );
    }

    #[test]
    fn every_packaged_icon_the_widgets_ask_for_exists() {
        for name in [
            "bolt",
            "lock",
            "folder",
            "download",
            "screenshot",
            "trash-full",
            "trash-empty",
            "network-0",
            "network-1",
            "network-2",
            "network-3",
            "network-off",
            "brightness-low",
            "brightness-high",
            "volume-off",
            "volume-down",
            "volume-up",
        ] {
            assert!(load_icon(&packaged_dir(), name).is_some(), "{name}");
        }
    }

    #[test]
    fn the_icon_actually_has_pixels_in_it() {
        let mut icon = load_icon(&packaged_dir(), "lock").expect("lock.svg loads");
        icon.flush();
        let data = icon.data().expect("exclusive access to a fresh surface");
        assert!(data.iter().any(|&b| b != 0), "rasterised to nothing");
    }

    #[test]
    fn names_that_are_not_plain_words_are_refused() {
        let dir = packaged_dir();
        for bad in ["", "../config", "a/b", "lock.svg", "lock\0", "no such icon"] {
            assert!(load_icon(&dir, bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn a_name_with_no_file_is_absent_not_an_error() {
        let names = vec!["lock".to_string(), "not-an-icon".to_string()];
        let loaded = load_icons(&packaged_dir(), &names);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].0, "lock");
    }
}
