//! Keeping the window row's previews fresh while the renderer has it open (D7).
//!
//! The renderer says which windows the row shows; this captures each at once and again every
//! [`REFRESH_EVERY`], and sends only the pictures that changed. Nothing is captured while no
//! row is open — the renderer sends an empty list on close, and a disconnect counts as one.
//!
//! Asked for live previews, it also asks the compositor for a stream of each window and, once
//! one flows, sends its frames instead of stills. A window whose stream cannot be had keeps
//! getting stills, so live is never worse than snapshots.

use crate::capture::{CaptureError, Capturer};
use crate::live::Live;
use crate::windows::{StreamEvent, WindowSource};
use listello_protocol::{AgentToCore, Icon, Limitation, PreviewFit, WindowId};
use std::collections::HashMap;
use std::os::fd::BorrowedFd;
use std::time::{Duration, Instant};

/// How stale a preview may get while the row is fresh. A second and a half is lively enough to
/// see a video move in a 60-pixel picture, and few enough captures that an open row costs
/// little.
pub const REFRESH_EVERY: Duration = Duration::from_millis(1500);

/// A row left open is a row nobody is looking at closely: after this, refresh slowly...
const SLOW_AFTER: Duration = Duration::from_secs(10);
const SLOW_REFRESH: Duration = Duration::from_secs(5);
/// ...and after this, not at all. The pictures stay; they just stop changing.
const STOP_AFTER: Duration = Duration::from_secs(60);

/// How long to wait before the next capture, for a row open this long; `None` for never.
fn refresh_after(open_for: Duration) -> Option<Duration> {
    if open_for < SLOW_AFTER {
        Some(REFRESH_EVERY)
    } else if open_for < STOP_AFTER {
        Some(SLOW_REFRESH)
    } else {
        None
    }
}

/// Where pictures come from. [`Capturer`] in the daemon, a fake in the tests.
pub trait Grab {
    fn grab(
        &self,
        handle: &str,
        height: u32,
        max_width: u32,
        how: PreviewFit,
    ) -> Result<Icon, CaptureError>;
}

impl Grab for Capturer {
    fn grab(
        &self,
        handle: &str,
        height: u32,
        max_width: u32,
        how: PreviewFit,
    ) -> Result<Icon, CaptureError> {
        self.capture(handle, height, max_width, how)
    }
}

/// Where live frames come from. [`Live`] in the daemon, a fake in the tests.
pub trait Streams {
    fn fd(&self) -> BorrowedFd<'_>;
    fn dispatch(&self);
    fn set_target(&self, height: u32, max_width: u32, fit: PreviewFit);
    /// Start reading `id` from the compositor's PipeWire node. `false` if that failed.
    fn open(&mut self, id: WindowId, node: u32) -> bool;
    fn close(&mut self, id: WindowId);
    fn take_frames(&self) -> Vec<(WindowId, Icon)>;
    /// Streams that broke since the last call, already closed.
    fn take_broken(&mut self) -> Vec<WindowId>;
}

impl Streams for Live {
    fn fd(&self) -> BorrowedFd<'_> {
        Live::fd(self)
    }
    fn dispatch(&self) {
        Live::dispatch(self)
    }
    fn set_target(&self, height: u32, max_width: u32, fit: PreviewFit) {
        Live::set_target(self, height, max_width, fit)
    }
    fn open(&mut self, id: WindowId, node: u32) -> bool {
        Live::open(self, id, node)
    }
    fn close(&mut self, id: WindowId) {
        Live::close(self, id)
    }
    fn take_frames(&self) -> Vec<(WindowId, Icon)> {
        Live::take_frames(self)
    }
    fn take_broken(&mut self) -> Vec<WindowId> {
        Live::take_broken(self)
    }
}

/// How a window in a live row is being fed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Feed {
    /// Stream asked of the compositor, not yet answered.
    Asked,
    /// Frames are coming from PipeWire.
    Streaming,
    /// No stream to be had: stills, as in snapshots mode.
    Stills,
}

