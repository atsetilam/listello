//! The apps the user pinned to the Dock from the Touch Bar, kept in
//! `$XDG_CONFIG_HOME/listello/pins.toml`.
//!
//! The renderer runs as `nobody` and cannot write anywhere, and `/etc/listello/config.toml` is
//! the administrator's; the user's own pins are the user's, so the agent keeps them. Until the
//! file exists the Dock shows the config's `pinned` list.

use listello_protocol::AppId;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Larger files are not a pin list.
const MAX_BYTES: u64 = 64 * 1024;
/// More pins than this is not a Dock; the rest are dropped rather than sent.
const MAX_PINS: usize = 256;

#[derive(Serialize, Deserialize)]
struct PinsFile {
    pinned: Vec<String>,
}

/// Where the pins live, or `None` with no home directory to keep them in.
pub fn default_path() -> Option<PathBuf> {
    crate::desktop::config_dir().map(|d| d.join("listello").join("pins.toml"))
}

/// The saved pins. `None` when there is no file, or one that cannot be read as a pin list --
/// in which case the Dock falls back to the config's list, and the next pin from the Touch
/// Bar writes a good file over it.
pub fn load(path: &Path) -> Option<Vec<AppId>> {
    if fs::metadata(path).ok()?.len() > MAX_BYTES {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    let file: PinsFile = toml::from_str(&text).ok()?;
    Some(clean(file.pinned.into_iter().map(AppId::new)))
}

/// Write `pins` in place of what was there: to a temporary file, then renamed over, so a
/// crash halfway never leaves half a list.
pub fn save(path: &Path, pins: &[AppId]) -> std::io::Result<Vec<AppId>> {
    let pins = clean(pins.iter().cloned());
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let file = PinsFile {
        pinned: pins.iter().map(|p| p.as_str().to_string()).collect(),
    };
    let text = toml::to_string(&file).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("toml.tmp");
    {
        let mut out = fs::File::create(&tmp)?;
        out.write_all(
            b"# The apps pinned to listello's Dock from the Touch Bar, in order.\n\
              # Hold an app on the Touch Bar and tap the pin to change this.\n",
        )?;
        out.write_all(text.as_bytes())?;
        out.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(pins)
}

/// No blanks, no duplicates, no more than [`MAX_PINS`]; order kept.
fn clean(pins: impl Iterator<Item = AppId>) -> Vec<AppId> {
    let mut out: Vec<AppId> = Vec::new();
    for pin in pins {
        if !pin.as_str().trim().is_empty() && !out.contains(&pin) && out.len() < MAX_PINS {
            out.push(pin);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("listello-pins-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn no_file_is_no_pins_and_a_saved_list_comes_back_in_order() {
        let path = temp_dir("roundtrip").join("listello").join("pins.toml");
        assert_eq!(load(&path), None);
        let pins = [
            AppId::new("org.kde.dolphin"),
            AppId::new("google-chrome"),
            AppId::new("org.kde.dolphin"),
            AppId::new(" "),
        ];
        let saved = save(&path, &pins).expect("saves");
        assert_eq!(
            saved,
            [AppId::new("org.kde.dolphin"), AppId::new("google-chrome")]
        );
        assert_eq!(load(&path), Some(saved));
        // An empty list is a real answer, not "no pins yet".
        save(&path, &[]).expect("saves");
        assert_eq!(load(&path), Some(Vec::new()));
    }

    #[test]
    fn a_file_that_is_not_a_pin_list_is_no_pins() {
        let dir = temp_dir("garbage");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pins.toml");
        fs::write(&path, "pinned = 7\n").unwrap();
        assert_eq!(load(&path), None);
    }
}
