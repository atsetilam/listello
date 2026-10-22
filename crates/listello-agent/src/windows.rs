//! Where windows come from (D4).
//!
//! Linux has no single way to list another client's windows, so the agent talks to one
//! [`WindowSource`] picked at start-up by probing the session; KDE's is the first and only
//! backend (`kwin.rs`). Everything above this trait -- the message loop, the Dock -- is
//! compositor-blind.
//!
//! A compositor decides which clients may see other clients' windows, and the answer is never
//! "any client that asks" (D11 amendment). So a backend can come up and still see nothing; it
//! says so with a [`Limitation`] rather than reporting an empty list that looks like our bug.

use listello_protocol::{Limitation, WindowCommand, WindowId, WindowInfo};
use std::os::fd::BorrowedFd;

/// What a backend needs from the loop: a descriptor to wait on, a way to be told it is
/// readable, and the current list.
pub trait WindowSource {
    /// Becomes readable when [`WindowSource::dispatch`] has something to do.
    fn poll_fd(&self) -> BorrowedFd<'_>;

    /// Read whatever arrived and flush what is queued. `Ok(true)` when the window list
    /// changed. An `Err` means the connection to the compositor is gone for good.
    fn dispatch(&mut self) -> anyhow::Result<bool>;

    /// Every window worth a Dock entry, in the order each first appeared -- *not* stacking
    /// order. Stacking order reshuffles on every focus change, and a Dock whose icons jump
    /// around whenever you click is unusable; a window keeps its place until it closes.
    fn windows(&self) -> Vec<WindowInfo>;

    /// Raise and focus a window, un-minimising it if need be. A stale id does nothing.
    fn activate(&mut self, id: WindowId);

    /// What the window's title-bar buttons do. A stale id does nothing.
    fn command(&mut self, id: WindowId, command: WindowCommand);

    /// The compositor's own name for a window, for asking it for a picture of it. `None` for
    /// a stale id.
    fn capture_handle(&self, id: WindowId) -> Option<String>;

    /// Ask the compositor for a video stream of a window, for live previews. `false` when it
    /// offers none to this process or the id is stale; otherwise the answer comes later
    /// through [`WindowSource::stream_events`].
    fn start_stream(&mut self, _id: WindowId) -> bool {
        false
    }

    /// Done with a window's stream. Unknown ids do nothing.
    fn end_stream(&mut self, _id: WindowId) {}

    /// What the compositor said about the streams asked for since the last call.
    fn stream_events(&mut self) -> Vec<(WindowId, StreamEvent)> {
        Vec::new()
    }
}

/// The compositor's answer about one window's stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamEvent {
    /// The stream is the PipeWire node with this id.
    Ready(u32),
    /// There will be no stream, or there is none any more.
    Ended(String),
}

/// The result of probing the session for a backend.
pub enum Probe {
    Ready(Box<dyn WindowSource>),
    /// No backend, and why, in words a person can act on.
    Unavailable(Limitation),
}

/// Pick a backend for this session. Only KDE for now (D4).
pub fn probe() -> Probe {
    crate::kwin::probe()
}

#[cfg(test)]
pub use fake::FakeWindowSource;

#[cfg(test)]
mod fake {
    use super::*;
    use std::os::fd::AsFd;
    use std::os::unix::net::UnixStream;

    /// A scripted window list for tests of everything above the trait.
    pub struct FakeWindowSource {
        pub windows: Vec<WindowInfo>,
        pub activated: Vec<WindowId>,
        pub commands: Vec<(WindowId, WindowCommand)>,
        pub changed: bool,
        /// Whether `start_stream` succeeds; the streams asked for and ended, in order.
        pub can_stream: bool,
        pub streams_started: Vec<WindowId>,
        pub streams_ended: Vec<WindowId>,
        /// Handed out by the next `stream_events`.
        pub stream_events: Vec<(WindowId, StreamEvent)>,
        wake: UnixStream,
    }

    impl FakeWindowSource {
        pub fn new(windows: Vec<WindowInfo>) -> FakeWindowSource {
            let (wake, _other) = UnixStream::pair().expect("socketpair");
            FakeWindowSource {
                windows,
                activated: Vec::new(),
                commands: Vec::new(),
                changed: false,
                can_stream: false,
                streams_started: Vec::new(),
                streams_ended: Vec::new(),
                stream_events: Vec::new(),
                wake,
            }
        }
    }

    impl WindowSource for FakeWindowSource {
        fn poll_fd(&self) -> BorrowedFd<'_> {
            self.wake.as_fd()
        }
        fn dispatch(&mut self) -> anyhow::Result<bool> {
            Ok(std::mem::take(&mut self.changed))
        }
        fn windows(&self) -> Vec<WindowInfo> {
            self.windows.clone()
        }
        fn activate(&mut self, id: WindowId) {
            self.activated.push(id);
        }
        fn command(&mut self, id: WindowId, command: WindowCommand) {
            self.commands.push((id, command));
        }
        fn capture_handle(&self, id: WindowId) -> Option<String> {
            self.windows
                .iter()
                .any(|w| w.id == id)
                .then(|| format!("fake-{}", id.0))
        }
        fn start_stream(&mut self, id: WindowId) -> bool {
            if self.can_stream {
                self.streams_started.push(id);
            }
            self.can_stream
        }
        fn end_stream(&mut self, id: WindowId) {
            self.streams_ended.push(id);
        }
        fn stream_events(&mut self) -> Vec<(WindowId, StreamEvent)> {
            std::mem::take(&mut self.stream_events)
        }
    }
}
