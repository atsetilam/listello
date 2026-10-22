// New in M1 step 3.
//
// tiny-dfr's loop wakes only on input or a 10 s timer and has nothing that can drive an
// animation (RESEARCH §2.1). `FrameClock` is the fix: something that wants motion calls
// `set_animating(true)` each time it recomputes whether it is still moving, and the loop starts
// waking at ~30 fps instead of idling until the next input or timeout. Nothing here draws or
// advances animation state itself — that is `Event::Frame` and its `elapsed_secs`, so a
// widget's state moves by measured time rather than by counting frames that may not have been
// delivered.
//
// The first real caller is pixel shift's sub-pixel interpolation (`PixelShiftManager::
// is_shifting`), which is what proves the clock wired in rather than merely declared. The Dock's
// scroll (M4) and the launch bounce (M6) are the callers this was actually built for.

use std::time::Instant;

/// ~30 fps. Fast enough for a scroll fling or the bounce; no widget planned needs more, and a
/// tighter interval just burns cycles on a laptop's Touch Bar.
const FRAME_INTERVAL_MS: u64 = 33;

pub struct FrameClock {
    last_tick: Instant,
    animating: bool,
}

impl FrameClock {
    pub fn new() -> FrameClock {
        FrameClock {
            last_tick: Instant::now(),
            animating: false,
        }
    }

    /// Level-triggered, not edge-triggered: callers pass their current state every time they
    /// recompute it, rather than pairing one `true` with one later `false`. A single global
    /// clock has exactly one thing it can be told, so there is nothing a request-counter would
    /// buy — if that stops being true once two widgets animate at once, this is where to add it.
    pub fn set_animating(&mut self, animating: bool) {
        // Time spent idle is not animation time: without this the first tick after a long idle
        // would report minutes elapsed and an animation would jump to its end.
        if animating && !self.animating {
            self.last_tick = Instant::now();
        }
        self.animating = animating;
    }

    pub fn animating(&self) -> bool {
        self.animating
    }

    /// How soon the clock wants the next wake, or `None` when nothing is animating and the loop
    /// is free to idle for as long as its other sources allow.
    pub fn next_timeout_ms(&self) -> Option<u64> {
        self.animating().then_some(FRAME_INTERVAL_MS)
    }

    /// Call after every wake. Fires once at least `FRAME_INTERVAL_MS` has passed since the last
    /// tick and the clock is still wanted, returning the elapsed time in seconds for
    /// `Event::Frame::elapsed_secs`.
    pub fn maybe_tick(&mut self) -> Option<f64> {
        if !self.animating() {
            return None;
        }
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_tick);
        if elapsed.as_millis() as u64 >= FRAME_INTERVAL_MS {
            self.last_tick = now;
            Some(elapsed.as_secs_f64())
        } else {
            None
        }
    }
}

impl Default for FrameClock {
    fn default() -> FrameClock {
        FrameClock::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn idle_clock_asks_for_nothing() {
        let clock = FrameClock::new();
        assert!(!clock.animating());
        assert_eq!(clock.next_timeout_ms(), None);
    }

    #[test]
    fn set_animating_toggles_the_timeout() {
        let mut clock = FrameClock::new();
        clock.set_animating(true);
        assert!(clock.animating());
        assert_eq!(clock.next_timeout_ms(), Some(FRAME_INTERVAL_MS));
        clock.set_animating(false);
        assert!(!clock.animating());
        assert_eq!(clock.next_timeout_ms(), None);
    }

    #[test]
    fn idle_time_is_not_counted_when_animation_starts() {
        let mut clock = FrameClock::new();
        sleep(Duration::from_millis(FRAME_INTERVAL_MS * 3));
        clock.set_animating(true);
        sleep(Duration::from_millis(FRAME_INTERVAL_MS + 5));
        let elapsed = clock.maybe_tick().expect("interval has passed");
        assert!(elapsed < 0.1, "counted the idle time: {elapsed}");
    }

    #[test]
    fn tick_waits_for_the_interval_then_fires() {
        let mut clock = FrameClock::new();
        clock.set_animating(true);
        assert_eq!(clock.maybe_tick(), None, "too soon since construction");
        sleep(Duration::from_millis(FRAME_INTERVAL_MS + 5));
        let elapsed = clock.maybe_tick().expect("interval has passed");
        assert!(elapsed > 0.0);
    }
}
