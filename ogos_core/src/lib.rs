use std::{
    fmt::*,
    ops::*,
    process::*
};
use windows::Win32::Foundation::*;

#[derive(Debug)]
pub enum Displayer<'a, T> {
    Borrowed(&'a T),
    Owned(T)
}
impl<T> Deref for Displayer<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Borrowed(t) => t,
            Self::Owned(t) => t
        }
    }
}
impl Display for Displayer<'_, Command> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        fmt_command(self, f)
    }
}
impl Display for Displayer<'_, &mut Command> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        fmt_command(self, f)
    }
}
impl Display for Displayer<'_, HWND> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{:p}", self)
    }
}
impl Display for Displayer<'_, Option<&str>> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{}", self.as_ref().map_or("<None>", |s| s))
    }
}
impl Display for Displayer<'_, RECT> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{{{}, {}, {}, {}}}{{{}, {}}}", self.left, self.top, self.right, self.bottom, self.width(), self.height())
    }
}
fn fmt_command(cmd: &Command, f: &mut Formatter<'_>) -> Result {
    let program = cmd.get_program().display();

    write!(f, "\"{}\"", program)?;
    for arg in cmd.get_args() {
        write!(f, " \"{}\"", arg.display())?;
    }

    Ok(())
}

pub trait AsDisplay {
    fn as_display(&self) -> Displayer<'_, Self> where Self: Sized;
}
impl<T> AsDisplay for T {
    fn as_display(&self) -> Displayer<'_, Self> {
        Displayer::Borrowed(self)
    }
}

pub trait IntoDisplay {
    fn into_display(self) -> Displayer<'static, Self> where Self: Sized;
}
impl<T> IntoDisplay for T {
    fn into_display(self) -> Displayer<'static, Self> {
        Displayer::Owned(self)
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
