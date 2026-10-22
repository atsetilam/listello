//! From an icon name to pixels the renderer can draw.
//!
//! The lookup follows the XDG icon theme rules (the `freedesktop-icons` crate does the
//! walking), the decode handles PNG and SVG, and the result is always a square RGBA image of
//! the requested size. The renderer runs as `nobody` and cannot read any of these files, which
//! is why the pixels cross the socket and the paths never do (D11).
//!
//! Every file here is arbitrary data from the user's disk. A malformed PNG, a 20000-pixel one
//! or an enormous SVG is refused and the app gets the fallback glyph -- never a panic in a
//! daemon.

use crate::desktop::{config_dir, Catalog};
use cairo::{Context, Format, ImageSurface};
use librsvg_rebind::prelude::*;
use listello_protocol::Icon;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

const MIN_SIZE: u32 = 16;
const MAX_SIZE: u32 = 256;
/// Larger source images are not icons.
const MAX_PNG_BYTES: u64 = 8 * 1024 * 1024;
const MAX_SVG_BYTES: u64 = 2 * 1024 * 1024;
const MAX_PNG_SIDE: u32 = 4096;

/// The theme to look icons up in: KDE's, else Breeze (see `themes_to_try` for the rest).
pub fn current_theme() -> String {
    config_dir()
        .and_then(|d| fs::read_to_string(d.join("kdeglobals")).ok())
        .and_then(|text| theme_from_kdeglobals(&text))
        // KDE writes no `[Icons]` section while the theme is its default, which is Breeze;
        // `hicolor` alone has almost none of the app icons (Konsole's terminal, for one).
        .unwrap_or_else(|| "breeze".to_string())
}

/// The themes to try for one name, in order: the user's, then the usual defaults, so an app
/// whose icon the chosen theme lacks still gets one.
fn themes_to_try(theme: &str) -> Vec<&str> {
    let mut themes = vec![theme];
    for fallback in ["breeze", "Adwaita", "hicolor"] {
        if !themes.contains(&fallback) {
            themes.push(fallback);
        }
    }
    themes
}

fn theme_from_kdeglobals(text: &str) -> Option<String> {
    let mut in_icons = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_icons = line == "[Icons]";
        } else if in_icons {
            if let Some(value) = line.strip_prefix("Theme=") {
                let value = value.trim();
                return (!value.is_empty()).then(|| value.to_string());
            }
        }
    }
    None
}

/// The pixels for an app, or `None` when nothing usable was found -- a normal answer.
pub fn icon_for(catalog: &Catalog, theme: &str, app_id: &str, size_px: u32) -> Option<Icon> {
    let size = size_px.clamp(MIN_SIZE, MAX_SIZE);
    catalog
        .icon_names(app_id)
        .iter()
        .filter_map(|name| {
            themes_to_try(theme)
                .into_iter()
                .find_map(|theme| find_file(name, size, theme))
        })
        .find_map(|path| load(&path, size))
}

fn find_file(name: &str, size: u32, theme: &str) -> Option<PathBuf> {
    if name.is_empty() {
        return None;
    }
    if name.starts_with('/') {
        return Path::new(name).is_file().then(|| PathBuf::from(name));
    }
    // A name is a name, not a path; refuse anything that tries to climb out.
    if name.contains('/') || name.contains("..") {
        return None;
    }
    freedesktop_icons::lookup(name)
        .with_size(size.min(u16::MAX as u32) as u16)
        .with_theme(theme)
        .with_cache()
        .find()
        .or_else(|| {
            ["png", "svg"]
                .iter()
                .map(|ext| PathBuf::from(format!("/usr/share/pixmaps/{name}.{ext}")))
                .find(|p| p.is_file())
        })
}

/// Decode a PNG or SVG into a `size`x`size` straight-RGBA icon.
pub fn load(path: &Path, size: u32) -> Option<Icon> {
    let surface = match path.extension().and_then(|e| e.to_str()) {
        Some("png") => render_png(path, size)?,
        Some("svg") => render_svg(path, size)?,
        _ => return None,
    };
    to_icon(surface)
}

fn render_png(path: &Path, size: u32) -> Option<ImageSurface> {
    let mut file = fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_PNG_BYTES {
        return None;
    }
    // The dimensions live in the IHDR chunk, right after the 8-byte signature. Reading them
    // first is what stops a 20000x20000 PNG from asking cairo for a gigabyte.
    let mut header = [0u8; 24];
    file.read_exact(&mut header).ok()?;
    let (width, height) = png_dimensions(&header)?;
    if width == 0 || height == 0 || width > MAX_PNG_SIDE || height > MAX_PNG_SIDE {
        return None;
    }
    let mut file = fs::File::open(path).ok()?;
    let source = ImageSurface::create_from_png(&mut file).ok()?;

    let target = ImageSurface::create(Format::ARgb32, size as i32, size as i32).ok()?;
    let cr = Context::new(&target).ok()?;
    // Fit inside the square, centred, keeping the aspect ratio.
    let scale = (size as f64 / width as f64).min(size as f64 / height as f64);
    cr.translate(
        (size as f64 - width as f64 * scale) / 2.0,
        (size as f64 - height as f64 * scale) / 2.0,
    );
    cr.scale(scale, scale);
    cr.set_source_surface(&source, 0.0, 0.0).ok()?;
    cr.source().set_filter(cairo::Filter::Best);
    cr.paint().ok()?;
    drop(cr);
    Some(target)
}

