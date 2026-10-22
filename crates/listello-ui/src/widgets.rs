//! The built-in widgets, each a pure function per D3: an immutable state snapshot and a
//! [`Rect`] in, draw commands and hit regions out.
//!
//! Nothing here may do I/O, read the clock, or hold a handle to the core — everything a widget
//! needs arrives in [`crate::UiState`], and everything it wants to happen is an
//! [`crate::hit::Action`] the shell decides on. Deterministic given its inputs, which is what
//! makes a widget testable by asserting on the command list it produced without rasterising
//! anything.

use crate::hit::Action;
use crate::state::IconKey;
use crate::widget::{Ctx, Output, Sizing, Widget};
use chrono::format::{Item, StrftimeItems};
use listello_protocol::{BatteryState, NetworkState};
use listello_surface::{measure_text, Align, DrawCmd, Rect};
use std::fmt::Write as _;

/// Room a clock that fits its text keeps after it: as much as the Dock's first icon has before
/// it at the strip's left end, so a clock last in the strip mirrors it.
const CLOCK_FIT_PAD: f64 = 20.0;

/// The time, formatted, in its slot.
pub struct ClockWidget {
    /// A `strftime` format string, from config. Default `"%H:%M"`.
    pub format: String,
    pub align: Align,
    /// The language of day and month names. Checked when the config was read.
    pub locale: chrono::Locale,
    /// Size the slot to the text plus [`CLOCK_FIT_PAD`] instead of `sizing`.
    pub fit: bool,
    pub sizing: Sizing,
}

impl ClockWidget {
    fn text(&self, now: &chrono::DateTime<chrono::FixedOffset>) -> String {
        let items: Vec<Item> = StrftimeItems::new_with_locale(&self.format, self.locale).collect();
        let mut text = String::new();
        if items.iter().any(|i| matches!(i, Item::Error))
            || write!(
                &mut text,
                "{}",
                now.format_localized_with_items(items.iter(), self.locale)
            )
            .is_err()
        {
            text = "??:??".to_string();
        }
        text
    }
}

impl Widget for ClockWidget {
    fn sizing(&self) -> Sizing {
        self.sizing
    }

    fn fitted_width(&self, ctx: &Ctx<'_>) -> Option<f64> {
        self.fit.then(|| {
            measure_text(&ctx.theme.text, &self.text(&ctx.state.now))
                .advance
                .ceil()
                + CLOCK_FIT_PAD
        })
    }

    fn render(&self, ctx: &Ctx<'_>, rect: Rect, out: &mut Output) {
        // Light grey, as Pock draws its status text.
        out.draw(DrawCmd::Text {
            rect,
            text: self.text(&ctx.state.now),
            style: ctx.theme.text.clone(),
            color: ctx.theme.foreground.with_alpha(0.8),
            align: self.align,
        });
    }
}

/// Space between a button's edge and its icon, and between its icon and its label.
const BUTTON_PADDING: f64 = 8.0;

/// A rounded rectangle with an optional label, that does something when pressed.
pub struct ButtonWidget {
    /// May be empty: an icon-only button is M4's, and a config may legitimately give a button
    /// keys and no label.
    pub label: String,
    /// A packaged icon name, resolved against the `Icons` store. With no label the icon is
    /// centred; with both, the icon sits on the left and the label takes the rest. An icon
    /// missing from the store draws the label alone — a normal state, never an error.
    pub icon: Option<String>,
    /// What a press on this button asks the shell to do. `Action::None` is fine: the button
    /// still lights up when held, and the shell can only know that from its hit region.
    pub action: Action,
    /// How much of the strip this button wants, told straight back to layout.
    pub sizing: Sizing,
}

impl Widget for ButtonWidget {
    fn sizing(&self) -> Sizing {
        self.sizing
    }

    fn render(&self, ctx: &Ctx<'_>, rect: Rect, out: &mut Output) {
        let color = if ctx.is_pressed(0) {
            ctx.theme.button_pressed
        } else {
            ctx.theme.button
        };
        out.draw(DrawCmd::RoundedRect {
            rect,
            radius: ctx.theme.corner_radius,
            color,
        });
        let image = self
            .icon
            .as_ref()
            .and_then(|name| ctx.icon(&IconKey::Named(name.clone())));
        let mut text_rect = rect;
        if let Some(image) = image {
            let (iw, ih) = (image.width() as f64, image.height() as f64);
            let icon_rect = if self.label.is_empty() {
                rect.center(iw, ih)
            } else {
                let x = rect.x + BUTTON_PADDING;
                text_rect = Rect::new(
                    x + iw + BUTTON_PADDING,
                    rect.y,
                    (rect.right() - x - iw - 2.0 * BUTTON_PADDING).max(0.0),
                    rect.height,
                );
                Rect::new(x, rect.y + (rect.height - ih) / 2.0, iw, ih)
            };
            out.draw(DrawCmd::Image {
                rect: icon_rect,
                image: image.clone(),
                alpha: 1.0,
                angle: 0.0,
            });
        }
        if !self.label.is_empty() {
            out.draw(DrawCmd::Text {
                rect: text_rect,
                text: self.label.clone(),
                style: ctx.theme.text.clone(),
                color: ctx.theme.foreground,
                align: Align::Center,
            });
        }
        out.hit(ctx.region(0), rect, self.action.clone());
    }
}

/// A wifi glyph, chosen from `state.network`'s signal strength in four buckets plus a
/// disconnected state.
///
/// Draws a packaged icon by name — `network-0` through `network-3`, or `network-off` — via
/// [`IconKey::Named`]. It never loads an icon and never holds a path (D3): the pixels were
/// placed in the store by the binary, and a missing icon is a normal state that draws nothing
/// rather than a panic. `None` network draws nothing, the same rule as the battery widget: a
/// machine with no wifi device must not be shown a lie.
pub struct NetworkWidget {
    /// Draw the network name (ssid) to the right of the glyph. Default false — the strip is
    /// narrow and M5's strip does not ask for it.
    pub show_ssid: bool,
    pub sizing: Sizing,
}

impl Widget for NetworkWidget {
    fn sizing(&self) -> Sizing {
        self.sizing
    }

    fn render(&self, ctx: &Ctx<'_>, rect: Rect, out: &mut Output) {
        let Some(network) = &ctx.state.network else {
            return;
        };
        let Some(name) = network_glyph_name(network) else {
            return;
        };
        let key = IconKey::Named(name.to_string());

        let Some(image) = ctx.icon(&key) else {
            // Missing icon is a normal state (W1 decision 1): draw the label instead. For the
            // network widget that label is the ssid when asked for, and nothing otherwise.
            if self.show_ssid {
                if let Some(ssid) = &network.ssid {
                    out.draw(DrawCmd::Text {
                        rect,
                        text: ssid.clone(),
                        style: ctx.theme.text.clone(),
                        color: ctx.theme.foreground,
                        align: Align::Center,
                    });
                }
            }
            return;
        };

        // The glyph is drawn at the control centre's glyph size, centred in the slot. Packaged
        // icons are rasterised larger than this, so it is a downscale, which stays sharp.
        let glyph_rect = if self.show_ssid {
            Rect::new(
                rect.x,
                rect.y + (rect.height - GLYPH_PX) / 2.0,
                GLYPH_PX,
                GLYPH_PX,
            )
        } else {
            rect.center(GLYPH_PX, GLYPH_PX)
        };
        out.draw(DrawCmd::Image {
            rect: glyph_rect,
            image: image.clone(),
            alpha: 1.0,
            angle: 0.0,
        });

        if self.show_ssid {
            if let Some(ssid) = &network.ssid {
                let text_x = glyph_rect.right() + 4.0;
                let text_rect = Rect::new(
                    text_x,
                    rect.y,
                    (rect.right() - text_x).max(0.0),
                    rect.height,
                );
                out.draw(DrawCmd::Text {
                    rect: text_rect,
                    text: ssid.clone(),
                    style: ctx.theme.text.clone(),
                    color: ctx.theme.foreground,
                    align: Align::Start,
                });
            }
        }
    }
}

