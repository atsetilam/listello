//! One end of the socket between `listello-core` and `listello-agent`.
//!
//! [`crate::wire`] turns messages into bytes and back and does no I/O; this is the thin layer
//! that owns a non-blocking `UnixStream` and drives it. Both binaries use the same code, so the
//! handshake and the limits below cannot drift apart between them.
//!
//! The owner is a single-threaded loop (D3), so nothing here blocks:
//!
//! - [`Link::send`] queues a frame and writes as much as the kernel takes. What it cannot write
//!   stays queued until [`Link::flush`]; the owner asks [`Link::wants_write`] to know whether
//!   to wait for writability.
//! - [`Link::receive`] reads what is there and returns every whole message, then stops.
//!
//! # The handshake
//!
//! Each side sends its `Hello` first thing, and the first message received must be the peer's
//! `Hello` with the same [`crate::PROTOCOL_VERSION`]. Anything else is an error and the owner
//! closes the socket — it does not retry into a hot loop and it does not negotiate (D11). The
//! `Hello` itself is consumed here; the owner never sees it.
//!
//! # A wedged peer must not wedge us
//!
//! The renderer is a daemon that has to keep drawing. If the peer stops reading, the outgoing
//! queue grows; past [`MAX_OUTGOING_BYTES`] the link reports [`LinkError::Stalled`] and the
//! owner drops the connection. Incoming size is capped by [`crate::wire`] before any buffer
//! grows.

use crate::wire::{self, Decoder, WireError};
use crate::{AgentToCore, CoreToAgent, PROTOCOL_VERSION};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::marker::PhantomData;
use std::os::unix::net::UnixStream;

/// How much a peer may leave unread before we give up on it. An icon is at most a few hundred
/// KiB, so this is many messages of slack and still a bound.
pub const MAX_OUTGOING_BYTES: usize = 4 * 1024 * 1024;

/// A message type that has a `Hello` variant. Implemented for the two directions.
pub trait Handshake: Serialize + DeserializeOwned {
    fn hello() -> Self;
    /// The protocol version, if this message is a `Hello`.
    fn hello_version(&self) -> Option<u32>;
}

impl Handshake for AgentToCore {
    fn hello() -> Self {
        AgentToCore::Hello {
            protocol_version: PROTOCOL_VERSION,
        }
    }
    fn hello_version(&self) -> Option<u32> {
        match self {
            AgentToCore::Hello { protocol_version } => Some(*protocol_version),
            _ => None,
        }
    }
}

impl Handshake for CoreToAgent {
    fn hello() -> Self {
        CoreToAgent::Hello {
            protocol_version: PROTOCOL_VERSION,
        }
    }
    fn hello_version(&self) -> Option<u32> {
        match self {
            CoreToAgent::Hello { protocol_version } => Some(*protocol_version),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum LinkError {
    /// The peer closed the connection. The ordinary way for it to end.
    Closed,
    Io(io::Error),
    Wire(WireError),
    /// The peer's first message was not a `Hello`.
    NoHello,
    /// The peer speaks a different protocol version.
    VersionMismatch {
        theirs: u32,
    },
    /// The peer is not reading what we send. See [`MAX_OUTGOING_BYTES`].
    Stalled,
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkError::Closed => write!(f, "the peer closed the connection"),
            LinkError::Io(e) => write!(f, "socket error: {e}"),
            LinkError::Wire(e) => write!(f, "{e}"),
            LinkError::NoHello => write!(f, "the peer did not start with a Hello"),
            LinkError::VersionMismatch { theirs } => write!(
                f,
                "protocol version mismatch: ours is {PROTOCOL_VERSION}, theirs is {theirs} \
                 (the two binaries must be installed from the same build)"
            ),
            LinkError::Stalled => write!(f, "the peer stopped reading"),
        }
    }
}

impl std::error::Error for LinkError {}

impl From<WireError> for LinkError {
    fn from(e: WireError) -> Self {
        LinkError::Wire(e)
    }
}

/// One connected end. `R` is what we receive, `W` is what we send.
pub struct Link<R, W> {
    stream: UnixStream,
    decoder: Decoder,
    out: VecDeque<u8>,
    greeted: bool,
    _dir: PhantomData<fn(W) -> R>,
}

/// The core's end: receives the agent's messages, sends actions.
pub type CoreLink = Link<AgentToCore, CoreToAgent>;
/// The agent's end: receives actions, sends state.
pub type AgentLink = Link<CoreToAgent, AgentToCore>;

impl<R: Handshake, W: Handshake> Link<R, W> {
    /// Take over `stream`, make it non-blocking and queue our `Hello`.
    pub fn new(stream: UnixStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        let mut link = Link {
            stream,
            decoder: Decoder::new(),
            out: VecDeque::new(),
            greeted: false,
            _dir: PhantomData,
        };
        // A Hello is a few bytes; encoding it cannot exceed the cap.
        let hello = wire::encode(&W::hello()).map_err(|e| io::Error::other(e.to_string()))?;
        link.out.extend(hello);
        Ok(link)
    }

