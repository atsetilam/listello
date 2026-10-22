//! The config file that says which widgets a strip shows, in what order, and how wide each one
//! is.
//!
//! tiny-dfr's config describes a fixed row of buttons (`ButtonConfig`). This schema describes an
//! ordered list of *widgets* of different kinds, because that is what D3's layout is a list of,
//! and because D8/M5 require that a personal layout be a config file rather than a
//! default anyone else is stuck with.
//!
//! ```toml
//! [strip]
//! gap = 8.0
//!
//! [[widget]]
//! kind = "button"
//! label = "F5"
//! keys = ["KEY_F5"]
//! width = 120.0
//!
//! [[widget]]
//! kind = "clock"
//! format = "%H:%M"
//! flex = 1.0
//!
//! [[widget]]
//! kind = "separator"
//! width = 2.0
//!
//! [[widget]]
//! kind = "network"
//! show_ssid = false
//! width = 40.0
//! ```
//!
//! `[[fn_widget]]` entries take the same keys and replace the whole strip while Fn is held, so
//! a layout can show media keys by default and F1–F12 under Fn, or the other way round.
//!
//! Parsing is the easy half; validation is the point. `deny_unknown_fields` everywhere means a
//! typo in a hand-written config file is an error the user can see, not a Touch Bar that
//! silently ignores half of what it was told. `gap = -4.0` and `flex = 0.0` are accepted by
//! serde and rejected here.
//!
//! Key actions are validated names (`KeyName`), not `input_linux::Key` — that crate does not
//! build on macOS, and this crate must build where M1's exit criterion runs (see `hit.rs`).
//!
//! This crate does no I/O (D3): `parse` takes a `&str` and knows no path. Reading
//! `/etc/listello/config.toml` and the inotify reload live in `listello-core`.

use crate::hit::KeyName;
use crate::widget::Sizing;
use serde::Deserialize;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiConfig {
    #[serde(default)]
    pub strip: StripConfig,
    /// `[[widget]]` in TOML, `widgets` in Rust: the singular reads better in a config file and
    /// the plural reads better in code.
    #[serde(default, rename = "widget")]
    pub widgets: Vec<WidgetConfig>,
    /// `[[fn_widget]]` in TOML: the strip shown instead of `widgets` while Fn is held (D6).
    /// Empty — the default — means holding Fn changes nothing.
    #[serde(default, rename = "fn_widget")]
    pub fn_widgets: Vec<WidgetConfig>,
    #[serde(default)]
    pub general: GeneralConfig,
    /// `[core]` shares this file but belongs to `listello-core` (its `CoreSettings`). Named
    /// here only so `deny_unknown_fields` accepts it; its contents are not this crate's to check.
    #[serde(default, rename = "core")]
    _core: serde::de::IgnoredAny,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StripConfig {
    /// Logical pixels between adjacent widgets. Never at the ends of the strip.
    #[serde(default = "default_gap")]
    pub gap: f64,
    /// Seconds a finger rests on an open app in the Dock before its window controls (close,
    /// minimise, maximise) appear. Shorter than this is a tap.
    #[serde(default = "default_hold_seconds")]
    pub hold_seconds: f64,
    /// Seconds the window controls stay up with nothing pressed. A touch anywhere else closes
    /// them sooner.
    #[serde(default = "default_controls_seconds")]
    pub controls_seconds: f64,
    /// Whether an app's row of windows shows a picture of each window (D7).
    #[serde(default)]
    pub window_previews: WindowPreviews,
    /// How that picture sits in a window's entry, when there is one.
    #[serde(default)]
    pub preview_style: PreviewStyle,
}

impl Default for StripConfig {
    fn default() -> Self {
        StripConfig {
            gap: default_gap(),
            hold_seconds: default_hold_seconds(),
            controls_seconds: default_controls_seconds(),
            window_previews: WindowPreviews::default(),
            preview_style: PreviewStyle::default(),
        }
    }
}

/// `[strip] preview_style`: how a window's picture sits in its entry in the row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewStyle {
    /// The whole window, small, at the entry's left; the title beside it.
    Whole,
    /// A band through the middle of the window, its full width, filling the entry, the title
    /// over it.
    #[default]
    Fill,
    /// The whole window squashed into the entry, the title over it.
    Stretch,
}

/// `[strip] window_previews` (D7).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowPreviews {
    /// Titles only, as before previews existed. Nothing is ever captured.
    #[default]
    Off,
    /// A still of each window when the row opens, refreshed every second and a half while it
    /// stays open.
    Snapshots,
    /// A fresh picture whenever a window changes, at most ten a second, while the row is open.
    /// Falls back to snapshots where the compositor will not stream a window.
    Live,
}

/// `[general]`: settings that are about the user rather than one widget (D8 amendment).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeneralConfig {
    #[serde(default)]
    pub language: Language,
    /// An icon in the system tray, with Open settings and Reload listello. Off unless the user
    /// turns it on. Only the settings app reads it; the renderer has no tray.
    #[serde(default)]
    pub tray: bool,
}

/// `[general] language`: the language of the text listello itself puts on the strip — today,
/// a clock's day and month names. A clock's own `locale` still wins for that clock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    /// The system's own date and time format (`LC_TIME`, else `LANG`), which on KDE is the
    /// Region & Language page's "Time" setting and may differ from the display language.
    #[default]
    System,
    Italian,
    English,
}

/// A clock's locale by its POSIX name. `it_IT.UTF-8` is accepted as `it_IT`, since that is how
/// `locale` prints it.
pub fn clock_locale(name: &str) -> Option<chrono::Locale> {
    let base = name.split(['.', '@']).next().unwrap_or(name);
    chrono::Locale::try_from(base).ok()
}