/// The packaged glyph name for a network state, or `None` when the widget should draw nothing.
///
/// Signal strength is bucketed into four glyphs — `network-0` (weakest) to `network-3`
/// (strongest) — with `network-off` for a wifi that exists but is not connected. `None` means
/// "draw nothing": a `connected` wifi with a non-finite signal is a lie the widget refuses to
/// tell, the same way the battery widget refuses a `NaN` charge.
fn network_glyph_name(network: &NetworkState) -> Option<&'static str> {
    if !network.connected {
        return Some("network-off");
    }
    let signal = network.signal;
    if !signal.is_finite() {
        return None;
    }
    let signal = signal.clamp(0.0, 1.0);
    let bucket = if signal < 0.25 {
        0
    } else if signal < 0.5 {
        1
    } else if signal < 0.75 {
        2
    } else {
        3
    };
    Some(match bucket {
        0 => "network-0",
        1 => "network-1",
        2 => "network-2",
        _ => "network-3",
    })
}

/// The battery: a bar, a percentage, or both, from `state.battery`.
///
/// `None` battery draws nothing at all — a desktop has no battery and the strip must not show
/// a lie, not even a zero bar. `charge` is external data: it is clamped to `0.0..=1.0`, and a
/// `NaN` or infinite one is treated like no battery.
pub struct BatteryWidget {
    pub show_percentage: bool,
    pub show_bar: bool,
    pub sizing: Sizing,
}

/// The bar's outer size, nub excluded, and the nub's size. About Pock's, which draws macOS's.
const BATTERY_BAR: (f64, f64) = (38.0, 19.0);
const BATTERY_NUB: (f64, f64) = (3.0, 7.0);
/// The outline's thickness, and the gap between it and the charge inside.
const BATTERY_LINE: f64 = 2.0;
const BATTERY_INSET: f64 = 2.0;
/// The least room the percentage is given, for when the text cannot be measured.
const BATTERY_TEXT_MIN_WIDTH: f64 = 40.0;
const BATTERY_GAP: f64 = 6.0;
/// The charging bolt, drawn over the bar as macOS does.
const BATTERY_BOLT: f64 = 22.0;

impl Widget for BatteryWidget {
    fn sizing(&self) -> Sizing {
        self.sizing
    }

    fn render(&self, ctx: &Ctx<'_>, rect: Rect, out: &mut Output) {
        let Some(battery) = &ctx.state.battery else {
            return;
        };
        let Some(charge) = battery_charge(battery) else {
            return;
        };
        if !self.show_bar && !self.show_percentage {
            return;
        }

        let percentage = format!("{}%", (charge * 100.0).round() as u32);
        // Sized to the text actually drawn: the font is the user's, and a fixed guess clipped
        // the "%" at the real panel's font size.
        let text_width = (measure_text(&ctx.theme.text, &percentage).advance.ceil() + 2.0)
            .max(BATTERY_TEXT_MIN_WIDTH);
        let bar_block = BATTERY_BAR.0 + BATTERY_NUB.0;
        // The percentage is the one thing that must always be readable, so the bar is given up
        // before the text is clipped.
        let width_of = |bar: bool| {
            let mut total = 0.0;
            if bar {
                total += bar_block;
            }
            if bar && self.show_percentage {
                total += BATTERY_GAP;
            }
            if self.show_percentage {
                total += text_width;
            }
            total
        };
        let show_bar = self.show_bar && !(self.show_percentage && width_of(true) > rect.width);
        // Pock's order, the percentage then the bar, packed to the left so the gap to the widget
        // before is the strip's gap and nothing more; any spare room is left at the right.
        let mut x = rect.x;

        if self.show_percentage {
            out.draw(DrawCmd::Text {
                rect: Rect::new(x, rect.y, text_width, rect.height),
                text: percentage,
                style: ctx.theme.text.clone(),
                color: ctx.theme.foreground.with_alpha(0.8),
                align: if show_bar { Align::End } else { Align::Center },
            });
            x += text_width + BATTERY_GAP;
        }

        if show_bar {
            let y = rect.y + (rect.height - BATTERY_BAR.1) / 2.0;
            let outline = Rect::new(x, y, BATTERY_BAR.0, BATTERY_BAR.1);
            let outline_color = ctx.theme.foreground.with_alpha(0.8);
            out.draw(DrawCmd::RoundedRect {
                rect: outline,
                radius: 5.0,
                color: outline_color,
            });
            let inner = outline.inset(BATTERY_LINE);
            out.draw(DrawCmd::RoundedRect {
                rect: inner,
                radius: 3.5,
                color: ctx.theme.background,
            });
            out.draw(DrawCmd::RoundedRect {
                rect: Rect::new(
                    outline.right() + 1.0,
                    rect.y + (rect.height - BATTERY_NUB.1) / 2.0,
                    BATTERY_NUB.0 - 1.0,
                    BATTERY_NUB.1,
                ),
                radius: 1.0,
                color: outline_color,
            });
            let charge_box = inner.inset(BATTERY_INSET);
            let fill_width = charge_box.width * charge;
            if fill_width > 0.0 {
                out.draw(DrawCmd::RoundedRect {
                    rect: Rect::new(charge_box.x, charge_box.y, fill_width, charge_box.height),
                    radius: 2.0,
                    color: if battery.charging {
                        ctx.theme.accent
                    } else {
                        ctx.theme.foreground
                    },
                });
            }
            // A bolt says "charging" in a way a colour alone does not; no bolt icon, no bolt.
            if battery.charging {
                if let Some(bolt) = ctx.icon(&IconKey::Named("bolt".into())) {
                    out.draw(DrawCmd::Image {
                        rect: outline.center(BATTERY_BOLT, BATTERY_BOLT),
                        image: bolt.clone(),
                        alpha: 1.0,
                        angle: 0.0,
                    });
                }
            }
        }
    }
}

/// The charge to draw, clamped to `0.0..=1.0`, or `None` when it is not a number.
fn battery_charge(battery: &BatteryState) -> Option<f64> {
    battery
        .charge
        .is_finite()
        .then(|| battery.charge.clamp(0.0, 1.0))
}

/// A thin vertical divider. Purely decorative — it never calls `out.hit`, because a separator
/// that swallowed a touch would be a dead zone with no visible reason to be one.
pub struct SeparatorWidget {
    pub sizing: Sizing,
}

impl Widget for SeparatorWidget {
    fn sizing(&self) -> Sizing {
        self.sizing
    }

    fn render(&self, ctx: &Ctx<'_>, rect: Rect, out: &mut Output) {
        // Pock's: two pixels of dark grey, two thirds of the strip's height.
        let width = 2.0_f64.min(rect.width);
        let bar = Rect::new(
            rect.x + (rect.width - width) / 2.0,
            rect.y + rect.height / 6.0,
            width,
            rect.height * 2.0 / 3.0,
        );
        out.draw(DrawCmd::Rect {
            rect: bar,
            color: ctx.theme.foreground.with_alpha(0.33),
        });
    }
}

/// The size glyphs are drawn at in the control centre and the network widget: Pock's, about.
/// Packaged icons are rasterised larger (for the folders), so these are scaled down, never up.
const GLYPH_PX: f64 = 32.0;

/// How far a pressed control-centre button's highlight sits inside its share of the panel.
const CONTROL_BUTTON_GAP: f64 = 4.0;

/// One pressable button inside a [`ControlCenterWidget`].
pub struct ControlButton {
    /// A packaged icon name, resolved against the `Icons` store. A missing icon draws the
    /// label instead, or nothing (W1 decision 1) — never a panic.
    pub icon: String,
    /// Optional text shown when the icon is missing. An empty string behaves as absent.
    pub label: Option<String>,
    /// What a press on this button asks the shell to do: keys emitted through uinput, or D11's
    /// one `SystemAction`.
    pub action: Action,
}

/// A row of buttons sharing one slot — M5's control centre (design decision 3: one widget,
/// many `part`s). Each button is a rounded rect with an icon, one hit region per button via
/// `ctx.region(part)`, and its own `Action`.
///
/// **Buttons, not sliders** — the volume, brightness and mute keys are `Action::Keys` that
/// `listello-core` emits through uinput exactly as it emits media keys; only lock is
/// `Action::System`, and none of them go through the agent (D11).
pub struct ControlCenterWidget {
    pub buttons: Vec<ControlButton>,
    pub sizing: Sizing,
}

impl Widget for ControlCenterWidget {
    fn sizing(&self) -> Sizing {
        self.sizing
    }

