//! The Dock: one tile per application, pinned apps first, then whatever else has a window.
//!
//! Tap an app with one window to focus it, an app with none (pinned, not running) to launch it,
//! and an app with several to open a row of labelled window entries instead (D7: titles, never
//! thumbnails). The strip scrolls sideways when it does not fit.
//!
//! A pure function like every widget (D3). Everything it needs is in the snapshot -- the window
//! list, the app icons the agent sent, which app is expanded, how far it is scrolled -- and
//! everything it wants done is an [`Action`] the shell decides on.
//!
//! The list of windows arrives with canonical app ids (the agent resolves a window's `app_id`
//! to its `.desktop` id), so `pinned = ["org.kde.dolphin"]` matches the Dolphin window without
//! this module knowing anything about desktop files.

use crate::config::PreviewStyle;
use crate::hit::Action;
use crate::layout::scroll_offset_clamped;
use crate::state::{IconKey, UiState, WindowControls, BOUNCE_PERIOD};
use crate::widget::{Ctx, Output, Sizing, Widget};
use listello_protocol::{AppId, PreviewFit, WindowCommand, WindowId, WindowInfo};
use listello_surface::{measure_text, Align, Color, DrawCmd, Rect, TextStyle};

/// Space between tiles. Pock spaces its icons a little under 100px apart on the panel, and
/// this with [`TILE_WIDTH`] is about that.
const GAP: f64 = 12.0;
/// Width of an application tile; its height is the strip's. Wider than the icon so the
/// pressable area is generous, but no tile is drawn: icons sit on the black, as in Pock.
const TILE_WIDTH: f64 = 80.0;
/// Distance from the top of the strip to the top of an app icon. Fixed whether or not the app
/// is running, so the icon never shifts when its dot comes and goes.
const ICON_TOP: f64 = 7.0;
/// The running dot: its side, and its gap above the strip's bottom edge.
const DOT: f64 = 5.0;
const DOT_BOTTOM: f64 = 2.0;
/// The expanded row: the back tile (chevron and the app's icon), and the name block after it.
const BACK_WIDTH: f64 = 104.0;
const HEADER_MAX_WIDTH: f64 = 220.0;
/// A window entry's width bounds. Entries share the room there is, within these.
const WINDOW_MIN_WIDTH: f64 = 200.0;
const WINDOW_MAX_WIDTH: f64 = 380.0;
/// Text inside a window entry starts this far from its edges.
const WINDOW_PADDING: f64 = 14.0;
/// A window's preview (D7, `[strip] window_previews`): this far in from the entry's edges, as
/// tall as the rest allows and this many times as wide, the shape of a typical window. Every
/// entry gets the same box, so the row does not jump when the pictures arrive.
const PREVIEW_INSET: f64 = 5.0;
const PREVIEW_ASPECT: f64 = 1.6;
/// With the picture filling the entry, it stops this far inside the entry's edges, so the
/// rounded corners still show around its square ones.
const FILL_INSET: f64 = 3.0;
/// And is darkened this much, so the title over it reads on any window.
const FILL_DIM: f64 = 0.45;
/// Extra room around the line between pinned and other apps.
const DIVIDER_PAD: f64 = 6.0;
/// Window titles are arbitrary user data: they are cut here, before drawing, so an enormous
/// one cannot become an enormous allocation or a slow text layout.
const MAX_TITLE_CHARS: usize = 60;
/// How high a launching app's icon hops.
const BOUNCE_HEIGHT: f64 = 7.0;
/// The region id for "the dock itself". Larger than any tile index, so it is a stable number
/// that never collides, and emitted last so a tile wins where both would match.
pub const BACKGROUND_PART: u16 = u16::MAX;
/// The four controls' region ids (three lights and the pin), just below the background's;
/// tiles stay below them.
const CONTROL_PART: u16 = u16::MAX - 4;
const LAST_TILE_PART: usize = CONTROL_PART as usize - 1;
/// The window controls: each light's diameter, the space between lights, and the space before
/// the first and after the last.
const LIGHT: f64 = 32.0;
const LIGHT_GAP: f64 = 20.0;
const LIGHTS_PAD: f64 = 14.0;
/// The room `n` lights take, margins included.
fn controls_width(n: usize) -> f64 {
    let n = n as f64;
    LIGHTS_PAD + n * LIGHT + (n - 1.0).max(0.0) * LIGHT_GAP + LIGHTS_PAD
}
/// How long the controls take to slide out from under the icon.
const CONTROLS_SLIDE_SECS: f64 = 0.18;
/// How much of the background is laid over the other apps while one app's controls are up.
const FADE: f64 = 0.65;
/// The wiggle of the icon whose controls are up: how far it turns each way, and how often.
const WIGGLE_ANGLE: f64 = 0.05;
const WIGGLE_HZ: f64 = 4.0;

/// One application as the Dock shows it: who it is, and which windows it has.
#[derive(Debug, PartialEq)]
pub struct AppGroup<'a> {
    pub app_id: &'a AppId,
    pub windows: Vec<&'a WindowInfo>,
    pub pinned: bool,
}

impl AppGroup<'_> {
    pub fn is_active(&self) -> bool {
        self.windows.iter().any(|w| w.active)
    }

    /// What tapping the tile does.
    pub fn tap(&self) -> Action {
        match self.windows.as_slice() {
            [] => Action::Launch(self.app_id.clone()),
            [only] => Action::Activate(only.id),
            _ => Action::ToggleApp(self.app_id.clone()),
        }
    }
}

