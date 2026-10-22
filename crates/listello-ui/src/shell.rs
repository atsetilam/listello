//! The surface-agnostic body of the loop (D3): protocol events and a widget list in, a `Frame`
//! and completed-press `Action`s out.
//!
//! `listello-sim` and `listello-core` both wrap this the same way — winit events or libinput
//! events go in, a `Frame` comes out — and neither binary has to know it isn't the other one.
//! `Shell` decides *what* a press asks for; it never decides how to do it. Emitting uinput
//! keys, running D11's one `SystemAction`, activating a window: all of that stays in the
//! binary, because a widget only ever describes what it wants (D3) and `Shell` inherits that
//! rule rather than lifting it.
//!
//! `Shell` never reads a clock. `now` arrives from the binary — on construction and on every
//! `set_now` call — which is what `state.rs` means by "the shell resolves the offset once":
//! once, outside this crate, never in here.

use crate::config::{clock_locale, ClockAlign, UiConfig, WidgetConfig, WindowPreviews};
use crate::dock::DockWidget;
use crate::hit::{Action, KeyName, RegionId};
use crate::layout::{render_strip, strip_layout, MAX_WIDGETS};
use crate::state::{Theme, UiState, WindowControls};
use crate::widget::{Ctx, Output, Widget};
use crate::widgets::{
    BatteryWidget, ButtonWidget, ClockWidget, ControlButton, ControlCenterWidget, FolderEntry,
    FoldersWidget, NetworkWidget, SeparatorWidget,
};
use chrono::{DateTime, FixedOffset};
use listello_protocol::{
    AgentToCore, AppId, CoreToAgent, InputEvent, SystemAction, Touch, TouchPhase, WindowId,
};
use listello_surface::{Align, DrawCmd, Frame, Point, Rect, Size};

/// Turns one config'd strip into a running strip: owns the widgets, the theme, and the bit of
/// state a widget cannot hold itself (which region is pressed, whether Fn is held, the time).
pub struct Shell {
    widgets: Vec<Box<dyn Widget>>,
    /// Shown instead of `widgets` while Fn is held. Empty means Fn does not change the strip.
    fn_widgets: Vec<Box<dyn Widget>>,
    theme: Theme,
    gap: f64,
    state: UiState,
    /// The finger holding `state.pressed`: its slot and where it last was. libinput's `Up`
    /// carries no position, so the release is judged here, not by the `Up` event.
    touch: Option<(u32, Point)>,
    /// The keys `Action::KeysDown` pressed and `Action::KeysUp` has not yet released.
    held_keys: Option<Vec<KeyName>>,
    /// A finger down on a scrollable widget: which widget, where the finger last was, and
    /// whether it has moved far enough to count as a drag rather than a tap.
    drag: Option<Drag>,
    /// A finger resting on something a long press means something for: how long it has rested,
    /// and on what. Ticked by [`Shell::advance`].
    hold: Option<Hold>,
    /// From config: seconds to a long press, and seconds the window controls stay up.
    hold_seconds: f64,
    controls_seconds: f64,
    /// The windows the agent was last asked to keep previews of. Empty: none, nothing captured.
    preview_watch: Vec<WindowId>,
    /// `[strip] window_previews = "live"`: ask the agent to stream rather than snap.
    live_previews: bool,
    /// Whether `preview_watch` was last asked for live, so a reload that only switches the
    /// mode still reaches the agent.
    watch_live: bool,
}

struct Hold {
    held: f64,
    region: RegionId,
    action: Action,
}

/// How far a finger must travel before a press on a scrollable widget becomes a scroll.
const DRAG_THRESHOLD: f64 = 10.0;

struct Drag {
    widget: u16,
    start_x: f64,
    last_x: f64,
    dragging: bool,
}

impl Shell {
    /// Builds one widget per `config.widgets` entry, in order — that order is what `Ctx::index`
    /// and therefore every `RegionId` this shell ever produces is stable against.
    pub fn new(config: &UiConfig, theme: Theme, now: DateTime<FixedOffset>) -> Shell {
        let mut state = UiState::new(now);
        state.window_previews = (config.strip.window_previews != WindowPreviews::Off)
            .then_some(config.strip.preview_style);
        Shell {
            widgets: config.widgets.iter().map(build_widget).collect(),
            fn_widgets: config.fn_widgets.iter().map(build_widget).collect(),
            theme,
            gap: config.strip.gap,
            state,
            touch: None,
            held_keys: None,
            drag: None,
            hold: None,
            hold_seconds: config.strip.hold_seconds,
            controls_seconds: config.strip.controls_seconds,
            preview_watch: Vec::new(),
            live_previews: config.strip.window_previews == WindowPreviews::Live,
            watch_live: false,
        }
    }

    /// Release whatever is still held down. The binary calls this before it drops the shell
    /// (config reload) so a replaced strip cannot leave a key stuck down — and repeating.
    pub fn release_held(&mut self) -> Vec<Action> {
        self.state.pressed = None;
        self.touch = None;
        self.drag = None;
        self.hold = None;
        self.held_keys
            .take()
            .map(Action::KeysUp)
            .into_iter()
            .collect()
    }

    /// The binary calls this when it observes `Event::TimeChanged` (or once per tick, if it
    /// chooses to): `listello-ui` has no clock of its own to consult (D3).
    pub fn set_now(&mut self, now: DateTime<FixedOffset>) {
        self.state.now = now;
    }

    /// The snapshot widgets will see on the next frame. The binary fills the packaged icon set
    /// through this before the first frame; nothing else should need it.
    pub fn state_mut(&mut self) -> &mut UiState {
        &mut self.state
    }

    pub fn state(&self) -> &UiState {
        &self.state
    }

    /// One `AgentToCore` message becomes state. **The only place that happens.**
    ///
    /// Every payload-bearing variant is a complete replacement, never a delta — the protocol
    /// says so, because the agent can die and reconnect at any moment and an incremental
    /// protocol needs a resync path that is only exercised on the day it is already broken. So
    /// this function assigns; it never merges, appends or diffs.
    ///
    /// Messages the renderer cannot use are dropped rather than escalated: an invalid `Icon` is
    /// simply not stored (the widget falls back to its label), and `Hello` and `Limited` change
    /// no widget-visible state — the socket owner handles the version check and the one-off log
    /// respectively.
    pub fn apply_agent(&mut self, msg: &AgentToCore) {
        match msg {
            AgentToCore::Windows(windows) => {
                self.state.windows = windows.clone();
                // The app has a window, so the launch is nearly over: let it land.
                for (app, launch) in self.state.launching.iter_mut() {
                    if windows.iter().any(|w| &w.app_id == app) {
                        launch.finish();
                    }
                }
                // Controls for a window that has gone are controls for nothing.
                if let Some(window) = self.state.controls.as_ref().and_then(|c| c.window) {
                    if !windows.iter().any(|w| w.id == window) {
                        self.state.controls = None;
                    }
                }
                // An expansion only makes sense while its app still has several windows.
                if let Some(app) = &self.state.expanded {
                    if windows.iter().filter(|w| &w.app_id == app).count() < 2 {
                        self.state.expanded = None;
                    }
                }
            }
            AgentToCore::Pins(pins) => self.state.pins = pins.clone(),
            AgentToCore::Battery(battery) => self.state.battery = *battery,
            AgentToCore::Network(network) => self.state.network = network.clone(),
            AgentToCore::Folder(folder) => {
                self.state
                    .folders
                    .insert(folder.key.clone(), folder.clone());
            }
            // A `None` icon is "the agent looked and found nothing" — a normal answer that gets
            // the fallback glyph, and explicitly not something to retry in a loop (protocol doc
            // comment). An invalid one is dropped by `insert_app` and looks the same downstream.
            AgentToCore::Icon {
                app_id,
                icon: Some(icon),
            } => {
                self.state.icons.insert_app(app_id.clone(), icon);
            }
            AgentToCore::Icon { icon: None, .. } => {}
            AgentToCore::LaunchStarted { app_id } => {
                // An app that already has a window is not launching, whatever the agent says.
                if !self.state.windows.iter().any(|w| &w.app_id == app_id) {
                    self.state
                        .launching
                        .insert(app_id.clone(), crate::state::Launch::started());
                }
            }
            // Kept only for a window the open row is showing: one that arrives after the row
            // closed would otherwise linger until the agent goes.
            AgentToCore::Preview { id, image } => {
                let image = image
                    .as_ref()
                    .filter(|i| i.is_valid() && self.preview_watch.contains(id))
                    .and_then(|i| listello_surface::image_from_rgba(i.width, i.height, &i.rgba));
                match image {
                    Some(image) => self.state.previews.insert(*id, image),
                    None => self.state.previews.remove(id),
                };
            }
            // The layout is the binary's to swap: it rebuilds the whole shell (`listello-core`).
            // A restart is the binary's too.
            AgentToCore::Hello { .. }
            | AgentToCore::Limited(_)
            | AgentToCore::UserConfig { .. }
            | AgentToCore::Restart
            | AgentToCore::Quit => {}
        }
    }