/// A clock's place in its slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockAlign {
    Start,
    #[default]
    Center,
    End,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WidgetConfig {
    Button {
        #[serde(default)]
        label: String,
        /// A packaged icon name. Alone it is centred; with a label it sits to its left.
        #[serde(default)]
        icon: Option<String>,
        #[serde(default)]
        keys: Vec<String>,
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        flex: Option<f64>,
    },
    Clock {
        #[serde(default = "default_clock_format")]
        format: String,
        /// Where the time sits in its slot: `"start"` keeps it one strip gap from the widget
        /// before, however long the day and month names are.
        #[serde(default)]
        align: ClockAlign,
        /// The language of day and month names, as a POSIX locale name: `"it_IT"`, `"en_GB"`,
        /// `"de_DE"`. Unset is English. Only names change; the layout is `format`'s.
        #[serde(default)]
        locale: Option<String>,
        /// With neither `width` nor `flex`, a clock is as wide as its text plus a margin, and
        /// the room it does not need goes to the flexible widgets.
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        flex: Option<f64>,
    },
    Separator {
        #[serde(default = "default_separator_width")]
        width: f64,
    },
    /// The Dock (M4): one tile per application, pinned ones first.
    Dock {
        /// Desktop file ids of the apps to keep in the Dock whether or not they are running,
        /// in this order: `org.kde.dolphin`, `firefox`.
        #[serde(default)]
        pinned: Vec<String>,
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        flex: Option<f64>,
    },
    /// The battery: a bar, a percentage, or both. Draws nothing on a machine without one.
    Battery {
        #[serde(default = "default_true")]
        show_percentage: bool,
        #[serde(default = "default_true")]
        show_bar: bool,
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        flex: Option<f64>,
    },
    Network {
        /// Draw the network name (ssid) next to the glyph. Default false — the strip is narrow
        /// and M5's strip does not ask for it.
        #[serde(default)]
        show_ssid: bool,
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        flex: Option<f64>,
    },
    /// A row of buttons sharing one slot — M5's control centre. `[[widget.button]]` in TOML.
    ///
    /// Buttons, not sliders (ROADMAP M5, deliberately different from Pock): five of the
    /// six are key events `listello-core` emits through uinput, and only lock is a
    /// [`crate::hit::Action::System`]. None of them go through the agent — D11's comment on
    /// `SystemAction` explains why (`listello-protocol`).
    ///
    /// Composite (design decision 3): one widget, one `Ctx::index`, and one hit region per
    /// button via `ctx.region(part)`, where `part` is the button's position in the row.
    ControlCenter {
        #[serde(default, rename = "button")]
        buttons: Vec<ControlButton>,
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        flex: Option<f64>,
    },
    /// A row of folder shortcuts — `[[widget.folder]]` in TOML. A tap opens the folder in the
    /// user's file manager; Trash can swap its icon when it is empty.
    ///
    /// Composite like the control centre: one widget, one hit region per folder.
    Folders {
        #[serde(default, rename = "folder")]
        folders: Vec<FolderConfig>,
        #[serde(default)]
        width: Option<f64>,
        #[serde(default)]
        flex: Option<f64>,
    },
}

/// One `[[widget.folder]]` entry of a folders widget.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FolderConfig {
    /// Names this entry between the config and the agent: the agent reports "this folder is
    /// empty" under this key. Unique within the widget.
    pub key: String,
    /// What a tap opens. An opaque string to the renderer (D11): it is never read here.
    pub path: String,
    /// Text shown when the icon is missing.
    #[serde(default)]
    pub label: Option<String>,
    /// A packaged icon name.
    pub icon: String,
    /// Shown instead of `icon` while the folder is empty — Trash wants this, the rest do not.
    #[serde(default)]
    pub empty_icon: Option<String>,
}

/// One `[[widget.button]]` entry of a control centre: an icon, an optional label, and exactly
/// one of `keys` (a [`crate::hit::KeyName`] list) or `system` (`"lock_session"`) — enforced by
/// `validate`, because both and neither are config mistakes a Touch Bar should refuse loudly.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlButton {
    /// A packaged icon name — `"lock"`, `"brightness-down"`, `"volume-up"`, … Resolved against
    /// the `Icons` store by the binary; a missing icon draws the label instead (W1 decision 1).
    pub icon: String,
    /// Optional text under or beside the icon. Absent on icon-only buttons.
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub keys: Vec<String>,
    /// The one system action M5 has: `"lock_session"`.
    #[serde(default)]
    pub system: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_gap() -> f64 {
    8.0
}

fn default_hold_seconds() -> f64 {
    0.5
}

fn default_controls_seconds() -> f64 {
    5.0
}

fn default_clock_format() -> String {
    "%H:%M".into()
}

fn default_separator_width() -> f64 {
    2.0
}

impl UiConfig {
    /// The `(key, path)` of every folder the strip shows, for the binary to hand to the agent
    /// to watch. A key used twice (it cannot be within one widget, but two widgets may) is
    /// listed once.
    pub fn watched_folders(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for widget in self.widgets.iter().chain(&self.fn_widgets) {
            if let WidgetConfig::Folders { folders, .. } = widget {
                for folder in folders {
                    if !out.iter().any(|(key, _)| *key == folder.key) {
                        out.push((folder.key.clone(), folder.path.clone()));
                    }
                }
            }
        }
        out
    }