#[derive(Default)]
pub struct Previews {
    grabber: Option<Box<dyn Grab>>,
    streams: Option<Box<dyn Streams>>,
    /// The open row asked for live previews.
    live: bool,
    /// Per window of a live row; a window not here has not been asked about yet.
    feeds: HashMap<WindowId, Feed>,
    /// Streams to end on the next poll, which has the compositor at hand.
    to_end: Vec<WindowId>,
    /// The first capture of a row is done: from then on stills only go to [`Feed::Stills`].
    captured_once: bool,
    watching: Vec<WindowId>,
    height: u32,
    max_width: u32,
    fit: PreviewFit,
    next_capture: Option<Instant>,
    /// When the row opened: refreshes slow down the longer it stays open.
    opened: Option<Instant>,
    /// What the renderer was last sent per window, so an unchanged picture is not resent.
    sent: HashMap<WindowId, Option<Icon>>,
    /// The compositor said no. Permanent for this process: nothing more is attempted.
    refused: bool,
}

impl Previews {
    pub fn new(grabber: Option<Box<dyn Grab>>) -> Previews {
        Previews {
            grabber,
            ..Previews::default()
        }
    }

    /// Supply live frames from `streams`, when there are any.
    pub fn with_streams(mut self, streams: Option<Box<dyn Streams>>) -> Previews {
        self.streams = streams;
        self
    }

