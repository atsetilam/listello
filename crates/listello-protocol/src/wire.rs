//! Framing: how a message becomes bytes on the socket, and back.
//!
//! D11 fixes the transport — a Unix socket at a fixed path, systemd socket activation,
//! length-prefixed serialised messages, peer credentials checked on connect. This module is
//! the length-prefixing and the serialisation; the socket itself belongs to whichever binary
//! opens it, and neither of them exists yet (M4).
//!
//! # No I/O here
//!
//! [`encode`] takes a message and gives bytes. [`Decoder`] takes bytes as they arrive — in
//! whatever sized pieces the kernel felt like — and gives back whole messages. Nothing in this
//! module reads, writes, blocks or allocates a file descriptor, which is what lets the whole
//! framing layer be tested without a socket, and lets the single-threaded loop (D3) drive the
//! socket with the same readiness machinery it uses for libinput.
//!
//! # The codec
//!
//! [`postcard`](https://docs.rs/postcard): compact, non-self-describing, no schema evolution.
//! That last property is why [`crate::PROTOCOL_VERSION`] is exchanged before anything else —
//! a peer from a different build does not misread a message, it is refused. Both binaries ship
//! from this repository and are installed together, so there is no version skew to be
//! tolerant of, and tolerance would only hide the bug.
//!
//! JSON was the alternative and loses on the one message that matters: a 128×128 icon is 64 KiB
//! of pixels, which base64 inflates by a third for no benefit to anyone, since nothing human
//! ever reads this stream.
//!
//! # The size cap is a security boundary, not a tuning knob
//!
//! The renderer is a privileged process reading from a socket in the user's session. A length
//! prefix it trusts is an allocation the peer chooses, so [`MAX_MESSAGE_BYTES`] is checked
//! *before* the buffer grows, and a frame that exceeds it kills the connection rather than the
//! daemon.

use serde::de::DeserializeOwned;
use serde::Serialize;

/// Where the renderer listens. Under `/run` because it is created by the system service; the
/// agent connects to it as the user.
pub const DEFAULT_SOCKET_PATH: &str = "/run/listello/agent.sock";

/// The largest single message either side will accept, payload only.
///
/// Generous next to what anything legitimately sends — a 256×256 RGBA icon is 256 KiB, and a
/// window list of a hundred windows is a few KiB — and small enough that a hostile or confused
/// peer cannot make the renderer allocate its way out of memory.
pub const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

/// The length prefix: a little-endian `u32` byte count, then that many bytes of payload.
const PREFIX_LEN: usize = 4;

#[derive(Debug)]
pub enum WireError {
    /// The payload did not deserialise. Always fatal for the connection: the stream is framed,
    /// so a message that does not parse means the two peers disagree about what the bytes
    /// mean, and every later frame is suspect too.
    Malformed(postcard::Error),
    /// A length prefix exceeded [`MAX_MESSAGE_BYTES`]. Close the connection; do not resync.
    TooLarge { len: usize },
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WireError::Malformed(e) => write!(f, "malformed message: {e}"),
            WireError::TooLarge { len } => write!(
                f,
                "message of {len} bytes exceeds the {MAX_MESSAGE_BYTES}-byte limit"
            ),
        }
    }
}

impl std::error::Error for WireError {}

