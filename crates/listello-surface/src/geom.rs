//! Geometry, in logical coordinates.
//!
//! "Logical" means: landscape, origin top-left, x to the right, y down, one unit per panel
//! pixel. That is the coordinate system layout and widgets live in, and the only one they
//! ever see. Whether the hardware underneath is portrait and rotated is the backend's
//! business (see [`crate::Rotation`]).

/// A size in logical pixels.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub const fn new(width: u32, height: u32) -> Size {
        Size { width, height }
    }

    /// The whole surface as a rectangle, which is what most callers actually want.
    pub fn to_rect(self) -> Rect {
        Rect::new(0.0, 0.0, self.width as f64, self.height as f64)
    }
}

/// A point in logical pixels.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Point {
        Point { x, y }
    }
}

/// An axis-aligned rectangle in logical pixels.
///
/// Sub-pixel coordinates are allowed and expected — layout divides a strip into N slots and
/// the boundaries rarely land on integers. Damage rectangles are snapped outward to whole
/// pixels by the backend, never by the caller.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    pub fn left(&self) -> f64 {
        self.x
    }
    pub fn top(&self) -> f64 {
        self.y
    }
    pub fn right(&self) -> f64 {
        self.x + self.width
    }
    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }
    pub fn center_x(&self) -> f64 {
        self.x + self.width / 2.0
    }
    pub fn center_y(&self) -> f64 {
        self.y + self.height / 2.0
    }

    pub fn is_empty(&self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.left() && p.x < self.right() && p.y >= self.top() && p.y < self.bottom()
    }

    /// Shrink on every side. Negative values grow it. Never returns a negative extent.
    pub fn inset(&self, by: f64) -> Rect {
        Rect::new(
            self.x + by,
            self.y + by,
            (self.width - 2.0 * by).max(0.0),
            (self.height - 2.0 * by).max(0.0),
        )
    }

    /// Grow on every side, clamping nothing. Used to turn a drawn region into a damage
    /// region, where erring outward is free and erring inward leaves stale pixels.
    pub fn outset(&self, by: f64) -> Rect {
        Rect::new(
            self.x - by,
            self.y - by,
            self.width + 2.0 * by,
            self.height + 2.0 * by,
        )
    }

    /// A rect of `size`, centred inside `self`. The overwhelmingly common icon placement.
    pub fn center(&self, width: f64, height: f64) -> Rect {
        Rect::new(
            (self.center_x() - width / 2.0).round(),
            (self.center_y() - height / 2.0).round(),
            width,
            height,
        )
    }

    /// The smallest rect containing both. An empty operand is ignored.
    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let x = self.left().min(other.left());
        let y = self.top().min(other.top());
        Rect::new(
            x,
            y,
            self.right().max(other.right()) - x,
            self.bottom().max(other.bottom()) - y,
        )
    }

    /// Snap outward to whole pixels. Backends use this before handing damage to hardware:
    /// a damage rect that is a fraction of a pixel too small leaves a visible seam.
    pub fn snap_out(&self) -> Rect {
        let x = self.x.floor();
        let y = self.y.floor();
        Rect::new(x, y, self.right().ceil() - x, self.bottom().ceil() - y)
    }

    /// Clamp into `bounds`. Returns an empty rect if they do not overlap.
    pub fn clamp_to(&self, bounds: &Rect) -> Rect {
        let x = self.left().max(bounds.left());
        let y = self.top().max(bounds.top());
        let right = self.right().min(bounds.right());
        let bottom = self.bottom().min(bounds.bottom());
        if right <= x || bottom <= y {
            Rect::default()
        } else {
            Rect::new(x, y, right - x, bottom - y)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_out_covers_the_whole_partial_pixel() {
        let r = Rect::new(1.25, 2.75, 3.5, 0.1).snap_out();
        assert_eq!(r, Rect::new(1.0, 2.0, 4.0, 1.0));
    }

    #[test]
    fn union_ignores_empty() {
        let a = Rect::new(10.0, 10.0, 5.0, 5.0);
        assert_eq!(a.union(&Rect::default()), a);
        assert_eq!(Rect::default().union(&a), a);
    }

    #[test]
    fn clamp_to_disjoint_is_empty() {
        let a = Rect::new(0.0, 0.0, 5.0, 5.0);
        let b = Rect::new(100.0, 100.0, 5.0, 5.0);
        assert!(a.clamp_to(&b).is_empty());
    }

    #[test]
    fn contains_is_half_open() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(r.contains(Point::new(0.0, 0.0)));
        assert!(!r.contains(Point::new(10.0, 5.0)));
    }
}