fn png_dimensions(header: &[u8; 24]) -> Option<(u32, u32)> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if header[..8] != SIGNATURE || &header[12..16] != b"IHDR" {
        return None;
    }
    let width = u32::from_be_bytes(header[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(header[20..24].try_into().ok()?);
    Some((width, height))
}

fn render_svg(path: &Path, size: u32) -> Option<ImageSurface> {
    if fs::metadata(path).ok()?.len() > MAX_SVG_BYTES {
        return None;
    }
    let handle = librsvg_rebind::Handle::from_file(path.to_str()?).ok()?;
    let target = ImageSurface::create(Format::ARgb32, size as i32, size as i32).ok()?;
    let cr = Context::new(&target).ok()?;
    let viewport = librsvg_rebind::Rectangle::new(0.0, 0.0, size as f64, size as f64);
    handle.render_document(&cr, &viewport).ok()?;
    drop(cr);
    Some(target)
}

/// Cairo hands back premultiplied native-endian ARGB; the wire format is straight RGBA.
pub fn to_icon(mut surface: ImageSurface) -> Option<Icon> {
    surface.flush();
    let (width, height) = (surface.width() as u32, surface.height() as u32);
    let stride = surface.stride() as usize;
    let data = surface.data().ok()?;
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let at = y * stride + x * 4;
            let px = u32::from_ne_bytes(data[at..at + 4].try_into().ok()?);
            let a = (px >> 24) & 0xff;
            let unpremultiply = |c: u32| {
                (c * 255 + a / 2)
                    .checked_div(a)
                    .map_or(0, |v| v.min(255) as u8)
            };
            rgba.extend_from_slice(&[
                unpremultiply((px >> 16) & 0xff),
                unpremultiply((px >> 8) & 0xff),
                unpremultiply(px & 0xff),
                a as u8,
            ]);
        }
    }
    Icon::new(width, height, rgba)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_chosen_theme_is_tried_first_then_the_usual_defaults_once_each() {
        assert_eq!(
            super::themes_to_try("oxygen"),
            ["oxygen", "breeze", "Adwaita", "hicolor"]
        );
        assert_eq!(
            super::themes_to_try("breeze"),
            ["breeze", "Adwaita", "hicolor"]
        );
    }

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("listello-icons-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A real PNG of the given size, filled with one opaque colour.
    fn write_png(path: &Path, width: i32, height: i32) {
        let surface = ImageSurface::create(Format::ARgb32, width, height).unwrap();
        let cr = Context::new(&surface).unwrap();
        cr.set_source_rgb(1.0, 0.0, 0.0);
        cr.paint().unwrap();
        drop(cr);
        let mut file = fs::File::create(path).unwrap();
        surface.write_to_png(&mut file).unwrap();
    }

    #[test]
    fn a_png_becomes_square_straight_rgba() {
        let dir = temp_dir("png");
        let path = dir.join("red.png");
        write_png(&path, 10, 10);
        let icon = load(&path, 32).expect("a decoded icon");
        assert_eq!((icon.width, icon.height), (32, 32));
        assert!(icon.is_valid());
        // The middle is opaque red, with the colour un-premultiplied.
        let mid = (16 * 32 + 16) * 4;
        assert_eq!(&icon.rgba[mid..mid + 4], &[255, 0, 0, 255]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_wide_png_is_letterboxed_not_stretched() {
        let dir = temp_dir("wide");
        let path = dir.join("wide.png");
        write_png(&path, 40, 10);
        let icon = load(&path, 32).expect("a decoded icon");
        // The top row is outside the 32x8 band the image occupies: fully transparent.
        assert_eq!(icon.rgba[3], 0);
        let mid = (16 * 32 + 16) * 4;
        assert_eq!(icon.rgba[mid + 3], 255);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_png_that_lies_about_its_size_is_refused_before_cairo_sees_it() {
        let dir = temp_dir("huge");
        let path = dir.join("huge.png");
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        bytes.extend_from_slice(&13u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&20000u32.to_be_bytes());
        bytes.extend_from_slice(&20000u32.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        fs::write(&path, bytes).unwrap();
        assert!(load(&path, 32).is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn garbage_is_refused_not_a_panic() {
        let dir = temp_dir("garbage");
        for (name, body) in [
            ("a.png", &b"not a png at all, just text padding"[..]),
            ("b.png", b""),
            ("c.svg", b"<svg"),
            ("d.xpm", b"/* XPM */"),
        ] {
            let path = dir.join(name);
            fs::write(&path, body).unwrap();
            assert!(load(&path, 32).is_none(), "{name}");
        }
        assert!(load(&dir.join("missing.png"), 32).is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_svg_is_rendered_at_the_requested_size() {
        let dir = temp_dir("svg");
        let path = dir.join("dot.svg");
        fs::write(
            &path,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48"><rect width="48" height="48" fill="#00ff00"/></svg>"##,
        )
        .unwrap();
        let icon = load(&path, 64).expect("a rendered icon");
        assert_eq!((icon.width, icon.height), (64, 64));
        let mid = (32 * 64 + 32) * 4;
        assert_eq!(&icon.rgba[mid..mid + 4], &[0, 255, 0, 255]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn names_that_climb_out_are_not_looked_up() {
        assert!(find_file("../../etc/passwd", 32, "hicolor").is_none());
        assert!(find_file("a/b", 32, "hicolor").is_none());
        assert!(find_file("", 32, "hicolor").is_none());
    }

    #[test]
    fn the_theme_comes_from_kdeglobals() {
        assert_eq!(
            theme_from_kdeglobals("[General]\nTheme=wrong\n[Icons]\nTheme=breeze-dark\n"),
            Some("breeze-dark".to_string())
        );
        assert_eq!(theme_from_kdeglobals("[General]\nx=y\n"), None);
    }
}
