//! Live window previews (D7, `"live"`): one PipeWire stream per window in the open row.
//!
//! KWin hands over a stream per window through `zkde_screencast_unstable_v1` (`kwin.rs`) as a
//! PipeWire node id; this connects to that node and reduces each frame, straight from the
//! shared buffer, to the strip-sized picture the row asks for. KWin sends a frame only when the
//! window changed, and no more often than the rate negotiated here, so a still window costs
//! nothing after its first frame.
//!
//! PipeWire runs on its own loop, which the agent's single thread drives like any other
//! descriptor: [`Live::fd`] in the poll set, [`Live::dispatch`] when it is readable. The
//! stream callbacks run inside `dispatch` and leave their pictures in a queue.

use crate::capture::{self, Raw};
use listello_protocol::{Icon, PreviewFit, WindowId};
use pipewire as pw;
use pw::spa;
use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::os::fd::BorrowedFd;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// At most this many pictures a second per window: a video in a 60-pixel picture looks alive at
/// ten, and each one costs KWin a readback of the whole window.
pub const MAX_FPS: u32 = 10;

/// Frames closer together than this are dropped here even if KWin sent them, so a compositor
/// that ignores the negotiated rate still cannot run the agent hot. Half the interval, so a
/// compositor that keeps to the rate never trips it.
const MIN_GAP: Duration = Duration::from_millis(1000 / MAX_FPS as u64 / 2);

/// The picture the row wants, the same for every window in it.
#[derive(Clone, Copy)]
struct Target {
    height: u32,
    max_width: u32,
    fit: PreviewFit,
}

/// What the callbacks leave for the loop.
struct Shared {
    target: Cell<Target>,
    frames: RefCell<Vec<(WindowId, Icon)>>,
    broken: RefCell<Vec<WindowId>>,
}

/// A stream's negotiated format, and when its last frame was used.
#[derive(Default)]
struct Negotiated {
    /// As one of the `QImage::Format` values `capture::fit` reads.
    format: Option<u64>,
    width: usize,
    height: usize,
    last: Option<Instant>,
}

struct Stream {
    stream: pw::stream::StreamRc,
    _listener: pw::stream::StreamListener<Negotiated>,
}

pub struct Live {
    mainloop: pw::main_loop::MainLoopRc,
    _context: pw::context::ContextRc,
    core: pw::core::CoreRc,
    streams: HashMap<WindowId, Stream>,
    shared: Rc<Shared>,
}

impl Live {
    /// `None` when there is no PipeWire to talk to; previews then stay stills.
    pub fn connect() -> Option<Live> {
        pw::init();
        let connected = (|| {
            let mainloop = pw::main_loop::MainLoopRc::new(None)?;
            let context = pw::context::ContextRc::new(&mainloop, None)?;
            let core = context.connect_rc(None)?;
            Ok::<_, pw::Error>((mainloop, context, core))
        })();
        match connected {
            Ok((mainloop, context, core)) => Some(Live {
                mainloop,
                _context: context,
                core,
                streams: HashMap::new(),
                shared: Rc::new(Shared {
                    target: Cell::new(Target {
                        height: 1,
                        max_width: 1,
                        fit: PreviewFit::Whole,
                    }),
                    frames: RefCell::new(Vec::new()),
                    broken: RefCell::new(Vec::new()),
                }),
            }),
            Err(e) => {
                eprintln!("listello-agent: no PipeWire, so live previews fall back to stills: {e}");
                None
            }
        }
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        self.mainloop.loop_().fd()
    }

    /// Run whatever PipeWire has ready, without waiting.
    pub fn dispatch(&self) {
        self.mainloop.loop_().iterate(pw::loop_::Timeout::None);
    }

    pub fn set_target(&self, height: u32, max_width: u32, fit: PreviewFit) {
        self.shared.target.set(Target {
            height: height.max(1),
            max_width: max_width.max(1),
            fit,
        });
    }

    /// Start reading window `id` from PipeWire node `node`. `false` if PipeWire refused.
    pub fn open(&mut self, id: WindowId, node: u32) -> bool {
        self.close(id);
        match self.open_stream(id, node) {
            Ok(stream) => {
                self.streams.insert(id, stream);
                true
            }
            Err(e) => {
                eprintln!(
                    "listello-agent: cannot read the stream of window {}: {e}",
                    id.0
                );
                false
            }
        }
    }

    pub fn close(&mut self, id: WindowId) {
        if let Some(stream) = self.streams.remove(&id) {
            let _ = stream.stream.disconnect();
        }
    }

    /// Pictures made since the last call, oldest first.
    pub fn take_frames(&self) -> Vec<(WindowId, Icon)> {
        std::mem::take(&mut self.shared.frames.borrow_mut())
    }

    /// Streams that went wrong since the last call. They are closed already.
    pub fn take_broken(&mut self) -> Vec<WindowId> {
        let broken = std::mem::take(&mut *self.shared.broken.borrow_mut());
        for &id in &broken {
            self.close(id);
        }
        broken
    }

