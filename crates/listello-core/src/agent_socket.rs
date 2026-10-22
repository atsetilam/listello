// New in M4 (A1). The renderer's end of the D11 socket.
//
// `listello-core` listens and the session agent connects, so a restarting renderer never needs
// the agent restarted or the other way round. The listening socket normally comes from systemd
// (`listello-core.socket`, passed as fd 3); run by hand there is none, and the socket is bound
// here instead, before privileges are dropped.
//
// Policy, all of it because the peer is a process in the user's session and we are a daemon:
//
// - One agent at a time. A second connection is accepted and closed at once (leaving it in the
//   backlog would keep the listener readable and spin the loop); the agent retries with backoff.
// - The peer must be an ordinary user (uid >= 1000) or root. A system service has no business
//   telling the Touch Bar what windows exist.
// - A connection that has not sent a valid `Hello` within `HELLO_DEADLINE` is dropped, so a
//   stranger cannot hold the only slot by saying nothing.
// - A peer that stops reading is dropped (`Link` caps the outgoing queue); the frame loop never
//   waits for it.

use anyhow::{Context, Result};
use listello_protocol::link::{CoreLink, LinkError};
use listello_protocol::wire::DEFAULT_SOCKET_PATH;
use listello_protocol::{AgentToCore, CoreToAgent};
use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};
use std::{
    fs,
    os::{
        fd::{AsFd, BorrowedFd, FromRawFd, RawFd},
        unix::{
            fs::PermissionsExt,
            net::{UnixListener, UnixStream},
        },
    },
    path::Path,
    time::{Duration, Instant},
};

/// First file descriptor systemd passes (`SD_LISTEN_FDS_START`).
const SD_LISTEN_FDS_START: RawFd = 3;
const HELLO_DEADLINE: Duration = Duration::from_secs(5);
/// Lowest uid treated as a person rather than a system service.
const FIRST_HUMAN_UID: u32 = 1000;

/// What happened on the socket since the last call.
#[derive(Debug, PartialEq)]
pub enum SocketEvent {
    Connected,
    Message(AgentToCore),
    Disconnected,
}

pub struct AgentSocket {
    listener: UnixListener,
    current: Option<Current>,
    next_id: u64,
}

struct Current {
    /// Distinguishes this connection from an earlier one that had the same descriptor number.
    id: u64,
    link: CoreLink,
    since: Instant,
    announced: bool,
}

impl AgentSocket {
    /// The socket systemd passed us, or a fresh one at the default path when there is none.
    /// Must run before the privilege drop: creating `/run/listello` needs root.
    pub fn open() -> Result<AgentSocket> {
        let listener = match inherited_listener() {
            Some(l) => l,
            None => bind_default()?,
        };
        listener.set_nonblocking(true)?;
        Ok(AgentSocket {
            listener,
            current: None,
            next_id: 1,
        })
    }

    pub fn listener_fd(&self) -> BorrowedFd<'_> {
        self.listener.as_fd()
    }

    /// The connected agent's fd, if any. Registered with epoll by the caller whenever it
    /// changes.
    pub fn connection_fd(&self) -> Option<BorrowedFd<'_>> {
        self.current.as_ref().map(|c| c.link.stream().as_fd())
    }

    pub fn connection_id(&self) -> Option<u64> {
        self.current.as_ref().map(|c| c.id)
    }

    pub fn wants_write(&self) -> bool {
        self.current.as_ref().is_some_and(|c| c.link.wants_write())
    }

    /// How long until an unconfirmed connection must be dropped, so the caller can wake for it.
    pub fn next_deadline(&self) -> Option<Duration> {
        let c = self.current.as_ref().filter(|c| !c.announced)?;
        Some(HELLO_DEADLINE.saturating_sub(c.since.elapsed()))
    }

    /// Queue an action for the agent. Silently dropped when no agent is connected: the renderer
    /// never depends on one (D11).
    pub fn send(&mut self, msg: &CoreToAgent) {
        let Some(c) = self.current.as_mut().filter(|c| c.announced) else {
            return;
        };
        if let Err(e) = c.link.send(msg) {
            eprintln!("listello: dropping the agent: {e}");
            self.current = None;
        }
    }

    /// Accept, read, flush and time out. Returns what the loop should hear about, in order.
    pub fn poll(&mut self) -> Vec<SocketEvent> {
        let mut events = Vec::new();

        if let Some(c) = self.current.as_mut() {
            let mut failure = None;
            if let Err(e) = c.link.flush() {
                failure = Some(e);
            }
            if failure.is_none() {
                match c.link.receive() {
                    Ok(messages) => {
                        if c.link.is_greeted() && !c.announced {
                            c.announced = true;
                            events.push(SocketEvent::Connected);
                        }
                        events.extend(messages.into_iter().map(SocketEvent::Message));
                    }
                    Err(e) => failure = Some(e),
                }
            }
            if failure.is_none() && !c.announced && c.since.elapsed() >= HELLO_DEADLINE {
                eprintln!("listello: a connection sent no Hello in time, closing it");
                self.current = None;
            }
            if let Some(e) = failure {
                let announced = self.current.take().is_some_and(|c| c.announced);
                if !matches!(e, LinkError::Closed) {
                    eprintln!("listello: closing the agent connection: {e}");
                }
                if announced {
                    events.push(SocketEvent::Disconnected);
                }
            }
        }

        self.accept_pending();
        events
    }

    fn accept_pending(&mut self) {
        loop {
            let stream = match self.listener.accept() {
                Ok((stream, _)) => stream,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return,
                Err(e) => {
                    eprintln!("listello: accept failed: {e}");
                    return;
                }
            };
            if self.current.is_some() {
                // Already served; dropping the stream closes it.
                continue;
            }
            if let Err(why) = check_peer(&stream) {
                eprintln!("listello: refusing a connection: {why}");
                continue;
            }
            match CoreLink::new(stream) {
                Ok(mut link) => {
                    // Our Hello goes out now; the peer's arrives on a later wake.
                    if link.flush().is_err() {
                        continue;
                    }
                    let id = self.next_id;
                    self.next_id += 1;
                    self.current = Some(Current {
                        id,
                        link,
                        since: Instant::now(),
                        announced: false,
                    });
                }
                Err(e) => eprintln!("listello: could not set up a connection: {e}"),
            }
        }
    }
}

