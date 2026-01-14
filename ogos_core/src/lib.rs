use std::{
    fmt::{self, *},
    ops::*,
    process::*
};
use windows::Win32::Foundation::*;

#[derive(Debug)]
pub enum DisplayWrap<'a, T> {
    Borrowed(&'a T),
    Owned(T)
}
impl<T> Deref for DisplayWrap<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Borrowed(t) => t,
            Self::Owned(t) => t
        }
    }
}
impl Display for DisplayWrap<'_, Command> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_command(self, f)
    }
}
impl Display for DisplayWrap<'_, &mut Command> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_command(self, f)
    }
}
impl Display for DisplayWrap<'_, HWND> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:p}", self)
    }
}
impl Display for DisplayWrap<'_, RECT> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{{}, {}, {}, {}}}{{{}, {}}}", self.left, self.top, self.right, self.bottom, self.width(), self.height())
    }
}
fn fmt_command(cmd: &Command, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let program = cmd.get_program().display();

    write!(f, "\"{}\"", program)?;
    for arg in cmd.get_args() {
        write!(f, " \"{}\"", arg.display())?;
    }

    Ok(())
}

pub trait AsDisplay {
    fn as_display(&self) -> DisplayWrap<'_, Self> where Self: Sized;
}
impl<T> AsDisplay for T {
    fn as_display(&self) -> DisplayWrap<'_, Self> {
        DisplayWrap::Borrowed(self)
    }
}

pub trait IntoDisplay {
    fn into_display(self) -> DisplayWrap<'static, Self> where Self: Sized;
}
impl<T> IntoDisplay for T {
    fn into_display(self) -> DisplayWrap<'static, Self> {
        DisplayWrap::Owned(self)
    }
}

pub trait RectExt {
    fn height(&self) -> i32;
    fn width(&self) -> i32;
}
impl RectExt for RECT {
    fn height(&self) -> i32 {
        self.bottom - self.top
    }

    fn width(&self) -> i32 {
        self.right - self.left
    }
}