    /// Every packaged icon name this config's widgets can draw, for the binary to load at
    /// startup. The network widget always asks for its five glyphs by fixed name; every other
    /// name is whatever the config wrote.
    pub fn icon_names(&self) -> std::collections::BTreeSet<String> {
        let mut names = std::collections::BTreeSet::new();
        for widget in self.widgets.iter().chain(&self.fn_widgets) {
            match widget {
                WidgetConfig::Button {
                    icon: Some(icon), ..
                } => {
                    names.insert(icon.clone());
                }
                WidgetConfig::ControlCenter { buttons, .. } => {
                    names.extend(buttons.iter().map(|b| b.icon.clone()));
                }
                WidgetConfig::Folders { folders, .. } => {
                    for folder in folders {
                        names.insert(folder.icon.clone());
                        names.extend(folder.empty_icon.clone());
                    }
                }
                WidgetConfig::Battery { .. } => {
                    names.insert("bolt".to_string());
                }
                WidgetConfig::Dock { .. } => {
                    names.extend(["pin".to_string(), "pin-off".to_string()]);
                }
                WidgetConfig::Network { .. } => {
                    names.extend(
                        [
                            "network-0",
                            "network-1",
                            "network-2",
                            "network-3",
                            "network-off",
                        ]
                        .map(String::from),
                    );
                }
                _ => {}
            }
        }
        names
    }

    /// Parse and validate. The only entry point callers should use.
    pub fn parse(toml_src: &str) -> Result<UiConfig, ConfigError> {
        let config: UiConfig = toml::from_str(toml_src).map_err(|e| ConfigError {
            widget: None,
            message: e.to_string(),
        })?;
        config.validate()?;
        Ok(config)
    }

    /// Give every clock that names no `locale` of its own the one `[general] language` asks
    /// for. `system_time_locale` is what "the system" means where this config is shown — the
    /// user's `LC_TIME` while their agent is connected, the machine's otherwise; a name
    /// listello does not know leaves the clock in English.
    pub fn resolve_language(&mut self, system_time_locale: Option<&str>) {
        let name = match self.general.language {
            Language::Italian => Some("it_IT"),
            Language::English => Some("en_US"),
            Language::System => system_time_locale.filter(|n| clock_locale(n).is_some()),
        };
        let Some(name) = name else { return };
        for widget in self.widgets.iter_mut().chain(self.fn_widgets.iter_mut()) {
            if let WidgetConfig::Clock { locale, .. } = widget {
                if locale.is_none() {
                    *locale = Some(name.to_string());
                }
            }
        }
    }

    /// Validate an already-parsed config. Called by `parse`.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !self.strip.gap.is_finite() || self.strip.gap < 0.0 {
            return Err(ConfigError {
                widget: None,
                message: format!(
                    "strip.gap must be finite and not negative, got {}",
                    self.strip.gap
                ),
            });
        }
        for (name, value) in [
            ("hold_seconds", self.strip.hold_seconds),
            ("controls_seconds", self.strip.controls_seconds),
        ] {
            if !value.is_finite() || value <= 0.0 {
                return Err(ConfigError {
                    widget: None,
                    message: format!(
                        "strip.{name} must be a positive number of seconds, got {value}"
                    ),
                });
            }
        }
        for (index, widget) in self.widgets.iter().enumerate() {
            widget.validate(index)?;
        }
        for (index, widget) in self.fn_widgets.iter().enumerate() {
            widget.validate(index).map_err(|mut e| {
                e.message = format!("fn_widget: {}", e.message);
                e
            })?;
        }
        Ok(())
    }
}

impl WidgetConfig {
    /// What this widget asks layout for. `width` wins if set; otherwise `flex`; otherwise
    /// `Flex(1.0)`. A separator is always `Fixed`.
    pub fn sizing(&self) -> Sizing {
        match self {
            WidgetConfig::Button { width, flex, .. } => Self::resolve(*width, *flex),
            WidgetConfig::Clock { width, flex, .. } => Self::resolve(*width, *flex),
            WidgetConfig::Separator { width } => Sizing::Fixed(*width),
            WidgetConfig::Network { width, flex, .. } => Self::resolve(*width, *flex),
            WidgetConfig::Battery { width, flex, .. } => Self::resolve(*width, *flex),
            WidgetConfig::Dock { width, flex, .. } => Self::resolve(*width, *flex),
            WidgetConfig::ControlCenter { width, flex, .. } => Self::resolve(*width, *flex),
            WidgetConfig::Folders { width, flex, .. } => Self::resolve(*width, *flex),
        }
    }

    fn resolve(width: Option<f64>, flex: Option<f64>) -> Sizing {
        if let Some(width) = width {
            Sizing::Fixed(width)
        } else if let Some(flex) = flex {
            Sizing::Flex(flex)
        } else {
            Sizing::Flex(1.0)
        }
    }

