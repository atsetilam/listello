//! Window pictures for the window row's previews, from KWin's `org.kde.KWin.ScreenShot2`.
//!
//! KWin answers this D-Bus interface only for a caller whose executable has a `.desktop` file
//! declaring `X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2` (RESEARCH §3.1), and
//! it checks the calling *process*, so the call has to come from here rather than from a
//! `gdbus` child. Allowed callers get no prompt.
//!
//! A capture is one full-size still, written raw into a pipe we hand over with the call. It is
//! read with a deadline and reduced at once, straight from KWin's bytes to the strip-sized
//! picture, reading only the pixels that picture needs: decoding and scaling the whole frame
//! first cost twice as much as the capture itself.

use cairo::{Format, ImageSurface};
use dbus::arg::{PropMap, RefArg, Variant};
use dbus::blocking::Connection;
use listello_protocol::{Icon, PreviewFit};
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use std::borrow::Cow;
use std::io::Read;
use std::os::fd::{AsFd, OwnedFd};
use std::time::{Duration, Instant};

const DEST: &str = "org.kde.KWin";
const PATH: &str = "/org/kde/KWin/ScreenShot2";
const INTERFACE: &str = "org.kde.KWin.ScreenShot2";
const NOT_AUTHORIZED: &str = "org.kde.KWin.ScreenShot2.Error.NoAuthorized";

/// How long one capture may hold up the agent's loop, from the call to the last byte.
const DEADLINE: Duration = Duration::from_millis(800);

/// Bigger than any real window. A larger claim is refused before anything is allocated.
const MAX_PIXELS: u64 = 8192 * 8192;

/// Why a capture produced no picture.
#[derive(Debug, PartialEq, Eq)]
pub enum CaptureError {
    /// KWin will not capture for this executable: no `.desktop` file, or one without the
    /// restricted-interface line. Permanent until reinstalled, so worth saying once.
    Refused,
    /// This window, this time: it is gone, minimised, or KWin had a bad moment.
    Failed(String),
}

pub struct Capturer {
    conn: Connection,
}

impl Capturer {
    /// `None` when there is no session bus. Whether KWin will actually answer is only known
    /// at the first capture.
    pub fn connect() -> Option<Capturer> {
        match Connection::new_session() {
            Ok(conn) => Some(Capturer { conn }),
            Err(e) => {
                eprintln!("listello-agent: no session bus, so no window previews: {e}");
                None
            }
        }
    }

    /// The window KWin knows as `uuid`, made to `height` × `max_width` as `how` says.
    pub fn capture(
        &self,
        uuid: &str,
        height: u32,
        max_width: u32,
        how: PreviewFit,
    ) -> Result<Icon, CaptureError> {
        let raw = self.grab(uuid)?;
        fit(&raw, height, max_width, how)
            .ok_or_else(|| CaptureError::Failed(format!("unknown pixel format {}", raw.format)))
    }

    fn grab(&self, uuid: &str) -> Result<Raw<'static>, CaptureError> {
        let started = Instant::now();
        let (mut reader, writer) =
            std::io::pipe().map_err(|e| CaptureError::Failed(format!("no pipe: {e}")))?;
        let mut options = PropMap::new();
        for key in ["include-decoration", "include-shadow", "include-cursor"] {
            options.insert(key.to_string(), Variant(Box::new(false) as Box<dyn RefArg>));
        }
        let proxy = self.conn.with_proxy(DEST, PATH, DEADLINE);
        // The writer moves into the call and is closed once sent, so KWin holds the only
        // write end and the read below ends at EOF.
        let (results,): (PropMap,) = proxy
            .method_call(
                INTERFACE,
                "CaptureWindow",
                (uuid, options, OwnedFd::from(writer)),
            )
            .map_err(|e| match e.name() {
                Some(NOT_AUTHORIZED) => CaptureError::Refused,
                _ => CaptureError::Failed(e.message().unwrap_or("no message").to_string()),
            })?;

        let number = |key: &str| results.get(key).and_then(|v| v.0.as_u64());
        let (Some(width), Some(height), Some(stride), Some(format)) = (
            number("width"),
            number("height"),
            number("stride"),
            number("format"),
        ) else {
            return Err(CaptureError::Failed(format!(
                "unexpected reply {results:?}"
            )));
        };
        if width == 0 || height == 0 || width * height > MAX_PIXELS || stride < width * 4 {
            return Err(CaptureError::Failed(format!(
                "implausible size {width}x{height}"
            )));
        }
        let len = (stride * height) as usize;
        let bytes = read_until(&mut reader, len, started + DEADLINE)?;
        Ok(Raw {
            bytes: Cow::Owned(bytes),
            width: width as usize,
            height: height as usize,
            stride: stride as usize,
            format,
        })
    }
}