/// Pinned apps in the order config lists them, then every other app with a window in the order
/// its first window appeared. An app keeps its place until its last window closes; focus
/// changes never move it.
pub fn group<'a>(pinned: &'a [AppId], windows: &'a [WindowInfo]) -> Vec<AppGroup<'a>> {
    let mut groups: Vec<AppGroup<'a>> = Vec::new();
    for app_id in pinned {
        if groups.iter().any(|g| g.app_id == app_id) {
            continue;
        }
        groups.push(AppGroup {
            app_id,
            windows: Vec::new(),
            pinned: true,
        });
    }
    for window in windows {
        match groups.iter_mut().find(|g| g.app_id == &window.app_id) {
            Some(group) => group.windows.push(window),
            None => groups.push(AppGroup {
                app_id: &window.app_id,
                windows: vec![window],
                pinned: false,
            }),
        }
    }
    groups
}

pub struct DockWidget {
    pub pinned: Vec<AppId>,
    pub sizing: Sizing,
}

impl DockWidget {
    /// The pinned apps: the user's, once they have pinned or unpinned anything from the Touch
    /// Bar, and until then the config's.
    fn pins<'s>(&'s self, state: &'s UiState) -> &'s [AppId] {
        state.pins.as_deref().unwrap_or(&self.pinned)
    }
}

impl Widget for DockWidget {
    fn sizing(&self) -> Sizing {
        self.sizing
    }

    fn scrollable(&self) -> bool {
        true
    }

    fn wanted_apps(&self) -> Vec<AppId> {
        self.pinned.clone()
    }

    /// One app's windows get the whole Touch Bar: listello's choice, where Pock keeps its
    /// other widgets beside them.
    ///
    /// So do an app's controls after a long press: they need room to the right of the icon,
    /// and near the Dock's end they would be cut off. The Dock starts at the strip's left edge
    /// either way, so the held icon does not move from under the finger.
    fn takes_whole_strip(&self, state: &UiState) -> bool {
        state.agent_connected
            && (state.controls.is_some() || {
                let groups = group(self.pins(state), &state.windows);
                expanded_group(&groups, state).is_some()
            })
    }

    fn render(&self, ctx: &Ctx<'_>, rect: Rect, out: &mut Output) {
        // With no agent there is nothing true to show: no window list, and a tap on a pinned
        // app could not launch anything. Draw nothing rather than tiles that lie (D11).
        if !ctx.state.agent_connected {
            return;
        }
        let pins = self.pins(ctx.state);
        let groups = group(pins, &ctx.state.windows);
        let expanded = expanded_group(&groups, ctx.state);

        let mut slots = match expanded {
            Some(g) => expanded_slots(ctx, g, rect.width, rect.height),
            None => app_slots(&groups),
        };
        add_controls(ctx, pins, &mut slots);
        let content_width = slots.last().map_or(0.0, |s| s.x + s.width);
        let mut offset = scroll_offset_clamped(ctx.scroll(), content_width, rect.width);
        // Controls past the end of a Dock that fills the strip: shift just enough to show them
        // whole, for this frame only, never moving the scroll the user left.
        if let Some(controls) = slots
            .iter()
            .find(|s| matches!(s.face, Face::Controls { .. }))
        {
            let right = controls.x + controls.width;
            if right - offset > rect.width {
                offset = right - rect.width;
            }
        }
        let target = ctx
            .state
            .controls
            .as_ref()
            .and_then(|c| slots.iter().position(|s| is_target(s, c)));

        out.draw(DrawCmd::PushClip(rect));
        for slot in &slots {
            let tile = Rect::new(rect.x + slot.x - offset, rect.y, slot.width, rect.height);
            if tile.right() <= rect.left() || tile.left() >= rect.right() {
                continue;
            }
            draw_slot(ctx, slot, tile, out);
            // With controls up, everything but the app they are for, and the controls, fades.
            let faded = target.is_some_and(|t| {
                !std::ptr::eq(slot, &slots[t]) && !matches!(slot.face, Face::Controls { .. })
            });
            if faded {
                out.draw(DrawCmd::Rect {
                    rect: tile,
                    color: ctx.theme.background.with_alpha(FADE),
                });
            }
            // A tile scrolled partly out of view is pressable only where it is visible, so it
            // cannot catch a touch that belongs to the widget next door.
            let visible = tile.clamp_to(&rect);
            if let (Some(part), false) = (slot.part, visible.is_empty()) {
                out.hit(ctx.region(part), visible, slot.action.clone());
            }
        }
        out.draw(DrawCmd::PopClip);
        // Last, so a tile wins where both match. Pressing the gaps must still start a scroll.
        out.hit(ctx.region(BACKGROUND_PART), rect, Action::None);
    }
}

/// The app whose windows are shown, while it still has at least two of them.
fn expanded_group<'g, 'a>(groups: &'g [AppGroup<'a>], state: &UiState) -> Option<&'g AppGroup<'a>> {
    state
        .expanded
        .as_ref()
        .and_then(|app| groups.iter().find(|g| g.app_id == app))
        .filter(|g| g.windows.len() >= 2)
}

/// What one slot of the row shows.
enum Face<'a> {
    App(&'a AppGroup<'a>),
    /// The way back from the expanded row, wearing the expanded app's icon.
    Back(&'a AppId),
    /// The app's name and window count, next to the back tile. Not pressable.
    Header {
        name: String,
        windows: usize,
    },
    /// A window entry, with the title already shortened to what is worth reading.
    Window {
        window: &'a WindowInfo,
        title: String,
    },
    Divider,
    /// The close, minimise and maximise lights for one window, sliding out to the right of the
    /// tile they belong to.
    Controls {
        lights: Vec<Light>,
        slide: f64,
    },
}

struct Slot<'a> {
    x: f64,
    width: f64,
    /// `None` for decoration that cannot be pressed.
    part: Option<u16>,
    action: Action,
    face: Face<'a>,
}