    fn validate(&self, index: usize) -> Result<(), ConfigError> {
        match self {
            WidgetConfig::Button {
                label,
                icon,
                keys,
                width,
                flex,
            } => {
                if label.is_empty() && keys.is_empty() && icon.is_none() {
                    return Err(ConfigError {
                        widget: Some(index),
                        message: "label must not be empty unless the button has an icon or at \
                                  least one key"
                            .into(),
                    });
                }
                if icon.as_deref() == Some("") {
                    return Err(ConfigError {
                        widget: Some(index),
                        message: "icon must not be empty".into(),
                    });
                }
                for key in keys {
                    if KeyName::new(key.clone()).is_none() {
                        return Err(ConfigError {
                            widget: Some(index),
                            message: format!("invalid key name: {key}"),
                        });
                    }
                }
                Self::validate_width_flex(index, *width, *flex)
            }
            WidgetConfig::Clock {
                format,
                locale,
                width,
                flex,
                ..
            } => {
                if format.is_empty() {
                    return Err(ConfigError {
                        widget: Some(index),
                        message: "clock format must not be empty".into(),
                    });
                }
                if let Some(name) = locale {
                    if clock_locale(name).is_none() {
                        return Err(ConfigError {
                            widget: Some(index),
                            message: format!(
                                "clock locale {name:?} is not one listello knows; \
                                 use a name like \"it_IT\" or \"en_GB\""
                            ),
                        });
                    }
                }
                Self::validate_width_flex(index, *width, *flex)
            }
            WidgetConfig::Separator { width } => {
                if !width.is_finite() || *width < 0.0 {
                    return Err(ConfigError {
                        widget: Some(index),
                        message: format!(
                            "separator width must be finite and not negative, got {width}"
                        ),
                    });
                }
                Ok(())
            }
            WidgetConfig::Network {
                show_ssid: _,
                width,
                flex,
            } => Self::validate_width_flex(index, *width, *flex),
            WidgetConfig::Battery { width, flex, .. } => {
                Self::validate_width_flex(index, *width, *flex)
            }
            WidgetConfig::Folders {
                folders,
                width,
                flex,
            } => {
                Self::validate_width_flex(index, *width, *flex)?;
                let fail = |message: String| ConfigError {
                    widget: Some(index),
                    message,
                };
                for (findex, folder) in folders.iter().enumerate() {
                    if folder.key.trim().is_empty() {
                        return Err(fail(format!("folder {findex}: key must not be empty")));
                    }
                    if folder.path.trim().is_empty() {
                        return Err(fail(format!("folder {findex}: path must not be empty")));
                    }
                    if folder.icon.is_empty() {
                        return Err(fail(format!("folder {findex}: icon must not be empty")));
                    }
                    if folders[..findex].iter().any(|f| f.key == folder.key) {
                        return Err(fail(format!(
                            "folder {findex}: duplicate key {:?}",
                            folder.key
                        )));
                    }
                }
                Ok(())
            }
            WidgetConfig::Dock {
                pinned,
                width,
                flex,
            } => {
                if let Some(bad) = pinned.iter().find(|p| p.trim().is_empty()) {
                    return Err(ConfigError {
                        widget: Some(index),
                        message: format!("pinned app ids must not be empty, got {bad:?}"),
                    });
                }
                Self::validate_width_flex(index, *width, *flex)
            }
            WidgetConfig::ControlCenter {
                buttons,
                width,
                flex,
            } => {
                Self::validate_width_flex(index, *width, *flex)?;
                for (bindex, button) in buttons.iter().enumerate() {
                    let both = !button.keys.is_empty() && button.system.is_some();
                    let neither = button.keys.is_empty() && button.system.is_none();
                    if both || neither {
                        return Err(ConfigError {
                            widget: Some(index),
                            message: format!(
                                "control centre button {} must set exactly one of `keys` or \
                                 `system` (it has {} of the two)",
                                bindex,
                                if both { "both" } else { "neither" }
                            ),
                        });
                    }
                    if button.icon.is_empty() {
                        return Err(ConfigError {
                            widget: Some(index),
                            message: format!(
                                "control centre button {bindex}: icon must not be empty"
                            ),
                        });
                    }
                    for key in &button.keys {
                        if KeyName::new(key.clone()).is_none() {
                            return Err(ConfigError {
                                widget: Some(index),
                                message: format!(
                                    "control centre button {bindex}: invalid key name: {key}"
                                ),
                            });
                        }
                    }
                    if let Some(system) = &button.system {
                        if system != "lock_session" {
                            return Err(ConfigError {
                                widget: Some(index),
                                message: format!(
                                    "control centre button {bindex}: unknown system action: \
                                     {system} (the only one is \"lock_session\")"
                                ),
                            });
                        }
                    }
                }
                Ok(())
            }
        }
    }

    fn validate_width_flex(
        index: usize,
        width: Option<f64>,
        flex: Option<f64>,
    ) -> Result<(), ConfigError> {
        if width.is_some() && flex.is_some() {
            return Err(ConfigError {
                widget: Some(index),
                message: "width and flex are mutually exclusive; set one or the other".into(),
            });
        }
        if let Some(width) = width {
            if !width.is_finite() || width < 0.0 {
                return Err(ConfigError {
                    widget: Some(index),
                    message: format!("width must be finite and not negative, got {width}"),
                });
            }
        }
        if let Some(flex) = flex {
            if !flex.is_finite() || flex <= 0.0 {
                return Err(ConfigError {
                    widget: Some(index),
                    message: format!("flex must be greater than zero, got {flex}"),
                });
            }
        }
        Ok(())
    }
}