    pub fn stream(&self) -> &UnixStream {
        &self.stream
    }

    /// Whether the peer's `Hello` has arrived and matched. Before this, the peer is a stranger
    /// and the owner should not treat the connection as an agent.
    pub fn is_greeted(&self) -> bool {
        self.greeted
    }

    /// Queue a message and write what the kernel will take.
    pub fn send(&mut self, msg: &W) -> Result<(), LinkError> {
        let frame = wire::encode(msg)?;
        if self.out.len() + frame.len() > MAX_OUTGOING_BYTES {
            return Err(LinkError::Stalled);
        }
        self.out.extend(frame);
        self.flush()
    }

    /// Whether bytes are queued that the kernel did not take yet.
    pub fn wants_write(&self) -> bool {
        !self.out.is_empty()
    }

    /// Write queued bytes until the kernel stops taking them.
    pub fn flush(&mut self) -> Result<(), LinkError> {
        while !self.out.is_empty() {
            let (front, _) = self.out.as_slices();
            match self.stream.write(front) {
                Ok(0) => return Err(LinkError::Closed),
                Ok(n) => {
                    self.out.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) if e.kind() == io::ErrorKind::BrokenPipe => return Err(LinkError::Closed),
                Err(e) => return Err(LinkError::Io(e)),
            }
        }
        Ok(())
    }

