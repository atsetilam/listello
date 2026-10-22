//! The user's own layout, `$XDG_CONFIG_HOME/listello/config.toml` (D8 amendment).
//!
//! Same format as `/etc/listello/config.toml`. The renderer runs as `nobody` and cannot read a
//! user's home, so the agent reads the file, checks it, and hands the text over; the renderer
//! shows it while this agent is connected. A file that does not validate is reported here, in
//! the user's own journal, and the machine's layout stays up.

use listello_protocol::AgentToCore;
use listello_ui::UiConfig;
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};
use std::ffi::OsStr;
use std::fs;
use std::os::fd::{AsFd, BorrowedFd};
use std::path::{Path, PathBuf};

/// Larger files are not a layout.
const MAX_BYTES: u64 = 1024 * 1024;
const FILE_NAME: &str = "config.toml";
/// The pins live in the same directory (`pins.rs`).
const PINS_NAME: &str = "pins.toml";

pub struct UserLayout {
    path: Option<PathBuf>,
    inotify: Option<Inotify>,
    time_locale: Option<String>,
    /// What was last sent, so an unchanged file is not sent again.
    sent: Option<AgentToCore>,
    /// The file was written, moved or removed since `take_touched` last said so.
    touched: bool,
}

impl UserLayout {
    /// Watch the directory, not the file: editors and the settings window replace it rather
    /// than write it in place. The directory is made if missing, as saving pins would.
    pub fn new(dir: Option<PathBuf>, time_locale: Option<String>) -> UserLayout {
        let inotify = dir.as_ref().and_then(|dir| {
            let _ = fs::create_dir_all(dir);
            let inotify = Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC).ok()?;
            let flags = AddWatchFlags::IN_CLOSE_WRITE
                | AddWatchFlags::IN_MOVED_TO
                | AddWatchFlags::IN_MOVED_FROM
                | AddWatchFlags::IN_DELETE;
            match inotify.add_watch(dir.as_path(), flags) {
                Ok(_) => Some(inotify),
                Err(e) => {
                    eprintln!("listello-agent: cannot watch {}: {e}", dir.display());
                    None
                }
            }
        });
        UserLayout {
            path: dir.map(|d| d.join(FILE_NAME)),
            inotify,
            time_locale,
            sent: None,
            touched: false,
        }
    }

    /// Whether the file changed since the last call, whatever it now says.
    pub fn take_touched(&mut self) -> bool {
        std::mem::take(&mut self.touched)
    }

    /// `[general] tray = true`, read loosely: the tray follows its own switch even while the
    /// rest of the file does not validate (the tray reads it the same way).
    pub fn tray_wanted(&self) -> bool {
        let text = self
            .path
            .as_deref()
            .and_then(|p| fs::read_to_string(p).ok());
        text.and_then(|t| t.parse::<toml::Table>().ok())
            .and_then(|t| t.get("general")?.get("tray")?.as_bool())
            == Some(true)
    }

    pub fn fd(&self) -> Option<BorrowedFd<'_>> {
        self.inotify.as_ref().map(|i| i.as_fd())
    }

    /// The whole answer, for a renderer that just connected.
    pub fn greeting(&mut self) -> AgentToCore {
        let msg = self.message();
        self.sent = Some(msg.clone());
        msg
    }

    /// Drain the watch: the new answer when the layout changed into something different, and
    /// whether the pins file next to it was touched.
    pub fn changed(&mut self) -> (Option<AgentToCore>, bool) {
        let Some(inotify) = self.inotify.as_ref() else {
            return (None, false);
        };
        let (mut layout, mut pins) = (false, false);
        while let Ok(events) = inotify.read_events() {
            if events.is_empty() {
                break;
            }
            for name in events.iter().filter_map(|e| e.name.as_deref()) {
                layout |= name == OsStr::new(FILE_NAME);
                pins |= name == OsStr::new(PINS_NAME);
            }
        }
        if !layout {
            return (None, pins);
        }
        self.touched = true;
        let msg = self.message();
        let changed = (self.sent.as_ref() != Some(&msg)).then(|| {
            self.sent = Some(msg.clone());
            msg
        });
        (changed, pins)
    }

    fn message(&self) -> AgentToCore {
        AgentToCore::UserConfig {
            layout: self.path.as_deref().and_then(read_layout),
            time_locale: self.time_locale.clone(),
        }
    }
}

