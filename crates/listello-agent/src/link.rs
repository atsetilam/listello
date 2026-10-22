//! The agent's connection to the renderer, kept alive forever.
//!
//! The renderer may not be up yet, may restart, or may refuse us (a different build). None of
//! those is the agent's problem to give up on: it retries with a growing delay, and a restart
//! of either side needs no restart of the other (D11). A version mismatch is logged once per
//! distinct cause instead of once per attempt.

use listello_protocol::link::{AgentLink, LinkError};
use listello_protocol::wire::DEFAULT_SOCKET_PATH;
use listello_protocol::{AgentToCore, CoreToAgent};
use std::os::fd::BorrowedFd;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

const FIRST_RETRY: Duration = Duration::from_millis(500);
const LONGEST_RETRY: Duration = Duration::from_secs(10);

pub struct Reconnecting {
    path: String,
    link: Option<AgentLink>,
    retry_at: Instant,
    delay: Duration,
    last_logged: Option<String>,
}

/// What a call changed, for the loop to react to.
#[derive(Debug, PartialEq)]
pub enum LinkEvent {
    Connected,
    Message(CoreToAgent),
    Disconnected,
}

impl Reconnecting {
    pub fn new(path: &str) -> Reconnecting {
        Reconnecting {
            path: path.to_string(),
            link: None,
            retry_at: Instant::now(),
            delay: FIRST_RETRY,
            last_logged: None,
        }
    }

    pub fn default_path() -> Reconnecting {
        Reconnecting::new(DEFAULT_SOCKET_PATH)
    }

    pub fn is_connected(&self) -> bool {
        self.link.is_some()
    }

    pub fn fd(&self) -> Option<BorrowedFd<'_>> {
        use std::os::fd::AsFd;
        self.link.as_ref().map(|l| l.stream().as_fd())
    }

    pub fn wants_write(&self) -> bool {
        self.link.as_ref().is_some_and(|l| l.wants_write())
    }

    /// How long until the next connection attempt, when disconnected.
    pub fn until_retry(&self) -> Option<Duration> {
        self.link
            .is_none()
            .then(|| self.retry_at.saturating_duration_since(Instant::now()))
    }

    pub fn send(&mut self, msg: &AgentToCore) -> Option<LinkEvent> {
        let link = self.link.as_mut()?;
        match link.send(msg) {
            Ok(()) => None,
            Err(e) => Some(self.drop_link(e)),
        }
    }

    /// Try to (re)connect if it is time, flush, and read. Returns what happened, in order.
    pub fn poll(&mut self) -> Vec<LinkEvent> {
        let mut events = Vec::new();
        if self.link.is_none() && Instant::now() >= self.retry_at {
            self.try_connect(&mut events);
        }
        if let Some(link) = self.link.as_mut() {
            let mut failure = link.flush().err();
            if failure.is_none() {
                match link.receive() {
                    Ok(messages) => events.extend(messages.into_iter().map(LinkEvent::Message)),
                    Err(e) => failure = Some(e),
                }
            }
            if let Some(e) = failure {
                events.push(self.drop_link(e));
            }
        }
        events
    }

    fn try_connect(&mut self, events: &mut Vec<LinkEvent>) {
        let connected = UnixStream::connect(&self.path)
            .map_err(|e| e.to_string())
            .and_then(|s| AgentLink::new(s).map_err(|e| e.to_string()));
        match connected {
            Ok(link) => {
                self.link = Some(link);
                self.delay = FIRST_RETRY;
                self.last_logged = None;
                events.push(LinkEvent::Connected);
            }
            Err(why) => {
                // Not running yet is the ordinary case at login; say it once, not every try.
                if self.last_logged.as_deref() != Some(why.as_str()) {
                    eprintln!(
                        "listello-agent: cannot reach the renderer at {}: {why}",
                        self.path
                    );
                    self.last_logged = Some(why);
                }
                self.schedule_retry();
            }
        }
    }

    fn drop_link(&mut self, why: LinkError) -> LinkEvent {
        self.link = None;
        let text = why.to_string();
        if !matches!(why, LinkError::Closed) && self.last_logged.as_deref() != Some(text.as_str()) {
            eprintln!("listello-agent: connection to the renderer lost: {text}");
            self.last_logged = Some(text);
        }
        self.schedule_retry();
        LinkEvent::Disconnected
    }

    fn schedule_retry(&mut self) {
        self.retry_at = Instant::now() + self.delay;
        self.delay = (self.delay * 2).min(LONGEST_RETRY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use listello_protocol::link::CoreLink;
    use std::os::unix::net::UnixListener;

    fn socket_path(name: &str) -> String {
        let dir =
            std::env::temp_dir().join(format!("listello-agent-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("s.sock").to_string_lossy().into_owned()
    }

    #[test]
    fn it_connects_once_the_renderer_appears_and_survives_its_restart() {
        let path = socket_path("restart");
        let mut link = Reconnecting::new(&path);
        // Nothing is listening yet: no event, and a retry is scheduled.
        assert!(link.poll().is_empty());
        assert!(!link.is_connected());
        assert!(link.until_retry().is_some());

        let listener = UnixListener::bind(&path).unwrap();
        link.retry_at = Instant::now();
        assert_eq!(link.poll(), vec![LinkEvent::Connected]);
        let (stream, _) = listener.accept().unwrap();
        let mut core = CoreLink::new(stream).unwrap();
        core.flush().unwrap();

        core.send(&CoreToAgent::ActivateWindow(listello_protocol::WindowId(5)))
            .unwrap();
        let events = link.poll();
        assert_eq!(
            events,
            vec![LinkEvent::Message(CoreToAgent::ActivateWindow(
                listello_protocol::WindowId(5)
            ))]
        );

        drop(core);
        assert_eq!(link.poll(), vec![LinkEvent::Disconnected]);
        assert!(!link.is_connected());

        // The renderer comes back; the agent finds it again with no restart.
        link.retry_at = Instant::now();
        assert_eq!(link.poll(), vec![LinkEvent::Connected]);
    }

    #[test]
    fn the_retry_delay_grows_and_is_capped() {
        let mut link = Reconnecting::new("/nonexistent/listello.sock");
        for _ in 0..10 {
            link.retry_at = Instant::now();
            link.poll();
        }
        assert_eq!(link.delay, LONGEST_RETRY);
    }
}
