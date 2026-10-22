//! Keys for a button: recorded from the keyboard, or picked from the keys a keyboard press in
//! a window cannot deliver (media and brightness keys go to Plasma, not to the window).
//!
//! Names are `input_linux::Key`'s, the ones `listello-core` turns back into key codes.

use input_linux::Key;

/// Media, brightness and function keys, in the order the window lists them.
pub const SPECIAL: &[&str] = &[
    "Mute",
    "VolumeDown",
    "VolumeUp",
    "BrightnessDown",
    "BrightnessUp",
    "IllumDown",
    "IllumUp",
    "PreviousSong",
    "PlayPause",
    "NextSong",
    "Print",
    "Esc",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "F9",
    "F10",
    "F11",
    "F12",
    "F13",
    "F14",
    "F15",
    "F16",
    "F17",
    "F18",
    "F19",
    "F20",
    "F21",
    "F22",
    "F23",
    "F24",
];

/// The key a press in the window was, from Qt's `nativeScanCode`: on Linux, under both
/// Wayland and X11, that is the evdev code plus 8.
pub fn from_native_scan_code(scan: i32) -> Option<String> {
    let code = u16::try_from(scan.checked_sub(8)?).ok()?;
    Key::from_code(code).ok().map(|k| format!("{k:?}"))
}

/// "LeftCtrl, C" as a person reads it: "Ctrl + C".
pub fn label(names: &str, italian: bool) -> String {
    names
        .split(',')
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(|n| match n {
            "LeftCtrl" | "RightCtrl" => "Ctrl".to_string(),
            "LeftShift" | "RightShift" => "Shift".to_string(),
            "LeftAlt" => "Alt".to_string(),
            "RightAlt" => "AltGr".to_string(),
            "LeftMeta" | "RightMeta" => "Meta".to_string(),
            "Mute" => if italian { "Muto" } else { "Mute" }.to_string(),
            "VolumeDown" => "Volume −".to_string(),
            "VolumeUp" => "Volume +".to_string(),
            "BrightnessDown" => if italian {
                "Luminosità −"
            } else {
                "Brightness −"
            }
            .to_string(),
            "BrightnessUp" => if italian {
                "Luminosità +"
            } else {
                "Brightness +"
            }
            .to_string(),
            "IllumDown" => if italian {
                "Luce tastiera −"
            } else {
                "Keyboard light −"
            }
            .to_string(),
            "IllumUp" => if italian {
                "Luce tastiera +"
            } else {
                "Keyboard light +"
            }
            .to_string(),
            "PreviousSong" => if italian {
                "Brano precedente"
            } else {
                "Previous track"
            }
            .to_string(),
            "PlayPause" => "Play/Pause".to_string(),
            "NextSong" => if italian {
                "Brano successivo"
            } else {
                "Next track"
            }
            .to_string(),
            "Print" => if italian { "Stamp" } else { "Print Screen" }.to_string(),
            "Space" => if italian { "Spazio" } else { "Space" }.to_string(),
            other => other
                .strip_prefix("Num")
                .filter(|d| d.len() == 1)
                .unwrap_or(other)
                .to_string(),
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::value::{Error as DeError, StrDeserializer};
    use serde::Deserialize;

    /// What `listello-core` does with a name; every name offered here must survive it.
    fn resolves(name: &str) -> bool {
        Key::deserialize(StrDeserializer::<DeError>::new(name)).is_ok()
    }

    #[test]
    fn every_special_key_is_one_the_renderer_can_press() {
        for name in SPECIAL {
            assert!(resolves(name), "{name}");
        }
    }

    #[test]
    fn a_recorded_key_is_named_the_way_the_renderer_reads_it() {
        // evdev 29 is the left Ctrl, 46 is C; Qt adds 8.
        let ctrl = from_native_scan_code(29 + 8).unwrap();
        let c = from_native_scan_code(46 + 8).unwrap();
        assert_eq!((ctrl.as_str(), c.as_str()), ("LeftCtrl", "C"));
        assert!(resolves(&ctrl) && resolves(&c));
        assert_eq!(from_native_scan_code(3), None);
        assert_eq!(label("LeftCtrl, C", false), "Ctrl + C");
        assert_eq!(label("Mute", true), "Muto");
    }
}