    fn render(&self, ctx: &Ctx<'_>, rect: Rect, out: &mut Output) {
        let n = self.buttons.len();
        if n == 0 {
            return;
        }
        // One rounded panel holding every glyph, as Pock draws it; a press lights its own part.
        out.draw(DrawCmd::RoundedRect {
            rect,
            radius: ctx.theme.corner_radius,
            color: ctx.theme.button,
        });
        let sub_w = (rect.width / n as f64).max(0.0);
        for (i, button) in self.buttons.iter().enumerate() {
            let part = i as u16;
            let button_rect = Rect::new(rect.x + sub_w * i as f64, rect.y, sub_w, rect.height);
            if ctx.is_pressed(part) {
                out.draw(DrawCmd::RoundedRect {
                    rect: button_rect.inset(CONTROL_BUTTON_GAP),
                    radius: ctx.theme.corner_radius,
                    color: ctx.theme.button_pressed,
                });
            }

            // The icon drawn at the glyph size, centred — a missing icon draws the label
            // instead, exactly like the network widget's fallback.
            let key = IconKey::Named(button.icon.clone());
            if let Some(image) = ctx.icon(&key) {
                out.draw(DrawCmd::Image {
                    rect: button_rect.center(GLYPH_PX, GLYPH_PX),
                    image: image.clone(),
                    alpha: 1.0,
                    angle: 0.0,
                });
            } else if let Some(label) = &button.label {
                if !label.is_empty() {
                    out.draw(DrawCmd::Text {
                        rect: button_rect,
                        text: label.clone(),
                        style: ctx.theme.text.clone(),
                        color: ctx.theme.foreground,
                        align: Align::Center,
                    });
                }
            }

            out.hit(ctx.region(part), button_rect, button.action.clone());
        }
    }
}

/// One folder shortcut inside a [`FoldersWidget`].
pub struct FolderEntry {
    /// The key the agent reports this folder's emptiness under.
    pub key: String,
    /// What a tap opens, carried untouched to `CoreToAgent::OpenPath`.
    pub path: String,
    pub label: Option<String>,
    pub icon: String,
    /// Used instead of `icon` while the agent says the folder is empty.
    pub empty_icon: Option<String>,
}

/// A row of folder shortcuts (design decision 3: one widget, many `part`s). One hit region per
/// folder, each [`Action::OpenPath`].
///
/// Which icon a folder wears reads `state.folders[key].empty`. A folder the agent has not
/// reported on — no agent, or not watched yet — wears its plain `icon`: never a panic on the
/// missing entry, and Trash does not claim to be empty before anyone knows.
pub struct FoldersWidget {
    pub folders: Vec<FolderEntry>,
    pub sizing: Sizing,
}

impl Widget for FoldersWidget {
    fn sizing(&self) -> Sizing {
        self.sizing
    }

