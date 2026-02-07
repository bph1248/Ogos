use ogos_core::*;

use serde::*;
use std::fmt::{self, *};
use windows::Win32::Foundation::*;

#[derive(Clone, Copy, Default)]
pub struct AnchorAbsolute {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32
}
impl AnchorAbsolute {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }
}
impl Display for AnchorAbsolute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{{}, {}, {}, {}}}{{{}, {}}}", self.left, self.top, self.right, self.bottom, self.width(), self.height())
    }
}
impl From<RECT> for AnchorAbsolute {
    fn from(value: RECT) -> Self {
        Self {
            left: value.left,
            top: value.top,
            right: value.right,
            bottom: value.bottom
        }
    }
}
impl Into<RECT> for AnchorAbsolute {
    fn into(self) -> RECT {
        RECT {
            left: self.left,
            top: self.top,
            right: self.right,
            bottom: self.bottom
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
pub struct AnchorRelative {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32
}
impl AnchorRelative {
    pub fn into_abs(self, screen_extent: Extent2d) -> AnchorAbsolute {
        AnchorAbsolute {
            left: self.left,
            top: self.top,
            right: screen_extent.width + self.right,
            bottom: screen_extent.height + self.bottom
        }
    }
}