fn check_peer(stream: &UnixStream) -> std::result::Result<(), String> {
    let creds = getsockopt(&stream.as_fd(), PeerCredentials)
        .map_err(|e| format!("cannot read peer credentials: {e}"))?;
    let uid = creds.uid();
    if uid == 0 || uid >= FIRST_HUMAN_UID {
        Ok(())
    } else {
        Err(format!("uid {uid} is a system account"))
    }
}

/// The listening socket systemd handed us, per `sd_listen_fds(3)`: `LISTEN_PID` must be us and
/// `LISTEN_FDS` at least one.
fn inherited_listener() -> Option<UnixListener> {
    let pid: u32 = std::env::var("LISTEN_PID").ok()?.parse().ok()?;
    let count: i32 = std::env::var("LISTEN_FDS").ok()?.parse().ok()?;
    if pid != std::process::id() || count < 1 {
        return None;
    }
    // SAFETY: systemd documents fd 3.. as open and owned by this process when LISTEN_PID and
    // LISTEN_FDS say so, and nothing else in this process has taken that descriptor.
    Some(unsafe { UnixListener::from_raw_fd(SD_LISTEN_FDS_START) })
}

/// Run by hand (no systemd): create `/run/listello` and bind the default path, open to every
/// user because the peer check above is what decides who may talk.
fn bind_default() -> Result<UnixListener> {
    let path = Path::new(DEFAULT_SOCKET_PATH);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o755))?;
    }
    let _ = fs::remove_file(path);
    let listener =
        UnixListener::bind(path).with_context(|| format!("binding {}", path.display()))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o666))?;
    Ok(listener)
}

#[cfg(test)]
mod tests {
    use super::*;
    use listello_protocol::link::AgentLink;

    fn socket_in(dir: &Path) -> (AgentSocket, std::path::PathBuf) {
        let path = dir.join("agent.sock");
        let listener = UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        (
            AgentSocket {
                listener,
                current: None,
                next_id: 1,
            },
            path,
        )
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("listello-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_agent_connects_speaks_and_leaves() {
        let dir = temp_dir("socket-flow");
        let (mut socket, path) = socket_in(&dir);
        let mut agent = AgentLink::new(UnixStream::connect(&path).unwrap()).unwrap();
        agent.flush().unwrap();

        // Accept, then hear the Hello.
        socket.poll();
        let events = socket.poll();
        assert_eq!(events, vec![SocketEvent::Connected]);

        agent.send(&AgentToCore::Battery(None)).unwrap();
        assert_eq!(
            socket.poll(),
            vec![SocketEvent::Message(AgentToCore::Battery(None))]
        );

        socket.send(&CoreToAgent::Launch {
            app_id: listello_protocol::AppId::new("x"),
        });
        assert_eq!(agent.receive().unwrap().len(), 1);

        drop(agent);
        assert_eq!(socket.poll(), vec![SocketEvent::Disconnected]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_silent_connection_is_not_announced_and_actions_to_it_are_dropped() {
        let dir = temp_dir("socket-silent");
        let (mut socket, path) = socket_in(&dir);
        let _silent = UnixStream::connect(&path).unwrap();
        assert!(socket.poll().is_empty());
        assert!(socket.poll().is_empty());
        // Not greeted, so nothing is queued towards it and nothing panics.
        socket.send(&CoreToAgent::ActivateWindow(listello_protocol::WindowId(1)));
        assert!(socket.next_deadline().is_some());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sending_with_no_agent_is_a_no_op() {
        let dir = temp_dir("socket-none");
        let (mut socket, _path) = socket_in(&dir);
        socket.send(&CoreToAgent::ActivateWindow(listello_protocol::WindowId(1)));
        assert!(socket.poll().is_empty());
        let _ = fs::remove_dir_all(dir);
    }
}