    fn render(&self, ctx: &Ctx<'_>, rect: Rect, out: &mut Output) {
        let n = self.folders.len();
        if n == 0 {
            return;
        }
        // Like the Dock: the icons sit on the black, and a press lights the area behind one.
        let sub_w = (rect.width / n as f64).max(0.0);
        for (i, folder) in self.folders.iter().enumerate() {
            let part = i as u16;
            let folder_rect = Rect::new(rect.x + sub_w * i as f64, rect.y, sub_w, rect.height);
            if ctx.is_pressed(part) {
                out.draw(DrawCmd::RoundedRect {
                    rect: folder_rect,
                    radius: ctx.theme.corner_radius,
                    color: ctx.theme.button,
                });
            }

            let empty = ctx
                .state
                .folders
                .get(&folder.key)
                .is_some_and(|state| state.empty);
            let icon = match (&folder.empty_icon, empty) {
                (Some(empty_icon), true) => empty_icon,
                _ => &folder.icon,
            };
            if let Some(image) = ctx.icon(&IconKey::Named(icon.clone())) {
                out.draw(DrawCmd::Image {
                    rect: folder_rect.center(image.width() as f64, image.height() as f64),
                    image: image.clone(),
                    alpha: 1.0,
                    angle: 0.0,
                });
            } else if let Some(label) = folder.label.as_deref().filter(|l| !l.is_empty()) {
                out.draw(DrawCmd::Text {
                    rect: folder_rect,
                    text: label.to_string(),
                    style: ctx.theme.text.clone(),
                    color: ctx.theme.foreground,
                    align: Align::Center,
                });
            }

            out.hit(
                ctx.region(part),
                folder_rect,
                Action::OpenPath(folder.path.clone()),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    mod clock {
        use crate::state::UiState;
        use crate::widget::{Ctx, Output, Sizing, Widget};
        use crate::widgets::ClockWidget;
        use crate::Theme;
        use cairo::{FontFace, FontSlant, FontWeight};
        use chrono::DateTime;
        use listello_surface::{Align, Color, DrawCmd, Rect, TextStyle};
        use std::rc::Rc;

        const RECT: Rect = Rect::new(0.0, 0.0, 120.0, 32.0);

        fn fixture_theme() -> Option<Theme> {
            let font =
                FontFace::toy_create("sans-serif", FontSlant::Normal, FontWeight::Normal).ok()?;
            let text = TextStyle::new(Rc::new(font), 16.0);
            Some(Theme {
                text,
                background: Color::BLACK,
                foreground: Color::WHITE,
                button: Color::grey(0.2),
                button_pressed: Color::grey(0.3),
                accent: Color::grey(0.8),
                corner_radius: 4.0,
            })
        }

        fn fixture() -> Option<(Theme, UiState)> {
            let theme = fixture_theme()?;
            let now = DateTime::parse_from_rfc3339("2026-08-03T14:05:00+02:00").ok()?;
            let state = UiState::new(now);
            Some((theme, state))
        }

        fn render(widget: &ClockWidget, theme: &Theme, state: &UiState, rect: Rect) -> Output {
            let ctx = Ctx {
                state,
                theme,
                index: 0,
            };
            let mut out = Output::new();
            widget.render(&ctx, rect, &mut out);
            out
        }

        #[test]
        fn formats_time_centred() {
            let Some((theme, state)) = fixture() else {
                eprintln!("skipping: could not build the toy font face");
                return;
            };
            let widget = ClockWidget {
                format: "%H:%M".to_string(),
                align: Align::Center,
                locale: chrono::Locale::POSIX,
                fit: false,
                sizing: Sizing::Fixed(120.0),
            };
            let out = render(&widget, &theme, &state, RECT);
            let cmds = out.commands();
            assert_eq!(cmds.len(), 1);
            assert!(
                matches!(
                    &cmds[0],
                    DrawCmd::Text {
                        rect,
                        text,
                        align,
                        ..
                    } if text.as_str() == "14:05" && rect == &RECT && align == &Align::Center
                ),
                "unexpected command: {cmds:?}"
            );
        }

        #[test]
        fn day_and_month_names_follow_the_locale_and_the_layout_follows_the_format() {
            let Some((theme, state)) = fixture() else {
                eprintln!("skipping: could not build the toy font face");
                return;
            };
            let text = |format: &str, locale| {
                let widget = ClockWidget {
                    format: format.to_string(),
                    align: Align::Start,
                    locale,
                    fit: false,
                    sizing: Sizing::Fixed(120.0),
                };
                match &render(&widget, &theme, &state, RECT).commands()[0] {
                    DrawCmd::Text { text, .. } => text.clone(),
                    other => panic!("not text: {other:?}"),
                }
            };
            let it = chrono::Locale::it_IT;
            assert_eq!(text("%a %-d %B %H:%M", it), "lun 3 agosto 14:05");
            assert_eq!(
                text("%a %-d %b %H:%M", chrono::Locale::POSIX),
                "Mon 3 Aug 14:05"
            );
            assert_eq!(text("%d/%m/%y %H:%M", it), "03/08/26 14:05");
            assert_eq!(
                text("%B %-d, %Y %I:%M %p", chrono::Locale::en_US),
                "August 3, 2026 02:05 PM"
            );
        }

        #[test]
        fn a_fitting_clock_is_as_wide_as_its_text_and_its_margin() {
            let Some((theme, state)) = fixture() else {
                eprintln!("skipping: could not build the toy font face");
                return;
            };
            let mut widget = ClockWidget {
                format: "%H:%M".to_string(),
                align: Align::Start,
                locale: chrono::Locale::POSIX,
                fit: true,
                sizing: Sizing::Flex(1.0),
            };
            let ctx = Ctx {
                state: &state,
                theme: &theme,
                index: 0,
            };
            let text = listello_surface::measure_text(&theme.text, "14:05").advance;
            assert_eq!(
                widget.fitted_width(&ctx),
                Some(text.ceil() + super::super::CLOCK_FIT_PAD)
            );
            widget.fit = false;
            assert_eq!(widget.fitted_width(&ctx), None);
        }

        #[test]
        fn emits_no_hit_region() {
            let Some((theme, state)) = fixture() else {
                eprintln!("skipping: could not build the toy font face");
                return;
            };
            let widget = ClockWidget {
                format: "%H:%M".to_string(),
                align: Align::Center,
                locale: chrono::Locale::POSIX,
                fit: false,
                sizing: Sizing::Fixed(120.0),
            };
            let out = render(&widget, &theme, &state, RECT);
            assert!(out.hits().is_empty());
        }

        #[test]
        fn formats_date() {
            let Some((theme, state)) = fixture() else {
                eprintln!("skipping: could not build the toy font face");
                return;
            };
            let widget = ClockWidget {
                format: "%Y-%m-%d".to_string(),
                align: Align::Center,
                locale: chrono::Locale::POSIX,
                fit: false,
                sizing: Sizing::Fixed(120.0),
            };
            let out = render(&widget, &theme, &state, RECT);
            assert_eq!(out.commands().len(), 1);
            assert!(
                matches!(
                    &out.commands()[0],
                    DrawCmd::Text { text, .. } if text.as_str() == "2026-08-03"
                ),
                "unexpected command: {:?}",
                out.commands()
            );
        }

        #[test]
        fn invalid_specifier_degrades_without_panicking() {
            let Some((theme, state)) = fixture() else {
                eprintln!("skipping: could not build the toy font face");
                return;
            };
            let widget = ClockWidget {
                format: "%Q".to_string(),
                align: Align::Center,
                locale: chrono::Locale::POSIX,
                fit: false,
                sizing: Sizing::Fixed(120.0),
            };
            let out = render(&widget, &theme, &state, RECT);
            assert_eq!(out.commands().len(), 1);
            assert!(
                matches!(
                    &out.commands()[0],
                    DrawCmd::Text { text, .. } if text.as_str() == "??:??"
                ),
                "unexpected command: {:?}",
                out.commands()
            );
        }

        #[test]
        fn empty_format_renders_empty_without_panicking() {
            let Some((theme, state)) = fixture() else {
                eprintln!("skipping: could not build the toy font face");
                return;
            };
            let widget = ClockWidget {
                format: String::new(),
                align: Align::Center,
                locale: chrono::Locale::POSIX,
                fit: false,
                sizing: Sizing::Fixed(120.0),
            };
            let out = render(&widget, &theme, &state, RECT);
            assert_eq!(out.commands().len(), 1);
            assert!(
                matches!(
                    &out.commands()[0],
                    DrawCmd::Text { text, .. } if text.as_str() == ""
                ),
                "unexpected command: {:?}",
                out.commands()
            );
        }

        #[test]
        fn renders_are_deterministic() {
            let Some((theme, state)) = fixture() else {
                eprintln!("skipping: could not build the toy font face");
                return;
            };
            let widget = ClockWidget {
                format: "%H:%M".to_string(),
                align: Align::Center,
                locale: chrono::Locale::POSIX,
                fit: false,
                sizing: Sizing::Flex(1.0),
            };
            let a = render(&widget, &theme, &state, RECT);
            let b = render(&widget, &theme, &state, RECT);
            assert_eq!(format!("{:?}", a.commands()), format!("{:?}", b.commands()));
        }

        #[test]
        fn sizing_returns_built_value() {
            let fixed = ClockWidget {
                format: "%H:%M".to_string(),
                align: Align::Center,
                locale: chrono::Locale::POSIX,
                fit: false,
                sizing: Sizing::Fixed(120.0),
            };
            assert_eq!(fixed.sizing(), Sizing::Fixed(120.0));

            let flex = ClockWidget {
                format: "%H:%M".to_string(),
                align: Align::Center,
                locale: chrono::Locale::POSIX,
                fit: false,
                sizing: Sizing::Flex(2.0),
            };
            assert_eq!(flex.sizing(), Sizing::Flex(2.0));
        }
    }

    mod button {
        use crate::hit::{Action, KeyName, RegionId};
        use crate::state::{Theme, UiState};
        use crate::widget::{Ctx, Output, Sizing, Widget};
        use crate::widgets::ButtonWidget;
        use listello_surface::{Align, Color, DrawCmd, Rect, TextStyle};
        use std::rc::Rc;

        /// A fixed `Theme` and `UiState` every test can build from. A `Theme` needs a cairo
        /// face; the toy API needs no fontconfig and works on the Mac, but can still fail, in
        /// which case the caller skips rather than unwraps.
        fn fixture() -> Option<(Theme, UiState)> {
            let font = cairo::FontFace::toy_create(
                "sans-serif",
                cairo::FontSlant::Normal,
                cairo::FontWeight::Normal,
            )
            .ok()?;
            let theme = Theme {
                text: TextStyle::new(Rc::new(font), 14.0),
                background: Color::BLACK,
                foreground: Color::WHITE,
                button: Color::grey(0.3),
                button_pressed: Color::grey(0.5),
                accent: Color::grey(0.8),
                corner_radius: 4.0,
            };
            let now = chrono::DateTime::parse_from_rfc3339("2026-08-03T14:05:00+02:00").ok()?;
            let state = UiState::new(now);
            Some((theme, state))
        }

        fn button(label: &str, action: Action, sizing: Sizing) -> ButtonWidget {
            ButtonWidget {
                label: label.to_string(),
                icon: None,
                action,
                sizing,
            }
        }

        const RECT: Rect = Rect::new(10.0, 5.0, 60.0, 30.0);

        /// Render `widget` as strip index 0 into `RECT`, and return the whole output.
        fn render_with(widget: &ButtonWidget, theme: &Theme, state: &UiState) -> Output {
            let ctx = Ctx {
                state,
                theme,
                index: 0,
            };
            let mut out = Output::new();
            widget.render(&ctx, RECT, &mut out);
            out
        }

        fn icon_button(label: &str, icon: &str) -> ButtonWidget {
            ButtonWidget {
                label: label.to_string(),
                icon: Some(icon.to_string()),
                action: Action::None,
                sizing: Sizing::Fixed(60.0),
            }
        }

        fn with_stub_icon(state: &mut UiState, name: &str) {
            let image = listello_surface::image_from_rgba(10, 10, &[0x90; 400]);
            if let Some(image) = image {
                state
                    .icons
                    .insert(crate::state::IconKey::Named(name.into()), image);
            }
        }

        #[test]
        fn an_icon_alone_is_centred() {
            let Some((theme, mut state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            with_stub_icon(&mut state, "mute");
            let out = render_with(&icon_button("", "mute"), &theme, &state);
            let cmds = out.commands();
            assert_eq!(cmds.len(), 2, "background and icon, no text");
            // RECT is 60x30 at (10, 5): a 10x10 icon centres at (35, 15).
            assert!(matches!(
                &cmds[1],
                DrawCmd::Image { rect, .. } if *rect == Rect::new(35.0, 15.0, 10.0, 10.0)
            ));
        }

        #[test]
        fn an_icon_and_a_label_sit_side_by_side() {
            let Some((theme, mut state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            with_stub_icon(&mut state, "mute");
            let out = render_with(&icon_button("Mute", "mute"), &theme, &state);
            let cmds = out.commands();
            assert_eq!(cmds.len(), 3);
            // Icon at the left padding, vertically centred; text takes what is left of it.
            assert!(matches!(
                &cmds[1],
                DrawCmd::Image { rect, .. } if *rect == Rect::new(18.0, 15.0, 10.0, 10.0)
            ));
            assert!(matches!(
                &cmds[2],
                DrawCmd::Text { rect, text, .. }
                    if text == "Mute" && *rect == Rect::new(36.0, 5.0, 26.0, 30.0)
            ));
        }

        #[test]
        fn a_missing_icon_draws_the_label_alone() {
            let Some((theme, state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            let out = render_with(&icon_button("Mute", "mute"), &theme, &state);
            let cmds = out.commands();
            assert_eq!(cmds.len(), 2);
            assert!(matches!(
                &cmds[1],
                DrawCmd::Text { rect, text, .. } if text == "Mute" && *rect == RECT
            ));
            // And a missing icon with no label is just the background, still pressable.
            let out = render_with(&icon_button("", "mute"), &theme, &state);
            assert_eq!(out.commands().len(), 1);
            assert_eq!(out.hits().len(), 1);
        }

        #[test]
        fn no_icon_behaves_as_before() {
            let Some((theme, mut state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            with_stub_icon(&mut state, "mute");
            let out = render_with(
                &button("Mute", Action::None, Sizing::Fixed(60.0)),
                &theme,
                &state,
            );
            assert_eq!(out.commands().len(), 2);
            assert!(matches!(&out.commands()[1], DrawCmd::Text { rect, .. } if *rect == RECT));
        }

        #[test]
        fn unpressed_emits_rect_then_label() {
            let Some((theme, state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            let widget = button("F5", Action::None, Sizing::Fixed(60.0));
            let out = render_with(&widget, &theme, &state);

            let cmds = out.commands();
            assert_eq!(
                cmds.len(),
                2,
                "the background rect and the text, in that order"
            );
            match &cmds[0] {
                DrawCmd::RoundedRect {
                    rect,
                    radius,
                    color,
                } => {
                    assert_eq!(*rect, RECT);
                    assert_eq!(*radius, theme.corner_radius);
                    assert_eq!(*color, theme.button);
                }
                other => panic!("first command must be a RoundedRect, got {other:?}"),
            }
            match &cmds[1] {
                DrawCmd::Text {
                    rect,
                    text,
                    color,
                    align,
                    ..
                } => {
                    assert_eq!(*rect, RECT);
                    assert_eq!(text.as_str(), "F5");
                    assert_eq!(*color, theme.foreground);
                    assert_eq!(*align, Align::Center);
                }
                other => panic!("second command must be a Text, got {other:?}"),
            }
        }

        #[test]
        fn own_region_pressed_uses_pressed_color() {
            let Some((theme, mut state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            state.pressed = Some(RegionId::new(0, 0));
            let widget = button("F5", Action::None, Sizing::Fixed(60.0));
            let out = render_with(&widget, &theme, &state);

            match &out.commands()[0] {
                DrawCmd::RoundedRect { color, .. } => assert_eq!(*color, theme.button_pressed),
                other => panic!("first command must be a RoundedRect, got {other:?}"),
            }
        }

        #[test]
        fn another_widgets_region_does_not_press_this_one() {
            let Some((theme, mut state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            state.pressed = Some(RegionId::new(1, 0));
            let widget = button("F5", Action::None, Sizing::Fixed(60.0));
            let out = render_with(&widget, &theme, &state);

            match &out.commands()[0] {
                DrawCmd::RoundedRect { color, .. } => assert_eq!(*color, theme.button),
                other => panic!("first command must be a RoundedRect, got {other:?}"),
            }
        }

        #[test]
        fn empty_label_emits_no_text() {
            let Some((theme, state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            let widget = button("", Action::None, Sizing::Fixed(60.0));
            let out = render_with(&widget, &theme, &state);

            let cmds = out.commands();
            assert_eq!(cmds.len(), 1, "only the background rect");
            assert!(
                matches!(&cmds[0], DrawCmd::RoundedRect { .. }),
                "the only command must be a RoundedRect, got {:?}",
                cmds[0]
            );
        }

        #[test]
        fn emits_one_hit_region_with_the_action_it_was_built_with() {
            let Some((theme, state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            let action = Action::Keys(vec![KeyName::new("KEY_F5").unwrap()]);
            let widget = button("F5", action.clone(), Sizing::Fixed(60.0));
            let out = render_with(&widget, &theme, &state);

            let hits = out.hits();
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].id, RegionId::new(0, 0));
            assert_eq!(hits[0].rect, RECT);
            assert_eq!(hits[0].action, action);
        }

        #[test]
        fn none_action_still_emits_one_hit_region() {
            let Some((theme, state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            let widget = button("", Action::None, Sizing::Flex(1.0));
            let out = render_with(&widget, &theme, &state);

            let hits = out.hits();
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].action, Action::None);
        }

        #[test]
        fn sizing_returns_what_it_was_built_with() {
            let fixed = button("", Action::None, Sizing::Fixed(40.0));
            let flex = button("", Action::None, Sizing::Flex(2.0));
            assert_eq!(fixed.sizing(), Sizing::Fixed(40.0));
            assert_eq!(flex.sizing(), Sizing::Flex(2.0));
        }
    }

    mod battery {
        use crate::state::{Theme, UiState};
        use crate::widget::{Ctx, Output, Sizing, Widget};
        use crate::widgets::BatteryWidget;
        use cairo::{FontFace, FontSlant, FontWeight};
        use chrono::DateTime;
        use listello_protocol::BatteryState;
        use listello_surface::{Color, DrawCmd, Rect, TextStyle};
        use std::rc::Rc;

        const RECT: Rect = Rect::new(0.0, 0.0, 100.0, 32.0);

        fn fixture() -> Option<(Theme, UiState)> {
            let font =
                FontFace::toy_create("sans-serif", FontSlant::Normal, FontWeight::Normal).ok()?;
            let theme = Theme {
                text: TextStyle::new(Rc::new(font), 14.0),
                background: Color::BLACK,
                foreground: Color::WHITE,
                button: Color::grey(0.2),
                button_pressed: Color::grey(0.3),
                accent: Color::grey(0.8),
                corner_radius: 4.0,
            };
            let now = DateTime::parse_from_rfc3339("2026-08-03T14:05:00+02:00").ok()?;
            Some((theme, UiState::new(now)))
        }

        fn widget(show_percentage: bool, show_bar: bool) -> BatteryWidget {
            BatteryWidget {
                show_percentage,
                show_bar,
                sizing: Sizing::Fixed(100.0),
            }
        }

        fn render(widget: &BatteryWidget, theme: &Theme, state: &UiState) -> Output {
            let ctx = Ctx {
                state,
                theme,
                index: 0,
            };
            let mut out = Output::new();
            widget.render(&ctx, RECT, &mut out);
            out
        }

        fn set(state: &mut UiState, charge: f64, charging: bool) {
            state.battery = Some(BatteryState {
                charge,
                charging,
                time_remaining_secs: None,
            });
        }

        /// The rounded rects after the outline, the interior and the nub: the fill, if any.
        fn fill(out: &Output) -> Option<(f64, Color)> {
            out.commands()
                .iter()
                .filter_map(|c| match c {
                    DrawCmd::RoundedRect { rect, color, .. } => Some((rect.width, *color)),
                    _ => None,
                })
                .nth(3)
        }

        /// Width of the fill, or `None` when no fill was drawn.
        fn fill_width(out: &Output) -> Option<f64> {
            fill(out).map(|(width, _)| width)
        }

        fn text(out: &Output) -> Option<String> {
            out.commands().iter().find_map(|c| match c {
                DrawCmd::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
        }

        #[test]
        fn no_battery_draws_nothing_at_all() {
            let Some((theme, state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            let out = render(&widget(true, true), &theme, &state);
            assert!(out.commands().is_empty());
            assert!(out.hits().is_empty());
        }

        #[test]
        fn the_fill_scales_with_the_charge() {
            let Some((theme, mut state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            // The charge box is 38 - 2 * (2 + 2) = 30 wide: outline, then a gap.
            set(&mut state, 0.0, false);
            assert_eq!(
                fill_width(&render(&widget(false, true), &theme, &state)),
                None
            );
            set(&mut state, 0.5, false);
            assert_eq!(
                fill_width(&render(&widget(false, true), &theme, &state)),
                Some(15.0)
            );
            set(&mut state, 1.0, false);
            assert_eq!(
                fill_width(&render(&widget(false, true), &theme, &state)),
                Some(30.0)
            );
        }

        #[test]
        fn out_of_range_charge_is_clamped_and_nan_draws_nothing() {
            let Some((theme, mut state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            set(&mut state, 1.7, false);
            let out = render(&widget(true, true), &theme, &state);
            assert_eq!(fill_width(&out), Some(30.0));
            assert_eq!(text(&out).as_deref(), Some("100%"));
            set(&mut state, -0.3, false);
            let out = render(&widget(true, true), &theme, &state);
            assert_eq!(fill_width(&out), None);
            assert_eq!(text(&out).as_deref(), Some("0%"));
            set(&mut state, f64::NAN, false);
            assert!(render(&widget(true, true), &theme, &state)
                .commands()
                .is_empty());
        }

        #[test]
        fn the_percentage_is_rounded_and_either_half_can_be_switched_off() {
            let Some((theme, mut state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            set(&mut state, 0.625, false);
            let both = render(&widget(true, true), &theme, &state);
            assert_eq!(text(&both).as_deref(), Some("63%"));
            assert!(fill_width(&both).is_some());
            let only_text = render(&widget(true, false), &theme, &state);
            assert_eq!(text(&only_text).as_deref(), Some("63%"));
            assert_eq!(only_text.commands().len(), 1);
            let only_bar = render(&widget(false, true), &theme, &state);
            assert_eq!(text(&only_bar), None);
            assert!(render(&widget(false, false), &theme, &state)
                .commands()
                .is_empty());
        }

        #[test]
        fn a_charging_battery_draws_a_bolt_when_the_icon_exists_and_makes_room_for_it() {
            let Some((theme, mut state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            if let Some(bolt) = listello_surface::image_from_rgba(8, 8, &[0xff; 256]) {
                state
                    .icons
                    .insert(crate::state::IconKey::Named("bolt".into()), bolt);
            }
            let images = |out: &Output| {
                out.commands()
                    .iter()
                    .filter(|c| matches!(c, DrawCmd::Image { .. }))
                    .count()
            };
            set(&mut state, 0.5, false);
            assert_eq!(images(&render(&widget(true, true), &theme, &state)), 0);
            set(&mut state, 0.5, true);
            assert_eq!(images(&render(&widget(true, true), &theme, &state)), 1);
        }

        #[test]
        fn the_percentage_gets_the_room_its_text_needs() {
            let Some((mut theme, mut state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            // A big font, as on the real panel: the old fixed 40px clipped the "%".
            theme.text = TextStyle::new(theme.text.font.clone(), 40.0);
            set(&mut state, 1.0, false);
            let out = render(&widget(true, false), &theme, &state);
            let width = out
                .commands()
                .iter()
                .find_map(|c| match c {
                    DrawCmd::Text { rect, .. } => Some(rect.width),
                    _ => None,
                })
                .expect("a percentage");
            let needed = listello_surface::measure_text(&theme.text, "100%").advance;
            assert!(width >= needed, "{width} < {needed}");
        }

        #[test]
        fn charging_changes_the_fill_colour_and_nothing_else() {
            let Some((theme, mut state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            let fill_color = |out: &Output| fill(out).map(|(_, color)| color);
            set(&mut state, 0.5, false);
            let discharging = render(&widget(true, true), &theme, &state);
            set(&mut state, 0.5, true);
            let charging = render(&widget(true, true), &theme, &state);
            assert_eq!(fill_color(&discharging), Some(theme.foreground));
            assert_eq!(fill_color(&charging), Some(theme.accent));
            assert_eq!(discharging.commands().len(), charging.commands().len());
        }
    }

    mod folders {
        use crate::hit::Action;
        use crate::state::{IconKey, Theme, UiState};
        use crate::widget::{Ctx, Output, Sizing, Widget};
        use crate::widgets::{FolderEntry, FoldersWidget};
        use cairo::{FontFace, FontSlant, FontWeight};
        use chrono::DateTime;
        use listello_protocol::FolderState;
        use listello_surface::{image_from_rgba, Color, DrawCmd, Rect, TextStyle};
        use std::rc::Rc;

        const RECT: Rect = Rect::new(0.0, 0.0, 200.0, 32.0);

        fn fixture() -> Option<(Theme, UiState)> {
            let font =
                FontFace::toy_create("sans-serif", FontSlant::Normal, FontWeight::Normal).ok()?;
            let theme = Theme {
                text: TextStyle::new(Rc::new(font), 14.0),
                background: Color::BLACK,
                foreground: Color::WHITE,
                button: Color::grey(0.2),
                button_pressed: Color::grey(0.3),
                accent: Color::grey(0.8),
                corner_radius: 4.0,
            };
            let now = DateTime::parse_from_rfc3339("2026-08-03T14:05:00+02:00").ok()?;
            Some((theme, UiState::new(now)))
        }

        fn entry(key: &str, path: &str, empty_icon: Option<&str>) -> FolderEntry {
            FolderEntry {
                key: key.into(),
                path: path.into(),
                label: Some(key.into()),
                icon: format!("{key}-icon"),
                empty_icon: empty_icon.map(str::to_string),
            }
        }

        fn widget(folders: Vec<FolderEntry>) -> FoldersWidget {
            FoldersWidget {
                folders,
                sizing: Sizing::Fixed(200.0),
            }
        }

        fn render(widget: &FoldersWidget, theme: &Theme, state: &UiState) -> Output {
            let ctx = Ctx {
                state,
                theme,
                index: 0,
            };
            let mut out = Output::new();
            widget.render(&ctx, RECT, &mut out);
            out
        }

        /// A square stub icon; the side tells two icons apart in a command list.
        fn put_icon(state: &mut UiState, name: &str, side: u32) {
            let pixels = vec![0x80; (side * side * 4) as usize];
            if let Some(image) = image_from_rgba(side, side, &pixels) {
                state.icons.insert(IconKey::Named(name.into()), image);
            }
        }

        fn images(out: &Output) -> usize {
            out.commands()
                .iter()
                .filter(|c| matches!(c, DrawCmd::Image { .. }))
                .count()
        }

        #[test]
        fn one_region_per_folder_each_opening_its_own_path() {
            let Some((theme, state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            let w = widget(vec![
                entry("shots", "/home/g/Pictures/Screenshots", None),
                entry("down", "/home/g/Downloads", None),
                entry("trash", "trash:/", Some("trash-empty")),
            ]);
            let out = render(&w, &theme, &state);
            let actions: Vec<_> = out.hits().iter().map(|h| h.action.clone()).collect();
            assert_eq!(
                actions,
                vec![
                    Action::OpenPath("/home/g/Pictures/Screenshots".into()),
                    Action::OpenPath("/home/g/Downloads".into()),
                    Action::OpenPath("trash:/".into()),
                ]
            );
        }

        #[test]
        fn an_empty_folder_wears_its_empty_icon_and_a_full_one_its_plain_icon() {
            let Some((theme, mut state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            put_icon(&mut state, "trash-icon", 4);
            put_icon(&mut state, "trash-empty", 6);
            let w = widget(vec![entry("trash", "trash:/", Some("trash-empty"))]);
            let mut drawn_side = |empty: bool| {
                state.folders.insert(
                    "trash".into(),
                    FolderState {
                        key: "trash".into(),
                        empty,
                    },
                );
                render(&w, &theme, &state)
                    .commands()
                    .iter()
                    .find_map(|c| match c {
                        DrawCmd::Image { image, .. } => Some(image.width()),
                        _ => None,
                    })
                    .expect("an icon was drawn")
            };
            assert_eq!(drawn_side(false), 4);
            assert_eq!(drawn_side(true), 6);
        }

        #[test]
        fn an_unreported_folder_uses_its_plain_icon_and_never_panics() {
            let Some((theme, mut state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            put_icon(&mut state, "trash-icon", 4);
            put_icon(&mut state, "trash-empty", 6);
            let w = widget(vec![entry("trash", "trash:/", Some("trash-empty"))]);
            // No entry for "trash" in the map: the plain icon, not the empty one.
            let out = render(&w, &theme, &state);
            assert_eq!(images(&out), 1);
            let side = out.commands().iter().find_map(|c| match c {
                DrawCmd::Image { image, .. } => Some(image.width()),
                _ => None,
            });
            assert_eq!(side, Some(4));
            assert_eq!(out.hits().len(), 1);
        }

        #[test]
        fn a_missing_icon_falls_back_to_the_label() {
            let Some((theme, state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            let out = render(&widget(vec![entry("down", "/d", None)]), &theme, &state);
            assert_eq!(images(&out), 0);
            assert!(out
                .commands()
                .iter()
                .any(|c| matches!(c, DrawCmd::Text { text, .. } if text == "down")));
            assert_eq!(out.hits().len(), 1);
        }

        #[test]
        fn no_folders_draws_nothing() {
            let Some((theme, state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            let out = render(&widget(vec![]), &theme, &state);
            assert!(out.commands().is_empty());
            assert!(out.hits().is_empty());
        }
    }

    mod network {
        use crate::state::{IconKey, Theme, UiState};
        use crate::widget::{Ctx, Output, Sizing, Widget};
        use crate::widgets::{network_glyph_name, NetworkWidget};
        use cairo::{FontFace, FontSlant, FontWeight, ImageSurface};
        use chrono::DateTime;
        use listello_protocol::NetworkState;
        use listello_surface::{image_from_rgba, Align, Color, DrawCmd, Rect, TextStyle};
        use std::rc::Rc;

        const RECT: Rect = Rect::new(0.0, 0.0, 48.0, 32.0);

        fn fixture() -> Option<(Theme, UiState)> {
            let font =
                FontFace::toy_create("sans-serif", FontSlant::Normal, FontWeight::Normal).ok()?;
            let theme = Theme {
                text: TextStyle::new(Rc::new(font), 14.0),
                background: Color::BLACK,
                foreground: Color::WHITE,
                button: Color::grey(0.2),
                button_pressed: Color::grey(0.3),
                accent: Color::grey(0.8),
                corner_radius: 4.0,
            };
            let now = DateTime::parse_from_rfc3339("2026-08-03T14:05:00+02:00").ok()?;
            Some((theme, UiState::new(now)))
        }

        /// A tiny stub icon, built in memory exactly as the golden test needs one. 4×4 of opaque
        /// grey is enough for the command list to carry it; the golden builds its own, larger.
        fn stub_icon() -> Option<ImageSurface> {
            image_from_rgba(4, 4, &[0xaa; 64])
        }

        fn network(state: &mut UiState, connected: bool, signal: f64, ssid: Option<&str>) {
            state.network = Some(NetworkState {
                connected,
                signal,
                ssid: ssid.map(str::to_string),
            });
        }

        fn render(widget: &NetworkWidget, theme: &Theme, state: &UiState) -> Output {
            let ctx = Ctx {
                state,
                theme,
                index: 0,
            };
            let mut out = Output::new();
            widget.render(&ctx, RECT, &mut out);
            out
        }

        fn with_icon(state: &mut UiState) {
            if let Some(icon) = stub_icon() {
                state.icons.insert(IconKey::Named("network-3".into()), icon);
            }
        }

        #[test]
        fn glyph_name_buckets_signal_strength_at_the_spec_boundaries() {
            // The phase spec's five boundary values. 0.25/0.5/0.75 are the bucket edges.
            let name = |signal: f64| {
                network_glyph_name(&NetworkState {
                    connected: true,
                    signal,
                    ssid: None,
                })
            };
            assert_eq!(name(0.0), Some("network-0"));
            assert_eq!(name(0.24), Some("network-0"));
            assert_eq!(name(0.25), Some("network-1"));
            assert_eq!(name(0.99), Some("network-3"));
            assert_eq!(name(1.0), Some("network-3"));
        }

        #[test]
        fn disconnected_state_uses_the_off_glyph_regardless_of_signal() {
            assert_eq!(
                network_glyph_name(&NetworkState {
                    connected: false,
                    signal: 0.0,
                    ssid: None,
                }),
                Some("network-off")
            );
            assert_eq!(
                network_glyph_name(&NetworkState {
                    connected: false,
                    signal: 0.99,
                    ssid: None,
                }),
                Some("network-off")
            );
        }

        #[test]
        fn non_finite_signal_means_draw_nothing() {
            assert_eq!(
                network_glyph_name(&NetworkState {
                    connected: true,
                    signal: f64::NAN,
                    ssid: None,
                }),
                None
            );
            assert_eq!(
                network_glyph_name(&NetworkState {
                    connected: true,
                    signal: f64::INFINITY,
                    ssid: None,
                }),
                None
            );
        }

        #[test]
        fn no_network_state_draws_nothing() {
            let Some((theme, state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            let widget = NetworkWidget {
                show_ssid: false,
                sizing: Sizing::Fixed(48.0),
            };
            let out = render(&widget, &theme, &state);
            assert!(out.commands().is_empty());
            assert!(out.hits().is_empty());
        }

        #[test]
        fn connected_network_draws_the_bucket_glyph() {
            let Some((theme, mut state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            network(&mut state, true, 0.99, None);
            // Only `network-3` is in the store: a request for any other bucket would find
            // nothing and draw nothing, so an Image command proves which key was asked for.
            with_icon(&mut state);
            let widget = NetworkWidget {
                show_ssid: false,
                sizing: Sizing::Fixed(48.0),
            };
            let out = render(&widget, &theme, &state);
            assert_eq!(out.commands().len(), 1);
            assert!(
                matches!(&out.commands()[0], DrawCmd::Image { alpha: 1.0, .. }),
                "expected an Image command, got {:?}",
                out.commands()[0]
            );
            assert!(out.hits().is_empty());
        }

        #[test]
        fn wrong_bucket_glyph_is_missing_so_nothing_is_drawn() {
            let Some((theme, mut state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            // 0.24 requests `network-0`, but only `network-3` is in the store.
            network(&mut state, true, 0.24, None);
            with_icon(&mut state);
            let widget = NetworkWidget {
                show_ssid: false,
                sizing: Sizing::Fixed(48.0),
            };
            let out = render(&widget, &theme, &state);
            assert!(out.commands().is_empty());
        }

        #[test]
        fn ssid_is_drawn_when_requested_and_the_glyph_exists() {
            let Some((theme, mut state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            network(&mut state, true, 0.99, Some("home-wifi"));
            with_icon(&mut state);
            let widget = NetworkWidget {
                show_ssid: true,
                sizing: Sizing::Fixed(48.0),
            };
            let out = render(&widget, &theme, &state);
            assert_eq!(out.commands().len(), 2);
            assert!(
                matches!(&out.commands()[1], DrawCmd::Text { text, .. } if text == "home-wifi"),
                "expected the ssid text, got {:?}",
                out.commands()[1]
            );
        }

        #[test]
        fn ssid_is_the_fallback_label_when_the_glyph_is_missing() {
            let Some((theme, mut state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            // Connected with a real ssid, but no icon in the store (fresh boot, agent not yet
            // answered, or the packaged set lacks this name): the label is drawn instead.
            network(&mut state, true, 0.99, Some("home-wifi"));
            let widget = NetworkWidget {
                show_ssid: true,
                sizing: Sizing::Fixed(48.0),
            };
            let out = render(&widget, &theme, &state);
            assert_eq!(out.commands().len(), 1);
            assert!(
                matches!(
                    &out.commands()[0],
                    DrawCmd::Text {
                        align: Align::Center,
                        ..
                    }
                ),
                "expected a centred fallback label, got {:?}",
                out.commands()[0]
            );
        }

        #[test]
        fn ssid_alone_never_draws_without_show_ssid() {
            let Some((theme, mut state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            network(&mut state, true, 0.99, Some("home-wifi"));
            let widget = NetworkWidget {
                show_ssid: false,
                sizing: Sizing::Fixed(48.0),
            };
            let out = render(&widget, &theme, &state);
            // No icon in the store and ssid hidden: nothing to draw, and no text either.
            assert!(out.commands().is_empty());
        }

        #[test]
        fn sizing_returns_what_it_was_built_with() {
            let fixed = NetworkWidget {
                show_ssid: false,
                sizing: Sizing::Fixed(48.0),
            };
            assert_eq!(fixed.sizing(), Sizing::Fixed(48.0));
            let flex = NetworkWidget {
                show_ssid: false,
                sizing: Sizing::Flex(2.0),
            };
            assert_eq!(flex.sizing(), Sizing::Flex(2.0));
        }
    }

    mod control_center {
        use crate::hit::{Action, KeyName, RegionId};
        use crate::state::{IconKey, Theme, UiState};
        use crate::widget::{Ctx, Output, Sizing, Widget};
        use crate::widgets::{ControlButton, ControlCenterWidget};
        use cairo::{FontFace, FontSlant, FontWeight};
        use chrono::DateTime;
        use listello_protocol::SystemAction;
        use listello_surface::{image_from_rgba, Align, Color, DrawCmd, Rect, TextStyle};
        use std::rc::Rc;

        const RECT: Rect = Rect::new(0.0, 0.0, 200.0, 32.0);

        fn fixture() -> Option<(Theme, UiState)> {
            let font =
                FontFace::toy_create("sans-serif", FontSlant::Normal, FontWeight::Normal).ok()?;
            let theme = Theme {
                text: TextStyle::new(Rc::new(font), 14.0),
                background: Color::BLACK,
                foreground: Color::WHITE,
                button: Color::grey(0.2),
                button_pressed: Color::grey(0.3),
                accent: Color::grey(0.8),
                corner_radius: 4.0,
            };
            let now = DateTime::parse_from_rfc3339("2026-08-03T14:05:00+02:00").ok()?;
            Some((theme, UiState::new(now)))
        }

        /// The usual six: lock, brightness ∓, mute, volume ∓.
        fn six_buttons() -> Vec<ControlButton> {
            vec![
                ControlButton {
                    icon: "lock".into(),
                    label: None,
                    action: Action::System(SystemAction::LockSession),
                },
                ControlButton {
                    icon: "brightness-down".into(),
                    label: None,
                    action: Action::Keys(vec![KeyName::new("KEY_BRIGHTNESSDOWN").unwrap()]),
                },
                ControlButton {
                    icon: "brightness-up".into(),
                    label: None,
                    action: Action::Keys(vec![KeyName::new("KEY_BRIGHTNESSUP").unwrap()]),
                },
                ControlButton {
                    icon: "mute".into(),
                    label: None,
                    action: Action::Keys(vec![KeyName::new("KEY_MUTE").unwrap()]),
                },
                ControlButton {
                    icon: "volume-down".into(),
                    label: None,
                    action: Action::Keys(vec![KeyName::new("KEY_VOLUMEDOWN").unwrap()]),
                },
                ControlButton {
                    icon: "volume-up".into(),
                    label: None,
                    action: Action::Keys(vec![KeyName::new("KEY_VOLUMEUP").unwrap()]),
                },
            ]
        }

        fn render(
            widget: &ControlCenterWidget,
            theme: &Theme,
            state: &UiState,
            rect: Rect,
        ) -> Output {
            let ctx = Ctx {
                state,
                theme,
                index: 0,
            };
            let mut out = Output::new();
            widget.render(&ctx, rect, &mut out);
            out
        }

        fn with_icon(state: &mut UiState, name: &str) {
            if let Some(icon) = image_from_rgba(8, 8, &[0xaa; 8 * 8 * 4]) {
                state.icons.insert(IconKey::Named(name.into()), icon);
            }
        }

        #[test]
        fn each_part_carries_the_action_it_was_built_with() {
            let Some((theme, state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            let buttons = six_buttons();
            let widget = ControlCenterWidget {
                buttons,
                sizing: Sizing::Fixed(200.0),
            };
            let out = render(&widget, &theme, &state, RECT);

            let hits = out.hits();
            assert_eq!(hits.len(), 6, "one hit region per button");
            let expected: Vec<Action> = six_buttons().into_iter().map(|b| b.action).collect();
            for (i, hit) in hits.iter().enumerate() {
                assert_eq!(hit.id, RegionId::new(0, i as u16), "part {i}");
                assert_eq!(hit.action, expected[i], "part {i}");
            }
        }

        #[test]
        fn one_panel_split_evenly_between_the_buttons() {
            let Some((theme, state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            let widget = ControlCenterWidget {
                buttons: six_buttons(),
                sizing: Sizing::Fixed(240.0),
            };
            let rect = Rect::new(RECT.x, RECT.y, 240.0, RECT.height);
            let out = render(&widget, &theme, &state, rect);

            // One background for the whole widget, and nothing else behind the glyphs.
            let panels: Vec<Rect> = out
                .commands()
                .iter()
                .filter_map(|c| match c {
                    DrawCmd::RoundedRect { rect, .. } => Some(*rect),
                    _ => None,
                })
                .collect();
            assert_eq!(panels, [rect]);
            // Six 40px parts, edge to edge, covering it exactly.
            let hits: Vec<Rect> = out.hits().iter().map(|h| h.rect).collect();
            assert_eq!(hits.len(), 6);
            for (i, r) in hits.iter().enumerate() {
                let expected = Rect::new(rect.x + i as f64 * 40.0, rect.y, 40.0, rect.height);
                assert_eq!(*r, expected, "button {i}");
            }
        }

        #[test]
        fn pressed_part_uses_the_pressed_colour() {
            let Some((theme, mut state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            // The third button (brightness up) is held down.
            state.pressed = Some(RegionId::new(0, 2));
            let widget = ControlCenterWidget {
                buttons: six_buttons(),
                sizing: Sizing::Fixed(200.0),
            };
            let out = render(&widget, &theme, &state, RECT);
            let rects: Vec<(Rect, Color)> = out
                .commands()
                .iter()
                .filter_map(|c| match c {
                    DrawCmd::RoundedRect { rect, color, .. } => Some((*rect, *color)),
                    _ => None,
                })
                .collect();
            // The panel, then the pressed button's highlight inside the third part.
            assert_eq!(rects.len(), 2);
            assert_eq!(rects[0], (RECT, theme.button));
            assert_eq!(rects[1].1, theme.button_pressed);
            let third = out.hits()[2].rect;
            assert!(rects[1].0.left() >= third.left() && rects[1].0.right() <= third.right());
        }

        #[test]
        fn icon_is_drawn_when_present_and_label_otherwise() {
            let Some((theme, mut state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            with_icon(&mut state, "lock");
            let widget = ControlCenterWidget {
                buttons: vec![ControlButton {
                    icon: "lock".into(),
                    label: Some("Lock".into()),
                    action: Action::System(SystemAction::LockSession),
                }],
                sizing: Sizing::Fixed(60.0),
            };
            let out = render(&widget, &theme, &state, RECT);
            assert_eq!(out.commands().len(), 2, "rounded rect then icon");
            assert!(
                matches!(&out.commands()[1], DrawCmd::Image { .. }),
                "expected an Image command, got {:?}",
                out.commands()[1]
            );

            // Same button without the icon in the store: the label replaces it.
            let bare_state = UiState::new(state.now);
            let out = render(&widget, &theme, &bare_state, RECT);
            assert_eq!(out.commands().len(), 2, "rounded rect then label");
            assert!(
                matches!(
                    &out.commands()[1],
                    DrawCmd::Text { text, align, .. }
                        if text == "Lock" && align == &Align::Center
                ),
                "expected a centred label, got {:?}",
                out.commands()[1]
            );
        }

        #[test]
        fn neither_icon_nor_label_draws_just_the_button() {
            let Some((theme, state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            let widget = ControlCenterWidget {
                buttons: vec![ControlButton {
                    icon: "ghost".into(),
                    label: None,
                    action: Action::None,
                }],
                sizing: Sizing::Fixed(60.0),
            };
            let out = render(&widget, &theme, &state, RECT);
            assert_eq!(out.commands().len(), 1, "only the rounded rect");
            assert!(
                matches!(&out.commands()[0], DrawCmd::RoundedRect { .. }),
                "the only command must be a RoundedRect, got {:?}",
                out.commands()[0]
            );
        }

        #[test]
        fn no_buttons_draws_and_hits_nothing() {
            let Some((theme, state)) = fixture() else {
                eprintln!("skipping: cairo could not create the toy font");
                return;
            };
            let widget = ControlCenterWidget {
                buttons: Vec::new(),
                sizing: Sizing::Fixed(60.0),
            };
            let out = render(&widget, &theme, &state, RECT);
            assert!(out.commands().is_empty());
            assert!(out.hits().is_empty());
        }

        #[test]
        fn sizing_returns_what_it_was_built_with() {
            let fixed = ControlCenterWidget {
                buttons: Vec::new(),
                sizing: Sizing::Fixed(200.0),
            };
            assert_eq!(fixed.sizing(), Sizing::Fixed(200.0));
            let flex = ControlCenterWidget {
                buttons: Vec::new(),
                sizing: Sizing::Flex(2.0),
            };
            assert_eq!(flex.sizing(), Sizing::Flex(2.0));
        }
    }
}
