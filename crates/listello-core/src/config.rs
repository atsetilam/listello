// Retained from tiny-dfr (https://github.com/AsahiLinux/tiny-dfr)
// Copyright The Asahi Linux Contributors
// SPDX-License-Identifier: MIT
//
// M1 phase 06: the fixed-row `ButtonConfig`/`ConfigProxy`/Fn-layer schema is gone, replaced by
// `listello_ui::UiConfig` (D3/D8's widget list). `ConfigManager` now owns loading and
// hot-reloading that instead, plus `CoreSettings` for the handful of settings that describe
// hardware (backlight for now) rather than a widget and so have no home in listello-ui, which
// knows nothing about hardware by design (D3). `load_font` is the one piece kept whole. The
// reload watch is on /etc/listello rather than the file, so a replaced config is noticed.
//
// Not carried forward, and why: `PrimaryLayerKeys`/`MediaLayerKeys`/`DoublePressSwitchLayers`
// described a fixed row of buttons with an Fn-triggered alternate layer; `UiConfig` describes
// an ordered widget list instead (D8: a personal layout is a config file, not this crate's
// default), and it has no layer concept yet. `ShowButtonOutlines`, `EnablePixelShift` and
// `FontTemplate` described rendering choices `Theme` and `pixel_shift.rs` used to make; neither
// has a place to plug into `Shell` yet (`Shell::frame` has no offset parameter, and `Theme` is
// built once by `main.rs`, not reloaded from this file). All of this is a real capability loss,
// not an oversight — it is what M1 dismantled and did not yet rebuild.

use crate::fonts::{FontConfig, Pattern};
use anyhow::{Context, Result};
use cairo::FontFace;
use freetype::Library as FtLibrary;
use listello_ui::UiConfig;
use nix::{
    errno::Errno,
    sys::inotify::{AddWatchFlags, InitFlags, Inotify, InotifyEvent, WatchDescriptor},
};
use serde::Deserialize;
use std::{fs::read_to_string, os::fd::AsFd, rc::Rc};

const USER_CFG_PATH: &str = "/etc/listello/config.toml";
const DEFAULT_CFG_PATH: &str = "/usr/share/listello/config.toml";

/// Hardware settings no widget can describe (D3: widgets are pure, no hardware access).
/// Deserialised leniently from the same file `UiConfig` reads, under its own `[core]` table —
/// no `deny_unknown_fields`, so it simply ignores `[strip]` and `[[widget]]`.
#[derive(Clone, Copy, Deserialize)]
#[serde(default)]
pub struct CoreSettings {
    pub adaptive_brightness: bool,
    pub active_brightness: u32,
}

impl Default for CoreSettings {
    fn default() -> CoreSettings {
        CoreSettings {
            adaptive_brightness: true,
            active_brightness: 128,
        }
    }
}

#[derive(Deserialize, Default)]
struct CoreSettingsFile {
    #[serde(default)]
    core: CoreSettings,
}

/// What one successful config (re)load produces: the widget strip and the hardware settings
/// that travel with it, since both come from the same file and a reload replaces both together.
pub struct LoadedConfig {
    pub ui: UiConfig,
    pub settings: CoreSettings,
}

fn read_config_text() -> Result<String> {
    read_to_string(USER_CFG_PATH)
        .or_else(|_| read_to_string(DEFAULT_CFG_PATH))
        .context("no readable config at /etc/listello or /usr/share/listello")
}

fn load_config() -> Result<LoadedConfig> {
    let text = read_config_text()?;
    let ui = UiConfig::parse(&text).map_err(|e| anyhow::anyhow!("{e}"))?;
    // A malformed `[core]` table is not fatal the way a malformed widget list is: it only
    // costs the daemon its backlight tuning, not its ability to show a strip at all. Fall back
    // to CoreSettings::default() rather than reject the whole file over it.
    let settings = toml::from_str::<CoreSettingsFile>(&text)
        .map(|f| f.core)
        .unwrap_or_default();
    Ok(LoadedConfig { ui, settings })
}

