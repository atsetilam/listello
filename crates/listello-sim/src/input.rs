//! winit window events → the loop's own vocabulary.
//!
//! A mouse is not a finger, and this module is the whole of the pretence that it is. Nothing
//! downstream can tell the difference: `listello-core` builds the same `Touch` values from
//! libinput, which is what makes a click in the simulator evidence about a tap on the panel —
//! the input-side counterpart of what `Surface` does for pixels (D10).
//!
//! Coordinates arrive in **physical** window pixels and leave in **logical** panel pixels.
//! `Window::inner_size()` is physical too, so the ratio is the whole mapping and
//! `scale_factor()` never appears (see `main.rs`).
//!
//! # Deliberate omissions
//!
//! - **`MouseWheel` is ignored.** The Touch Bar has no scroll wheel: the Dock scrolls by
//!   *dragging*, which the `Motion` row already covers. Mapping a wheel would give the Dock a
//!   code path that exists only in the simulator. M4 revisits it when there is a Dock.
//! - **The Fn key is not wired.** `InputEvent::FnKey` exists (D6) but nothing in M1 draws an
//!   Fn layer, and a mapping with no consumer is untestable. M3.

use listello_protocol::{Event, InputEvent, Touch, TouchPhase};
use listello_surface::{Point, Size};
use winit::event::{ElementState, MouseButton, WindowEvent};

/// Translates winit window events into [`Event`]s. Holds the last cursor position because
/// winit reports movement and buttons separately while `Touch` carries both.
#[derive(Default)]
pub struct InputTranslator {
    /// Last cursor position, in logical panel pixels. Recorded on every `CursorMoved`, even
    /// when nothing is emitted, because the position that moved silently past while the button
    /// was up is the position the next `Pressed` reports.
    cursor: Point,
    /// Whether the left button is currently down.
    down: bool,
}

impl InputTranslator {
    /// `window` is the window's size in physical pixels and `panel` the logical panel size.
    /// Returns `None` for events the simulator does not model.
    pub fn translate(&mut self, ev: &WindowEvent, window: Size, panel: Size) -> Option<Event> {
        match ev {
            WindowEvent::CloseRequested => Some(Event::Shutdown),
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = to_panel(position.x, position.y, window, panel);
                self.down
                    .then(|| touch_event(self.cursor, TouchPhase::Motion))
            }
            WindowEvent::MouseInput { state, button, .. } => match (state, button, self.down) {
                (ElementState::Pressed, MouseButton::Left, _) => {
                    self.down = true;
                    Some(touch_event(self.cursor, TouchPhase::Down))
                }
                (ElementState::Released, MouseButton::Left, true) => {
                    self.down = false;
                    Some(touch_event(self.cursor, TouchPhase::Up))
                }
                // Left, released, nothing to release. Any other button: not a finger.
                _ => None,
            },
            WindowEvent::CursorLeft { .. } if self.down => {
                self.down = false;
                Some(touch_event(self.cursor, TouchPhase::Cancel))
            }
            _ => None,
        }
    }
}

/// A `Touch` event at `cursor`. `slot` is always 0: the panel is multi-touch and a mouse is
/// not, and inventing slots would fabricate test coverage the simulator cannot honestly give.
fn touch_event(cursor: Point, phase: TouchPhase) -> Event {
    Event::Input(InputEvent::Touch(Touch {
        slot: 0,
        phase,
        x: cursor.x,
        y: cursor.y,
    }))
}