/// Serialise one message into a length-prefixed frame.
///
/// Fails only if the message is somehow larger than [`MAX_MESSAGE_BYTES`], which for our own
/// messages means an icon that should have been rejected long before it got here.
pub fn encode<T: Serialize>(msg: &T) -> Result<Vec<u8>, WireError> {
    let payload = postcard::to_stdvec(msg).map_err(WireError::Malformed)?;
    if payload.len() > MAX_MESSAGE_BYTES {
        return Err(WireError::TooLarge { len: payload.len() });
    }
    let mut frame = Vec::with_capacity(PREFIX_LEN + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Reassembles messages from a byte stream that arrives in arbitrary pieces.
///
/// Feed it whatever `read()` returned with [`Decoder::feed`], then call [`Decoder::next_message`] until
/// it yields `None`. One `read()` can carry half a message, three messages, or three and a
/// half, and all three cases go through the same path here rather than through a special case
/// at the call site.
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
}

impl Decoder {
    pub fn new() -> Decoder {
        Decoder::default()
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next complete message, if one has arrived.
    ///
    /// `Ok(None)` means "not yet, read more" — the ordinary case, not an error. Any `Err` is
    /// terminal for the connection: drop it, and let the reconnect path deal with it. Do not
    /// try to skip a bad frame and carry on; the framing is only trustworthy while every
    /// prefix has been trustworthy.
    pub fn next_message<T: DeserializeOwned>(&mut self) -> Result<Option<T>, WireError> {
        if self.buf.len() < PREFIX_LEN {
            return Ok(None);
        }
        let mut prefix = [0u8; PREFIX_LEN];
        prefix.copy_from_slice(&self.buf[..PREFIX_LEN]);
        let len = u32::from_le_bytes(prefix) as usize;

        // Checked before the buffer is allowed to grow to hold it: the peer chooses this
        // number, so it is not allowed to choose an allocation.
        if len > MAX_MESSAGE_BYTES {
            return Err(WireError::TooLarge { len });
        }
        if self.buf.len() < PREFIX_LEN + len {
            return Ok(None);
        }

        let msg = postcard::from_bytes(&self.buf[PREFIX_LEN..PREFIX_LEN + len])
            .map_err(WireError::Malformed)?;
        self.buf.drain(..PREFIX_LEN + len);
        Ok(Some(msg))
    }

    /// Bytes held but not yet forming a whole message. For diagnostics and for tests; the
    /// loop has no reason to look.
    pub fn pending_bytes(&self) -> usize {
        self.buf.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentToCore, AppId, Icon, WindowId, WindowInfo, PROTOCOL_VERSION};

    fn windows() -> AgentToCore {
        AgentToCore::Windows(vec![WindowInfo {
            id: WindowId(7),
            app_id: AppId::new("org.kde.dolphin"),
            title: "Downloads — Dolphin".to_string(),
            active: true,
            minimized: false,
        }])
    }

    #[test]
    fn a_message_survives_the_round_trip() {
        let mut dec = Decoder::new();
        dec.feed(&encode(&windows()).unwrap());
        assert_eq!(dec.next_message::<AgentToCore>().unwrap(), Some(windows()));
        assert_eq!(dec.next_message::<AgentToCore>().unwrap(), None);
        assert_eq!(dec.pending_bytes(), 0);
    }

    #[test]
    fn a_frame_split_across_reads_is_reassembled() {
        let frame = encode(&windows()).unwrap();
        let mut dec = Decoder::new();
        // One byte at a time is the pathological case the kernel is entitled to hand us.
        for (i, byte) in frame.iter().enumerate() {
            dec.feed(&[*byte]);
            let got = dec.next_message::<AgentToCore>().unwrap();
            if i + 1 == frame.len() {
                assert_eq!(got, Some(windows()));
            } else {
                assert_eq!(
                    got,
                    None,
                    "yielded a message from {} of {} bytes",
                    i + 1,
                    frame.len()
                );
            }
        }
    }

    #[test]
    fn several_frames_in_one_read_all_come_back() {
        let hello = AgentToCore::Hello {
            protocol_version: PROTOCOL_VERSION,
        };
        let mut stream = encode(&hello).unwrap();
        stream.extend(encode(&windows()).unwrap());
        stream.extend(encode(&AgentToCore::Battery(None)).unwrap());

        let mut dec = Decoder::new();
        dec.feed(&stream);
        assert_eq!(dec.next_message::<AgentToCore>().unwrap(), Some(hello));
        assert_eq!(dec.next_message::<AgentToCore>().unwrap(), Some(windows()));
        assert_eq!(
            dec.next_message::<AgentToCore>().unwrap(),
            Some(AgentToCore::Battery(None))
        );
        assert_eq!(dec.next_message::<AgentToCore>().unwrap(), None);
    }

    #[test]
    fn an_icon_crosses_as_pixels_unchanged() {
        let icon = Icon::new(2, 2, (0u8..16).collect()).unwrap();
        let msg = AgentToCore::Icon {
            app_id: AppId::new("firefox"),
            icon: Some(icon.clone()),
        };
        let mut dec = Decoder::new();
        dec.feed(&encode(&msg).unwrap());
        match dec.next_message::<AgentToCore>().unwrap() {
            Some(AgentToCore::Icon {
                icon: Some(got), ..
            }) => {
                assert_eq!(got, icon);
                assert!(got.is_valid());
            }
            other => panic!("expected an icon, got {other:?}"),
        }
    }

    #[test]
    fn an_oversized_prefix_is_refused_without_allocating() {
        let mut dec = Decoder::new();
        dec.feed(&(u32::MAX).to_le_bytes());
        assert!(matches!(
            dec.next_message::<AgentToCore>(),
            Err(WireError::TooLarge { .. })
        ));
        // Nothing was buffered on account of the claimed length.
        assert_eq!(dec.pending_bytes(), PREFIX_LEN);
    }

    #[test]
    fn a_corrupt_payload_is_an_error_not_a_panic() {
        let mut frame = encode(&windows()).unwrap();
        let last = frame.len() - 1;
        frame[last] ^= 0xff;
        frame[PREFIX_LEN] ^= 0xff;
        let mut dec = Decoder::new();
        dec.feed(&frame);
        // Either it fails to parse or it parses to something else; what it must never do is
        // abort the daemon.
        let _ = dec.next_message::<AgentToCore>();
    }
}
