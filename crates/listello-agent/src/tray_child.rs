//! The tray icon, run as the agent's own child (D8, second amendment).
//!
//! With `[general] tray = true` in the user's file the agent starts `listello-settings --tray`,
//! so the icon lives and dies with listello: stopping the agent's unit takes it along. The tray
//! leaves by itself when the setting goes off (it watches the file), and the agent starts it
//! again when it comes back on.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

pub struct TrayChild {
    child: Option<Child>,
}

impl TrayChild {
    pub fn new() -> TrayChild {
        TrayChild { child: None }
    }

    /// Collect the tray if it has left, and start it when it is wanted and not running.
    pub fn sync(&mut self, wanted: bool) {
        if let Some(child) = self.child.as_mut() {
            if !matches!(child.try_wait(), Ok(None)) {
                self.child = None;
            }
        }
        if wanted && self.child.is_none() {
            self.child = spawn();
        }
    }

    /// Collect the tray if it has left, without starting anything.
    pub fn reap(&mut self) {
        if let Some(child) = self.child.as_mut() {
            if !matches!(child.try_wait(), Ok(None)) {
                self.child = None;
            }
        }
    }
}

fn spawn() -> Option<Child> {
    let exe = settings_exe();
    let mut command = Command::new(&exe);
    command.arg("--tray").stdin(Stdio::null());
    crate::actions::unblock_signals(&mut command);
    match command.spawn() {
        Ok(child) => Some(child),
        Err(e) => {
            eprintln!(
                "listello-agent: could not start the tray ({}): {e}",
                exe.display()
            );
            None
        }
    }
}

/// `listello-settings` next to the agent, which is where the package puts both. An agent
/// started before an upgrade runs a file Linux has since renamed "… (deleted)"; the new one is
/// at the old path.
fn settings_exe() -> PathBuf {
    let own = std::env::current_exe().ok();
    let own = own
        .as_deref()
        .and_then(|p| p.to_str())
        .map(|p| p.strip_suffix(" (deleted)").unwrap_or(p));
    own.and_then(|p| Path::new(p).parent())
        .map(|dir| dir.join("listello-settings"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("listello-settings"))
}