    /// What the agent should be capturing for the window row, when that has changed since the
    /// last call: the open row's windows, or nothing. The binary sends it as is. Always `None`
    /// with previews off, so nothing is ever captured (D7).
    pub fn preview_request(&mut self, panel: Size) -> Option<CoreToAgent> {
        let wanted: Vec<WindowId> = match &self.state.expanded {
            Some(app) if self.state.window_previews.is_some() && self.state.agent_connected => {
                let ids: Vec<WindowId> = self
                    .state
                    .windows
                    .iter()
                    .filter(|w| &w.app_id == app)
                    .map(|w| w.id)
                    .collect();
                if ids.len() >= 2 {
                    ids
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        };
        let live = self.live_previews && !wanted.is_empty();
        if wanted == self.preview_watch && live == self.watch_live {
            return None;
        }
        self.watch_live = live;
        self.state.previews.retain(|id, _| wanted.contains(id));
        self.preview_watch = wanted.clone();
        let style = self.state.window_previews.unwrap_or_default();
        let (max_width_px, height_px) = crate::dock::preview_size(panel.height as f64, style);
        Some(CoreToAgent::WatchPreviews {
            windows: wanted,
            height_px,
            max_width_px,
            fit: crate::dock::preview_fit(style),
            live,
        })
    }

    /// Move the launch bounce on by `elapsed_secs` of measured time, dropping launches that have
    /// run out. Returns whether anything is still moving, i.e. whether the next frame is wanted.
    pub fn advance(&mut self, elapsed_secs: f64) -> bool {
        if elapsed_secs.is_finite() && elapsed_secs > 0.0 {
            for launch in self.state.launching.values_mut() {
                launch.age += elapsed_secs;
            }
            self.state.launching.retain(|_, launch| !launch.is_over());
            if let Some(controls) = &mut self.state.controls {
                controls.age += elapsed_secs;
                if controls.age >= self.controls_seconds {
                    self.state.controls = None;
                }
            }
            if let Some(hold) = &mut self.hold {
                hold.held += elapsed_secs;
                if hold.held >= self.hold_seconds {
                    self.long_press();
                }
            }
        }
        self.is_animating()
    }

    /// Whether anything on the strip is moving and so needs a frame every tick. A finger
    /// resting where a long press means something counts: nothing else would wake the loop to
    /// notice that it has rested long enough.
    pub fn is_animating(&self) -> bool {
        !self.state.launching.is_empty() || self.hold.is_some() || self.state.controls.is_some()
    }

    /// A finger has rested long enough. On an app with one window, or a window in the
    /// expanded row, that window's controls come up; on an app with several windows, the row of
    /// its windows opens, as a tap would. Either way the finger's release then does nothing.
    fn long_press(&mut self) {
        let Some(hold) = self.hold.take() else {
            return;
        };
        self.state.pressed = None;
        self.drag = None;
        match hold.action {
            Action::Activate(window) => {
                if let Some(w) = self.state.windows.iter().find(|w| w.id == window) {
                    self.state.controls = Some(WindowControls {
                        app: w.app_id.clone(),
                        window: Some(window),
                        age: 0.0,
                    });
                }
            }
            // A pinned app that is not running: only its pin to offer.
            Action::Launch(app) => {
                self.state.controls = Some(WindowControls {
                    app,
                    window: None,
                    age: 0.0,
                });
            }
            toggle @ Action::ToggleApp(_) => {
                self.complete(hold.region, toggle);
            }
            _ => {}
        }
    }

    /// Every app the strip could draw an icon for: the apps with an open window, and the ones
    /// a widget wants shown regardless (the Dock's pinned list). The binary asks the agent for
    /// the ones it has no pixels for yet.
    pub fn wanted_app_ids(&self) -> Vec<AppId> {
        let mut seen = std::collections::HashSet::new();
        let open = self
            .state
            .windows
            .iter()
            .map(|w| w.app_id.clone())
            .chain(self.state.pins.iter().flatten().cloned());
        let wanted = self
            .widgets
            .iter()
            .chain(self.fn_widgets.iter())
            .flat_map(|w| w.wanted_apps());
        open.chain(wanted)
            .filter(|a| seen.insert(a.clone()))
            .collect()
    }

    /// Agent connected or gone. On disconnect every agent-sourced field is cleared, because a
    /// stale window list is worse than no window list — see [`UiState::clear_agent_state`].
    pub fn set_agent_connected(&mut self, connected: bool) {
        if connected {
            self.state.agent_connected = true;
        } else {
            self.state.clear_agent_state();
            // The agent stops capturing by itself when the renderer goes.
            self.preview_watch.clear();
            self.watch_live = false;
        }
    }

    /// Take over everything the agent told `old`, for a config reload: the new strip is a new
    /// `Shell`, but the agent is still connected and will not repeat what it already said, so
    /// without this the Dock, battery and folders would go blank until something changed.
    pub fn carry_over_agent_state(&mut self, old: &mut Shell) {
        let from = std::mem::take(&mut old.state.icons);
        self.state.icons = from;
        self.state.agent_connected = old.state.agent_connected;
        self.state.battery = old.state.battery;
        self.state.network = old.state.network.take();
        self.state.folders = std::mem::take(&mut old.state.folders);
        self.state.pins = old.state.pins.take();
        self.state.windows = std::mem::take(&mut old.state.windows);
        self.state.launching = std::mem::take(&mut old.state.launching);
        // The agent is still capturing for the old strip's row, which closes with it: keeping
        // what it was asked lets the next `preview_request` tell it to stop.
        self.preview_watch = std::mem::take(&mut old.preview_watch);
        self.watch_live = old.watch_live;
    }

    /// Move widget `index`'s horizontal scroll by `delta` logical pixels.
    ///
    /// Accumulated unclamped and never negative: the widget clamps against its own content
    /// width at draw time, because this struct does not know how wide anything is. A non-finite
    /// delta is ignored rather than poisoning the offset — it would reach a drawing coordinate.
    ///
    /// **Nothing calls this yet.** Driving it from `TouchPhase::Motion` needs a widget that
    /// declares itself scrollable, and the Dock is the first one; that wiring lands with it
    /// (plan-m5-widgets W7). The storage is here now so the two tracks agree on the shape.
    pub fn scroll_by(&mut self, index: u16, delta: f64) {
        if !delta.is_finite() {
            return;
        }
        let entry = self.state.scroll.entry(index).or_insert(0.0);
        *entry = (*entry + delta).max(0.0);
    }

    /// One input event in; the actions a *completed* press asks for out. Empty for everything
    /// except a `TouchPhase::Up` that lands back on the region it went down on.
    pub fn handle_input(&mut self, input: InputEvent, panel: Size) -> Vec<Action> {
        match input {
            InputEvent::Touch(t) => self.handle_touch(t, panel),
            InputEvent::FnKey { pressed } => {
                // Key repeat and a second Fn key both re-deliver the same state; only a real
                // change swaps layers.
                let swapped = self.state.fn_held != pressed && !self.fn_widgets.is_empty();
                self.state.fn_held = pressed;
                if swapped {
                    // A finger already down belongs to a region of the layer that just went
                    // away; letting its release land on the new layer would fire a stranger's
                    // action.
                    self.release_held()
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// The frame to present for the strip's current state. Always a full repaint (`Frame::full`)
    /// — M1 has no damage tracking yet, and correctness first is the point of this phase; a
    /// partial-damage `Shell` is a later, measured optimisation, not a default to assume now.
    ///
    /// Starts with `Clear(theme.background)`: the canvas is opaque and every frame has to say
    /// what shows through wherever no widget draws (`raster.rs`'s rule, not this crate's).
    pub fn frame(&self, panel: Size) -> Frame {
        let (mut commands, _hits) = self.layout(panel).into_parts();
        commands.insert(0, DrawCmd::Clear(self.theme.background));
        Frame::full(commands)
    }

    /// Where each widget of the strip now showing sits, in order — for a picture of the strip
    /// that can be pointed at (the settings window). Empty while one widget takes the whole
    /// strip.
    pub fn widget_rects(&self, panel: Size) -> Vec<Rect> {
        let refs: Vec<&dyn Widget> = self.active_widgets().iter().map(AsRef::as_ref).collect();
        if refs.iter().any(|w| w.takes_whole_strip(&self.state)) {
            return Vec::new();
        }
        let bounds = Rect::new(0.0, 0.0, panel.width as f64, panel.height as f64);
        strip_layout(&refs, &self.state, &self.theme, bounds, self.gap)
    }

    fn layout(&self, panel: Size) -> Output {
        let refs: Vec<&dyn Widget> = self.active_widgets().iter().map(AsRef::as_ref).collect();
        let bounds = Rect::new(0.0, 0.0, panel.width as f64, panel.height as f64);
        let whole = refs
            .iter()
            .enumerate()
            .take(MAX_WIDGETS)
            .find(|(_, w)| w.takes_whole_strip(&self.state));
        if let Some((i, w)) = whole {
            let ctx = Ctx {
                state: &self.state,
                theme: &self.theme,
                index: i as u16,
            };
            let mut out = Output::new();
            out.draw(DrawCmd::PushClip(bounds));
            w.render(&ctx, bounds, &mut out);
            out.draw(DrawCmd::PopClip);
            return out;
        }
        render_strip(&refs, &self.state, &self.theme, bounds, self.gap)
    }

    fn is_scrollable(&self, widget: u16) -> bool {
        self.active_widgets()
            .get(widget as usize)
            .is_some_and(|w| w.scrollable())
    }

    fn active_widgets(&self) -> &Vec<Box<dyn Widget>> {
        if self.state.fn_held && !self.fn_widgets.is_empty() {
            &self.fn_widgets
        } else {
            &self.widgets
        }
    }

    /// A press that landed: the actions it asks the binary for. Some actions are the shell's
    /// own business and never leave it.
    fn complete(&mut self, id: RegionId, action: Action) -> Vec<Action> {
        match action {
            Action::ToggleApp(app) => {
                self.state.expanded = if self.state.expanded.as_ref() == Some(&app) {
                    None
                } else {
                    Some(app)
                };
                // The row that replaces this one starts at its left edge.
                self.state.scroll.remove(&id.widget);
                Vec::new()
            }
            Action::Window(..) => {
                self.state.controls = None;
                vec![action]
            }
            Action::SetPins(ref pins) => {
                // Shown at once; the agent's answer confirms it, or puts back what it kept.
                self.state.pins = Some(pins.clone());
                self.state.controls = None;
                vec![action]
            }
            Action::Activate(_) => {
                // Focusing a window ends the expanded view that offered it.
                if self.state.expanded.take().is_some() {
                    self.state.scroll.remove(&id.widget);
                }
                vec![action]
            }
            other => vec![other],
        }
    }

    /// The topmost hit region under `point`, and the action it carries, if any.
    fn hit_at(&self, panel: Size, point: Point) -> Option<(RegionId, Action)> {
        self.layout(panel)
            .hits()
            .iter()
            .find(|h| h.rect.contains(point))
            .map(|h| (h.id, h.action.clone()))
    }

    /// Press-and-release-inside fires the action; anything else — release outside, or a
    /// `Cancel` — is silently dropped. A widget cannot tell `Shell` this itself (D3: pure
    /// function, no callback), so this is the one place that rule is spent.
    fn handle_touch(&mut self, t: Touch, panel: Size) -> Vec<Action> {
        let point = Point::new(t.x, t.y);
        match t.phase {
            TouchPhase::Down => {
                // A finger that never reported its `Up` must not leave its keys down under the
                // new one.
                let mut out = self.release_held();
                let hit = self.hit_at(panel, point);
                self.touch = Some((t.slot, point));
                // With window controls up, a touch anywhere but on one of them only puts them
                // away.
                if self.state.controls.is_some()
                    && !matches!(hit, Some((_, Action::Window(..) | Action::SetPins(_))))
                {
                    self.state.controls = None;
                    return out;
                }
                self.state.pressed = hit.as_ref().map(|(id, _)| *id);
                self.hold = hit
                    .as_ref()
                    .filter(|(_, a)| {
                        matches!(
                            a,
                            Action::Activate(_) | Action::ToggleApp(_) | Action::Launch(_)
                        )
                    })
                    .map(|(id, action)| Hold {
                        held: 0.0,
                        region: *id,
                        action: action.clone(),
                    });
                self.drag = hit
                    .as_ref()
                    .filter(|(id, _)| self.is_scrollable(id.widget))
                    .map(|(id, _)| Drag {
                        widget: id.widget,
                        start_x: point.x,
                        last_x: point.x,
                        dragging: false,
                    });
                // Keys go down on touch, not on release: a held volume or brightness key
                // repeats, as it did under tiny-dfr. Everything else fires on release.
                if let Some((_, Action::Keys(keys))) = hit {
                    self.held_keys = Some(keys.clone());
                    out.push(Action::KeysDown(keys));
                }
                out
            }
            TouchPhase::Motion => {
                let Some((slot, last)) = &mut self.touch else {
                    return Vec::new();
                };
                if *slot != t.slot {
                    return Vec::new();
                }
                *last = point;
                if let Some(drag) = &mut self.drag {
                    if !drag.dragging && (point.x - drag.start_x).abs() > DRAG_THRESHOLD {
                        // From here on this is a scroll, not a press: the tile lights down
                        // and nothing fires on release.
                        drag.dragging = true;
                        self.state.pressed = None;
                        self.hold = None;
                    }
                    if drag.dragging {
                        // Dragging left moves the content left, which is a larger offset.
                        let delta = drag.last_x - point.x;
                        drag.last_x = point.x;
                        let widget = drag.widget;
                        self.scroll_by(widget, delta);
                        return Vec::new();
                    }
                }
                // Sliding off the button lets go of its keys.
                let still_on = self.hit_at(panel, point).map(|(id, _)| id) == self.state.pressed;
                if !still_on {
                    self.hold = None;
                }
                if still_on {
                    Vec::new()
                } else {
                    self.held_keys
                        .take()
                        .map(Action::KeysUp)
                        .into_iter()
                        .collect()
                }
            }
            TouchPhase::Up => {
                let Some((slot, last)) = self.touch else {
                    return Vec::new();
                };
                if slot != t.slot {
                    return Vec::new();
                }
                self.touch = None;
                self.hold = None;
                if self.drag.take().is_some_and(|d| d.dragging) {
                    self.state.pressed = None;
                    return Vec::new();
                }
                let pressed = self.state.pressed.take();
                if let Some(keys) = self.held_keys.take() {
                    return vec![Action::KeysUp(keys)];
                }
                match (pressed, self.hit_at(panel, last)) {
                    // A `Keys` button that was slid off already had its keys released above.
                    (Some(_), Some((_, Action::Keys(_)))) => Vec::new(),
                    (Some(pressed_id), Some((id, action))) if pressed_id == id => {
                        self.complete(id, action)
                    }
                    _ => Vec::new(),
                }
            }
            TouchPhase::Cancel => self.release_held(),
        }
    }
}

/// One config entry to one boxed widget. Key names are re-validated with `KeyName::new` rather
/// than trusted from `UiConfig::parse`'s pass — cheap, and it means this function has no
/// `Result` to thread through `Shell::new` for a case `parse` already turned away in practice.
/// An unparseable name is dropped from the button's keys, not a panic: `deny_unknown_fields`
/// and `validate()` catch it long before this point in the normal path.
fn build_widget(cfg: &WidgetConfig) -> Box<dyn Widget> {
    let sizing = cfg.sizing();
    match cfg {
        WidgetConfig::Button {
            label, icon, keys, ..
        } => {
            let action = if keys.is_empty() {
                Action::None
            } else {
                Action::Keys(
                    keys.iter()
                        .filter_map(|k| KeyName::new(k.clone()))
                        .collect(),
                )
            };
            Box::new(ButtonWidget {
                label: label.clone(),
                icon: icon.clone(),
                action,
                sizing,
            }) as Box<dyn Widget>
        }
        WidgetConfig::Clock {
            format,
            align,
            locale,
            width,
            flex,
        } => Box::new(ClockWidget {
            fit: width.is_none() && flex.is_none(),
            format: format.clone(),
            // `validate` refused a name it did not know; English if one got here anyway.
            locale: locale
                .as_deref()
                .and_then(clock_locale)
                .unwrap_or(chrono::Locale::POSIX),
            align: match align {
                ClockAlign::Start => Align::Start,
                ClockAlign::Center => Align::Center,
                ClockAlign::End => Align::End,
            },
            sizing,
        }),
        WidgetConfig::Separator { .. } => Box::new(SeparatorWidget { sizing }),
        WidgetConfig::Dock { pinned, .. } => Box::new(DockWidget {
            pinned: pinned.iter().map(|p| AppId::new(p.trim())).collect(),
            sizing,
        }),
        WidgetConfig::Battery {
            show_percentage,
            show_bar,
            ..
        } => Box::new(BatteryWidget {
            show_percentage: *show_percentage,
            show_bar: *show_bar,
            sizing,
        }),
        WidgetConfig::Network { show_ssid, .. } => Box::new(NetworkWidget {
            show_ssid: *show_ssid,
            sizing,
        }),
        WidgetConfig::Folders { folders, .. } => Box::new(FoldersWidget {
            folders: folders
                .iter()
                .map(|f| FolderEntry {
                    key: f.key.clone(),
                    path: f.path.clone(),
                    label: f.label.clone(),
                    icon: f.icon.clone(),
                    empty_icon: f.empty_icon.clone(),
                })
                .collect(),
            sizing,
        }),
        WidgetConfig::ControlCenter { buttons, .. } => Box::new(ControlCenterWidget {
            buttons: buttons.iter().map(build_control_button).collect(),
            sizing,
        }),
    }
}

/// One config'd control-centre button to the widget layer's one. `keys` is re-validated with
/// `KeyName::new` and `system` with [`system_action`], each exactly as `UiConfig::parse` already
/// did — cheap, and it keeps this function total for the case `validate` already turned away in
/// practice: a button that somehow has neither collapses to `Action::None`, never a panic.
fn build_control_button(cfg: &crate::config::ControlButton) -> ControlButton {
    let action = if !cfg.keys.is_empty() {
        Action::Keys(
            cfg.keys
                .iter()
                .filter_map(|k| KeyName::new(k.clone()))
                .collect(),
        )
    } else {
        match cfg.system.as_deref().and_then(system_action) {
            Some(system) => Action::System(system),
            None => Action::None,
        }
    };
    ControlButton {
        icon: cfg.icon.clone(),
        label: cfg.label.clone(),
        action,
    }
}

/// The one system action M5 has, by its config name. `None` for a name `validate` would already
/// have rejected — this is the no-panic fallback for a config that somehow got here anyway.
fn system_action(name: &str) -> Option<SystemAction> {
    match name {
        "lock_session" => Some(SystemAction::LockSession),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairo::{FontFace, FontSlant, FontWeight};
    use listello_protocol::{
        AppId, BatteryState, FolderState, Limitation, NetworkState, WindowCommand, WindowId,
        WindowInfo,
    };
    use listello_surface::{Color, DrawCmd, TextStyle};
    use std::rc::Rc;

    const PANEL: Size = Size::new(300, 30);

    fn theme() -> Theme {
        let font = FontFace::toy_create("sans-serif", FontSlant::Normal, FontWeight::Normal)
            .expect("toy font");
        Theme {
            text: TextStyle::new(Rc::new(font), 14.0),
            background: Color::BLACK,
            foreground: Color::WHITE,
            button: Color::grey(0.3),
            button_pressed: Color::grey(0.5),
            accent: Color::grey(0.8),
            corner_radius: 4.0,
        }
    }

    fn now() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-08-04T09:00:00+02:00").expect("valid timestamp")
    }

    fn config(src: &str) -> UiConfig {
        UiConfig::parse(src).expect("valid config")
    }

    fn touch(phase: TouchPhase, x: f64, y: f64) -> InputEvent {
        InputEvent::Touch(Touch {
            slot: 0,
            phase,
            x,
            y,
        })
    }

    fn two_buttons() -> UiConfig {
        config(
            "[[widget]]\nkind = \"button\"\nlabel = \"A\"\nkeys = [\"KEY_A\"]\nwidth = 100.0\n\
             [[widget]]\nkind = \"button\"\nlabel = \"B\"\nkeys = [\"KEY_B\"]\nwidth = 100.0\n",
        )
    }

    fn media_by_default_f_keys_under_fn() -> UiConfig {
        config(
            "[[widget]]\nkind = \"button\"\nlabel = \"Mute\"\nkeys = [\"KEY_MUTE\"]\nwidth = 100.0\n\
             [[fn_widget]]\nkind = \"button\"\nlabel = \"F1\"\nkeys = [\"KEY_F1\"]\nwidth = 100.0\n",
        )
    }

    fn keys(name: &str) -> Vec<KeyName> {
        vec![KeyName::new(name).unwrap()]
    }

    fn down(name: &str) -> Vec<Action> {
        vec![Action::KeysDown(keys(name))]
    }

    fn up(name: &str) -> Vec<Action> {
        vec![Action::KeysUp(keys(name))]
    }

    /// Touch down then up at `x`; the actions of both, in order.
    fn tap(shell: &mut Shell, x: f64) -> Vec<Action> {
        let mut out = shell.handle_input(touch(TouchPhase::Down, x, 15.0), PANEL);
        out.extend(shell.handle_input(touch(TouchPhase::Up, x, 15.0), PANEL));
        out
    }

    #[test]
    fn holding_fn_swaps_in_the_fn_layer_and_releasing_swaps_back() {
        let mut shell = Shell::new(&media_by_default_f_keys_under_fn(), theme(), now());
        let both = |name: &str| [down(name), up(name)].concat();
        assert_eq!(tap(&mut shell, 10.0), both("KEY_MUTE"));
        shell.handle_input(InputEvent::FnKey { pressed: true }, PANEL);
        assert_eq!(tap(&mut shell, 10.0), both("KEY_F1"));
        shell.handle_input(InputEvent::FnKey { pressed: false }, PANEL);
        assert_eq!(tap(&mut shell, 10.0), both("KEY_MUTE"));
    }

    #[test]
    fn fn_does_nothing_without_an_fn_layer() {
        let mut shell = Shell::new(&two_buttons(), theme(), now());
        let before = shell.frame(PANEL).commands.len();
        shell.handle_input(InputEvent::FnKey { pressed: true }, PANEL);
        assert_eq!(shell.frame(PANEL).commands.len(), before);
        assert_eq!(tap(&mut shell, 10.0).len(), 2);
    }

    #[test]
    fn a_finger_down_when_the_layer_swaps_lets_go_of_its_key_and_fires_nothing_more() {
        let mut shell = Shell::new(&media_by_default_f_keys_under_fn(), theme(), now());
        assert_eq!(
            shell.handle_input(touch(TouchPhase::Down, 10.0, 15.0), PANEL),
            down("KEY_MUTE")
        );
        assert_eq!(
            shell.handle_input(InputEvent::FnKey { pressed: true }, PANEL),
            up("KEY_MUTE")
        );
        assert_eq!(
            shell.handle_input(touch(TouchPhase::Up, 10.0, 15.0), PANEL),
            Vec::new()
        );
    }

    #[test]
    fn frame_renders_one_command_per_widget_at_minimum() {
        let cfg = two_buttons();
        let shell = Shell::new(&cfg, theme(), now());
        let frame = shell.frame(PANEL);
        // Clear, then each button's RoundedRect plus a Text (non-empty label), each widget
        // inside its own PushClip/PopClip pair.
        let drawn = frame
            .commands
            .iter()
            .filter(|c| !matches!(c, DrawCmd::PushClip(_) | DrawCmd::PopClip))
            .count();
        assert_eq!(drawn, 5);
    }

    /// The key goes down when the finger lands and stays down while it is held — that is what
    /// makes a held volume key repeat — and comes up when the finger lifts.
    #[test]
    fn keys_go_down_on_touch_and_up_on_release() {
        let mut shell = Shell::new(&two_buttons(), theme(), now());
        assert_eq!(
            shell.handle_input(touch(TouchPhase::Down, 10.0, 15.0), PANEL),
            down("KEY_A")
        );
        assert_eq!(
            shell.handle_input(touch(TouchPhase::Motion, 12.0, 15.0), PANEL),
            Vec::new()
        );
        assert_eq!(
            shell.handle_input(touch(TouchPhase::Up, 12.0, 15.0), PANEL),
            up("KEY_A")
        );
    }

    #[test]
    fn sliding_off_the_button_lets_go_of_the_key_once() {
        let mut shell = Shell::new(&two_buttons(), theme(), now());
        shell.handle_input(touch(TouchPhase::Down, 10.0, 15.0), PANEL);
        // x = 190 is past the gap (button A ends at 100, button B starts at 108).
        assert_eq!(
            shell.handle_input(touch(TouchPhase::Motion, 190.0, 15.0), PANEL),
            up("KEY_A")
        );
        assert_eq!(
            shell.handle_input(touch(TouchPhase::Up, 0.0, 0.0), PANEL),
            Vec::new()
        );
    }

    /// libinput's `Up` carries no position, so the event loop sends `(0, 0)`. Which key goes up
    /// is whichever went down — judging by the `Up` event once made every button except the one
    /// at the left edge unpressable on real hardware.
    #[test]
    fn release_does_not_depend_on_where_the_up_event_says_it_was() {
        let mut shell = Shell::new(&two_buttons(), theme(), now());
        assert_eq!(
            shell.handle_input(touch(TouchPhase::Down, 150.0, 15.0), PANEL),
            down("KEY_B")
        );
        assert_eq!(
            shell.handle_input(touch(TouchPhase::Up, 0.0, 0.0), PANEL),
            up("KEY_B")
        );
    }

    #[test]
    fn up_from_another_finger_does_not_release_the_key() {
        let mut shell = Shell::new(&two_buttons(), theme(), now());
        shell.handle_input(touch(TouchPhase::Down, 150.0, 15.0), PANEL);
        let other = InputEvent::Touch(Touch {
            slot: 1,
            phase: TouchPhase::Up,
            x: 0.0,
            y: 0.0,
        });
        assert_eq!(shell.handle_input(other, PANEL), Vec::new());
        assert_eq!(
            shell.handle_input(touch(TouchPhase::Up, 0.0, 0.0), PANEL),
            up("KEY_B")
        );
    }

    #[test]
    fn cancel_releases_the_key_and_a_later_up_fires_nothing() {
        let mut shell = Shell::new(&two_buttons(), theme(), now());
        assert_eq!(
            shell.handle_input(touch(TouchPhase::Down, 10.0, 15.0), PANEL),
            down("KEY_A")
        );
        assert_eq!(
            shell.handle_input(touch(TouchPhase::Cancel, 10.0, 15.0), PANEL),
            up("KEY_A")
        );
        assert_eq!(
            shell.handle_input(touch(TouchPhase::Up, 10.0, 15.0), PANEL),
            Vec::new()
        );
    }

    #[test]
    fn a_new_finger_down_releases_the_key_the_lost_one_left_behind() {
        let mut shell = Shell::new(&two_buttons(), theme(), now());
        shell.handle_input(touch(TouchPhase::Down, 10.0, 15.0), PANEL);
        let actions = shell.handle_input(touch(TouchPhase::Down, 150.0, 15.0), PANEL);
        assert_eq!(actions, [up("KEY_A"), down("KEY_B")].concat());
    }

    #[test]
    fn release_held_lets_go_of_a_key_still_down() {
        let mut shell = Shell::new(&two_buttons(), theme(), now());
        shell.handle_input(touch(TouchPhase::Down, 10.0, 15.0), PANEL);
        assert_eq!(shell.release_held(), up("KEY_A"));
        assert_eq!(shell.release_held(), Vec::new());
    }

    #[test]
    fn pressed_state_reaches_the_button_as_a_different_colour() {
        let cfg = config("[[widget]]\nkind = \"button\"\nlabel = \"A\"\nwidth = 100.0\n");
        let mut shell = Shell::new(&cfg, theme(), now());
        shell.handle_input(touch(TouchPhase::Down, 10.0, 15.0), PANEL);
        let frame = shell.frame(PANEL);
        let pressed_color = theme().button_pressed;
        assert!(frame
            .commands
            .iter()
            .any(|c| matches!(c, DrawCmd::RoundedRect { color, .. } if *color == pressed_color)));
    }

    #[test]
    fn a_button_with_an_empty_label_still_fires_its_keys() {
        let cfg = config(
            "[[widget]]\nkind = \"button\"\nlabel = \"\"\nkeys = [\"KEY_A\"]\nwidth = 100.0\n",
        );
        let mut shell = Shell::new(&cfg, theme(), now());
        assert_eq!(tap(&mut shell, 10.0), [down("KEY_A"), up("KEY_A")].concat());
    }

    #[test]
    fn separator_never_swallows_a_touch() {
        let cfg = config(
            "[[widget]]\nkind = \"separator\"\nwidth = 40.0\n\
             [[widget]]\nkind = \"button\"\nlabel = \"A\"\nkeys = [\"KEY_A\"]\nflex = 1.0\n",
        );
        let mut shell = Shell::new(&cfg, theme(), now());
        // x = 20 lands in the middle of the separator's 40px slot.
        shell.handle_input(touch(TouchPhase::Down, 20.0, 15.0), PANEL);
        let actions = shell.handle_input(touch(TouchPhase::Up, 20.0, 15.0), PANEL);
        assert_eq!(actions, Vec::new());
    }

    #[test]
    fn agent_payloads_replace_rather_than_accumulate() {
        let cfg = two_buttons();
        let mut shell = Shell::new(&cfg, theme(), now());
        let window = |id: u64| WindowInfo {
            id: WindowId(id),
            app_id: AppId::new("org.kde.dolphin"),
            title: "Home".into(),
            active: false,
            minimized: false,
        };
        shell.apply_agent(&AgentToCore::Windows(vec![window(1), window(2)]));
        assert_eq!(shell.state().windows.len(), 2);
        // A second Windows message is the *complete* list, not an addition to the first.
        shell.apply_agent(&AgentToCore::Windows(vec![window(3)]));
        assert_eq!(shell.state().windows.len(), 1);
        assert_eq!(shell.state().windows[0].id, WindowId(3));
    }

    #[test]
    fn folders_accumulate_by_key_because_each_message_is_one_folder() {
        let cfg = two_buttons();
        let mut shell = Shell::new(&cfg, theme(), now());
        shell.apply_agent(&AgentToCore::Folder(FolderState {
            key: "trash".into(),
            empty: true,
        }));
        shell.apply_agent(&AgentToCore::Folder(FolderState {
            key: "downloads".into(),
            empty: false,
        }));
        shell.apply_agent(&AgentToCore::Folder(FolderState {
            key: "trash".into(),
            empty: false,
        }));
        assert_eq!(shell.state().folders.len(), 2);
        assert!(!shell.state().folders["trash"].empty);
    }

    #[test]
    fn disconnect_clears_everything_the_agent_supplied() {
        let cfg = two_buttons();
        let mut shell = Shell::new(&cfg, theme(), now());
        shell.set_agent_connected(true);
        shell.apply_agent(&AgentToCore::Battery(Some(BatteryState {
            charge: 0.4,
            charging: true,
            time_remaining_secs: Some(3600),
        })));
        shell.apply_agent(&AgentToCore::Network(Some(NetworkState {
            connected: true,
            signal: 0.8,
            ssid: Some("home".into()),
        })));
        assert!(shell.state().battery.is_some());

        shell.set_agent_connected(false);
        assert!(!shell.state().agent_connected);
        assert!(shell.state().battery.is_none());
        assert!(shell.state().network.is_none());
    }

    #[test]
    fn messages_the_renderer_cannot_use_change_nothing() {
        let cfg = two_buttons();
        let mut shell = Shell::new(&cfg, theme(), now());
        shell.apply_agent(&AgentToCore::Hello {
            protocol_version: listello_protocol::PROTOCOL_VERSION,
        });
        shell.apply_agent(&AgentToCore::Limited(Limitation::NoWindowManagement));
        // Not a panic, and not a state change either: the socket owner deals with both.
        assert!(shell.state().windows.is_empty());
        assert!(!shell.state().agent_connected);
    }

    #[test]
    fn an_icon_the_agent_could_not_find_is_not_an_error() {
        let cfg = two_buttons();
        let mut shell = Shell::new(&cfg, theme(), now());
        shell.apply_agent(&AgentToCore::Icon {
            app_id: AppId::new("org.kde.dolphin"),
            icon: None,
        });
        assert!(shell.state().icons.is_empty());
    }

    #[test]
    fn scroll_accumulates_and_never_goes_negative() {
        let cfg = two_buttons();
        let mut shell = Shell::new(&cfg, theme(), now());
        shell.scroll_by(0, 40.0);
        shell.scroll_by(0, 15.0);
        assert_eq!(shell.state().scroll.get(&0).copied(), Some(55.0));
        shell.scroll_by(0, -1000.0);
        assert_eq!(shell.state().scroll.get(&0).copied(), Some(0.0));
        // A NaN would reach a drawing coordinate; it is dropped, not accumulated.
        shell.scroll_by(0, f64::NAN);
        assert_eq!(shell.state().scroll.get(&0).copied(), Some(0.0));
    }

    #[test]
    fn set_now_changes_what_a_clock_widget_draws() {
        let cfg = config("[[widget]]\nkind = \"clock\"\nformat = \"%H:%M\"\nflex = 1.0\n");
        let mut shell = Shell::new(&cfg, theme(), now());
        let text = |cmds: &[DrawCmd]| -> String {
            cmds.iter()
                .find_map(|c| match c {
                    DrawCmd::Text { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .expect("clock widget draws a Text command")
        };
        let first = text(&shell.frame(PANEL).commands);
        let later = DateTime::parse_from_rfc3339("2026-08-04T10:30:00+02:00").unwrap();
        shell.set_now(later);
        let second = text(&shell.frame(PANEL).commands);
        assert_ne!(first, second);
    }

    // --- The Dock: scrolling by dragging, and expanding a multi-window app. ---

    use listello_protocol::{WindowId as WId, WindowInfo as WInfo};

    fn dock_config() -> UiConfig {
        config("[[widget]]\nkind = \"dock\"\npinned = [\"idle\"]\nwidth = 600.0\n")
    }

    fn win(id: u64, app: &str) -> WInfo {
        WInfo {
            id: WId(id),
            app_id: AppId::new(app),
            title: format!("{app} {id}"),
            active: false,
            minimized: false,
        }
    }

    fn dock_shell(windows: Vec<WInfo>) -> Shell {
        let mut shell = Shell::new(&dock_config(), theme(), now());
        shell.set_agent_connected(true);
        shell.apply_agent(&AgentToCore::Windows(windows));
        shell
    }

    /// Tile `n` (0-based) centre, with the 80-wide tiles and 12-wide gaps the Dock draws.
    fn tile_x(n: usize) -> f64 {
        n as f64 * 92.0 + 40.0
    }

    /// The centre of the region that would fire `action`, as the strip is laid out now.
    fn x_of(shell: &Shell, action: &Action) -> f64 {
        shell
            .layout(PANEL)
            .hits()
            .iter()
            .find(|h| &h.action == action)
            .map(|h| h.rect.center_x())
            .expect("a region with that action")
    }

    fn preview_shell(previews: &str) -> Shell {
        let cfg = config(&format!(
            "[strip]\nwindow_previews = \"{previews}\"\n\
             [[widget]]\nkind = \"dock\"\npinned = [\"idle\"]\nwidth = 600.0\n"
        ));
        let mut shell = Shell::new(&cfg, theme(), now());
        shell.set_agent_connected(true);
        shell.apply_agent(&AgentToCore::Windows(vec![
            win(1, "ed"),
            win(2, "ed"),
            win(3, "other"),
        ]));
        shell
    }

    fn picture() -> listello_protocol::Icon {
        listello_protocol::Icon::new(2, 1, vec![255; 8]).unwrap()
    }

    #[test]
    fn with_previews_off_the_agent_is_never_asked_to_capture() {
        let mut shell = preview_shell("off");
        shell.state.expanded = Some(AppId::new("ed"));
        assert_eq!(shell.preview_request(PANEL), None);
    }

    #[test]
    fn opening_the_row_asks_for_its_windows_and_closing_it_stops_the_capture() {
        let mut shell = preview_shell("snapshots");
        assert_eq!(shell.preview_request(PANEL), None);

        shell.state.expanded = Some(AppId::new("ed"));
        let (max_width_px, height_px) =
            crate::dock::preview_size(PANEL.height as f64, crate::config::PreviewStyle::Fill);
        assert_eq!(
            shell.preview_request(PANEL),
            Some(CoreToAgent::WatchPreviews {
                windows: vec![WId(1), WId(2)],
                height_px,
                max_width_px,
                fit: listello_protocol::PreviewFit::Crop,
                live: false,
            })
        );
        // Asked once, not on every turn of the loop.
        assert_eq!(shell.preview_request(PANEL), None);

        shell.apply_agent(&AgentToCore::Preview {
            id: WId(1),
            image: Some(picture()),
        });
        // A window the row does not show: not kept.
        shell.apply_agent(&AgentToCore::Preview {
            id: WId(3),
            image: Some(picture()),
        });
        assert_eq!(shell.state().previews.len(), 1);
        // `None` takes a picture away again.
        shell.apply_agent(&AgentToCore::Preview {
            id: WId(1),
            image: None,
        });
        assert!(shell.state().previews.is_empty());
        shell.apply_agent(&AgentToCore::Preview {
            id: WId(2),
            image: Some(picture()),
        });

        shell.state.expanded = None;
        assert!(matches!(
            shell.preview_request(PANEL),
            Some(CoreToAgent::WatchPreviews { windows, .. }) if windows.is_empty()
        ));
        assert!(shell.state().previews.is_empty());
    }

    #[test]
    fn live_previews_ask_the_agent_to_stream() {
        let mut shell = preview_shell("live");
        shell.state.expanded = Some(AppId::new("ed"));
        assert!(matches!(
            shell.preview_request(PANEL),
            Some(CoreToAgent::WatchPreviews { live: true, windows, .. }) if windows.len() == 2
        ));
    }

    #[test]
    fn a_reload_tells_the_agent_to_stop_capturing_for_the_old_row() {
        let mut old = preview_shell("snapshots");
        old.state.expanded = Some(AppId::new("ed"));
        assert!(old.preview_request(PANEL).is_some());
        let mut new = Shell::new(&dock_config(), theme(), now());
        new.carry_over_agent_state(&mut old);
        assert!(matches!(
            new.preview_request(PANEL),
            Some(CoreToAgent::WatchPreviews { windows, .. }) if windows.is_empty()
        ));
    }

    #[test]
    fn a_lost_agent_takes_the_previews_and_the_watch_with_it() {
        let mut shell = preview_shell("snapshots");
        shell.state.expanded = Some(AppId::new("ed"));
        shell.preview_request(PANEL);
        shell.apply_agent(&AgentToCore::Preview {
            id: WId(1),
            image: Some(picture()),
        });
        shell.set_agent_connected(false);
        assert!(shell.state().previews.is_empty());
        assert_eq!(shell.preview_request(PANEL), None);
    }

    #[test]
    fn a_launch_keeps_bouncing_after_its_window_appears_until_the_hop_lands() {
        let mut shell = dock_shell(vec![]);
        shell.apply_agent(&AgentToCore::LaunchStarted {
            app_id: AppId::new("idle"),
        });
        assert!(shell.is_animating());
        // Early in a hop: it simply finishes (0.2 s in, 0.5 s left).
        assert!(shell.advance(0.2));
        shell.apply_agent(&AgentToCore::Windows(vec![win(1, "idle")]));
        assert!(shell.is_animating(), "must not freeze in mid-air");
        assert!(shell.advance(0.4));
        assert!(!shell.advance(0.2));
    }

    #[test]
    fn a_window_arriving_at_the_end_of_a_hop_gets_one_more_full_hop() {
        let mut shell = dock_shell(vec![]);
        shell.apply_agent(&AgentToCore::LaunchStarted {
            app_id: AppId::new("idle"),
        });
        // 0.6 s in: only 0.1 s left, too short to see, so one whole extra hop follows.
        assert!(shell.advance(0.6));
        shell.apply_agent(&AgentToCore::Windows(vec![win(1, "idle")]));
        assert!(shell.advance(0.7));
        assert!(!shell.advance(0.2));
    }

    #[test]
    fn a_second_window_does_not_stretch_the_last_hop() {
        let mut launch = crate::state::Launch::started();
        launch.age = 0.2;
        launch.finish();
        let end = launch.ends_at;
        launch.age = 0.5;
        launch.finish();
        assert_eq!(launch.ends_at, end);
    }

    #[test]
    fn a_launch_that_never_opens_a_window_gives_up() {
        let mut shell = dock_shell(vec![]);
        shell.apply_agent(&AgentToCore::LaunchStarted {
            app_id: AppId::new("idle"),
        });
        assert!(shell.advance(crate::state::LAUNCH_TIMEOUT_SECS - 1.0));
        assert!(!shell.advance(2.0));
        assert!(shell.state().launching.is_empty());
    }

    #[test]
    fn an_app_that_already_has_a_window_does_not_bounce_and_a_dead_agent_stops_it() {
        let mut shell = dock_shell(vec![win(1, "solo")]);
        shell.apply_agent(&AgentToCore::LaunchStarted {
            app_id: AppId::new("solo"),
        });
        assert!(!shell.is_animating());
        shell.apply_agent(&AgentToCore::LaunchStarted {
            app_id: AppId::new("idle"),
        });
        shell.set_agent_connected(false);
        assert!(!shell.is_animating());
    }

    #[test]
    fn tapping_an_idle_pinned_app_launches_it_and_a_single_window_app_focuses_it() {
        let mut shell = dock_shell(vec![win(7, "solo")]);
        assert_eq!(
            tap(&mut shell, tile_x(0)),
            vec![Action::Launch(AppId::new("idle"))]
        );
        // The divider sits between pinned and running apps.
        assert_eq!(
            tap(&mut shell, tile_x(1) + 12.0),
            vec![Action::Activate(WId(7))]
        );
    }

    #[test]
    fn tapping_a_multi_window_app_opens_its_entries_without_asking_the_binary_for_anything() {
        let mut shell = dock_shell(vec![win(1, "ed"), win(2, "ed")]);
        assert!(tap(&mut shell, tile_x(1) + 12.0).is_empty());
        assert_eq!(shell.state().expanded, Some(AppId::new("ed")));
        // The test panel is narrow: the first entry is in view, the second scrolled off.
        let first = x_of(&shell, &Action::Activate(WId(1)));
        assert_eq!(tap(&mut shell, first), vec![Action::Activate(WId(1))]);
        // Focusing a window closes the expanded view.
        assert_eq!(shell.state().expanded, None);
    }

    #[test]
    fn an_app_s_windows_take_the_whole_strip_and_give_it_back() {
        let cfg = config(
            "[[widget]]
kind = \"dock\"
width = 100.0
[[widget]]
kind = \"button\"
label = \"Mute\"
keys = [\"KEY_MUTE\"]
",
        );
        let mut shell = Shell::new(&cfg, theme(), now());
        shell.set_agent_connected(true);
        shell.apply_agent(&AgentToCore::Windows(vec![win(1, "ed"), win(2, "ed")]));
        let mute = Action::Keys(vec![KeyName::new("KEY_MUTE").expect("a key name")]);
        let ids = |shell: &Shell| -> Vec<u16> {
            shell
                .layout(PANEL)
                .hits()
                .iter()
                .map(|h| h.id.widget)
                .collect()
        };
        assert!(ids(&shell).contains(&1));

        assert!(tap(&mut shell, tile_x(0)).is_empty());
        // Only the Dock is left, under its own index, and it spans the panel.
        let out = shell.layout(PANEL);
        assert!(out.hits().iter().all(|h| h.id.widget == 0));
        assert!(!out.hits().iter().any(|h| h.action == mute));
        let background = out.hits().last().expect("the dock's background");
        assert_eq!(background.rect, Rect::new(0.0, 0.0, 300.0, 30.0));

        // The back tile brings the other widgets back.
        assert!(tap(&mut shell, 20.0).is_empty());
        assert!(ids(&shell).contains(&1));
    }

    /// Press at `x` and hold it there for `secs` of frame time, without lifting.
    fn hold(shell: &mut Shell, x: f64, secs: f64) {
        shell.handle_input(touch(TouchPhase::Down, x, 15.0), PANEL);
        assert!(
            shell.is_animating(),
            "a held press must keep the clock ticking"
        );
        shell.advance(secs);
    }

    fn lift(shell: &mut Shell, x: f64) -> Vec<Action> {
        shell.handle_input(touch(TouchPhase::Up, x, 15.0), PANEL)
    }

    #[test]
    fn a_long_press_on_a_one_window_app_brings_up_its_controls_and_the_release_does_nothing() {
        let mut shell = dock_shell(vec![win(7, "solo")]);
        let x = tile_x(1) + 12.0;
        hold(&mut shell, x, 0.3);
        assert_eq!(shell.state().controls, None, "not yet: 0.3 s is a tap");
        shell.advance(0.3);
        assert_eq!(
            shell.state().controls.as_ref().and_then(|c| c.window),
            Some(WId(7))
        );
        assert!(
            lift(&mut shell, x).is_empty(),
            "the release must not focus the window"
        );

        // Once slid out, each light carries its command for that window.
        shell.advance(1.0);
        for command in [
            WindowCommand::Close,
            WindowCommand::Minimize,
            WindowCommand::ToggleMaximize,
        ] {
            assert!(shell
                .layout(PANEL)
                .hits()
                .iter()
                .any(|h| h.action == Action::Window(WId(7), command)));
        }
    }

    #[test]
    fn a_light_sends_its_command_and_puts_the_controls_away() {
        let mut shell = dock_shell(vec![win(7, "solo")]);
        hold(&mut shell, tile_x(1) + 12.0, 0.6);
        lift(&mut shell, tile_x(1) + 12.0);
        shell.advance(1.0);
        let close = Action::Window(WId(7), WindowCommand::Close);
        let x = x_of(&shell, &close);
        assert_eq!(tap(&mut shell, x), vec![close]);
        assert_eq!(shell.state().controls, None);
    }

    #[test]
    fn a_touch_elsewhere_only_puts_the_controls_away() {
        let mut shell = dock_shell(vec![win(7, "solo")]);
        hold(&mut shell, tile_x(1) + 12.0, 0.6);
        lift(&mut shell, tile_x(1) + 12.0);
        // The idle pinned app: a tap would launch it, but this one only dismisses.
        assert!(tap(&mut shell, tile_x(0)).is_empty());
        assert_eq!(shell.state().controls, None);
        assert_eq!(
            tap(&mut shell, tile_x(0)),
            vec![Action::Launch(AppId::new("idle"))]
        );
    }

    #[test]
    fn the_controls_go_away_by_themselves_and_both_times_come_from_config() {
        let cfg = config(
            "[strip]\nhold_seconds = 0.2\ncontrols_seconds = 2.0\n[[widget]]\nkind = \"dock\"\nwidth = 300.0\n",
        );
        let mut shell = Shell::new(&cfg, theme(), now());
        shell.set_agent_connected(true);
        shell.apply_agent(&AgentToCore::Windows(vec![win(7, "solo")]));
        hold(&mut shell, tile_x(0), 0.25);
        lift(&mut shell, tile_x(0));
        assert!(shell.state().controls.is_some());
        shell.advance(1.9);
        assert!(shell.state().controls.is_some());
        shell.advance(0.2);
        assert_eq!(shell.state().controls, None);
        assert!(!shell.is_animating());
    }

    #[test]
    fn a_long_press_on_a_many_window_app_opens_its_row_and_a_window_there_gets_controls() {
        let mut shell = dock_shell(vec![win(1, "ed"), win(2, "ed")]);
        hold(&mut shell, tile_x(1) + 12.0, 0.6);
        assert!(lift(&mut shell, tile_x(1) + 12.0).is_empty());
        assert_eq!(shell.state().expanded, Some(AppId::new("ed")));
        assert_eq!(shell.state().controls, None);

        let first = x_of(&shell, &Action::Activate(WId(1)));
        hold(&mut shell, first, 0.6);
        assert!(lift(&mut shell, first).is_empty());
        assert_eq!(
            shell.state().controls.as_ref().and_then(|c| c.window),
            Some(WId(1))
        );
    }

    #[test]
    fn a_drag_is_not_a_long_press_and_a_closed_window_takes_its_controls_with_it() {
        let windows: Vec<_> = (0..12).map(|i| win(i, &format!("app{i}"))).collect();
        let mut shell = dock_shell(windows);
        shell.handle_input(touch(TouchPhase::Down, tile_x(1), 15.0), PANEL);
        shell.handle_input(touch(TouchPhase::Motion, tile_x(1) - 40.0, 15.0), PANEL);
        shell.advance(1.0);
        assert_eq!(shell.state().controls, None);
        lift(&mut shell, tile_x(1) - 40.0);

        let mut shell = dock_shell(vec![win(7, "solo")]);
        hold(&mut shell, tile_x(1) + 12.0, 0.6);
        shell.apply_agent(&AgentToCore::Windows(vec![]));
        assert_eq!(shell.state().controls, None);
    }

    /// The pin's action as the strip is laid out now, whatever list it carries.
    fn pin_action(shell: &Shell) -> Action {
        shell
            .layout(PANEL)
            .hits()
            .iter()
            .find(|h| matches!(h.action, Action::SetPins(_)))
            .map(|h| h.action.clone())
            .expect("a pin")
    }

    #[test]
    fn pinning_a_running_app_adds_it_after_the_config_s_pins_and_asks_the_agent_to_keep_it() {
        let mut shell = dock_shell(vec![win(7, "solo")]);
        hold(&mut shell, tile_x(1) + 12.0, 0.6);
        lift(&mut shell, tile_x(1) + 12.0);
        shell.advance(1.0);
        let pin = pin_action(&shell);
        let want = Action::SetPins(vec![AppId::new("idle"), AppId::new("solo")]);
        assert_eq!(pin, want);
        let x = x_of(&shell, &pin);
        assert_eq!(tap(&mut shell, x), vec![want]);
        // Shown at once, before the agent answers, and the controls are put away.
        assert_eq!(
            shell.state().pins,
            Some(vec![AppId::new("idle"), AppId::new("solo")])
        );
        assert_eq!(shell.state().controls, None);
    }

    #[test]
    fn a_pinned_app_that_is_not_running_offers_only_its_pin_which_unpins_it() {
        let mut shell = dock_shell(vec![]);
        shell.apply_agent(&AgentToCore::Pins(Some(vec![
            AppId::new("kept"),
            AppId::new("idle"),
        ])));
        // The agent's list wins over the config's: "kept" is the first tile now.
        hold(&mut shell, tile_x(1), 0.6);
        assert!(
            lift(&mut shell, tile_x(1)).is_empty(),
            "the hold must not launch it"
        );
        shell.advance(1.0);
        let lights: Vec<Action> = shell
            .layout(PANEL)
            .hits()
            .iter()
            .filter(|h| matches!(h.action, Action::SetPins(_) | Action::Window(..)))
            .map(|h| h.action.clone())
            .collect();
        assert_eq!(lights, vec![Action::SetPins(vec![AppId::new("kept")])]);
    }

    #[test]
    fn the_agent_s_pins_are_wanted_for_icons_and_forgotten_with_the_agent() {
        let mut shell = dock_shell(vec![]);
        shell.apply_agent(&AgentToCore::Pins(Some(vec![AppId::new("kept")])));
        assert!(shell.wanted_app_ids().contains(&AppId::new("kept")));
        shell.set_agent_connected(false);
        assert_eq!(shell.state().pins, None);
    }

    #[test]
    fn controls_take_the_whole_strip_keep_the_held_icon_in_place_and_are_never_cut_off() {
        let cfg = config(
            "[[widget]]
kind = \"dock\"
pinned = [\"idle\"]
width = 180.0
[[widget]]
kind = \"button\"
label = \"Mute\"
keys = [\"KEY_MUTE\"]
",
        );
        let mut shell = Shell::new(&cfg, theme(), now());
        shell.set_agent_connected(true);
        shell.apply_agent(&AgentToCore::Windows(vec![win(7, "solo")]));
        let solo = |shell: &Shell| -> Rect {
            shell
                .layout(PANEL)
                .hits()
                .iter()
                .find(|h| h.action == Action::Activate(WId(7)))
                .map(|h| h.rect)
                .expect("the solo tile")
        };
        let before = solo(&shell);
        hold(&mut shell, before.center_x(), 0.6);
        lift(&mut shell, before.center_x());
        // Still sliding: the button is gone already and the held icon has not moved.
        let out = shell.layout(PANEL);
        assert!(out.hits().iter().all(|h| h.id.widget == 0));
        assert_eq!(solo(&shell), before);

        // Fully out, the lights would end past this narrow panel; the Dock shifts to show them.
        shell.advance(1.0);
        let lights: Vec<Rect> = shell
            .layout(PANEL)
            .hits()
            .iter()
            .filter(|h| matches!(h.action, Action::Window(..) | Action::SetPins(_)))
            .map(|h| h.rect)
            .collect();
        assert_eq!(lights.len(), 4);
        for light in lights {
            assert!(light.right() <= PANEL.width as f64 + 1e-9, "{light:?}");
        }

        // Put away, everything is back where it was.
        tap(&mut shell, 5.0);
        assert_eq!(shell.state().controls, None);
        assert!(shell.layout(PANEL).hits().iter().any(|h| h.id.widget == 1));
        assert_eq!(solo(&shell), before);
    }

    #[test]
    fn the_back_tile_collapses_the_expanded_row() {
        let mut shell = dock_shell(vec![win(1, "ed"), win(2, "ed")]);
        tap(&mut shell, tile_x(1) + 12.0);
        assert!(shell.state().expanded.is_some());
        assert!(tap(&mut shell, 32.0).is_empty());
        assert_eq!(shell.state().expanded, None);
    }

    #[test]
    fn an_expansion_ends_when_the_app_is_down_to_one_window() {
        let mut shell = dock_shell(vec![win(1, "ed"), win(2, "ed")]);
        tap(&mut shell, tile_x(1) + 12.0);
        shell.apply_agent(&AgentToCore::Windows(vec![win(1, "ed")]));
        assert_eq!(shell.state().expanded, None);
    }

    #[test]
    fn dragging_across_the_dock_scrolls_it_and_fires_nothing() {
        let windows: Vec<_> = (0..12).map(|i| win(i, &format!("app{i}"))).collect();
        let mut shell = dock_shell(windows);
        let mut actions = shell.handle_input(touch(TouchPhase::Down, 250.0, 15.0), PANEL);
        actions.extend(shell.handle_input(touch(TouchPhase::Motion, 200.0, 15.0), PANEL));
        actions.extend(shell.handle_input(touch(TouchPhase::Motion, 150.0, 15.0), PANEL));
        actions.extend(shell.handle_input(touch(TouchPhase::Up, 150.0, 15.0), PANEL));
        assert!(actions.is_empty());
        // The finger moved 100 to the left, so the content moved 100 to the right.
        assert_eq!(shell.state().scroll.get(&0).copied(), Some(100.0));
        assert_eq!(shell.state().pressed, None);
    }

    #[test]
    fn a_wobble_under_the_threshold_is_still_a_tap() {
        let mut shell = dock_shell(vec![win(7, "solo")]);
        let x = tile_x(1) + 12.0;
        let mut actions = shell.handle_input(touch(TouchPhase::Down, x, 15.0), PANEL);
        actions.extend(shell.handle_input(touch(TouchPhase::Motion, x + 4.0, 15.0), PANEL));
        actions.extend(shell.handle_input(touch(TouchPhase::Up, x + 4.0, 15.0), PANEL));
        assert_eq!(actions, vec![Action::Activate(WId(7))]);
    }

    #[test]
    fn pinned_apps_are_wanted_even_with_no_window_open() {
        let shell = dock_shell(vec![win(1, "ed")]);
        assert_eq!(
            shell.wanted_app_ids(),
            vec![AppId::new("ed"), AppId::new("idle")]
        );
    }

    #[test]
    fn losing_the_agent_closes_an_expanded_row() {
        let mut shell = dock_shell(vec![win(1, "ed"), win(2, "ed")]);
        tap(&mut shell, tile_x(1) + 12.0);
        shell.set_agent_connected(false);
        assert_eq!(shell.state().expanded, None);
        assert!(shell.state().windows.is_empty());
    }

    #[test]
    fn a_reload_keeps_what_the_agent_said() {
        let mut old = Shell::new(&config("[[widget]]\nkind = \"clock\"\n"), theme(), now());
        old.set_agent_connected(true);
        old.apply_agent(&AgentToCore::Windows(vec![WInfo {
            id: WId(1),
            app_id: AppId::new("ed"),
            title: "ed".into(),
            active: false,
            minimized: false,
        }]));
        old.apply_agent(&AgentToCore::Battery(Some(
            listello_protocol::BatteryState {
                charge: 0.5,
                charging: false,
                time_remaining_secs: None,
            },
        )));
        old.apply_agent(&AgentToCore::Folder(listello_protocol::FolderState {
            key: "t".into(),
            empty: true,
        }));
        old.apply_agent(&AgentToCore::Pins(Some(vec![AppId::new("ed")])));

        let mut new = Shell::new(&config("[[widget]]\nkind = \"clock\"\n"), theme(), now());
        new.carry_over_agent_state(&mut old);
        assert!(new.state().agent_connected);
        assert_eq!(new.state().pins, Some(vec![AppId::new("ed")]));
        assert_eq!(new.state().windows.len(), 1);
        assert!(new.state().battery.is_some());
        assert!(new.state().folders["t"].empty);
    }

    #[test]
    fn a_control_centre_button_press_fires_the_action_its_config_gave_it() {
        // Five key buttons and one lock button, packed into a 260px control centre widget.
        let cfg = config(
            "[[widget]]
kind = \"control_center\"
width = 260.0
[[widget.button]]
icon = \"lock\"
system = \"lock_session\"
[[widget.button]]
icon = \"brightness-down\"
keys = [\"KEY_BRIGHTNESSDOWN\"]
[[widget.button]]
icon = \"brightness-up\"
keys = [\"KEY_BRIGHTNESSUP\"]
[[widget.button]]
icon = \"mute\"
keys = [\"KEY_MUTE\"]
[[widget.button]]
icon = \"volume-down\"
keys = [\"KEY_VOLUMEDOWN\"]
[[widget.button]]
icon = \"volume-up\"
keys = [\"KEY_VOLUMEUP\"]
",
        );
        let mut shell = Shell::new(&cfg, theme(), now());

        // Six 40px buttons, five 4px gaps: button 0 occupies x 0..40, button 2 x 88..128.
        // Press and release inside button 2 (brightness up): the key goes down when the finger
        // lands and up when it lifts, like any other key button.
        let pressed = shell.handle_input(touch(TouchPhase::Down, 90.0, 15.0), PANEL);
        assert_eq!(pressed, down("KEY_BRIGHTNESSUP"));
        let released = shell.handle_input(touch(TouchPhase::Up, 90.0, 15.0), PANEL);
        assert_eq!(released, up("KEY_BRIGHTNESSUP"));

        // Button 0 is the lock button: a press there asks for the one SystemAction.
        shell.handle_input(touch(TouchPhase::Down, 10.0, 15.0), PANEL);
        let actions = shell.handle_input(touch(TouchPhase::Up, 10.0, 15.0), PANEL);
        assert_eq!(actions, vec![Action::System(SystemAction::LockSession)]);
    }

    #[test]
    fn a_control_centre_button_does_not_fire_inside_a_neighbour() {
        let cfg = config(
            "[[widget]]
kind = \"control_center\"
width = 260.0
[[widget.button]]
icon = \"lock\"
system = \"lock_session\"
[[widget.button]]
icon = \"mute\"
keys = [\"KEY_MUTE\"]
",
        );
        let mut shell = Shell::new(&cfg, theme(), now());
        // Two 130px halves of one panel, touching at x = 130: each side of the seam fires its
        // own button and only its own.
        shell.handle_input(touch(TouchPhase::Down, 125.0, 15.0), PANEL);
        let actions = shell.handle_input(touch(TouchPhase::Up, 125.0, 15.0), PANEL);
        assert_eq!(actions, vec![Action::System(SystemAction::LockSession)]);
        shell.handle_input(touch(TouchPhase::Down, 135.0, 15.0), PANEL);
        let actions = shell.handle_input(touch(TouchPhase::Up, 135.0, 15.0), PANEL);
        assert!(!actions.contains(&Action::System(SystemAction::LockSession)));
        assert!(!actions.is_empty());
    }
}
