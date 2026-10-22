//! The picture of the strip at the top of the window: drawn by the same `Shell` and the same
//! rasteriser as the panel (D8 amendment: never a second imitation of it), at the panel's own
//! size, with the user's real app icons.

use crate::model::Layer;
use listello_agent::desktop::{Catalog, Overrides};
use listello_protocol::{AgentToCore, AppId, BatteryState, Icon, NetworkState};
use listello_surface::{packaged, Color, PixelSurface, Size, Surface, TextStyle};
use listello_ui::{IconKey, Shell, Theme, UiConfig};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

/// Used only where no Touch Bar can be found (a machine without one): the T2 MacBook Pro's.
const FALLBACK_PANEL: Size = Size::new(2008, 60);
/// What `listello-core` asks the agent for, and so what the Dock draws.
const APP_ICON_PX: u32 = 40;

pub struct Picture {
    pub png: Vec<u8>,
    /// Each widget's left edge and width, in panel pixels.
    pub spans: Vec<(f64, f64)>,
    /// The same, with a narrow widget (a separator) widened into the gaps beside it, so it
    /// can be pointed at and dragged.
    pub targets: Vec<(f64, f64)>,
}

/// Narrower widgets than this, in panel pixels, are widened for pointing at.
const MIN_TARGET: f64 = 28.0;

fn targets(spans: &[(f64, f64)]) -> Vec<(f64, f64)> {
    spans
        .iter()
        .enumerate()
        .map(|(i, &(left, width))| {
            if width >= MIN_TARGET {
                return (left, width);
            }
            // Out into the gaps, never over a neighbour.
            let floor = i.checked_sub(1).map_or(left, |p| spans[p].0 + spans[p].1);
            let ceiling = spans.get(i + 1).map_or(left + width, |n| n.0);
            let grow = (MIN_TARGET - width) / 2.0;
            let start = (left - grow).max(floor);
            let end = (left + width + grow).min(ceiling);
            (start, end - start)
        })
        .collect()
}

pub struct Painter {
    pub panel: Size,
    share: PathBuf,
    catalog: Catalog,
    icon_theme: String,
    app_icons: HashMap<String, Option<Icon>>,
    battery: Option<BatteryState>,
}

impl Painter {
    pub fn new(share: PathBuf) -> Painter {
        Painter {
            panel: panel_size().unwrap_or(FALLBACK_PANEL),
            share,
            catalog: Catalog::scan(Overrides::load()),
            icon_theme: listello_agent::icons::current_theme(),
            app_icons: HashMap::new(),
            battery: battery(),
        }
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// An app's icon, looked up once.
    pub fn app_icon(&mut self, app: &str) -> Option<Icon> {
        let (catalog, theme) = (&self.catalog, &self.icon_theme);
        self.app_icons
            .entry(app.to_string())
            .or_insert_with(|| listello_agent::icons::icon_for(catalog, theme, app, APP_ICON_PX))
            .clone()
    }

    /// The strip (or the Fn layer) as `config` would show it, with `pins` in the Dock.
    pub fn paint(
        &mut self,
        config: &UiConfig,
        layer: Layer,
        pins: &[String],
        time_locale: Option<&str>,
    ) -> Picture {
        let mut config = config.clone();
        config.resolve_language(time_locale);
        // An empty Fn layer means holding Fn changes nothing; the picture says so by being
        // empty rather than by showing the strip again.
        if layer == Layer::Fn && config.fn_widgets.is_empty() {
            config.widgets.clear();
        }
        let now = chrono::Local::now().fixed_offset();
        let mut shell = Shell::new(&config, theme(), now);
        for (name, image) in packaged::load_icons(&self.share, &config.icon_names()) {
            shell.state_mut().icons.insert(IconKey::Named(name), image);
        }
        shell.set_agent_connected(true);
        shell.apply_agent(&AgentToCore::Pins(Some(
            pins.iter().map(|p| AppId::new(p.clone())).collect(),
        )));
        for app in shell.wanted_app_ids() {
            let icon = self.app_icon(app.as_str());
            shell.apply_agent(&AgentToCore::Icon { app_id: app, icon });
        }
        shell.apply_agent(&AgentToCore::Battery(self.battery));
        shell.apply_agent(&AgentToCore::Network(Some(NetworkState {
            connected: true,
            signal: 1.0,
            ssid: None,
        })));
        shell.state_mut().fn_held = layer == Layer::Fn;

        let spans: Vec<(f64, f64)> = shell
            .widget_rects(self.panel)
            .iter()
            .map(|r| (r.x, r.width))
            .collect();
        let targets = targets(&spans);
        let png = PixelSurface::new(self.panel)
            .and_then(|mut s| {
                s.present(&shell.frame(self.panel))?;
                s.png_bytes()
            })
            .unwrap_or_default();
        Picture {
            png,
            spans,
            targets,
        }
    }
}

/// `listello-core`'s theme, with cairo's own font lookup standing in for its fontconfig one;
/// both find the same Inter.
fn theme() -> Theme {
    let face =
        cairo::FontFace::toy_create("Inter", cairo::FontSlant::Normal, cairo::FontWeight::Normal)
            .map(Rc::new)
            .expect("cairo always has a toy font");
    Theme {
        text: TextStyle::new(face, 23.0),
        background: Color::BLACK,
        foreground: Color::WHITE,
        button: Color::grey(0.2),
        button_pressed: Color::grey(0.4),
        accent: Color::rgb(0.3, 0.85, 0.4),
        corner_radius: 8.0,
    }
}

/// The Touch Bar's size, landscape, read the way `listello-core` finds the panel: the
/// connected DRM output whose first mode is at least thirty times taller than it is wide. Its
/// sysfs entry is readable by anyone, unlike the device.
fn panel_size() -> Option<Size> {
    for entry in fs::read_dir("/sys/class/drm").ok()?.flatten() {
        let dir = entry.path();
        if fs::read_to_string(dir.join("status")).ok()?.trim() != "connected" {
            continue;
        }
        let Some(mode) = fs::read_to_string(dir.join("modes"))
            .ok()
            .and_then(|m| m.lines().next().map(str::to_string))
        else {
            continue;
        };
        let Some((w, h)) = mode.split_once('x') else {
            continue;
        };
        let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) else {
            continue;
        };
        if w > 0 && h / w >= 30 {
            return Some(Size::new(h, w));
        }
    }
    None
}

/// The battery as it is now, so the picture shows what the panel shows.
fn battery() -> Option<BatteryState> {
    for entry in fs::read_dir("/sys/class/power_supply").ok()?.flatten() {
        let dir = entry.path();
        if fs::read_to_string(dir.join("type")).ok()?.trim() != "Battery" {
            continue;
        }
        let capacity: f64 = fs::read_to_string(dir.join("capacity"))
            .ok()?
            .trim()
            .parse()
            .ok()?;
        let status = fs::read_to_string(dir.join("status")).unwrap_or_default();
        return Some(BatteryState {
            charge: (capacity / 100.0).clamp(0.0, 1.0),
            charging: matches!(status.trim(), "Charging" | "Full"),
            time_remaining_secs: None,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_separator_is_widened_into_its_gaps_and_no_further() {
        let spans = [(0.0, 100.0), (112.0, 2.0), (126.0, 100.0)];
        let t = super::targets(&spans);
        assert_eq!(t[0], (0.0, 100.0));
        assert_eq!(t[1], (100.0, 26.0));
        assert_eq!(t[2], (126.0, 100.0));
        let t = super::targets(&[(500.0, 2.0)]);
        assert_eq!(t[0], (500.0, 2.0), "alone, there is no gap to grow into");
    }
}