/// Read exactly `len` bytes, waiting at most until `deadline`.
fn read_until(
    reader: &mut std::io::PipeReader,
    len: usize,
    deadline: Instant,
) -> Result<Vec<u8>, CaptureError> {
    let mut buf = vec![0u8; len];
    let mut filled = 0;
    while filled < len {
        let left = deadline.saturating_duration_since(Instant::now());
        let timeout = PollTimeout::try_from(left.as_millis() as i32).unwrap_or(PollTimeout::ZERO);
        let mut fds = [PollFd::new(reader.as_fd(), PollFlags::POLLIN)];
        match poll(&mut fds, timeout) {
            Ok(0) => return Err(CaptureError::Failed("KWin was too slow".to_string())),
            Ok(_) => {}
            Err(nix::errno::Errno::EINTR) => continue,
            Err(e) => return Err(CaptureError::Failed(format!("poll: {e}"))),
        }
        match reader.read(&mut buf[filled..]) {
            Ok(0) => {
                return Err(CaptureError::Failed(
                    "the picture was cut short".to_string(),
                ))
            }
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(CaptureError::Failed(format!("read: {e}"))),
        }
    }
    Ok(buf)
}

/// One frame as KWin wrote it: `stride * height` bytes in a `QImage::Format`. Owned when it
/// came down a pipe, borrowed when it sits in a PipeWire buffer (`live.rs`).
pub struct Raw<'a> {
    pub bytes: Cow<'a, [u8]>,
    pub width: usize,
    pub height: usize,
    pub stride: usize,
    pub format: u64,
}

// The `QImage::Format` values KWin can hand over.
pub const RGB32: u64 = 4;
const ARGB32: u64 = 5;
pub const ARGB32_PREMULTIPLIED: u64 = 6;
pub const RGBX8888: u64 = 16;
const RGBA8888: u64 = 17;
pub const RGBA8888_PREMULTIPLIED: u64 = 18;

impl Raw<'_> {
    fn known_format(&self) -> bool {
        matches!(
            self.format,
            RGB32 | ARGB32 | ARGB32_PREMULTIPLIED | RGBX8888 | RGBA8888 | RGBA8888_PREMULTIPLIED
        )
    }

    /// The pixel at (`x`, `y`) as premultiplied `[r, g, b, a]`, whatever the format.
    fn pixel(&self, x: usize, y: usize) -> [u32; 4] {
        let p = &self.bytes[y * self.stride + x * 4..][..4];
        let (r, g, b, a, premultiplied) = match self.format {
            // The 32-bit Qt formats are native-endian 0xAARRGGBB words.
            RGB32 | ARGB32 | ARGB32_PREMULTIPLIED => {
                let w = u32::from_ne_bytes([p[0], p[1], p[2], p[3]]);
                let a = if self.format == RGB32 { 255 } else { w >> 24 };
                (
                    (w >> 16) & 0xff,
                    (w >> 8) & 0xff,
                    w & 0xff,
                    a,
                    self.format != ARGB32,
                )
            }
            // The 8888 formats are bytes in R, G, B, A order.
            _ => {
                let a = if self.format == RGBX8888 {
                    255
                } else {
                    p[3] as u32
                };
                (
                    p[0] as u32,
                    p[1] as u32,
                    p[2] as u32,
                    a,
                    self.format != RGBA8888,
                )
            }
        };
        if premultiplied {
            [r, g, b, a]
        } else {
            let pm = |c: u32| (c * a + 127) / 255;
            [pm(r), pm(g), pm(b), a]
        }
    }
}

/// Source pixels averaged per picture pixel, at most this many each way. Enough to keep text
/// and edges from shimmering between refreshes; reading all of them would be reading the
/// whole frame again.
const SAMPLES: usize = 4;