fn app_slots<'a>(groups: &'a [AppGroup<'a>]) -> Vec<Slot<'a>> {
    let mut slots = Vec::new();
    let mut x = 0.0;
    for (i, g) in groups.iter().enumerate() {
        // The line between the pinned apps and the rest.
        if i > 0 && groups[i - 1].pinned && !g.pinned {
            x += DIVIDER_PAD;
            slots.push(Slot {
                x,
                width: 2.0,
                part: None,
                action: Action::None,
                face: Face::Divider,
            });
            x += 2.0 + DIVIDER_PAD;
        } else if i > 0 {
            x += GAP;
        }
        slots.push(Slot {
            x,
            width: TILE_WIDTH,
            part: Some(i.min(LAST_TILE_PART) as u16),
            action: g.tap(),
            face: Face::App(g),
        });
        x += TILE_WIDTH;
    }
    slots
}

/// The back tile, the app's name, then one entry per window. The icon appears once, on the back
/// tile; the entries carry only titles, so the titles get the room.
fn expanded_slots<'a>(
    ctx: &Ctx<'_>,
    group: &'a AppGroup<'a>,
    width: f64,
    height: f64,
) -> Vec<Slot<'a>> {
    let titles: Vec<&str> = group.windows.iter().map(|w| w.title.as_str()).collect();
    let (suffix, titles) = strip_common_suffix(&titles);
    let name = suffix.unwrap_or_default();
    let header_style = small_text(ctx);
    let header_width = measure_text(&header_style, &name)
        .advance
        .max(measure_text(&header_style, &window_count(group.windows.len())).advance)
        .ceil()
        .min(HEADER_MAX_WIDTH)
        + GAP;

    let mut slots = vec![
        Slot {
            x: 0.0,
            width: BACK_WIDTH,
            part: Some(0),
            action: Action::ToggleApp(group.app_id.clone()),
            face: Face::Back(group.app_id),
        },
        Slot {
            x: BACK_WIDTH + GAP,
            width: header_width,
            part: None,
            action: Action::None,
            face: Face::Header {
                name,
                windows: group.windows.len(),
            },
        },
    ];
    let mut x = BACK_WIDTH + GAP + header_width;
    let n = group.windows.len().max(1) as f64;
    // A preview takes its room from beside the title, not from the title.
    let extra = if ctx.state.window_previews == Some(PreviewStyle::Whole) {
        preview_box(Rect::new(0.0, 0.0, 0.0, height)).width + PREVIEW_INSET
    } else {
        0.0
    };
    let entry_width =
        ((width - x - GAP * n) / n).clamp(WINDOW_MIN_WIDTH + extra, WINDOW_MAX_WIDTH + extra);
    for (i, (w, title)) in group.windows.iter().zip(titles).enumerate() {
        x += GAP;
        slots.push(Slot {
            x,
            width: entry_width,
            part: Some((i + 1).min(LAST_TILE_PART) as u16),
            action: Action::Activate(w.id),
            face: Face::Window { window: w, title },
        });
        x += entry_width;
    }
    slots
}

/// What a slot can bring up controls for: an app tile with no window or one window, or a
/// window entry. An app with several windows opens its row instead.
fn slot_target<'s>(slot: &'s Slot<'_>) -> Option<(&'s AppId, Option<WindowId>)> {
    match &slot.face {
        Face::App(g) if g.windows.is_empty() => Some((g.app_id, None)),
        Face::App(g) if g.windows.len() == 1 => Some((g.app_id, Some(g.windows[0].id))),
        Face::Window { window, .. } => Some((&window.app_id, Some(window.id))),
        _ => None,
    }
}

fn is_target(slot: &Slot<'_>, controls: &WindowControls) -> bool {
    slot_target(slot) == Some((&controls.app, controls.window))
}

/// One of the controls: its look and what it does.
struct Light {
    look: LightLook,
    action: Action,
}