    fn open_stream(&self, id: WindowId, node: u32) -> Result<Stream, pw::Error> {
        let stream = pw::stream::StreamRc::new(
            self.core.clone(),
            "listello-preview",
            pw::properties::properties! {
                *pw::keys::MEDIA_TYPE => "Video",
                *pw::keys::MEDIA_CATEGORY => "Capture",
                *pw::keys::MEDIA_ROLE => "Screen",
            },
        )?;
        let shared = self.shared.clone();
        let broken = self.shared.clone();
        let listener = stream
            .add_local_listener_with_user_data(Negotiated::default())
            .state_changed(move |_, _, _, new| {
                if let pw::stream::StreamState::Error(why) = new {
                    eprintln!(
                        "listello-agent: the stream of window {} failed: {why}",
                        id.0
                    );
                    broken.broken.borrow_mut().push(id);
                }
            })
            .param_changed(|_, negotiated, kind, param| {
                if kind != spa::param::ParamType::Format.as_raw() {
                    return;
                }
                let Some(param) = param else {
                    return;
                };
                let mut info = spa::param::video::VideoInfoRaw::new();
                if info.parse(param).is_err() {
                    return;
                }
                negotiated.format = qt_format(info.format());
                negotiated.width = info.size().width as usize;
                negotiated.height = info.size().height as usize;
            })
            .process(move |stream, negotiated| {
                let Some(mut buffer) = stream.dequeue_buffer() else {
                    return;
                };
                let now = Instant::now();
                if negotiated
                    .last
                    .is_some_and(|last| now.saturating_duration_since(last) < MIN_GAP)
                {
                    return;
                }
                let Some(format) = negotiated.format else {
                    return;
                };
                let Some(data) = buffer.datas_mut().first_mut() else {
                    return;
                };
                let chunk = data.chunk();
                // An empty or damaged chunk carries no picture: KWin sends those for cursor
                // moves and the like.
                if chunk.size() == 0 || chunk.flags().contains(spa::buffer::ChunkFlags::CORRUPTED) {
                    return;
                }
                let (offset, stride) = (chunk.offset() as usize, chunk.stride());
                let stride = if stride > 0 {
                    stride as usize
                } else {
                    negotiated.width * 4
                };
                let Some(bytes) = data.data().and_then(|b| b.get(offset..)) else {
                    return;
                };
                let raw = Raw {
                    bytes: Cow::Borrowed(bytes),
                    width: negotiated.width,
                    height: negotiated.height,
                    stride,
                    format,
                };
                let target = shared.target.get();
                if let Some(icon) = capture::fit(&raw, target.height, target.max_width, target.fit)
                {
                    negotiated.last = Some(now);
                    shared.frames.borrow_mut().push((id, icon));
                }
            })
            .register()?;

        let params = format_params();
        let mut params = [spa::pod::Pod::from_bytes(&params).expect("a pod we just built")];
        stream.connect(
            spa::utils::Direction::Input,
            Some(node),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )?;
        Ok(Stream {
            stream,
            _listener: listener,
        })
    }
}

/// A PipeWire video format as the `QImage::Format` value `capture::fit` reads. The BGR ones
/// are bytes B, G, R, A in memory, which is Qt's native-word layout only on little-endian.
fn qt_format(format: spa::param::video::VideoFormat) -> Option<u64> {
    use spa::param::video::VideoFormat;
    let little = cfg!(target_endian = "little");
    match format {
        VideoFormat::RGBA => Some(capture::RGBA8888_PREMULTIPLIED),
        VideoFormat::RGBx => Some(capture::RGBX8888),
        VideoFormat::BGRA if little => Some(capture::ARGB32_PREMULTIPLIED),
        VideoFormat::BGRx if little => Some(capture::RGB32),
        _ => None,
    }
}

/// What this consumer accepts: the four 32-bit layouts, any size, in shared memory (no
/// modifier is offered, so KWin does not pick DMA-BUF), at most [`MAX_FPS`].
fn format_params() -> Vec<u8> {
    use spa::param::format::{FormatProperties, MediaSubtype, MediaType};
    use spa::param::video::VideoFormat;
    use spa::pod::{object, property};
    use spa::utils::{Fraction, Rectangle};
    let object = object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        property!(FormatProperties::MediaType, Id, MediaType::Video),
        property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
        property!(
            FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            VideoFormat::BGRx,
            VideoFormat::BGRx,
            VideoFormat::BGRA,
            VideoFormat::RGBx,
            VideoFormat::RGBA
        ),
        property!(
            FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            Rectangle {
                width: 1920,
                height: 1080
            },
            Rectangle {
                width: 1,
                height: 1
            },
            Rectangle {
                width: 8192,
                height: 8192
            }
        ),
        property!(
            FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            Fraction { num: 0, denom: 1 },
            Fraction { num: 0, denom: 1 },
            Fraction { num: 360, denom: 1 }
        ),
        property!(
            FormatProperties::VideoMaxFramerate,
            Choice,
            Range,
            Fraction,
            Fraction {
                num: MAX_FPS,
                denom: 1
            },
            Fraction { num: 1, denom: 1 },
            Fraction {
                num: MAX_FPS,
                denom: 1
            }
        ),
    );
    spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(object),
    )
    .expect("a format pod serialises")
    .0
    .into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_format_offer_parses_back_as_a_pod() {
        let bytes = format_params();
        assert!(spa::pod::Pod::from_bytes(&bytes).is_some());
    }

    #[test]
    fn only_formats_the_reducer_reads_are_mapped() {
        use spa::param::video::VideoFormat;
        assert_eq!(
            qt_format(VideoFormat::RGBA),
            Some(capture::RGBA8888_PREMULTIPLIED)
        );
        assert_eq!(qt_format(VideoFormat::I420), None);
    }
}