/// Make `raw` into a picture for a `max_width` × `height` box, as `how` says. A whole window
/// is never scaled up. A crop is the full width of the window and as much of its height as
/// the box's shape allows, from the middle: a reminder of what the window looks like rather
/// than something to read.
pub fn fit(raw: &Raw, height: u32, max_width: u32, how: PreviewFit) -> Option<Icon> {
    if !raw.known_format() || raw.width == 0 || raw.height == 0 {
        return None;
    }
    // A buffer shorter than its claimed size would be read past its end.
    if raw.stride < raw.width * 4 || raw.bytes.len() < raw.stride * (raw.height - 1) + raw.width * 4
    {
        return None;
    }
    let (w, h) = (raw.width as f64, raw.height as f64);
    let (box_w, box_h) = (max_width.max(1) as f64, height.max(1) as f64);
    // Output size, the scale each way, and where the source's top-left lands.
    let (out_w, out_h, sx, sy, dx, dy) = match how {
        PreviewFit::Whole => {
            let s = (box_h / h).min(box_w / w).min(1.0);
            let (ow, oh) = ((w * s).round().max(1.0), (h * s).round().max(1.0));
            (ow, oh, ow / w, oh / h, 0.0, 0.0)
        }
        PreviewFit::Stretch => (box_w, box_h, box_w / w, box_h / h, 0.0, 0.0),
        PreviewFit::Crop => {
            let s = (box_w / w).max(box_h / h);
            let (dx, dy) = ((box_w - w * s) / 2.0, (box_h - h * s) / 2.0);
            (box_w, box_h, s, s, dx, dy)
        }
    };
    let (out_w, out_h) = (out_w as usize, out_h as usize);
    let mut rgba = Vec::with_capacity(out_w * out_h * 4);
    // The source span one picture pixel covers, and the samples taken across it.
    let span = |o: usize, d: f64, s: f64, limit: usize| {
        let lo = ((o as f64 - d) / s).max(0.0);
        let hi = ((o as f64 + 1.0 - d) / s).min(limit as f64);
        let lo_i = (lo as usize).min(limit - 1);
        let hi_i = (hi.ceil() as usize).clamp(lo_i + 1, limit);
        let n = (hi_i - lo_i).min(SAMPLES);
        (0..n).map(move |k| {
            // Evenly spread, nudged by one on every other sample, so a pattern as fine as the
            // spacing (alternate stripes, a text's stems) averages out instead of landing on
            // the same phase every time.
            (lo_i + (hi_i - lo_i) * (2 * k + 1) / (2 * n) + (k & 1)).min(hi_i - 1)
        })
    };
    for oy in 0..out_h {
        for ox in 0..out_w {
            let mut sum = [0u32; 4];
            let mut count = 0u32;
            for y in span(oy, dy, sy, raw.height) {
                for x in span(ox, dx, sx, raw.width) {
                    let p = raw.pixel(x, y);
                    for c in 0..4 {
                        sum[c] += p[c];
                    }
                    count += 1;
                }
            }
            let count = count.max(1);
            let a = sum[3] / count;
            let straight = |c: u32| {
                ((c / count) * 255 + a / 2)
                    .checked_div(a)
                    .map_or(0, |v| v.min(255)) as u8
            };
            rgba.extend_from_slice(&[
                straight(sum[0]),
                straight(sum[1]),
                straight(sum[2]),
                a as u8,
            ]);
        }
    }
    Icon::new(out_w as u32, out_h as u32, rgba)
}