enum LightLook {
    /// A coloured light with a mark on it, like a macOS title bar button.
    Window(Color, &'static str),
    /// The pin, grey, wearing this packaged icon.
    Pin(&'static str),
}

/// The controls for `controls`: close, minimise and maximise when there is a window, and
/// always the pin, which pins the app or, if it is pinned already, unpins it.
fn lights(controls: &WindowControls, pins: &[AppId]) -> Vec<Light> {
    let mut out = Vec::new();
    if let Some(window) = controls.window {
        for (color, mark, command) in [
            (Color::rgb(1.0, 0.37, 0.34), "×", WindowCommand::Close),
            (Color::rgb(1.0, 0.74, 0.18), "−", WindowCommand::Minimize),
            (
                Color::rgb(0.16, 0.78, 0.25),
                "+",
                WindowCommand::ToggleMaximize,
            ),
        ] {
            out.push(Light {
                look: LightLook::Window(color, mark),
                action: Action::Window(window, command),
            });
        }
    }
    let pinned = pins.contains(&controls.app);
    let new_pins = if pinned {
        pins.iter()
            .filter(|p| *p != &controls.app)
            .cloned()
            .collect()
    } else {
        pins.iter().cloned().chain([controls.app.clone()]).collect()
    };
    out.push(Light {
        look: LightLook::Pin(if pinned { "pin-off" } else { "pin" }),
        action: Action::SetPins(new_pins),
    });
    out
}

/// Put the controls, when they are up, right after the slot they belong to, and push
/// everything after it to the right by the room they take.
fn add_controls(ctx: &Ctx<'_>, pins: &[AppId], slots: &mut Vec<Slot<'_>>) {
    let Some(controls) = &ctx.state.controls else {
        return;
    };
    let Some(at) = slots.iter().position(|s| is_target(s, controls)) else {
        return;
    };
    let lights = lights(controls, pins);
    let t = (controls.age / CONTROLS_SLIDE_SECS).clamp(0.0, 1.0);
    // Ease out: quick at first, settling into place.
    let slide = 1.0 - (1.0 - t) * (1.0 - t);
    let width = controls_width(lights.len()) * slide;
    for later in &mut slots[at + 1..] {
        later.x += width;
    }
    let x = slots[at].x + slots[at].width;
    slots.insert(
        at + 1,
        Slot {
            x,
            width,
            part: None,
            action: Action::None,
            face: Face::Controls { lights, slide },
        },
    );
}

/// How far the icon whose controls are up is turned, `age` seconds after they came up.
fn wiggle(age: f64) -> f64 {
    if !age.is_finite() {
        return 0.0;
    }
    WIGGLE_ANGLE * (age * WIGGLE_HZ * std::f64::consts::TAU).sin()
}

fn draw_slot(ctx: &Ctx<'_>, slot: &Slot<'_>, tile: Rect, out: &mut Output) {
    let wiggling = ctx
        .state
        .controls
        .as_ref()
        .filter(|c| is_target(slot, c))
        .map(|c| wiggle(c.age));
    let pressed = slot.part.is_some_and(|p| ctx.is_pressed(p));
    match &slot.face {
        Face::Controls { lights, slide } => draw_controls(ctx, lights, *slide, tile, out),
        Face::Divider => out.draw(DrawCmd::Rect {
            rect: Rect::new(
                tile.x,
                tile.y + 10.0,
                tile.width,
                (tile.height - 20.0).max(0.0),
            ),
            color: ctx.theme.foreground.with_alpha(0.33),
        }),
        Face::App(g) => {
            // No tile: the icon sits on the black, and a press lights the area behind it.
            if pressed {
                out.draw(DrawCmd::RoundedRect {
                    rect: tile,
                    radius: ctx.theme.corner_radius,
                    color: ctx.theme.button,
                });
            }
            let hop = ctx
                .state
                .launching
                .get(g.app_id)
                .map_or(0.0, |launch| bounce_lift(launch.age));
            draw_app_icon(
                ctx,
                g.app_id,
                tile,
                Some(ICON_TOP),
                hop,
                wiggling.unwrap_or(0.0),
                out,
            );
            if !g.windows.is_empty() {
                // The "running" dot under the icon; one dot per app, however many windows.
                // White for the app that has the focus, grey for the rest.
                out.draw(DrawCmd::RoundedRect {
                    rect: Rect::new(
                        tile.center_x() - DOT / 2.0,
                        tile.bottom() - DOT - DOT_BOTTOM,
                        DOT,
                        DOT,
                    ),
                    radius: DOT / 2.0,
                    color: if g.is_active() {
                        ctx.theme.foreground
                    } else {
                        ctx.theme.foreground.with_alpha(0.6)
                    },
                });
            }
        }
        Face::Back(app) => {
            out.draw(DrawCmd::RoundedRect {
                rect: tile,
                radius: ctx.theme.corner_radius,
                color: if pressed {
                    ctx.theme.button_pressed
                } else {
                    ctx.theme.button
                },
            });
            let chevron = Rect::new(tile.x + 4.0, tile.y, 32.0, tile.height);
            out.draw(DrawCmd::Text {
                rect: chevron,
                text: "‹".to_string(),
                style: ctx.theme.text.clone(),
                color: ctx.theme.foreground.with_alpha(0.75),
                align: Align::Center,
            });
            let icon_area = Rect::new(
                chevron.right(),
                tile.y,
                tile.right() - chevron.right() - 8.0,
                tile.height,
            );
            draw_app_icon(ctx, app, icon_area, None, 0.0, 0.0, out);
        }
        Face::Header { name, windows } => {
            let style = small_text(ctx);
            let half = tile.height / 2.0;
            let top = Rect::new(tile.x, tile.y + 4.0, tile.width - GAP, half - 4.0);
            let bottom = Rect::new(tile.x, tile.y + half, tile.width - GAP, half - 4.0);
            let count = window_count(*windows);
            if name.is_empty() {
                out.draw(DrawCmd::Text {
                    rect: Rect::new(tile.x, tile.y, tile.width - GAP, tile.height),
                    text: count,
                    style,
                    color: ctx.theme.foreground.with_alpha(0.6),
                    align: Align::Start,
                });
            } else {
                out.draw(DrawCmd::Text {
                    rect: top,
                    text: elide_middle(&style, name, top.width),
                    style: style.clone(),
                    color: ctx.theme.foreground,
                    align: Align::Start,
                });
                out.draw(DrawCmd::Text {
                    rect: bottom,
                    text: count,
                    style,
                    color: ctx.theme.foreground.with_alpha(0.6),
                    align: Align::Start,
                });
            }
        }
        Face::Window { window, title } => {
            // A rounded rect cannot turn; an entry with its controls up sways instead.
            let tile = match wiggling {
                Some(angle) => Rect::new(tile.x + angle * 40.0, tile.y, tile.width, tile.height),
                None => tile,
            };
            out.draw(DrawCmd::RoundedRect {
                rect: tile,
                radius: ctx.theme.corner_radius,
                color: if pressed {
                    ctx.theme.button_pressed
                } else {
                    ctx.theme.button
                },
            });
            let mut text_left = tile.x + WINDOW_PADDING;
            let mut over_picture = false;
            match ctx.state.window_previews {
                Some(PreviewStyle::Whole) => {
                    let preview = preview_box(tile);
                    draw_preview(ctx, window.id, preview, out);
                    text_left = preview.right() + WINDOW_PADDING;
                }
                Some(style) => {
                    over_picture = draw_fill_preview(ctx, window.id, style, tile, pressed, out);
                }
                None => {}
            }
            if window.active {
                // A thin bar under the title marks the window that has the focus.
                out.draw(DrawCmd::RoundedRect {
                    rect: Rect::new(tile.center_x() - 12.0, tile.bottom() - 6.0, 24.0, 3.0),
                    radius: 1.5,
                    color: ctx.theme.foreground.with_alpha(0.8),
                });
            }
            let text_rect = Rect::new(
                text_left,
                tile.y,
                (tile.right() - WINDOW_PADDING - text_left).max(0.0),
                tile.height,
            );
            let text = elide_middle(&ctx.theme.text, title, text_rect.width);
            out.draw(DrawCmd::PushClip(text_rect));
            if over_picture {
                // A soft shadow keeps the title readable over a bright window.
                out.draw(DrawCmd::Text {
                    rect: Rect::new(
                        text_rect.x + 1.0,
                        text_rect.y + 1.5,
                        text_rect.width,
                        text_rect.height,
                    ),
                    text: text.clone(),
                    style: ctx.theme.text.clone(),
                    color: ctx.theme.background.with_alpha(0.8),
                    align: Align::Center,
                });
            }
            out.draw(DrawCmd::Text {
                rect: text_rect,
                text,
                style: ctx.theme.text.clone(),
                color: if window.active || over_picture {
                    ctx.theme.foreground
                } else {
                    ctx.theme.foreground.with_alpha(0.75)
                },
                align: Align::Center,
            });
            out.draw(DrawCmd::PopClip);
        }
    }
}

/// Where a window entry's preview goes: at its left, inset on every side.
fn preview_box(entry: Rect) -> Rect {
    let height = (entry.height - 2.0 * PREVIEW_INSET).max(1.0).floor();
    Rect::new(
        entry.x + PREVIEW_INSET,
        entry.y + PREVIEW_INSET,
        (height * PREVIEW_ASPECT).floor(),
        height,
    )
}

/// The size of picture to ask the agent for, on a strip `strip_height` pixels tall, as
/// (width, height). For `Whole`, the preview box the picture is fitted into; otherwise the
/// widest an entry gets, so one picture serves every entry width.
pub fn preview_size(strip_height: f64, style: PreviewStyle) -> (u32, u32) {
    match style {
        PreviewStyle::Whole => {
            let b = preview_box(Rect::new(0.0, 0.0, 0.0, strip_height));
            (b.width as u32, b.height as u32)
        }
        _ => {
            let inner = |v: f64| (v - 2.0 * FILL_INSET).max(1.0).floor() as u32;
            (inner(WINDOW_MAX_WIDTH), inner(strip_height))
        }
    }
}

/// What the agent is asked to make of each window for `style`.
pub fn preview_fit(style: PreviewStyle) -> PreviewFit {
    match style {
        PreviewStyle::Whole => PreviewFit::Whole,
        PreviewStyle::Fill => PreviewFit::Crop,
        PreviewStyle::Stretch => PreviewFit::Stretch,
    }
}

/// The picture filling the entry, darkened for the title that goes over it. `Fill` keeps the
/// picture's middle when the entry is narrower than the picture; `Stretch` squeezes it in.
/// Returns whether there was a picture: without one the entry looks as it does with previews
/// off.
fn draw_fill_preview(
    ctx: &Ctx<'_>,
    id: WindowId,
    style: PreviewStyle,
    tile: Rect,
    pressed: bool,
    out: &mut Output,
) -> bool {
    let Some(image) = ctx.state.previews.get(&id) else {
        return false;
    };
    let area = Rect::new(
        tile.x + FILL_INSET,
        tile.y + FILL_INSET,
        (tile.width - 2.0 * FILL_INSET).max(0.0),
        (tile.height - 2.0 * FILL_INSET).max(0.0),
    );
    let rect = match style {
        PreviewStyle::Stretch => area,
        _ => {
            let (w, h) = (image.width() as f64, image.height() as f64);
            let scale = (area.width / w).max(area.height / h);
            let (w, h) = ((w * scale).round(), (h * scale).round());
            Rect::new(
                area.x + ((area.width - w) / 2.0).round(),
                area.y + ((area.height - h) / 2.0).round(),
                w,
                h,
            )
        }
    };
    out.draw(DrawCmd::PushClip(area));
    out.draw(DrawCmd::Image {
        rect,
        image: image.clone(),
        alpha: 1.0,
        angle: 0.0,
    });
    out.draw(DrawCmd::Rect {
        rect: area,
        color: ctx
            .theme
            .background
            .with_alpha(if pressed { FILL_DIM / 2.0 } else { FILL_DIM }),
    });
    out.draw(DrawCmd::PopClip);
    true
}

/// The picture, centred in its box and as large as fits; a dark placeholder until there is
/// one, or when there never will be (a minimised window).
fn draw_preview(ctx: &Ctx<'_>, id: WindowId, area: Rect, out: &mut Output) {
    let Some(image) = ctx.state.previews.get(&id) else {
        out.draw(DrawCmd::RoundedRect {
            rect: area,
            radius: 4.0,
            color: ctx.theme.background.with_alpha(0.5),
        });
        return;
    };
    let (w, h) = (image.width() as f64, image.height() as f64);
    let scale = (area.width / w).min(area.height / h);
    let (w, h) = ((w * scale).round(), (h * scale).round());
    out.draw(DrawCmd::Image {
        rect: Rect::new(
            area.x + ((area.width - w) / 2.0).round(),
            area.y + ((area.height - h) / 2.0).round(),
            w,
            h,
        ),
        image: image.clone(),
        alpha: 1.0,
        angle: 0.0,
    });
}

/// The lights, macOS's colours and order and then the pin, anchored to the slot's right edge
/// so they slide out from under the tile before them. Each is pressable once fully out.
fn draw_controls(ctx: &Ctx<'_>, lights: &[Light], slide: f64, slot: Rect, out: &mut Output) {
    let glyph = TextStyle::new(ctx.theme.text.font.clone(), LIGHT * 0.8);
    let left = slot.right() - controls_width(lights.len()) + LIGHTS_PAD;
    for (i, light) in lights.iter().enumerate() {
        let x = left + i as f64 * (LIGHT + LIGHT_GAP);
        let disc = Rect::new(x, slot.center_y() - LIGHT / 2.0, LIGHT, LIGHT);
        match light.look {
            LightLook::Window(color, mark) => {
                out.draw(DrawCmd::RoundedRect {
                    rect: disc,
                    radius: LIGHT / 2.0,
                    color,
                });
                out.draw(DrawCmd::Text {
                    rect: Rect::new(disc.x, disc.y - 1.0, disc.width, disc.height),
                    text: mark.to_string(),
                    style: glyph.clone(),
                    color: Color::rgba(0.0, 0.0, 0.0, 0.55),
                    align: Align::Center,
                });
            }
            LightLook::Pin(icon) => {
                out.draw(DrawCmd::RoundedRect {
                    rect: disc,
                    radius: LIGHT / 2.0,
                    color: ctx.theme.button_pressed,
                });
                if let Some(image) = ctx.icon(&IconKey::Named(icon.to_string())) {
                    out.draw(DrawCmd::Image {
                        rect: disc.center(LIGHT * 0.6, LIGHT * 0.6),
                        image: image.clone(),
                        alpha: 1.0,
                        angle: 0.0,
                    });
                }
            }
        }
        let pressable = Rect::new(
            disc.x - LIGHT_GAP / 2.0,
            slot.y,
            LIGHT + LIGHT_GAP,
            slot.height,
        )
        .clamp_to(&slot);
        if slide >= 1.0 && !pressable.is_empty() {
            out.hit(
                ctx.region(CONTROL_PART + i as u16),
                pressable,
                light.action.clone(),
            );
        }
    }
}

/// The app's icon at its natural size, centred across `area` and `top` below its top edge
/// (centred both ways when `None`), raised by `hop`; its first letter when the agent has none.
fn draw_app_icon(
    ctx: &Ctx<'_>,
    app: &AppId,
    area: Rect,
    top: Option<f64>,
    hop: f64,
    angle: f64,
    out: &mut Output,
) {
    match ctx.icon(&IconKey::App(app.clone())) {
        Some(image) => {
            let (w, h) = (image.width() as f64, image.height() as f64);
            let mut at = area.center(w, h);
            if let Some(top) = top {
                at.y = area.y + top;
            }
            at.y -= hop;
            out.draw(DrawCmd::Image {
                rect: at,
                image: image.clone(),
                alpha: 1.0,
                angle,
            });
        }
        None => out.draw(DrawCmd::Text {
            rect: Rect::new(area.x, area.y - hop, area.width, area.height),
            text: initial(app),
            style: ctx.theme.text.clone(),
            color: ctx.theme.foreground,
            align: Align::Center,
        }),
    }
}

/// The theme's face at three quarters of its size, for two lines in the height of one.
fn small_text(ctx: &Ctx<'_>) -> TextStyle {
    TextStyle::new(ctx.theme.text.font.clone(), ctx.theme.text.size * 0.75)
}

fn window_count(n: usize) -> String {
    if n == 1 {
        "1 window".to_string()
    } else {
        format!("{n} windows")
    }
}

/// Separators apps put between a title and their own name.
const TITLE_SEPARATORS: [&str; 3] = [" - ", " — ", " – "];

/// When every title ends with the same " - Name" (Chrome's " - Google Chrome", Konsole's
/// " — Konsole"), that ending says nothing about which window is which: drop it from each, and
/// hand it back as the app's name. Titles that would become empty keep their ending.
fn strip_common_suffix(titles: &[&str]) -> (Option<String>, Vec<String>) {
    let cut = |t: &str| -> Option<(usize, String)> {
        TITLE_SEPARATORS
            .iter()
            .filter_map(|sep| t.rfind(sep).map(|i| (i, t[i + sep.len()..].to_string())))
            .max_by_key(|(i, _)| *i)
    };
    let first = titles.first().and_then(|t| cut(t));
    let shared = first.filter(|(_, name)| {
        !name.trim().is_empty()
            && titles.len() >= 2
            && titles
                .iter()
                .all(|t| cut(t).is_some_and(|(i, n)| &n == name && !t[..i].trim().is_empty()))
    });
    match shared {
        Some((_, name)) => {
            let stripped = titles
                .iter()
                .map(|t| t[..cut(t).map_or(t.len(), |(i, _)| i)].to_string())
                .collect();
            (Some(name), stripped)
        }
        None => (None, titles.iter().map(|t| t.to_string()).collect()),
    }
}

/// `text`, shortened with "…" in the middle until it fits `width` at `style`. The start of a
/// title is usually the page or document; the end is usually where it lives; the middle is
/// the part worth losing. Cut to [`MAX_TITLE_CHARS`] first, so the search stays short.
fn elide_middle(style: &TextStyle, text: &str, width: f64) -> String {
    let text = truncate(text, MAX_TITLE_CHARS);
    if measure_text(style, &text).advance <= width {
        return text;
    }
    let chars: Vec<char> = text.chars().collect();
    let shorten = |keep: usize| -> String {
        let head = keep.div_ceil(2);
        let tail = keep / 2;
        let mut s: String = chars[..head].iter().collect();
        s.push('…');
        s.extend(&chars[chars.len() - tail..]);
        s
    };
    // The longest `keep` that fits; text width grows with `keep`, so bisect.
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if measure_text(style, &shorten(mid)).advance <= width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    shorten(lo)
}

/// How far a launching app's icon is above its resting place, `age` seconds after the launch
/// began: a run of hops, each touching the floor.
fn bounce_lift(age: f64) -> f64 {
    if !age.is_finite() || age < 0.0 {
        return 0.0;
    }
    let phase = (age % BOUNCE_PERIOD) / BOUNCE_PERIOD;
    BOUNCE_HEIGHT * (phase * std::f64::consts::PI).sin()
}

/// One uppercase letter standing in for a missing icon: the last dotted segment's first
/// letter, so `org.kde.dolphin` is `D`.
fn initial(app: &AppId) -> String {
    app.as_str()
        .rsplit('.')
        .next()
        .and_then(|s| s.chars().next())
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".to_string())
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Theme, UiState};
    use listello_protocol::WindowId;
    use listello_surface::{Color, TextStyle};
    use std::rc::Rc;

    fn win(id: u64, app: &str, active: bool) -> WindowInfo {
        WindowInfo {
            id: WindowId(id),
            app_id: AppId::new(app),
            title: format!("{app} window {id}"),
            active,
            minimized: false,
        }
    }

    fn ids(groups: &[AppGroup<'_>]) -> Vec<String> {
        groups
            .iter()
            .map(|g| g.app_id.as_str().to_string())
            .collect()
    }

    #[test]
    fn pinned_apps_come_first_in_config_order_then_running_ones_by_first_window() {
        let pinned = [AppId::new("files"), AppId::new("browser")];
        let windows = [
            win(1, "terminal", false),
            win(2, "browser", true),
            win(3, "editor", false),
            win(4, "terminal", false),
        ];
        let groups = group(&pinned, &windows);
        assert_eq!(ids(&groups), ["files", "browser", "terminal", "editor"]);
        assert!(groups[0].pinned && groups[1].pinned);
        assert!(!groups[2].pinned);
        assert!(groups[0].windows.is_empty());
        assert_eq!(groups[2].windows.len(), 2);
    }

    #[test]
    fn focus_changes_do_not_reorder_the_dock() {
        let pinned = [];
        let a = [win(1, "a", true), win(2, "b", false)];
        let b = [win(1, "a", false), win(2, "b", true)];
        assert_eq!(ids(&group(&pinned, &a)), ids(&group(&pinned, &b)));
    }

    #[test]
    fn a_duplicate_pin_is_one_tile() {
        let pinned = [AppId::new("x"), AppId::new("x")];
        assert_eq!(group(&pinned, &[]).len(), 1);
    }

    #[test]
    fn the_tap_depends_on_how_many_windows_there_are() {
        let pinned = [AppId::new("idle")];
        let windows = [
            win(1, "one", false),
            win(2, "two", false),
            win(3, "two", false),
        ];
        let groups = group(&pinned, &windows);
        assert_eq!(groups[0].tap(), Action::Launch(AppId::new("idle")));
        assert_eq!(groups[1].tap(), Action::Activate(WindowId(1)));
        assert_eq!(groups[2].tap(), Action::ToggleApp(AppId::new("two")));
    }

    #[test]
    fn the_bounce_starts_and_ends_each_hop_on_the_floor_and_peaks_between() {
        assert!(bounce_lift(0.0).abs() < 1e-9);
        assert!((bounce_lift(BOUNCE_PERIOD / 2.0) - BOUNCE_HEIGHT).abs() < 1e-9);
        assert!(bounce_lift(BOUNCE_PERIOD).abs() < 1e-9);
        assert!((bounce_lift(BOUNCE_PERIOD * 3.5) - BOUNCE_HEIGHT).abs() < 1e-9);
        assert_eq!(bounce_lift(f64::NAN), 0.0);
        assert_eq!(bounce_lift(-1.0), 0.0);
    }

    #[test]
    fn titles_are_cut_and_the_cut_is_marked() {
        assert_eq!(truncate("short", 10), "short");
        let long = "x".repeat(1_000_000);
        let cut = truncate(&long, 20);
        assert_eq!(cut.chars().count(), 20);
        assert!(cut.ends_with('…'));
        // Multi-byte characters are counted as characters, never split.
        assert_eq!(truncate("ééééé", 3), "éé…");
    }

    #[test]
    fn a_missing_icon_is_stood_in_for_by_a_letter() {
        assert_eq!(initial(&AppId::new("org.kde.dolphin")), "D");
        assert_eq!(initial(&AppId::new("firefox")), "F");
        assert_eq!(initial(&AppId::new("")), "?");
    }

    fn theme() -> Option<Theme> {
        let font = cairo::FontFace::toy_create(
            "sans-serif",
            cairo::FontSlant::Normal,
            cairo::FontWeight::Normal,
        )
        .ok()?;
        Some(Theme {
            text: TextStyle::new(Rc::new(font), 14.0),
            background: Color::BLACK,
            foreground: Color::WHITE,
            button: Color::grey(0.2),
            button_pressed: Color::grey(0.4),
            accent: Color::grey(0.8),
            corner_radius: 4.0,
        })
    }

    fn state(windows: Vec<WindowInfo>) -> UiState {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-22T09:00:00+00:00").unwrap();
        let mut s = UiState::new(now);
        s.agent_connected = true;
        s.windows = windows;
        s
    }

    const RECT: Rect = Rect::new(100.0, 0.0, 600.0, 60.0);

    fn render(dock: &DockWidget, state: &UiState) -> Option<Output> {
        let theme = theme()?;
        let mut out = Output::new();
        dock.render(
            &Ctx {
                state,
                theme: &theme,
                index: 2,
            },
            RECT,
            &mut out,
        );
        Some(out)
    }

    fn dock(pinned: &[&str]) -> DockWidget {
        DockWidget {
            pinned: pinned.iter().map(|p| AppId::new(*p)).collect(),
            sizing: Sizing::Flex(1.0),
        }
    }

    #[test]
    fn with_no_agent_it_draws_nothing_at_all() {
        let mut s = state(vec![win(1, "a", false)]);
        s.agent_connected = false;
        let Some(out) = render(&dock(&["pinned"]), &s) else {
            return;
        };
        assert!(out.commands().is_empty());
        assert!(out.hits().is_empty());
    }

    #[test]
    fn each_app_is_a_tile_with_its_action_and_the_background_is_pressable_last() {
        let s = state(vec![win(1, "a", false), win(2, "b", true)]);
        let Some(out) = render(&dock(&[]), &s) else {
            return;
        };
        let hits = out.hits();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].action, Action::Activate(WindowId(1)));
        assert_eq!(hits[1].action, Action::Activate(WindowId(2)));
        assert_eq!(hits[2].id.part, BACKGROUND_PART);
        assert_eq!(hits[2].rect, RECT);
        assert_eq!(hits[0].id.widget, 2);
        // Everything is drawn inside one clip.
        assert!(matches!(out.commands().first(), Some(DrawCmd::PushClip(r)) if *r == RECT));
        assert!(matches!(out.commands().last(), Some(DrawCmd::PopClip)));
    }

