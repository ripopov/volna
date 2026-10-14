//! Logical-pixel geometry and input types shared by every frontend.

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[must_use]
pub const fn point(x: f32, y: f32) -> Point {
    Point { x, y }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

#[must_use]
pub const fn size(width: f32, height: f32) -> Size {
    Size { width, height }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub origin: Point,
    pub size: Size,
}

impl Rect {
    #[must_use]
    pub const fn new(origin: Point, size: Size) -> Rect {
        Rect { origin, size }
    }

    #[must_use]
    pub const fn from_xywh(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect {
            origin: Point { x, y },
            size: Size { width, height },
        }
    }

    #[must_use]
    pub fn left(&self) -> f32 {
        self.origin.x
    }

    #[must_use]
    pub fn top(&self) -> f32 {
        self.origin.y
    }

    #[must_use]
    pub fn right(&self) -> f32 {
        self.origin.x + self.size.width
    }

    #[must_use]
    pub fn bottom(&self) -> f32 {
        self.origin.y + self.size.height
    }

    #[must_use]
    pub fn width(&self) -> f32 {
        self.size.width
    }

    #[must_use]
    pub fn height(&self) -> f32 {
        self.size.height
    }

    #[must_use]
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.origin.x && p.x < self.right() && p.y >= self.origin.y && p.y < self.bottom()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.size.width <= 0.0 || self.size.height <= 0.0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Keyboard modifiers are independent flags and may be held in any combination."
)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub platform: bool,
}

impl Modifiers {
    #[must_use]
    pub fn secondary(&self) -> bool {
        if cfg!(target_os = "macos") {
            self.platform
        } else {
            self.control
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorIcon {
    Default,
    PointingHand,
    ResizeLeftRight,
    ResizeUpDown,
    Grabbing,
}

#[must_use]
pub fn snap(v: f32) -> f32 {
    v.round()
}