/// Write a straight-RGBA picture as PNG, for `--capture`.
pub fn save_png(icon: &Icon, path: &std::path::Path) -> anyhow::Result<()> {
    let mut surface = ImageSurface::create(Format::ARgb32, icon.width as i32, icon.height as i32)?;
    let stride = surface.stride() as usize;
    {
        let mut data = surface.data()?;
        for (i, p) in icon.rgba.as_chunks::<4>().0.iter().enumerate() {
            let (x, y) = (i % icon.width as usize, i / icon.width as usize);
            let a = p[3] as u32;
            let pm = |c: u8| (c as u32 * a + 127) / 255;
            let word = a << 24 | pm(p[0]) << 16 | pm(p[1]) << 8 | pm(p[2]);
            data[y * stride + x * 4..][..4].copy_from_slice(&word.to_ne_bytes());
        }
    }
    surface.write_to_png(&mut std::fs::File::create(path)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame in KWin's native-word format, each pixel from `colour(x, y)` as 0xAARRGGBB.
    fn frame(width: usize, height: usize, colour: impl Fn(usize, usize) -> u32) -> Raw<'static> {
        let mut bytes = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                bytes.extend_from_slice(&colour(x, y).to_ne_bytes());
            }
        }
        Raw {
            bytes: Cow::Owned(bytes),
            width,
            height,
            stride: width * 4,
            format: ARGB32_PREMULTIPLIED,
        }
    }

    fn rgba(icon: &Icon, x: usize, y: usize) -> [u8; 4] {
        let at = (y * icon.width as usize + x) * 4;
        icon.rgba[at..at + 4].try_into().unwrap()
    }

    #[test]
    fn both_qt_byte_orders_read_as_the_same_pixel() {
        // Opaque orange, as a native 0xAARRGGBB word and as R, G, B, A bytes.
        let word = 0xff_ff_80_00u32.to_ne_bytes().to_vec();
        let bytes = vec![0xff, 0x80, 0x00, 0xff];
        for (b, format) in [
            (word.clone(), 6),
            (word, 4),
            (bytes.clone(), 17),
            (bytes, 16),
        ] {
            let raw = Raw {
                bytes: Cow::Owned(b),
                width: 1,
                height: 1,
                stride: 4,
                format,
            };
            assert_eq!(raw.pixel(0, 0), [0xff, 0x80, 0x00, 0xff], "format {format}");
        }
    }

    #[test]
    fn straight_alpha_is_premultiplied_and_comes_back_straight() {
        let raw = Raw {
            bytes: Cow::Owned(vec![0xff, 0xff, 0xff, 0x80]),
            width: 1,
            height: 1,
            stride: 4,
            format: 17,
        };
        assert_eq!(raw.pixel(0, 0), [0x80, 0x80, 0x80, 0x80]);
        let icon = fit(&raw, 1, 1, PreviewFit::Whole).unwrap();
        assert_eq!(rgba(&icon, 0, 0), [0xff, 0xff, 0xff, 0x80]);
    }

    #[test]
    fn an_unknown_format_is_refused() {
        let raw = Raw {
            bytes: Cow::Owned(vec![0; 4]),
            width: 1,
            height: 1,
            stride: 4,
            format: 999,
        };
        assert!(fit(&raw, 1, 1, PreviewFit::Whole).is_none());
    }

    #[test]
    fn a_wide_window_fits_the_width_and_a_tall_one_the_height() {
        let icon = fit(
            &frame(2000, 500, |_, _| 0xff000000),
            50,
            100,
            PreviewFit::Whole,
        )
        .unwrap();
        assert_eq!((icon.width, icon.height), (100, 25));
        let icon = fit(
            &frame(1600, 1000, |_, _| 0xff000000),
            50,
            100,
            PreviewFit::Whole,
        )
        .unwrap();
        assert_eq!((icon.width, icon.height), (80, 50));
    }

    #[test]
    fn crop_and_stretch_fill_the_box_exactly() {
        for size in [(1600, 1000), (300, 40), (100, 2000)] {
            let raw = frame(size.0, size.1, |_, _| 0xff000000);
            for how in [PreviewFit::Crop, PreviewFit::Stretch] {
                let icon = fit(&raw, 60, 380, how).unwrap();
                assert_eq!((icon.width, icon.height), (380, 60), "{size:?} {how:?}");
            }
        }
    }

    #[test]
    fn a_crop_is_the_full_width_band_through_the_middle() {
        // Left half red, right half blue; top third green, bottom third yellow. The crop
        // spans the whole width and sits in the middle third.
        let raw = frame(1800, 1200, |x, y| match y {
            0..400 => 0xff00ff00,
            800.. => 0xffffff00,
            _ if x < 900 => 0xffff0000,
            _ => 0xff0000ff,
        });
        let icon = fit(&raw, 60, 360, PreviewFit::Crop).unwrap();
        for y in [0, 30, 59] {
            assert_eq!(rgba(&icon, 2, y), [255, 0, 0, 255], "row {y}");
            assert_eq!(rgba(&icon, 357, y), [0, 0, 255, 255], "row {y}");
        }
    }

    #[test]
    fn a_reduction_averages_rather_than_picks() {
        // Fine black and white stripes become grey, not one or the other.
        let raw = frame(
            400,
            400,
            |x, _| if x % 2 == 0 { 0xff000000 } else { 0xffffffff },
        );
        let icon = fit(&raw, 10, 10, PreviewFit::Stretch).unwrap();
        let [r, ..] = rgba(&icon, 5, 5);
        assert!((100..156).contains(&r), "{r}");
    }
}