    /// PipeWire's descriptor, to wait on while a live row is open.
    pub fn live_fd(&self) -> Option<BorrowedFd<'_>> {
        self.streams.as_ref().map(|s| s.fd())
    }

    /// PipeWire's descriptor is readable.
    pub fn dispatch_live(&self) {
        if let Some(streams) = &self.streams {
            streams.dispatch();
        }
    }

    /// The renderer's [`listello_protocol::CoreToAgent::WatchPreviews`].
    pub fn watch(
        &mut self,
        windows: Vec<WindowId>,
        height: u32,
        max_width: u32,
        fit: PreviewFit,
        live: bool,
        now: Instant,
    ) {
        self.watching = windows;
        self.fit = fit;
        self.height = height.max(1);
        self.max_width = max_width.max(1);
        self.sent.clear();
        self.next_capture = (!self.watching.is_empty()).then_some(now);
        self.opened = self.next_capture;
        self.captured_once = false;
        // A new row starts every stream afresh: its size may differ, and so may its windows.
        self.to_end.extend(self.feeds.drain().map(|(id, _)| id));
        self.live = live && self.streams.is_some() && !self.watching.is_empty();
        if let (true, Some(streams)) = (self.live, &self.streams) {
            streams.set_target(self.height, self.max_width, fit);
        }
    }

    /// The renderer went away, or a new one arrived: it is watching nothing.
    pub fn stop(&mut self) {
        self.watch(Vec::new(), 0, 0, PreviewFit::Whole, false, Instant::now());
    }

    pub fn until_due(&self, now: Instant) -> Option<Duration> {
        self.next_capture
            .map(|at| at.saturating_duration_since(now))
    }

    /// Whether PipeWire's descriptor is worth waiting on.
    pub fn streaming(&self) -> bool {
        self.live
    }

    /// Capture whatever is due, pass on live frames, and return the pictures that changed.
    pub fn poll(
        &mut self,
        mut source: Option<&mut (dyn WindowSource + 'static)>,
        now: Instant,
    ) -> Vec<AgentToCore> {
        let mut out = Vec::new();
        for id in std::mem::take(&mut self.to_end) {
            if let Some(source) = source.as_deref_mut() {
                source.end_stream(id);
            }
            if let Some(streams) = &mut self.streams {
                streams.close(id);
            }
        }
        if self.live {
            self.poll_live(source.as_deref_mut(), &mut out);
        }
        out.extend(self.poll_stills(source.as_deref(), now));
        out
    }

    /// Start, follow and end the open row's streams, and send their frames.
    fn poll_live(
        &mut self,
        mut source: Option<&mut (dyn WindowSource + 'static)>,
        out: &mut Vec<AgentToCore>,
    ) {
        let Some(streams) = &mut self.streams else {
            return;
        };
        let events = source
            .as_deref_mut()
            .map(|s| s.stream_events())
            .unwrap_or_default();
        let mut fell_back = false;
        for (id, event) in events {
            let Some(feed) = self.feeds.get_mut(&id) else {
                continue;
            };
            *feed = match event {
                StreamEvent::Ready(node) if *feed == Feed::Asked && streams.open(id, node) => {
                    Feed::Streaming
                }
                StreamEvent::Ready(_) => {
                    if let Some(source) = source.as_deref_mut() {
                        source.end_stream(id);
                    }
                    Feed::Stills
                }
                StreamEvent::Ended(why) => {
                    eprintln!("listello-agent: no stream of window {}: {why}", id.0);
                    streams.close(id);
                    Feed::Stills
                }
            };
            fell_back |= *feed == Feed::Stills;
        }
        for id in streams.take_broken() {
            if let Some(source) = source.as_deref_mut() {
                source.end_stream(id);
            }
            if let Some(feed) = self.feeds.get_mut(&id) {
                *feed = Feed::Stills;
                fell_back = true;
            }
        }

        let windows = source.as_deref().map(|s| s.windows()).unwrap_or_default();
        for &id in &self.watching {
            let minimized = windows.iter().find(|w| w.id == id).map(|w| w.minimized);
            match (minimized, self.feeds.get(&id)) {
                // Shown and not yet asked about: ask.
                (Some(false), None) => {
                    let asked = source.as_deref_mut().is_some_and(|s| s.start_stream(id));
                    let feed = if asked { Feed::Asked } else { Feed::Stills };
                    fell_back |= feed == Feed::Stills;
                    self.feeds.insert(id, feed);
                }
                // Minimised or gone: KWin paints nothing, so end the stream, and ask again if
                // the window comes back.
                (Some(true) | None, Some(_)) => {
                    self.feeds.remove(&id);
                    if let Some(source) = source.as_deref_mut() {
                        source.end_stream(id);
                    }
                    streams.close(id);
                    if self.sent.get(&id) != Some(&None) {
                        self.sent.insert(id, None);
                        out.push(AgentToCore::Preview { id, image: None });
                    }
                }
                _ => {}
            }
        }

        for (id, image) in streams.take_frames() {
            if self.feeds.get(&id) != Some(&Feed::Streaming) {
                continue;
            }
            let image = Some(image);
            if self.sent.get(&id) != Some(&image) {
                self.sent.insert(id, image.clone());
                out.push(AgentToCore::Preview { id, image });
            }
        }

        // A window that just lost its stream gets a still at once, then on the usual schedule.
        if fell_back && self.captured_once {
            self.next_capture = Some(Instant::now());
        }
    }

    /// Whether a window gets stills: always, unless a stream feeds or is about to feed it.
    fn wants_stills(&self, id: WindowId) -> bool {
        !self.live
            || !self.captured_once
            || matches!(self.feeds.get(&id), None | Some(Feed::Stills))
    }

    fn poll_stills(&mut self, source: Option<&dyn WindowSource>, now: Instant) -> Vec<AgentToCore> {
        match self.next_capture {
            Some(at) if at <= now => {}
            _ => return Vec::new(),
        }
        let open_for = self
            .opened
            .map_or(Duration::ZERO, |at| now.saturating_duration_since(at));
        self.next_capture = refresh_after(open_for).map(|wait| now + wait);
        let windows = source.map(|s| s.windows()).unwrap_or_default();
        let mut out = Vec::new();
        let first = !self.captured_once;
        self.captured_once = true;
        for &id in &self.watching {
            // The first round captures every window, so the row is never blank while streams
            // start; after that a streamed window is the stream's alone.
            if !first && !self.wants_stills(id) {
                continue;
            }
            let minimized = windows.iter().find(|w| w.id == id).map(|w| w.minimized);
            // A minimised window is not painted, so there is nothing worth asking for.
            let image = match (minimized, &self.grabber, self.refused) {
                (Some(false), Some(grabber), false) => {
                    let handle = source.and_then(|s| s.capture_handle(id));
                    let grabbed =
                        handle.map(|h| grabber.grab(&h, self.height, self.max_width, self.fit));
                    match grabbed {
                        Some(Ok(icon)) => Some(icon),
                        Some(Err(CaptureError::Refused)) => {
                            eprintln!(
                                "listello-agent: KWin refuses window pictures to this \
                                 executable; its .desktop file must declare \
                                 X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2"
                            );
                            self.refused = true;
                            out.push(AgentToCore::Limited(Limitation::NoWindowCapture));
                            None
                        }
                        Some(Err(CaptureError::Failed(why))) => {
                            eprintln!("listello-agent: no picture of window {}: {why}", id.0);
                            None
                        }
                        None => None,
                    }
                }
                _ => None,
            };
            if self.sent.get(&id) != Some(&image) {
                self.sent.insert(id, image.clone());
                out.push(AgentToCore::Preview { id, image });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::windows::FakeWindowSource;
    use listello_protocol::{AppId, WindowInfo};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    /// Answers with a 1×1 picture whose colour is `shade`, and counts the calls.
    struct FakeGrab {
        shade: Rc<Cell<u8>>,
        calls: Rc<RefCell<Vec<String>>>,
        refuse: bool,
    }

    impl Grab for FakeGrab {
        fn grab(&self, handle: &str, _: u32, _: u32, _: PreviewFit) -> Result<Icon, CaptureError> {
            self.calls.borrow_mut().push(handle.to_string());
            if self.refuse {
                return Err(CaptureError::Refused);
            }
            Ok(Icon::new(1, 1, vec![self.shade.get(), 0, 0, 255]).unwrap())
        }
    }

    fn window(id: u64, minimized: bool) -> WindowInfo {
        WindowInfo {
            id: WindowId(id),
            app_id: AppId::new("a"),
            title: String::new(),
            active: false,
            minimized,
        }
    }

    type Calls = Rc<RefCell<Vec<String>>>;

    fn setup(refuse: bool) -> (Previews, Rc<Cell<u8>>, Calls) {
        let shade = Rc::new(Cell::new(10));
        let calls = Rc::new(RefCell::new(Vec::new()));
        let grab = FakeGrab {
            shade: shade.clone(),
            calls: calls.clone(),
            refuse,
        };
        (Previews::new(Some(Box::new(grab))), shade, calls)
    }

    #[test]
    fn nothing_is_captured_until_a_row_is_watched() {
        let (mut previews, _, calls) = setup(false);
        let mut source = FakeWindowSource::new(vec![window(1, false)]);
        assert_eq!(previews.until_due(Instant::now()), None);
        assert!(previews.poll(Some(&mut source), Instant::now()).is_empty());
        assert!(calls.borrow().is_empty());
    }

    #[test]
    fn a_watched_row_is_captured_at_once_then_only_changes_are_sent() {
        let (mut previews, shade, calls) = setup(false);
        let mut source = FakeWindowSource::new(vec![window(1, false), window(2, false)]);
        let t = Instant::now();
        previews.watch(
            vec![WindowId(1), WindowId(2)],
            60,
            100,
            PreviewFit::Whole,
            false,
            t,
        );

        let first = previews.poll(Some(&mut source), t);
        assert_eq!(first.len(), 2);
        assert_eq!(*calls.borrow(), ["fake-1", "fake-2"]);

        // Not due yet.
        assert!(previews
            .poll(Some(&mut source), t + Duration::from_millis(100))
            .is_empty());
        // Due, but nothing changed: captured, not sent.
        assert!(previews
            .poll(Some(&mut source), t + REFRESH_EVERY)
            .is_empty());
        assert_eq!(calls.borrow().len(), 4);
        // Changed: sent.
        shade.set(20);
        assert_eq!(
            previews
                .poll(Some(&mut source), t + REFRESH_EVERY * 2)
                .len(),
            2
        );
    }

    #[test]
    fn a_row_left_open_is_refreshed_more_slowly_then_not_at_all() {
        let (mut previews, _, calls) = setup(false);
        let mut source = FakeWindowSource::new(vec![window(1, false)]);
        let t = Instant::now();
        previews.watch(vec![WindowId(1)], 60, 100, PreviewFit::Whole, false, t);
        previews.poll(Some(&mut source), t);
        assert_eq!(previews.until_due(t), Some(REFRESH_EVERY));
        let later = t + SLOW_AFTER;
        previews.poll(Some(&mut source), later);
        assert_eq!(previews.until_due(later), Some(SLOW_REFRESH));
        let last = t + STOP_AFTER;
        previews.poll(Some(&mut source), last);
        assert_eq!(previews.until_due(last), None);
        assert_eq!(calls.borrow().len(), 3);
    }

    #[test]
    fn closing_the_row_stops_capturing() {
        let (mut previews, _, calls) = setup(false);
        let mut source = FakeWindowSource::new(vec![window(1, false)]);
        let t = Instant::now();
        previews.watch(vec![WindowId(1)], 60, 100, PreviewFit::Whole, false, t);
        previews.poll(Some(&mut source), t);
        previews.watch(Vec::new(), 60, 100, PreviewFit::Whole, false, t);
        assert_eq!(previews.until_due(t), None);
        assert!(previews
            .poll(Some(&mut source), t + REFRESH_EVERY)
            .is_empty());
        assert_eq!(calls.borrow().len(), 1);
    }

    #[test]
    fn minimised_and_stale_windows_get_no_picture_and_no_capture() {
        let (mut previews, _, calls) = setup(false);
        let mut source = FakeWindowSource::new(vec![window(1, true)]);
        let t = Instant::now();
        previews.watch(
            vec![WindowId(1), WindowId(9)],
            60,
            100,
            PreviewFit::Whole,
            false,
            t,
        );
        assert_eq!(
            previews.poll(Some(&mut source), t),
            vec![
                AgentToCore::Preview {
                    id: WindowId(1),
                    image: None
                },
                AgentToCore::Preview {
                    id: WindowId(9),
                    image: None
                },
            ]
        );
        assert!(calls.borrow().is_empty());
    }

    /// Live frames on demand: `push` stands in for PipeWire delivering one.
    #[derive(Default)]
    struct FakeStreams {
        frames: Rc<RefCell<Vec<(WindowId, Icon)>>>,
        opened: Rc<RefCell<Vec<(WindowId, u32)>>>,
        closed: Rc<RefCell<Vec<WindowId>>>,
        broken: Rc<RefCell<Vec<WindowId>>>,
        wake: Option<std::os::unix::net::UnixStream>,
    }

    impl Streams for FakeStreams {
        fn fd(&self) -> BorrowedFd<'_> {
            use std::os::fd::AsFd;
            self.wake.as_ref().expect("a socket").as_fd()
        }
        fn dispatch(&self) {}
        fn set_target(&self, _: u32, _: u32, _: PreviewFit) {}
        fn open(&mut self, id: WindowId, node: u32) -> bool {
            self.opened.borrow_mut().push((id, node));
            true
        }
        fn close(&mut self, id: WindowId) {
            self.closed.borrow_mut().push(id);
        }
        fn take_frames(&self) -> Vec<(WindowId, Icon)> {
            std::mem::take(&mut self.frames.borrow_mut())
        }
        fn take_broken(&mut self) -> Vec<WindowId> {
            std::mem::take(&mut self.broken.borrow_mut())
        }
    }

    struct LiveRig {
        previews: Previews,
        source: FakeWindowSource,
        grabs: Calls,
        frames: Rc<RefCell<Vec<(WindowId, Icon)>>>,
        opened: Rc<RefCell<Vec<(WindowId, u32)>>>,
        broken: Rc<RefCell<Vec<WindowId>>>,
    }

    /// A live row over windows 1 and 2, polled once: stills taken, streams asked for.
    fn live_rig(can_stream: bool) -> (LiveRig, Instant) {
        let (previews, _, grabs) = setup(false);
        let streams = FakeStreams {
            wake: std::os::unix::net::UnixStream::pair().ok().map(|p| p.0),
            ..FakeStreams::default()
        };
        let (frames, opened, broken) = (
            streams.frames.clone(),
            streams.opened.clone(),
            streams.broken.clone(),
        );
        let mut previews = previews.with_streams(Some(Box::new(streams)));
        let mut source = FakeWindowSource::new(vec![window(1, false), window(2, false)]);
        source.can_stream = can_stream;
        let t = Instant::now();
        previews.watch(
            vec![WindowId(1), WindowId(2)],
            60,
            100,
            PreviewFit::Whole,
            true,
            t,
        );
        assert_eq!(previews.poll(Some(&mut source), t).len(), 2);
        let rig = LiveRig {
            previews,
            source,
            grabs,
            frames,
            opened,
            broken,
        };
        (rig, t)
    }

    fn frame(shade: u8) -> Icon {
        Icon::new(1, 1, vec![0, shade, 0, 255]).unwrap()
    }

    #[test]
    fn a_live_row_starts_with_stills_then_sends_stream_frames_instead() {
        let (mut rig, t) = live_rig(true);
        assert_eq!(rig.source.streams_started, [WindowId(1), WindowId(2)]);
        assert_eq!(rig.grabs.borrow().len(), 2);

        rig.source.stream_events = vec![
            (WindowId(1), StreamEvent::Ready(41)),
            (WindowId(2), StreamEvent::Ready(42)),
        ];
        rig.previews.poll(Some(&mut rig.source), t);
        assert_eq!(*rig.opened.borrow(), [(WindowId(1), 41), (WindowId(2), 42)]);

        rig.frames.borrow_mut().push((WindowId(2), frame(7)));
        let out = rig.previews.poll(Some(&mut rig.source), t);
        assert_eq!(
            out,
            vec![AgentToCore::Preview {
                id: WindowId(2),
                image: Some(frame(7))
            }]
        );
        // Streamed windows get no more stills.
        rig.previews.poll(Some(&mut rig.source), t + REFRESH_EVERY);
        assert_eq!(rig.grabs.borrow().len(), 2);
        // The same frame again is not resent.
        rig.frames.borrow_mut().push((WindowId(2), frame(7)));
        assert!(rig.previews.poll(Some(&mut rig.source), t).is_empty());
    }

    #[test]
    fn without_streams_a_live_row_is_a_snapshot_row() {
        let (mut rig, t) = live_rig(false);
        assert!(rig.source.streams_started.is_empty());
        rig.previews.poll(Some(&mut rig.source), t + REFRESH_EVERY);
        assert_eq!(rig.grabs.borrow().len(), 4);
    }

    #[test]
    fn a_stream_that_fails_or_breaks_falls_back_to_stills_at_once() {
        let (mut rig, t) = live_rig(true);
        rig.source.stream_events = vec![
            (WindowId(1), StreamEvent::Ready(41)),
            (WindowId(2), StreamEvent::Ended("no".into())),
        ];
        rig.previews.poll(Some(&mut rig.source), t);
        // Window 2 is due a still straight away; window 1 streams.
        rig.previews.poll(
            Some(&mut rig.source),
            Instant::now() + Duration::from_millis(1),
        );
        assert_eq!(rig.grabs.borrow()[2..], ["fake-2"]);

        rig.broken.borrow_mut().push(WindowId(1));
        rig.previews.poll(Some(&mut rig.source), Instant::now());
        assert!(rig.source.streams_ended.contains(&WindowId(1)));
        rig.previews.poll(
            Some(&mut rig.source),
            Instant::now() + Duration::from_millis(1),
        );
        assert!(rig.grabs.borrow()[3..].contains(&"fake-1".to_string()));
    }

    #[test]
    fn a_minimised_window_loses_its_stream_and_closing_the_row_ends_them_all() {
        let (mut rig, t) = live_rig(true);
        rig.source.stream_events = vec![
            (WindowId(1), StreamEvent::Ready(41)),
            (WindowId(2), StreamEvent::Ready(42)),
        ];
        rig.previews.poll(Some(&mut rig.source), t);
        rig.source.windows[0].minimized = true;
        let out = rig.previews.poll(Some(&mut rig.source), t);
        assert_eq!(
            out,
            vec![AgentToCore::Preview {
                id: WindowId(1),
                image: None
            }]
        );
        assert_eq!(rig.source.streams_ended, [WindowId(1)]);
        // A frame already queued for it is dropped.
        rig.frames.borrow_mut().push((WindowId(1), frame(3)));
        assert!(rig.previews.poll(Some(&mut rig.source), t).is_empty());

        rig.previews
            .watch(Vec::new(), 60, 100, PreviewFit::Whole, false, t);
        rig.previews.poll(Some(&mut rig.source), t);
        assert!(rig.source.streams_ended.contains(&WindowId(2)));
        assert!(!rig.previews.streaming());
    }

    #[test]
    fn a_refusal_is_reported_once_and_never_retried() {
        let (mut previews, _, calls) = setup(true);
        let mut source = FakeWindowSource::new(vec![window(1, false), window(2, false)]);
        let t = Instant::now();
        previews.watch(
            vec![WindowId(1), WindowId(2)],
            60,
            100,
            PreviewFit::Whole,
            false,
            t,
        );
        let out = previews.poll(Some(&mut source), t);
        let limits = out
            .iter()
            .filter(|m| matches!(m, AgentToCore::Limited(Limitation::NoWindowCapture)))
            .count();
        assert_eq!(limits, 1);
        assert_eq!(calls.borrow().len(), 1);
        previews.poll(Some(&mut source), t + REFRESH_EVERY);
        assert_eq!(calls.borrow().len(), 1);
    }
}
