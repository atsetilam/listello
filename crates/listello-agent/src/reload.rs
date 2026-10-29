//! "Reload listello" (D8 amendment): the tray and the settings window ask by sending the agent
//! `SIGUSR1` (`systemctl --user kill --kill-whom=main -s USR1 listello-agent`), and the agent
//! tells the renderer to start over. The agent is the one process that both the user can
//! signal and the renderer listens to.
//!
//! `SIGUSR2` is "Stop listello": the agent's unit uses it as its `KillSignal`, so stopping the
//! unit (or logging out) lets the agent tell the renderer to give the panel back before it goes.
//!
//! `SIGHUP` is "Check for updates" from the tray's menu (`updates`).
//!
//! The signals arrive through a signalfd, so they are one more descriptor in the single poll
//! loop.

use nix::sys::signal::{SigSet, Signal};
use nix::sys::signalfd::{SfdFlags, SignalFd};
use std::os::fd::{AsFd, BorrowedFd};
use std::time::{Duration, Instant};

/// systemd gives up on a unit restarted five times in ten seconds, and a renderer it gave up on
/// leaves the Touch Bar dark. Repeated clicks inside this gap are one reload.
const AT_MOST_EVERY: Duration = Duration::from_secs(3);

pub struct Reload {
    fd: Option<SignalFd>,
    last: Option<Instant>,
    quit: bool,
    check: bool,
}

impl Reload {
    /// Block `SIGUSR1` and read it from a descriptor instead. Called before any thread starts,
    /// since a blocked mask is inherited and a thread without it would take the default action,
    /// which is to die.
    pub fn new() -> Reload {
        let mut mask = SigSet::empty();
        mask.add(Signal::SIGUSR1);
        mask.add(Signal::SIGUSR2);
        mask.add(Signal::SIGHUP);
        let fd = mask
            .thread_block()
            .and_then(|()| {
                SignalFd::with_flags(&mask, SfdFlags::SFD_NONBLOCK | SfdFlags::SFD_CLOEXEC)
            })
            .inspect_err(|e| eprintln!("listello-agent: Reload will not work: {e}"))
            .ok();
        Reload {
            fd,
            last: None,
            quit: false,
            check: false,
        }
    }

    pub fn fd(&self) -> Option<BorrowedFd<'_>> {
        self.fd.as_ref().map(|f| f.as_fd())
    }

    /// Drain the signals; whether to reload now.
    pub fn asked(&mut self, now: Instant) -> bool {
        let Some(fd) = self.fd.as_mut() else {
            return false;
        };
        let mut asked = false;
        while let Ok(Some(info)) = fd.read_signal() {
            if info.ssi_signo == Signal::SIGUSR2 as u32 {
                self.quit = true;
            } else if info.ssi_signo == Signal::SIGHUP as u32 {
                self.check = true;
            } else {
                asked = true;
            }
        }
        asked && self.allowed(now)
    }

    /// Whether the agent was told to stop. Call after `asked`, which reads the signals.
    pub fn quit(&self) -> bool {
        self.quit
    }

    /// Whether an update check was asked for since the last call. Call after `asked`.
    pub fn take_check(&mut self) -> bool {
        std::mem::take(&mut self.check)
    }

    fn allowed(&mut self, now: Instant) -> bool {
        if self
            .last
            .is_some_and(|last| now.duration_since(last) < AT_MOST_EVERY)
        {
            return false;
        }
        self.last = Some(now);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_close_together_are_one_reload() {
        let mut reload = Reload {
            fd: None,
            last: None,
            quit: false,
            check: false,
        };
        let t = Instant::now();
        assert!(reload.allowed(t));
        assert!(!reload.allowed(t + Duration::from_secs(1)));
        assert!(reload.allowed(t + AT_MOST_EVERY));
    }
}