    #[test]
    fn a_running_app_gets_a_dot_and_an_idle_pinned_one_does_not() {
        let s = state(vec![win(1, "run", false)]);
        let Some(out) = render(&dock(&["idle"]), &s) else {
            return;
        };
        let dots = out
            .commands()
            .iter()
            .filter(|c| matches!(c, DrawCmd::RoundedRect { rect, .. } if rect.width == 5.0))
            .count();
        assert_eq!(dots, 1);
    }

    #[test]
    fn tapping_a_multi_window_app_expands_to_labelled_entries_with_a_way_back() {
        let mut s = state(vec![
            win(1, "ed", false),
            win(2, "ed", true),
            win(3, "other", false),
        ]);
        s.expanded = Some(AppId::new("ed"));
        let Some(out) = render(&dock(&[]), &s) else {
            return;
        };
        let actions: Vec<_> = out.hits().iter().map(|h| h.action.clone()).collect();
        assert_eq!(
            actions,
            [
                Action::ToggleApp(AppId::new("ed")),
                Action::Activate(WindowId(1)),
                Action::Activate(WindowId(2)),
                Action::None,
            ]
        );
        let titles: Vec<_> = out
            .commands()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Text { text, .. } if text.contains("window") => Some(text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(titles, ["2 windows", "ed window 1", "ed window 2"]);
        // The icon is drawn once, on the back tile, not again on every entry.
        let icons = out
            .commands()
            .iter()
            .filter(|c| matches!(c, DrawCmd::Text { text, .. } if text == "E"))
            .count();
        assert_eq!(icons, 1);
    }

    #[test]
    fn the_app_name_every_title_ends_with_is_dropped_and_becomes_the_header() {
        let (name, titles) =
            strip_common_suffix(&["Inbox - Gmail - Google Chrome", "listello - Google Chrome"]);
        assert_eq!(name.as_deref(), Some("Google Chrome"));
        assert_eq!(titles, ["Inbox - Gmail", "listello"]);
        // Konsole's em dash counts too.
        let (name, titles) = strip_common_suffix(&["~ : fish — Konsole", "top — Konsole"]);
        assert_eq!(name.as_deref(), Some("Konsole"));
        assert_eq!(titles, ["~ : fish", "top"]);
    }

    #[test]
    fn titles_that_do_not_share_an_ending_are_left_alone() {
        for titles in [
            &["a - X", "b - Y"][..],
            &["a - X", "plain"][..],
            // Stripping would leave a title with nothing in it.
            &["a - X", " - X"][..],
            // One window says nothing about what is common.
            &["a - X"][..],
        ] {
            let (name, kept) = strip_common_suffix(titles);
            assert_eq!(name, None, "{titles:?}");
            assert_eq!(kept, titles, "{titles:?}");
        }
    }

    #[test]
    fn a_title_too_wide_loses_its_middle_and_keeps_both_ends() {
        let Some(theme) = theme() else {
            return;
        };
        let style = &theme.text;
        let title = "Inbox (3) - someone@example.com - Gmail";
        assert_eq!(elide_middle(style, title, 10_000.0), title);
        let width = measure_text(style, title).advance / 2.0;
        let short = elide_middle(style, title, width);
        assert!(measure_text(style, &short).advance <= width);
        assert!(
            short.starts_with("Inbox") && short.ends_with("Gmail"),
            "{short}"
        );
        assert!(short.contains('…'));
        // No room at all still yields something drawable, never a panic.
        assert_eq!(elide_middle(style, title, 0.0), "…");
    }

    #[test]
    fn an_expanded_app_that_lost_its_windows_shows_the_normal_row() {
        let mut s = state(vec![win(1, "ed", false)]);
        s.expanded = Some(AppId::new("ed"));
        let Some(out) = render(&dock(&[]), &s) else {
            return;
        };
        assert_eq!(out.hits()[0].action, Action::Activate(WindowId(1)));
    }

    #[test]
    fn scrolling_shifts_tiles_and_hit_regions_never_leave_the_dock() {
        let windows: Vec<_> = (0..12).map(|i| win(i, &format!("app{i}"), false)).collect();
        let mut s = state(windows);
        s.scroll.insert(2, 200.0);
        let Some(out) = render(&dock(&[]), &s) else {
            return;
        };
        for hit in out.hits() {
            assert!(hit.rect.left() >= RECT.left() - 1e-9, "{:?}", hit.rect);
            assert!(hit.rect.right() <= RECT.right() + 1e-9, "{:?}", hit.rect);
        }
        // The first tile (x = 0) is scrolled out; the first pressable tile is a later app.
        assert_ne!(out.hits()[0].action, Action::Activate(WindowId(0)));
    }

    #[test]
    fn scrolling_past_the_end_stops_at_the_last_tile() {
        let windows: Vec<_> = (0..12).map(|i| win(i, &format!("app{i}"), false)).collect();
        let mut s = state(windows);
        s.scroll.insert(2, 1.0e9);
        let Some(out) = render(&dock(&[]), &s) else {
            return;
        };
        let last = out
            .hits()
            .iter()
            .rev()
            .nth(1)
            .expect("the last tile is before the background region");
        assert_eq!(last.action, Action::Activate(WindowId(11)));
        assert!((last.rect.right() - RECT.right()).abs() < 1e-9);
    }

    #[test]
    fn a_hostile_title_is_cut_before_drawing() {
        let mut w = win(1, "ed", false);
        w.title = "y".repeat(5_000_000);
        let mut s = state(vec![w, win(2, "ed", false)]);
        s.expanded = Some(AppId::new("ed"));
        let Some(out) = render(&dock(&[]), &s) else {
            return;
        };
        for c in out.commands() {
            if let DrawCmd::Text { text, .. } = c {
                assert!(text.chars().count() <= MAX_TITLE_CHARS);
            }
        }
    }
}