pub fn load_font(name: &str) -> Rc<FontFace> {
    let fontconfig = FontConfig::new();
    let mut pattern = Pattern::new(name);
    fontconfig.perform_substitutions(&mut pattern);
    let pat_match = match fontconfig.match_pattern(&pattern) {
        Ok(pat) => pat,
        Err(_) => panic!("Unable to find specified font. If you are using the default config, make sure you have at least one font installed")
    };
    let file_name = pat_match.get_file_name();
    let file_idx = pat_match.get_font_index();
    let ft_library = FtLibrary::init().unwrap();
    let face = ft_library.new_face(file_name, file_idx).unwrap();
    Rc::new(FontFace::create_from_ft(&face).unwrap())
}

pub struct ConfigManager {
    inotify_fd: Inotify,
    watch_desc: Option<WatchDescriptor>,
}

/// The directory holding `USER_CFG_PATH`, and the file's name inside it.
const USER_CFG_DIR: &str = "/etc/listello";
const USER_CFG_NAME: &str = "config.toml";

/// Watch the config's directory, not the file. A file watch follows one inode, so a config
/// replaced rather than edited in place (`install`, `cp` over a new file, most editors' atomic
/// save) left the watch on the old one and the new config was never noticed.
fn arm_inotify(inotify_fd: &Inotify) -> Option<WatchDescriptor> {
    let flags = AddWatchFlags::IN_MOVED_TO | AddWatchFlags::IN_CLOSE_WRITE;
    match inotify_fd.add_watch(USER_CFG_DIR, flags) {
        Ok(wd) => Some(wd),
        Err(Errno::ENOENT) => None,
        Err(e) => {
            eprintln!("listello: cannot watch {USER_CFG_DIR} for config changes: {e}");
            None
        }
    }
}

impl ConfigManager {
    pub fn new() -> ConfigManager {
        let inotify_fd = Inotify::init(InitFlags::IN_NONBLOCK).unwrap();
        let watch_desc = arm_inotify(&inotify_fd);
        ConfigManager {
            inotify_fd,
            watch_desc,
        }
    }

    /// The startup load. Unlike a reload, a bad config here has nothing to fall back to —
    /// `real_main` lets it panic into the crash bitmap, same as tiny-dfr always has.
    pub fn load_config(&self) -> Result<LoadedConfig> {
        load_config()
    }

    /// Whether the watched file changed and reparsed cleanly since the last call. `None` both
    /// when nothing happened and when a reparse failed — a bad edit-in-progress must not take
    /// a running daemon down, so the caller keeps whatever it already had (logged here, not
    /// there, since this is the one place that knows *why* nothing came back).
    pub fn poll_reload(&mut self) -> Option<LoadedConfig> {
        if self.watch_desc.is_none() {
            self.watch_desc = arm_inotify(&self.inotify_fd);
            return None;
        }
        match self.inotify_fd.read_events() {
            Err(Errno::EAGAIN) => None,
            r => self.handle_events(r),
        }
    }

    #[cold]
    fn handle_events(&mut self, evts: Result<Vec<InotifyEvent>, Errno>) -> Option<LoadedConfig> {
        let mut result = None;
        let Ok(evts) = evts else {
            return None;
        };
        for evt in evts {
            if Some(evt.wd) != self.watch_desc {
                continue;
            }
            // The directory itself went away: watch again once it is back.
            if evt.mask.contains(AddWatchFlags::IN_IGNORED) {
                self.watch_desc = None;
                continue;
            }
            if evt.name.as_deref() != Some(std::ffi::OsStr::new(USER_CFG_NAME)) {
                continue;
            }
            match load_config() {
                Ok(cfg) => result = Some(cfg),
                Err(e) => {
                    eprintln!("listello: config reload failed, keeping previous config: {e:#}")
                }
            }
        }
        result
    }

    pub fn fd(&self) -> &impl AsFd {
        &self.inotify_fd
    }
}
