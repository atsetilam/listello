//! Starting and stopping listello as a user (D8, second amendment).
//!
//! listello is something a user starts and stops. The settings window is what the application
//! menu opens, so opening it starts the agent when it is stopped (and, the very first time ever,
//! asks whether to start it at every login). Stopping stops the agent's unit, which takes the
//! tray with it and tells the renderer to give the Touch Bar back to the T2's own keys; the
//! window notices and closes.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const AGENT: &str = "listello-agent.service";

/// Left behind once the first-start question has been answered, so it is asked once per user.
const ASKED: &str = "asked-start-at-login";

/// The window and the tray act on the real unit unless this is a look at the window without a
/// screen (`LISTELLO_SETTINGS_SCREENSHOT`) or a development run that asks to be left alone.
pub fn enabled() -> bool {
    std::env::var_os("LISTELLO_SETTINGS_SCREENSHOT").is_none()
        && std::env::var_os("LISTELLO_SETTINGS_NO_LIFECYCLE").is_none()
}

fn systemctl(args: &[&str]) -> bool {
    match Command::new("systemctl")
        .arg("--user")
        .args(args)
        .stdout(Stdio::null())
        .status()
    {
        Ok(status) => status.success(),
        Err(e) => {
            eprintln!("listello-settings: could not run systemctl: {e}");
            false
        }
    }
}

fn systemctl_output(args: &[&str]) -> String {
    Command::new("systemctl")
        .arg("--user")
        .args(args)
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Whether a unit in this `ActiveState` is stopped for good. "activating" is the agent coming
/// back after a crash, and "deactivating" a stop in progress: neither is a reason to act.
fn is_stopped_state(state: &str) -> bool {
    matches!(state, "inactive" | "failed")
}

pub fn is_stopped() -> bool {
    is_stopped_state(&systemctl_output(&[
        "show",
        "-p",
        "ActiveState",
        "--value",
        AGENT,
    ]))
}

pub fn is_running() -> bool {
    systemctl(&["is-active", "--quiet", AGENT])
}

/// Start the agent if it is not running. Returns once systemd has started it.
pub fn ensure_started() {
    if !is_running() && !systemctl(&["start", AGENT]) {
        eprintln!("listello-settings: could not start the agent");
    }
}

/// "Stop listello": the whole of it. Does not wait: the stop kills the caller too when the
/// caller is the tray.
pub fn stop() {
    if !systemctl(&["--no-block", "stop", AGENT]) {
        eprintln!("listello-settings: could not stop the agent");
    }
}

pub fn start_at_login() -> bool {
    systemctl_output(&["is-enabled", AGENT]) == "enabled"
}

pub fn set_start_at_login(on: bool) {
    let verb = if on { "enable" } else { "disable" };
    if !systemctl(&[verb, AGENT]) {
        eprintln!("listello-settings: could not {verb} the agent");
    }
}

fn marker_in(state_home: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    let state = state_home
        .filter(|p| p.is_absolute())
        .or_else(|| home.map(|h| h.join(".local/state")))?;
    Some(state.join("listello").join(ASKED))
}

fn marker() -> Option<PathBuf> {
    marker_in(
        std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
        listello_agent::desktop::home(),
    )
}

/// Whether the first-start question has never been answered for this user.
pub fn first_start() -> bool {
    marker().is_some_and(|m| !m.exists())
}

/// Record the answer to the first-start question: start at every login, or only now.
pub fn answer_first_start(every_time: bool) {
    set_start_at_login(every_time);
    if let Some(marker) = marker() {
        write_marker(&marker);
    }
}

fn write_marker(marker: &Path) {
    if let Some(dir) = marker.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(marker, "") {
        eprintln!("listello-settings: could not remember the answer: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_unit_that_is_down_for_good_counts_as_stopped() {
        assert!(is_stopped_state("inactive"));
        assert!(is_stopped_state("failed"));
        assert!(!is_stopped_state("active"));
        assert!(!is_stopped_state("activating"));
        assert!(!is_stopped_state("deactivating"));
        assert!(!is_stopped_state(""), "an unreadable state is not a stop");
    }

    #[test]
    fn the_marker_lives_in_the_state_directory() {
        let home = Some(PathBuf::from("/home/u"));
        assert_eq!(
            marker_in(None, home.clone()),
            Some(PathBuf::from(
                "/home/u/.local/state/listello/asked-start-at-login"
            ))
        );
        assert_eq!(
            marker_in(Some(PathBuf::from("/s")), home.clone()),
            Some(PathBuf::from("/s/listello/asked-start-at-login"))
        );
        // A relative XDG_STATE_HOME is ignored, as the spec says.
        assert_eq!(
            marker_in(Some(PathBuf::from("rel")), home),
            Some(PathBuf::from(
                "/home/u/.local/state/listello/asked-start-at-login"
            ))
        );
        assert_eq!(marker_in(None, None), None);
    }

    #[test]
    fn writing_the_marker_makes_the_question_not_needed() {
        let dir = std::env::temp_dir().join(format!("listello-lifecycle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let marker = marker_in(Some(dir.clone()), None).unwrap();
        assert!(!marker.exists());
        write_marker(&marker);
        assert!(marker.exists());
    }
}