    /// Read what has arrived and return every complete message, `Hello` excluded.
    ///
    /// An `Err` is terminal: drop the link. Messages that arrived in the same read as the
    /// error are discarded with it, which is right — a stream that just broke its framing is
    /// not to be trusted for what came before it either.
    pub fn receive(&mut self) -> Result<Vec<R>, LinkError> {
        let mut buf = [0u8; 16 * 1024];
        let mut closed = false;
        loop {
            match self.stream.read(&mut buf) {
                Ok(0) => {
                    closed = true;
                    break;
                }
                Ok(n) => self.decoder.feed(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) if e.kind() == io::ErrorKind::ConnectionReset => {
                    closed = true;
                    break;
                }
                Err(e) => return Err(LinkError::Io(e)),
            }
        }

        let mut messages = Vec::new();
        while let Some(msg) = self.decoder.next_message::<R>()? {
            if !self.greeted {
                match msg.hello_version() {
                    Some(v) if v == PROTOCOL_VERSION => {
                        self.greeted = true;
                        continue;
                    }
                    Some(theirs) => return Err(LinkError::VersionMismatch { theirs }),
                    None => return Err(LinkError::NoHello),
                }
            }
            // A second Hello is noise from a confused peer, not a message for the owner.
            if msg.hello_version().is_some() {
                continue;
            }
            messages.push(msg);
        }
        if closed {
            // Messages that arrived together with the close are delivered first; the close
            // itself is reported by the next call.
            if messages.is_empty() {
                return Err(LinkError::Closed);
            }
        }
        Ok(messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppId, WindowId};

    fn pair() -> (CoreLink, AgentLink) {
        let (a, b) = UnixStream::pair().expect("socketpair");
        (
            CoreLink::new(a).expect("core link"),
            AgentLink::new(b).expect("agent link"),
        )
    }

    #[test]
    fn the_handshake_is_consumed_and_messages_flow_both_ways() {
        let (mut core, mut agent) = pair();
        core.flush().unwrap();
        agent.flush().unwrap();
        assert!(agent.receive().unwrap().is_empty());
        assert!(core.receive().unwrap().is_empty());
        assert!(core.is_greeted() && agent.is_greeted());

        agent.send(&AgentToCore::Battery(None)).unwrap();
        core.send(&CoreToAgent::ActivateWindow(WindowId(3)))
            .unwrap();
        assert_eq!(core.receive().unwrap(), vec![AgentToCore::Battery(None)]);
        assert_eq!(
            agent.receive().unwrap(),
            vec![CoreToAgent::ActivateWindow(WindowId(3))]
        );
    }

    #[test]
    fn a_peer_that_skips_the_hello_is_refused() {
        let (a, mut b) = UnixStream::pair().unwrap();
        let mut core = CoreLink::new(a).unwrap();
        let frame = wire::encode(&AgentToCore::Battery(None)).unwrap();
        b.write_all(&frame).unwrap();
        assert!(matches!(core.receive(), Err(LinkError::NoHello)));
    }

    #[test]
    fn a_different_protocol_version_is_refused() {
        let (a, mut b) = UnixStream::pair().unwrap();
        let mut core = CoreLink::new(a).unwrap();
        let frame = wire::encode(&AgentToCore::Hello {
            protocol_version: PROTOCOL_VERSION + 1,
        })
        .unwrap();
        b.write_all(&frame).unwrap();
        assert!(matches!(
            core.receive(),
            Err(LinkError::VersionMismatch { .. })
        ));
    }

    #[test]
    fn a_message_split_across_reads_arrives_whole() {
        let (a, mut b) = UnixStream::pair().unwrap();
        let mut core = CoreLink::new(a).unwrap();
        let mut bytes = wire::encode(&AgentToCore::hello()).unwrap();
        bytes.extend(
            wire::encode(&AgentToCore::LaunchStarted {
                app_id: AppId::new("x"),
            })
            .unwrap(),
        );
        let (first, rest) = bytes.split_at(bytes.len() - 3);
        b.write_all(first).unwrap();
        assert!(core.receive().unwrap().is_empty());
        b.write_all(rest).unwrap();
        assert_eq!(core.receive().unwrap().len(), 1);
    }

    #[test]
    fn a_hostile_length_prefix_closes_the_link() {
        let (a, mut b) = UnixStream::pair().unwrap();
        let mut core = CoreLink::new(a).unwrap();
        b.write_all(&u32::MAX.to_le_bytes()).unwrap();
        assert!(matches!(
            core.receive(),
            Err(LinkError::Wire(WireError::TooLarge { .. }))
        ));
    }

    #[test]
    fn a_closed_peer_is_reported_as_closed() {
        let (a, b) = UnixStream::pair().unwrap();
        let mut core = CoreLink::new(a).unwrap();
        drop(b);
        assert!(matches!(core.receive(), Err(LinkError::Closed)));
    }

    #[test]
    fn a_peer_that_never_reads_is_cut_off_instead_of_buffered_forever() {
        let (a, _b) = UnixStream::pair().unwrap();
        let mut core = CoreLink::new(a).unwrap();
        let big = CoreToAgent::OpenPath {
            path: "p".repeat(1024 * 1024),
        };
        let mut stalled = false;
        for _ in 0..64 {
            if matches!(core.send(&big), Err(LinkError::Stalled)) {
                stalled = true;
                break;
            }
        }
        assert!(stalled, "an unread peer must eventually stall the link");
    }
}