/// Maps a physical window coordinate onto the logical panel, clamped inside it.
///
/// Separate from [`InputTranslator::translate`] because it is the part that is wrong on a
/// Retina display if it is wrong at all, and it is testable with no window and no winit event.
fn to_panel(x: f64, y: f64, window: Size, panel: Size) -> Point {
    let map = |v: f64, win: u32, pan: u32| -> f64 {
        if win == 0 {
            return 0.0;
        }
        (v * pan as f64 / win as f64).clamp(0.0, pan as f64)
    };
    Point::new(
        map(x, window.width, panel.width),
        map(y, window.height, panel.height),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANEL: Size = Size::new(2008, 60);
    const WINDOW: Size = Size::new(2008, 60);

    fn moved(x: f64, y: f64) -> WindowEvent {
        WindowEvent::CursorMoved {
            device_id: winit::event::DeviceId::dummy(),
            position: winit::dpi::PhysicalPosition::new(x, y),
        }
    }

    fn pressed(button: MouseButton) -> WindowEvent {
        WindowEvent::MouseInput {
            device_id: winit::event::DeviceId::dummy(),
            state: ElementState::Pressed,
            button,
        }
    }

    fn released(button: MouseButton) -> WindowEvent {
        WindowEvent::MouseInput {
            device_id: winit::event::DeviceId::dummy(),
            state: ElementState::Released,
            button,
        }
    }

    fn left() -> WindowEvent {
        WindowEvent::CursorLeft {
            device_id: winit::event::DeviceId::dummy(),
        }
    }

    fn touch(phase: TouchPhase, x: f64, y: f64) -> Event {
        Event::Input(InputEvent::Touch(Touch {
            slot: 0,
            phase,
            x,
            y,
        }))
    }

    fn translate(t: &mut InputTranslator, ev: &WindowEvent) -> Option<Event> {
        t.translate(ev, WINDOW, PANEL)
    }

    #[test]
    fn close_requested_is_a_shutdown() {
        let mut t = InputTranslator::default();
        assert_eq!(
            t.translate(
                &WindowEvent::CloseRequested,
                Size::new(2008, 60),
                Size::new(2008, 60)
            ),
            Some(Event::Shutdown)
        );
    }

    #[test]
    fn a_retina_window_maps_onto_the_same_panel_point() {
        let panel = Size::new(2008, 60);
        assert_eq!(
            to_panel(1004.0, 30.0, Size::new(2008, 60), panel),
            Point::new(1004.0, 30.0)
        );
        assert_eq!(
            to_panel(2008.0, 60.0, Size::new(4016, 120), panel),
            Point::new(1004.0, 30.0)
        );
    }

    #[test]
    fn a_zero_sized_window_yields_a_point_inside_the_panel_rather_than_nan() {
        let p = to_panel(10.0, 10.0, Size::new(0, 0), Size::new(2008, 60));
        assert!(p.x.is_finite() && p.y.is_finite());
        assert_eq!(p, Point::new(0.0, 0.0));
    }

    #[test]
    fn press_after_move_yields_down_at_the_moved_point() {
        let mut t = InputTranslator::default();
        assert_eq!(translate(&mut t, &moved(1004.0, 30.0)), None);
        assert_eq!(
            translate(&mut t, &pressed(MouseButton::Left)),
            Some(touch(TouchPhase::Down, 1004.0, 30.0))
        );
    }

    #[test]
    fn a_retina_window_maps_presses_onto_the_same_panel_point() {
        let mut t = InputTranslator::default();
        let event = moved(2008.0, 60.0);
        assert_eq!(t.translate(&event, Size::new(4016, 120), PANEL), None);
        let event = pressed(MouseButton::Left);
        assert_eq!(
            t.translate(&event, Size::new(4016, 120), PANEL),
            Some(touch(TouchPhase::Down, 1004.0, 30.0))
        );
    }

    #[test]
    fn a_silent_move_is_recorded_for_the_following_press() {
        let mut t = InputTranslator::default();
        assert_eq!(translate(&mut t, &moved(500.0, 12.0)), None);
        assert_eq!(
            translate(&mut t, &pressed(MouseButton::Left)),
            Some(touch(TouchPhase::Down, 500.0, 12.0))
        );
    }

    #[test]
    fn a_move_while_down_yields_motion_at_the_new_point() {
        let mut t = InputTranslator::default();
        assert_eq!(translate(&mut t, &moved(1004.0, 30.0)), None);
        assert_eq!(
            translate(&mut t, &pressed(MouseButton::Left)),
            Some(touch(TouchPhase::Down, 1004.0, 30.0))
        );
        assert_eq!(
            translate(&mut t, &moved(700.0, 40.0)),
            Some(touch(TouchPhase::Motion, 700.0, 40.0))
        );
    }

    #[test]
    fn a_release_without_a_press_yields_nothing() {
        let mut t = InputTranslator::default();
        assert_eq!(translate(&mut t, &released(MouseButton::Left)), None);
    }

    #[test]
    fn release_after_press_yields_up_then_nothing() {
        let mut t = InputTranslator::default();
        assert_eq!(translate(&mut t, &moved(1004.0, 30.0)), None);
        assert_eq!(
            translate(&mut t, &pressed(MouseButton::Left)),
            Some(touch(TouchPhase::Down, 1004.0, 30.0))
        );
        assert_eq!(
            translate(&mut t, &released(MouseButton::Left)),
            Some(touch(TouchPhase::Up, 1004.0, 30.0))
        );
        assert_eq!(translate(&mut t, &released(MouseButton::Left)), None);
    }

    #[test]
    fn leaving_while_down_cancels_and_release_afterwards_is_silent() {
        let mut t = InputTranslator::default();
        assert_eq!(translate(&mut t, &moved(1004.0, 30.0)), None);
        assert_eq!(
            translate(&mut t, &pressed(MouseButton::Left)),
            Some(touch(TouchPhase::Down, 1004.0, 30.0))
        );
        assert_eq!(
            translate(&mut t, &left()),
            Some(touch(TouchPhase::Cancel, 1004.0, 30.0))
        );
        assert_eq!(translate(&mut t, &released(MouseButton::Left)), None);

        let mut t = InputTranslator::default();
        assert_eq!(translate(&mut t, &left()), None);
    }

    #[test]
    fn a_right_button_does_not_set_down() {
        let mut t = InputTranslator::default();
        assert_eq!(translate(&mut t, &moved(1004.0, 30.0)), None);
        assert_eq!(translate(&mut t, &pressed(MouseButton::Right)), None);
        assert_eq!(translate(&mut t, &released(MouseButton::Left)), None);
    }
}