/// The file's text when it is there and validates; `None`, with the reason logged, otherwise.
fn read_layout(path: &Path) -> Option<String> {
    let len = fs::metadata(path).ok()?.len();
    if len > MAX_BYTES {
        eprintln!(
            "listello-agent: {} is too large to be a layout; showing the machine's",
            path.display()
        );
        return None;
    }
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("listello-agent: cannot read {}: {e}", path.display());
            return None;
        }
    };
    match UiConfig::parse(&text) {
        Ok(_) => Some(text),
        Err(e) => {
            eprintln!(
                "listello-agent: {} is not a valid layout, showing the machine's: {e}",
                path.display()
            );
            None
        }
    }
}

/// What "follow the system" means for day and month names: `LC_ALL`, else `LC_TIME`, else
/// `LANG` — the order the C library itself uses.
pub fn time_locale() -> Option<String> {
    ["LC_ALL", "LC_TIME", "LANG"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("listello-layout-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn layout_of(msg: &AgentToCore) -> Option<&str> {
        match msg {
            AgentToCore::UserConfig { layout, .. } => layout.as_deref(),
            _ => panic!("not a layout"),
        }
    }

    #[test]
    fn no_file_is_none_and_a_valid_one_is_sent_whole() {
        let dir = temp_dir("valid");
        let mut user = UserLayout::new(Some(dir.clone()), Some("it_IT.UTF-8".into()));
        let greeting = user.greeting();
        assert_eq!(layout_of(&greeting), None);
        assert_eq!(
            greeting,
            AgentToCore::UserConfig {
                layout: None,
                time_locale: Some("it_IT.UTF-8".into())
            }
        );

        let text = "[general]\nlanguage = \"italian\"\n[[widget]]\nkind = \"clock\"\n";
        let tmp = dir.join("config.toml.new");
        fs::write(&tmp, text).unwrap();
        fs::rename(&tmp, dir.join(FILE_NAME)).unwrap();
        let changed = user.changed().0.expect("the new file is noticed");
        assert_eq!(layout_of(&changed), Some(text));
        // Nothing new happened since.
        assert_eq!(user.changed(), (None, false));
    }

    #[test]
    fn the_tray_is_wanted_only_when_the_file_says_so_even_if_the_layout_is_bad() {
        let dir = temp_dir("tray");
        fs::create_dir_all(&dir).unwrap();
        let user = UserLayout::new(Some(dir.clone()), None);
        assert!(!user.tray_wanted(), "no file");
        fs::write(
            dir.join(FILE_NAME),
            "[general]\ntray = true\n[[widget]]\nkind = \"toaster\"\n",
        )
        .unwrap();
        assert!(user.tray_wanted(), "the layout is bad but the switch is on");
        fs::write(dir.join(FILE_NAME), "[general]\ntray = false\n").unwrap();
        assert!(!user.tray_wanted());
    }

    #[test]
    fn a_file_that_does_not_validate_is_none() {
        let dir = temp_dir("invalid");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(FILE_NAME), "[[widget]]\nkind = \"toaster\"\n").unwrap();
        let mut user = UserLayout::new(Some(dir), None);
        assert_eq!(layout_of(&user.greeting()), None);
    }

    #[test]
    fn a_layout_going_bad_is_sent_as_none() {
        let dir = temp_dir("goes-bad");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(FILE_NAME), "[[widget]]\nkind = \"clock\"\n").unwrap();
        let mut user = UserLayout::new(Some(dir.clone()), None);
        assert!(layout_of(&user.greeting()).is_some());
        fs::write(dir.join(FILE_NAME), "[[widget]]\nkind = 7\n").unwrap();
        assert_eq!(layout_of(&user.changed().0.expect("noticed")), None);
        fs::remove_file(dir.join(FILE_NAME)).unwrap();
        assert_eq!(user.changed(), (None, false), "still none: nothing to send");
        fs::write(dir.join(PINS_NAME), "pinned = []\n").unwrap();
        assert_eq!(user.changed(), (None, true), "the pins are noticed too");
    }
}
