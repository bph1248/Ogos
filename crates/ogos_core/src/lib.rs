use eframe::egui;
use once_cell::sync::*;
use serde::*;
use std::{
    borrow::*,
    fmt::{self, *},
    ops::*,
    path::*,
    process::*
};
use windows::Win32::Foundation::*;
use zune_png::zune_core::colorspace::*;

pub static CURRENT_EXE_DIR: OnceCell<PathBuf> = OnceCell::new();
pub static CURRENT_EXE_FILE_NAME: OnceCell<String> = OnceCell::new();

#[macro_export]
macro_rules! default {
    () => {
        std::default::Default::default()
    };
}
#[macro_export]
macro_rules! _elapsed {
    ($id:expr, $body:expr) => {
        let begin = std::time::Instant::now();

        $body

        if begin.elapsed().as_micros() > 10 {
            info!("elapsed: {}: {}", $id, begin.elapsed().as_micros());
        }
    };
}
#[macro_export]
macro_rules! into {
    () => {
        |x| x.into()
    };
}
#[macro_export]
macro_rules! loan {
    ($src:expr, $dst:ident => $body:expr) => {
        let $dst = std::mem::take(&mut $src);

        $body

        $src = $dst;
    };

    ($src:expr, mut $dst:ident => $body:expr) => {
        let mut $dst = std::mem::take(&mut $src);

        $body

        $src = $dst;
    };
}
#[macro_export]
macro_rules! now {
    () => {
        std::time::Instant::now()
    };
}

#[derive(Clone, Copy, Default)]
pub struct Extent2d {
    pub width: i32,
    pub height: i32
}
impl Extent2d {
    pub fn into_rect(self) -> RECT {
        RECT {
            left: 0,
            top: 0,
            right: self.width,
            bottom: self.height
        }
    }
}
impl fmt::Display for Extent2d {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}
impl From<Extent2d> for RECT {
    fn from(value: Extent2d) -> Self {
        RECT {
            left: 0,
            top: 0,
            right: value.width,
            bottom: value.height
        }
    }
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq)]
#[serde(from = "[u32; 2]")]
pub struct Extent2dU {
    pub width: u32,
    pub height: u32
}
impl Extent2dU {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height
        }
    }

    pub fn width_f32(&self) -> f32 {
        self.width as f32
    }

    pub fn height_f32(&self) -> f32 {
        self.height as f32
    }
}
impl From<[u32; 2]> for Extent2dU {
    fn from(value: [u32; 2]) -> Self {
        Self {
            width: value[0],
            height: value[1]
        }
    }
}
impl From<Extent2dU> for egui::Vec2 {
    fn from(value: Extent2dU) -> Self {
        egui::vec2(value.width as f32, value.height as f32)
    }
}

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
impl fmt::Display for Displayer<'_, Command> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        fmt_command(self, f)
    }
}
impl fmt::Display for Displayer<'_, &mut Command> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        fmt_command(self, f)
    }
}
impl fmt::Display for Displayer<'_, HWND> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{:p}", self.0)
    }
}
impl<T> fmt::Display for Displayer<'_, Option<T>> where
    T: fmt::Display
{
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self.as_ref() {
            Some(v) => write!(f, "{}", v),
            None => write!(f, "<None>")
        }
    }
}
impl fmt::Display for Displayer<'_, RECT> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{{{}, {}, {}, {}}}{{{}, {}}}", self.left, self.top, self.right, self.bottom, self.width(), self.height())
    }
}
impl<T> fmt::Display for Displayer<'_, Vec<T>> where
    T: fmt::Display
{
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        let mut iter = self.iter();

        write!(f, "[")?;
        if let Some(v) = iter.next() {
            write!(f, "{}", v)?;
        }
        for v in iter {
            write!(f, ", {}", v)?;
        }
        write!(f, "]")?;

        Ok(())
    }
}
impl fmt::Display for Displayer<'_, ColorSpace> {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self.deref() {
            ColorSpace::RGB => write!(f, "RGB"),
            ColorSpace::RGBA => write!(f, "RGBA"),
            ColorSpace::YCbCr => write!(f, "YCbCr"),
            ColorSpace::Luma => write!(f, "Luma"),
            ColorSpace::LumaA => write!(f, "LumaA"),
            ColorSpace::YCCK => write!(f, "YCCK"),
            ColorSpace::CMYK => write!(f, "CMYK"),
            ColorSpace::BGR => write!(f, "BGR"),
            ColorSpace::BGRA => write!(f, "BGRA"),
            ColorSpace::Unknown => write!(f, "Unknown"),
            ColorSpace::ARGB => write!(f, "ARGB"),
            ColorSpace::HSL => write!(f, "HSL"),
            ColorSpace::HSV => write!(f, "HSV"),
            ColorSpace::MultiBand(non_zero) => write!(f, "MultiBand({})", non_zero),
            _ => write!(f, "Reserved")
        }
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FileKind {
    Dir,
    Image,
    Manga,
    Vid,
    Other,
    Unknown
}

pub trait As<'a, F: 'a> {
    fn as_<T: From<&'a F>>(&'a self) -> T;
}
impl<'a, F: 'a> As<'a, F> for F {
    fn as_<T: From<&'a F>>(&'a self) -> T {
        T::from(self)
    }
}

pub trait AsDisplay {
    fn as_display(&self) -> Displayer<'_, Self> where Self: Sized;
}
impl<T> AsDisplay for T {
    fn as_display(&self) -> Displayer<'_, Self> {
        Displayer::Borrowed(self)
    }
}

pub trait AsStaticCowPath {
    fn as_static_cow_path(&'static self) -> Cow<'static, Path>;
}
impl AsStaticCowPath for str {
    fn as_static_cow_path(&'static self) -> Cow<'static, Path> {
        Path::new(self).into()
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