/// Why a config was refused. Carries the widget's index so the message can point at a line the
/// user can find.
#[derive(Clone, Debug, PartialEq)]
pub struct ConfigError {
    /// Index into `UiConfig::widgets`, or `None` for a whole-file problem.
    pub widget: Option<usize>,
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.widget {
            Some(index) => write!(f, "widget {index}: {}", self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// A complete button whose body can be extended with `width`/`flex`/`keys`/a label override.
    fn button(body: &str) -> String {
        format!("[[widget]]\nkind = \"button\"\n{body}\n")
    }

    fn parse_ok(src: impl AsRef<str>) -> UiConfig {
        UiConfig::parse(src.as_ref()).expect("config should parse and validate")
    }

    fn reject(src: impl AsRef<str>) -> ConfigError {
        UiConfig::parse(src.as_ref()).expect_err("config should be refused")
    }

    /// The template installed to `/usr/share/listello` is the config the daemon falls back to
    /// with no user file. If it does not parse, the daemon panics on first start.
    /// The example layout shipped next to the template must stay a config that loads, and it
    /// must use every widget kind M5 added.
    #[test]
    fn the_example_layout_parses_and_uses_every_m5_widget() {
        let cfg = parse_ok(include_str!(
            "../../../share/listello/config.example-dock.toml"
        ));
        let kinds: Vec<&str> = cfg
            .widgets
            .iter()
            .map(|w| match w {
                WidgetConfig::Dock { .. } => "dock",
                WidgetConfig::Separator { .. } => "separator",
                WidgetConfig::Folders { .. } => "folders",
                WidgetConfig::ControlCenter { .. } => "control_center",
                WidgetConfig::Network { .. } => "network",
                WidgetConfig::Battery { .. } => "battery",
                WidgetConfig::Clock { .. } => "clock",
                _ => "other",
            })
            .collect();
        assert_eq!(
            kinds,
            [
                "dock",
                "separator",
                "folders",
                "control_center",
                "network",
                "battery",
                "clock"
            ]
        );
        assert_eq!(cfg.fn_widgets.len(), 12);
        assert_eq!(cfg.watched_folders().len(), 3);
    }

    #[test]
    fn the_long_press_times_default_and_must_be_positive() {
        let c = parse_ok("[[widget]]\nkind = \"dock\"\n");
        assert_eq!(c.strip.hold_seconds, 0.5);
        assert_eq!(c.strip.controls_seconds, 5.0);
        for bad in [
            "hold_seconds = 0.0",
            "controls_seconds = -1.0",
            "hold_seconds = nan",
        ] {
            let text = format!("[strip]\n{bad}\n[[widget]]\nkind = \"dock\"\n");
            assert!(UiConfig::parse(&text).is_err(), "{bad}");
        }
    }

    #[test]
    fn window_previews_are_off_unless_asked_for() {
        let c = parse_ok("[[widget]]\nkind = \"dock\"\n");
        assert_eq!(c.strip.window_previews, WindowPreviews::Off);
        let c = parse_ok("[strip]\nwindow_previews = \"snapshots\"\n[[widget]]\nkind = \"dock\"\n");
        assert_eq!(c.strip.window_previews, WindowPreviews::Snapshots);
        let c = parse_ok("[strip]\nwindow_previews = \"live\"\n[[widget]]\nkind = \"dock\"\n");
        assert_eq!(c.strip.window_previews, WindowPreviews::Live);
        let c = parse_ok("[[widget]]\nkind = \"dock\"\n");
        assert_eq!(c.strip.preview_style, PreviewStyle::Fill);
        for style in ["whole", "fill", "stretch"] {
            let text =
                format!("[strip]\npreview_style = \"{style}\"\n[[widget]]\nkind = \"dock\"\n");
            assert!(UiConfig::parse(&text).is_ok(), "{style}");
        }
        assert!(UiConfig::parse(
            "[strip]\npreview_style = \"zoom\"\n[[widget]]\nkind = \"dock\"\n"
        )
        .is_err());
        for bad in ["\"video\"", "true", "\"on\""] {
            let text = format!("[strip]\nwindow_previews = {bad}\n[[widget]]\nkind = \"dock\"\n");
            assert!(UiConfig::parse(&text).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_clock_locale_is_a_posix_name_listello_knows() {
        for good in ["it_IT", "it_IT.UTF-8", "en_GB", "de_DE"] {
            let text = format!("[[widget]]\nkind = \"clock\"\nlocale = \"{good}\"\n");
            assert!(UiConfig::parse(&text).is_ok(), "{good}");
        }
        for bad in ["italian", "xx_YY", ""] {
            let text = format!("[[widget]]\nkind = \"clock\"\nlocale = \"{bad}\"\n");
            assert!(UiConfig::parse(&text).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_tray_is_off_unless_turned_on() {
        let dock = "[[widget]]\nkind = \"dock\"\n";
        assert!(!parse_ok(dock).general.tray);
        assert!(
            parse_ok(format!("[general]\ntray = true\n{dock}"))
                .general
                .tray
        );
        reject(format!("[general]\ntray = \"yes\"\n{dock}"));
    }

    #[test]
    fn the_language_names_a_clocks_day_names_unless_the_clock_names_its_own() {
        let clock_locale_of = |c: &UiConfig, i: usize| match &c.widgets[i] {
            WidgetConfig::Clock { locale, .. } => locale.clone(),
            _ => panic!("not a clock"),
        };
        let two_clocks =
            "[[widget]]\nkind = \"clock\"\n[[widget]]\nkind = \"clock\"\nlocale = \"de_DE\"\n";
        let mut c = parse_ok(format!("[general]\nlanguage = \"italian\"\n{two_clocks}"));
        c.resolve_language(Some("en_US.UTF-8"));
        assert_eq!(clock_locale_of(&c, 0).as_deref(), Some("it_IT"));
        assert_eq!(clock_locale_of(&c, 1).as_deref(), Some("de_DE"));

        let mut c = parse_ok(format!("[general]\nlanguage = \"english\"\n{two_clocks}"));
        c.resolve_language(Some("it_IT.UTF-8"));
        assert_eq!(clock_locale_of(&c, 0).as_deref(), Some("en_US"));

        // "system" is the default, and follows whatever the system's time format is.
        let mut c = parse_ok(two_clocks);
        assert_eq!(c.general.language, Language::System);
        c.resolve_language(Some("it_IT.UTF-8"));
        assert_eq!(clock_locale_of(&c, 0).as_deref(), Some("it_IT.UTF-8"));
        let mut c = parse_ok(two_clocks);
        c.resolve_language(Some("C"));
        assert_eq!(clock_locale_of(&c, 0), None);

        for bad in ["\"french\"", "\"it_IT\"", "1"] {
            let text = format!("[general]\nlanguage = {bad}\n");
            assert!(UiConfig::parse(&text).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_clock_can_sit_at_the_start_of_its_slot() {
        let c = parse_ok("[[widget]]\nkind = \"clock\"\nalign = \"start\"\n");
        assert!(matches!(
            &c.widgets[0],
            WidgetConfig::Clock {
                align: ClockAlign::Start,
                ..
            }
        ));
        assert!(UiConfig::parse("[[widget]]\nkind = \"clock\"\nalign = \"left\"\n").is_err());
    }

    #[test]
    fn a_dock_takes_pinned_apps_and_a_size() {
        let c = parse_ok("[[widget]]\nkind = \"dock\"\npinned = [\"org.kde.dolphin\", \"firefox\"]\nflex = 2.0\n");
        assert!(matches!(
            &c.widgets[0],
            WidgetConfig::Dock { pinned, .. } if pinned == &["org.kde.dolphin", "firefox"]
        ));
        assert_eq!(c.widgets[0].sizing(), Sizing::Flex(2.0));
        // Pinning nothing is fine: only apps with windows show.
        parse_ok("[[widget]]\nkind = \"dock\"\n");
    }

    #[test]
    fn a_dock_rejects_an_empty_pin_and_unknown_keys() {
        let err = reject("[[widget]]\nkind = \"dock\"\npinned = [\"  \"]\n");
        assert_eq!(err.widget, Some(0));
        assert!(UiConfig::parse("[[widget]]\nkind = \"dock\"\npin = []\n").is_err());
    }

    #[test]
    fn fn_widgets_parse_and_default_to_empty() {
        let plain = parse_ok(button("label = \"A\""));
        assert!(plain.fn_widgets.is_empty());
        let layered = parse_ok(format!(
            "{}[[fn_widget]]\nkind = \"button\"\nlabel = \"F1\"\nkeys = [\"KEY_F1\"]\n",
            button("label = \"A\"")
        ));
        assert_eq!(layered.fn_widgets.len(), 1);
    }

    #[test]
    fn a_bad_fn_widget_is_rejected_and_named() {
        let err = UiConfig::parse(
            "[[fn_widget]]\nkind = \"button\"\nlabel = \"F1\"\nkeys = [\"bad name\"]\n",
        )
        .unwrap_err();
        assert!(err.message.starts_with("fn_widget:"), "{err}");
    }

    #[test]
    fn shipped_template_parses() {
        parse_ok(include_str!("../../../share/listello/config.toml"));
    }

    #[test]
    fn rejects_unknown_top_level_table() {
        reject("[cor]\nadaptive_brightness = true");
    }

    #[test]
    fn rejects_negative_gap() {
        let err = reject("[strip]\ngap = -4.0");
        assert_eq!(err.widget, None);
        assert!(err.message.contains("gap"), "{err}");
    }

    #[test]
    fn rejects_nan_gap() {
        let err = reject("[strip]\ngap = nan");
        assert_eq!(err.widget, None);
        assert!(err.message.contains("gap"), "{err}");
    }

    #[test]
    fn rejects_width_and_flex_together() {
        let err = reject(button("label = \"x\"\nwidth = 10.0\nflex = 1.0"));
        assert_eq!(err.widget, Some(0));
        assert!(err.message.contains("width"), "{err}");
        assert!(err.message.contains("flex"), "{err}");
    }

    #[test]
    fn rejects_zero_flex() {
        let err = reject(button("label = \"x\"\nflex = 0.0"));
        assert_eq!(err.widget, Some(0));
        assert!(err.message.contains("flex"), "{err}");
    }

    #[test]
    fn rejects_negative_width() {
        let err = reject(button("label = \"x\"\nwidth = -1.0"));
        assert_eq!(err.widget, Some(0));
        assert!(err.message.contains("width"), "{err}");
    }

    #[test]
    fn a_folders_widget_parses_its_entries_and_the_optional_empty_icon() {
        let cfg = parse_ok(
            "[[widget]]
kind = \"folders\"
[[widget.folder]]
key = \"down\"
path = \"/home/g/Downloads\"
icon = \"folder-download\"
[[widget.folder]]
key = \"trash\"
path = \"trash:/\"
label = \"Trash\"
icon = \"trash-full\"
empty_icon = \"trash-empty\"
",
        );
        let WidgetConfig::Folders { folders, .. } = &cfg.widgets[0] else {
            panic!("expected a folders widget");
        };
        assert_eq!(folders.len(), 2);
        assert_eq!(folders[0].empty_icon, None);
        assert_eq!(folders[1].empty_icon.as_deref(), Some("trash-empty"));
    }

    #[test]
    fn a_folders_widget_rejects_duplicate_keys_and_empty_fields() {
        let folder = |key: &str, path: &str| {
            format!("[[widget.folder]]\nkey = \"{key}\"\npath = \"{path}\"\nicon = \"i\"\n")
        };
        let widget = |body: String| format!("[[widget]]\nkind = \"folders\"\n{body}");

        let dup = reject(widget(folder("a", "/x") + &folder("a", "/y")));
        assert_eq!(dup.widget, Some(0));
        assert!(dup.message.contains("duplicate"), "{dup}");
        assert!(reject(widget(folder("", "/x"))).message.contains("key"));
        assert!(reject(widget(folder("a", " "))).message.contains("path"));
    }

    #[test]
    fn error_index_points_at_the_offending_widget() {
        let src = "\
[[widget]]
kind = \"button\"
label = \"ok\"

[[widget]]
kind = \"button\"
label = \"bad\"
flex = -1.0
";
        let err = reject(src);
        assert_eq!(err.widget, Some(1));
        assert!(err.message.contains("flex"), "{err}");
    }

    #[test]
    fn a_button_may_be_an_icon_with_no_label_and_no_keys() {
        let cfg = parse_ok(button("icon = \"mute\""));
        assert!(matches!(
            &cfg.widgets[0],
            WidgetConfig::Button { icon: Some(i), label, .. } if i == "mute" && label.is_empty()
        ));
        let err = reject(button("icon = \"\"\nlabel = \"x\""));
        assert!(err.message.contains("icon"), "{err}");
    }

    #[test]
    fn watched_folders_lists_each_key_once_in_order() {
        let cfg = parse_ok(
            "[[widget]]
kind = \"folders\"
[[widget.folder]]
key = \"a\"
path = \"/a\"
icon = \"i\"
[[widget.folder]]
key = \"b\"
path = \"trash:/\"
icon = \"i\"
[[widget]]
kind = \"folders\"
[[widget.folder]]
key = \"a\"
path = \"/elsewhere\"
icon = \"i\"
",
        );
        assert_eq!(
            cfg.watched_folders(),
            [("a".into(), "/a".into()), ("b".into(), "trash:/".into())]
        );
    }

    #[test]
    fn icon_names_collects_every_icon_a_config_can_draw() {
        let cfg = parse_ok(
            "[[widget]]
kind = \"button\"
icon = \"mute\"
[[widget]]
kind = \"button\"
label = \"F1\"
[[widget]]
kind = \"control_center\"
[[widget.button]]
icon = \"lock\"
system = \"lock_session\"
[[widget]]
kind = \"folders\"
[[widget.folder]]
key = \"t\"
path = \"trash:/\"
icon = \"trash-full\"
empty_icon = \"trash-empty\"
[[widget]]
kind = \"network\"
[[widget]]
kind = \"battery\"
",
        );
        let names: Vec<_> = cfg.icon_names().into_iter().collect();
        assert_eq!(
            names,
            [
                "bolt",
                "lock",
                "mute",
                "network-0",
                "network-1",
                "network-2",
                "network-3",
                "network-off",
                "trash-empty",
                "trash-full"
            ]
        );
    }

    #[test]
    fn rejects_empty_label_without_keys() {
        let err = reject(button("label = \"\""));
        assert_eq!(err.widget, Some(0));
        assert!(err.message.contains("label"), "{err}");
    }

    #[test]
    fn rejects_key_name_with_a_space() {
        let err = reject(button("label = \"x\"\nkeys = [\"KEY F5\"]"));
        assert_eq!(err.widget, Some(0));
        assert!(err.message.contains("KEY F5"), "{err}");
    }

    #[test]
    fn rejects_empty_key_name() {
        let err = reject(button("label = \"x\"\nkeys = [\"\"]"));
        assert_eq!(err.widget, Some(0));
        assert!(err.message.contains("key"), "{err}");
    }

    #[test]
    fn rejects_empty_clock_format() {
        let err = reject("[[widget]]\nkind = \"clock\"\nformat = \"\"");
        assert_eq!(err.widget, Some(0));
        assert!(err.message.contains("format"), "{err}");
    }

    #[test]
    fn rejects_unknown_widget_kind() {
        let err = reject("[[widget]]\nkind = \"weather\"");
        assert!(err.message.contains("weather"), "{err}");
    }

    #[test]
    fn rejects_unknown_field() {
        let err = reject(button("label = \"x\"\ncolour = \"red\""));
        assert!(err.message.contains("colour"), "{err}");
    }

    #[test]
    fn empty_input_parses_to_defaults() {
        let cfg = parse_ok("");
        assert_eq!(cfg.strip.gap, 8.0);
        assert!(cfg.widgets.is_empty());
    }

    #[test]
    fn empty_label_with_keys_is_accepted() {
        let cfg = parse_ok(button("label = \"\"\nkeys = [\"KEY_F5\"]"));
        assert_eq!(cfg.widgets.len(), 1);
    }

    #[test]
    fn sizing_from_flex() {
        let cfg = parse_ok(button("label = \"x\"\nflex = 0.5"));
        assert_eq!(cfg.widgets[0].sizing(), Sizing::Flex(0.5));
    }

    #[test]
    fn sizing_defaults_to_flex_one() {
        let cfg = parse_ok(button("label = \"x\""));
        assert_eq!(cfg.widgets[0].sizing(), Sizing::Flex(1.0));
    }

    #[test]
    fn sizing_from_width() {
        let cfg = parse_ok(button("label = \"x\"\nwidth = 120.0"));
        assert_eq!(cfg.widgets[0].sizing(), Sizing::Fixed(120.0));
    }

    #[test]
    fn separator_defaults_to_two_pixels() {
        let cfg = parse_ok("[[widget]]\nkind = \"separator\"");
        assert_eq!(cfg.widgets[0].sizing(), Sizing::Fixed(2.0));
    }

    #[test]
    fn clock_defaults_to_hhmm() {
        let cfg = parse_ok("[[widget]]\nkind = \"clock\"");
        match &cfg.widgets[0] {
            WidgetConfig::Clock { format, .. } => assert_eq!(format, "%H:%M"),
            other => panic!("expected a clock widget, got {other:?}"),
        }
    }

    #[test]
    fn network_defaults_ssid_hidden() {
        let cfg = parse_ok("[[widget]]\nkind = \"network\"");
        match &cfg.widgets[0] {
            WidgetConfig::Network {
                show_ssid, width, ..
            } => {
                assert!(!show_ssid);
                assert_eq!(*width, None);
            }
            other => panic!("expected a network widget, got {other:?}"),
        }
    }

    #[test]
    fn network_accepts_show_ssid_and_width() {
        let cfg = parse_ok("[[widget]]\nkind = \"network\"\nshow_ssid = true\nwidth = 48.0");
        match &cfg.widgets[0] {
            WidgetConfig::Network {
                show_ssid, width, ..
            } => {
                assert!(show_ssid);
                assert_eq!(*width, Some(48.0));
            }
            other => panic!("expected a network widget, got {other:?}"),
        }
        assert_eq!(cfg.widgets[0].sizing(), Sizing::Fixed(48.0));
    }

    #[test]
    fn full_example_parses_and_yields_expected_sizes() {
        let src = "\
[strip]
gap = 8.0

[[widget]]
kind = \"button\"
label = \"F5\"
keys = [\"KEY_F5\"]
width = 120.0

[[widget]]
kind = \"clock\"
format = \"%H:%M\"
flex = 1.0

[[widget]]
kind = \"separator\"
width = 2.0
";
        let cfg = parse_ok(src);
        assert_eq!(cfg.strip.gap, 8.0);
        assert_eq!(cfg.widgets.len(), 3);
        let sizes: Vec<Sizing> = cfg.widgets.iter().map(WidgetConfig::sizing).collect();
        assert_eq!(
            sizes,
            vec![Sizing::Fixed(120.0), Sizing::Flex(1.0), Sizing::Fixed(2.0)]
        );
    }

    mod control_center {
        use super::*;

        /// A control centre: the `[[widget]]` header plus `opts` (width/flex lines, if any),
        /// followed by the `[[widget.button]]` blocks. `opts` must come before the buttons —
        /// in TOML a key after `[[widget.button]]` belongs to that button table.
        fn cc(buttons: &str, opts: &str) -> String {
            format!("[[widget]]\nkind = \"control_center\"\n{opts}{buttons}")
        }

        fn button(icon: &str, action: &str) -> String {
            format!("[[widget.button]]\nicon = {icon}\n{action}\n")
        }

        /// The usual set: lock + brightness ∓, mute, volume ∓.
        fn full_buttons() -> String {
            format!(
                "{}\n{}\n{}\n{}\n{}\n{}",
                button("\"lock\"", "system = \"lock_session\""),
                button("\"brightness-down\"", "keys = [\"KEY_BRIGHTNESSDOWN\"]"),
                button("\"brightness-up\"", "keys = [\"KEY_BRIGHTNESSUP\"]"),
                button("\"mute\"", "keys = [\"KEY_MUTE\"]"),
                button("\"volume-down\"", "keys = [\"KEY_VOLUMEDOWN\"]"),
                button("\"volume-up\"", "keys = [\"KEY_VOLUMEUP\"]"),
            )
        }

        fn reject_button(err: ConfigError, want: &str) {
            assert_eq!(err.widget, Some(0));
            assert!(err.message.contains(want), "{err}");
        }

        #[test]
        fn six_button_row_parses_with_width() {
            let cfg = parse_ok(cc(&full_buttons(), "width = 240.0\n"));
            match &cfg.widgets[0] {
                WidgetConfig::ControlCenter { buttons, width, .. } => {
                    assert_eq!(buttons.len(), 6);
                    assert_eq!(*width, Some(240.0));
                    // Exactly one of keys/system per button, in the right shape.
                    assert_eq!(buttons[0].keys.len(), 0);
                    assert_eq!(buttons[0].system.as_deref(), Some("lock_session"));
                    assert_eq!(buttons[5].keys.len(), 1);
                    assert_eq!(buttons[5].system, None);
                }
                other => panic!("expected a control_center widget, got {other:?}"),
            }
            assert_eq!(cfg.widgets[0].sizing(), Sizing::Fixed(240.0));
        }

        #[test]
        fn keys_and_system_together_are_rejected() {
            let err = reject(cc(
                &format!(
                    "{}{}",
                    button("\"lock\"", "system = \"lock_session\""),
                    "keys = [\"KEY_MUTE\"]\n",
                ),
                "",
            ));
            reject_button(err, "exactly one");
        }

        #[test]
        fn neither_keys_nor_system_is_rejected() {
            let err = reject(cc(&button("\"lock\"", ""), ""));
            reject_button(err, "exactly one");
        }

        #[test]
        fn unknown_system_action_is_rejected() {
            let err = reject(cc(&button("\"lock\"", "system = \"shutdown\""), ""));
            reject_button(err, "shutdown");
        }

        #[test]
        fn invalid_key_name_inside_a_button_is_rejected() {
            let err = reject(cc(&button("\"mute\"", "keys = [\"KEY MUTE\"]"), ""));
            reject_button(err, "KEY MUTE");
        }

        #[test]
        fn empty_icon_is_rejected() {
            let err = reject(cc(&button("\"\"", "system = \"lock_session\""), ""));
            reject_button(err, "icon");
        }

        #[test]
        fn unknown_field_on_a_button_is_rejected() {
            let err = reject(cc(
                &button(
                    "\"lock\"",
                    "label = \"x\"\ncolour = \"red\"\nsystem = \"lock_session\"",
                ),
                "",
            ));
            assert!(err.message.contains("colour"), "{err}");
        }

        #[test]
        fn label_is_optional_and_accepted() {
            let cfg = parse_ok(cc(
                &button("\"lock\"", "label = \"Lock\"\nsystem = \"lock_session\""),
                "",
            ));
            match &cfg.widgets[0] {
                WidgetConfig::ControlCenter { buttons, .. } => {
                    assert_eq!(buttons[0].label.as_deref(), Some("Lock"));
                }
                other => panic!("expected a control_center widget, got {other:?}"),
            }
        }

        #[test]
        fn width_and_flex_still_cannot_both_be_set() {
            let err = reject(cc(&full_buttons(), "width = 10.0\nflex = 1.0\n"));
            assert_eq!(err.widget, Some(0));
            assert!(err.message.contains("width"), "{err}");
        }
    }
}
