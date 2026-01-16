use crate::{
    audio::*,
    config::*,
    display::*,
    win32::*,
    window_foreground,
    window_shift::*
};
use ogos_core::*;
use ogos_err::*;

use discord_rich_presence::*;
use log::*;
use once_cell::sync::*;
use paste::*;
use serde::*;
use strum::*;
use subenum::*;
use std::{
    borrow::*,
    fmt::{self, Display},
    ops::*,
    path::*,
    process::*,
    sync::*,
    thread,
    time::*
};
use sysinfo::*;
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        UI::{
            WindowsAndMessaging::*,
            Input::KeyboardAndMouse::*,
        }
    }
};

pub(crate) const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Clone, Copy, Default)]
pub(crate) struct Extent2d {
    pub(crate) width: i32,
    pub(crate) height: i32
}
impl Extent2d {
    pub(crate) fn into_rect(self) -> RECT {
        RECT {
            left: 0,
            top: 0,
            right: self.width,
            bottom: self.height
        }
    }
}
impl Display for Extent2d {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}", self.width, self.height)
    }
}
impl Into<RECT> for Extent2d {
    fn into(self) -> RECT {
        RECT {
            left: 0,
            top: 0,
            right: self.width,
            bottom: self.height
        }
    }
}

#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(from = "[u32; 2]")]
pub(crate) struct Extent2dU {
    pub(crate) width: u32,
    pub(crate) height: u32
}
impl From<[u32; 2]> for Extent2dU {
    fn from(value: [u32; 2]) -> Self {
        Self {
            width: value[0],
            height: value[1]
        }
    }
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub(crate) struct Tid(pub(crate) u32);
impl From<u32> for Tid {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Tpids {
    pub(crate) thread: u32,
    pub(crate) proc: u32
}
impl Display for Tpids {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}, {}", self.thread, self.proc)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum FileKind {
    Dir,
    Image,
    Vid,
    Other
}

#[derive(Display)]
#[subenum(
    BroadcastMsg(derive(Clone, Copy, IntoStaticStr)),
    CursorWatchMsg,
    PipeMsg(derive(Deserialize, Display, Serialize)),
    ReadyMsg(derive(Display)),
    WindowForegroundMsg(derive(Display, IntoStaticStr)),
    WindowShiftMsg(derive(Display, IntoStaticStr))
)]
pub(crate) enum Msg {
    #[subenum(PipeMsg)]
    Ack,
    #[subenum(PipeMsg)]
    ActiveGame(Option<String>),
    #[subenum(CursorWatchMsg)]
    Begin,
    #[subenum(WindowForegroundMsg, WindowShiftMsg)]
    BroadcastMsg(BroadcastMsg),
    #[subenum(PipeMsg)]
    Close,
    #[subenum(WindowShiftMsg)]
    Destroy(usize),
    #[subenum(CursorWatchMsg)]
    DisplayChange(Extent2d),
    #[subenum(WindowShiftMsg)]
    MenuStart,
    #[subenum(WindowShiftMsg)]
    MenuEnd,
    #[subenum(WindowForegroundMsg)]
    PipeMsg(PipeMsg),
    #[subenum(ReadyMsg)]
    PipeServer,
    #[subenum(WindowForegroundMsg)]
    Taskbar(Box<window_foreground::Taskbar>),
    #[subenum(ReadyMsg)]
    WindowWatch(Tid),
    #[subenum(WindowForegroundMsg)]
    WinEventHookAllForeground { hwnd: usize },
    #[subenum(WindowForegroundMsg)]
    WinEventHookAllOtherForegroundDestroy { hook: usize, hwnd: usize },
    #[subenum(WindowForegroundMsg)]
    WinEventHookExplorerDestroy { hwnd: usize },
    #[subenum(WindowForegroundMsg)]
    WinEventHookForegroundLocationChange { hwnd: usize },
    #[subenum(WindowForegroundMsg)]
    WinEventHookShellExperienceHostDestroy { hook: usize, hwnd: usize },
    #[subenum(WindowForegroundMsg)]
    WinEventHookShellExperienceHostLocationChange { hook: usize, hwnd: usize },
    #[subenum(WindowForegroundMsg)]
    WinEventHookTaskbarLocationChange { hwnd: usize },
    #[subenum(BroadcastMsg)]
    WmDisplayChange(LPARAM),
    #[subenum(WindowForegroundMsg)]
    WmMouseMove(LPARAM, Instant),
    #[subenum(BroadcastMsg)]
    WmReloadConfig
}
impl Name for BroadcastMsg {
    fn name(&self) -> &'static str {
        self.into()
    }
}
impl Name for WindowForegroundMsg {
    fn name(&self) -> &'static str {
        self.into()
    }
}
impl Name for WindowShiftMsg {
    fn name(&self) -> &'static str {
        self.into()
    }
}

#[subenum(GamesSetting, VideoSetting)]
pub(crate) enum Setting<'a> {
    #[subenum(GamesSetting)]
    ActiveGame,
    #[subenum(GamesSetting)]
    CursorSize(usize),
    #[subenum(GamesSetting)]
    Discord(DiscordIpcClient),
    #[subenum(GamesSetting, VideoSetting)]
    DisplayMode(DisplayMode),
    #[subenum(VideoSetting)]
    NovideoSrgb(NovideoSrgbInfo<'a>),
    #[subenum(VideoSetting)]
    SampleRate(Hz),
    #[subenum(GamesSetting)]
    ScreenExtent(Extent2dU)
}

macro_rules! impl_WmOgos {
    ($first:ident, $($rest:ident),*) => {
        #[repr(u32)]
        pub(crate) enum WmOgos {
            $first = WM_USER + 1,
            $($rest,)*
        }

        paste! {
            pub(crate) const [<WM_OGOS_ $first:snake:upper>]: u32 = WmOgos::$first as u32;
            $(
                pub(crate) const [<WM_OGOS_ $rest:snake:upper>]: u32 = WmOgos::$rest as u32;
            )*
        }
    };
}
impl_WmOgos! {
    Close,
    ReloadConfig,
    Tray,
    RequestWinEventHooks,
    RequestWinEventUnhooks
}

pub(crate) static CONFIG: OnceCell<RwLock<Config>> = OnceCell::new();
pub(crate) static CURRENT_EXE_DIR: OnceCell<PathBuf> = OnceCell::new();

macro_rules! default {
    () => {
        std::default::Default::default()
    };
}
macro_rules! _elapsed {
    ($($s:stmt;)+) => {
        let begin = std::time::Instant::now();

        $($s)+

        info!("elapsed: {}", begin.elapsed().as_micros());
    };
}
macro_rules! into {
    () => {
        |x| x.into()
    };
}
macro_rules! now {
    () => {
        std::time::Instant::now()
    };
}
pub(crate) use default;
#[allow(unused_imports)]
pub(crate) use _elapsed;
pub(crate) use into;
pub(crate) use now;

pub(crate) trait BoolExt {
    fn and_then<T>(self, f: impl FnOnce() -> Option<T>) -> Option<T>;
    fn _as_str(&self) -> &'static str;
    fn as_win32_bool(&self) -> BOOL;
}
impl BoolExt for bool {
    fn and_then<T>(self, f: impl FnOnce() -> Option<T>) -> Option<T> {
        match self {
            true => f(),
            false => None
        }
    }

    fn _as_str(&self) -> &'static str {
        match *self {
            true => "true",
            false => "false"
        }
    }

    fn as_win32_bool(&self) -> BOOL {
        match self {
            true => BOOL(1),
            false => BOOL(0)
        }
    }
}

pub(crate) trait Name {
    fn name(&self) -> &'static str;
}

pub(crate) trait PathExt {
    fn confirm(&self) -> ResVar<&Self>;
    fn static_confirm(&'static self) -> ResVar<&'static Self>;
    fn get_dir(&self) -> ResVar<&Path>;
    fn get_file_ext(&self) -> ResVar<&str>;
    fn get_file_kind(&self) -> ResVar<FileKind>;
    fn get_file_name(&self) -> ResVar<&str>; // With extension
    fn get_file_stem(&self) -> ResVar<&str>; // Without extension
}
impl PathExt for Path {
    fn confirm(&self) -> ResVar<&Self> {
        if !self.try_exists()? {
            Err(ErrVar::MissingFile { path: self.to_owned().into() })?;
        }

        Ok(self)
    }

    fn static_confirm(&'static self) -> ResVar<&'static Self> {
        if !self.try_exists()? {
            Err(ErrVar::MissingFile { path: self.into() })?;
        }

        Ok(self)
    }

    fn get_dir(&self) -> ResVar<&Path> {
        self.parent()
            .ok_or(ErrVar::InvalidPathParent)
    }

    fn get_file_ext(&self) -> ResVar<&str> {
        self.extension()
            .ok_or(ErrVar::InvalidFileExt)?
            .to_str()
            .ok_or(ErrVar::FailedToStr)
    }

    fn get_file_kind(&self) -> ResVar<FileKind> {
        Ok(match self.is_dir() {
            true => FileKind::Dir,
            false => {
                let ext = self.get_file_ext()?;

                get_file_kind(ext)
            }
        })
    }

    fn get_file_name(&self) -> ResVar<&str> {
        self.file_name()
            .ok_or(ErrVar::InvalidFileName)?
            .to_str()
            .ok_or(ErrVar::FailedToStr)
    }

    fn get_file_stem(&self) -> ResVar<&str> {
        let stem = match self.is_dir() {
            true => self.file_name(),
            false => self.file_stem()
        };

        stem.ok_or(ErrVar::InvalidFileStem)?
            .to_str()
            .ok_or(ErrVar::FailedToStr)
    }
}

pub(crate) trait PathBufExt {
    fn confirm(self) -> ResVar<PathBuf>;
}
impl PathBufExt for PathBuf {
    fn confirm(self) -> ResVar<Self> {
        if !self.try_exists()? {
            return Err(ErrVar::MissingFile { path: self.into() })
        }

        Ok(self)
    }
}

pub(crate) trait RectExt {
    fn get_congruent_delta_from_anchor(&self, anchor_abs: AnchorAbsolute, leeway: u32) -> Option<Delta>;
    fn height(&self) -> i32;
    fn is_centered(&self, screen_extent: Extent2d) -> bool;
    fn sub(&self, rhs: Self) -> Self;
    fn width(&self) -> i32;
}
impl RectExt for RECT {
    fn get_congruent_delta_from_anchor(&self, anchor_abs: AnchorAbsolute, leeway: u32) -> Option<Delta> {
        let diffs = self.sub(anchor_abs.into());

        let is_congruent =
            diffs.left == diffs.right &&
            diffs.top == diffs.bottom &&
            diffs.left.unsigned_abs() <= leeway &&
            diffs.top.unsigned_abs() <= leeway;

        is_congruent.then_some(
            Delta {
                x: diffs.left,
                y: diffs.top
            }
        )
    }

    fn height(&self) -> i32 {
        self.bottom - self.top
    }

    fn is_centered(&self, screen_extent: Extent2d) -> bool {
        self.left == (screen_extent.width - self.right) &&
        self.top == (screen_extent.height - self.bottom)
    }

    fn sub(&self, rhs: Self) -> Self {
        Self {
            left: self.left - rhs.left,
            top: self.top - rhs.top,
            right: self.right - rhs.right,
            bottom: self.bottom - rhs.bottom
        }
    }

    fn width(&self) -> i32 {
        self.right - self.left
    }
}

pub(crate) trait StrExt {
    fn to_wide_128(&self) -> [u16; 128];
    unsafe fn to_win_str(&self) -> WinStr;
}
impl StrExt for &str {
    fn to_wide_128(&self) -> [u16; 128] {
        let mut buf = [0_u16; 128];
        let encoded = self.encode_utf16().collect::<Vec<_>>();

        let len = encoded.len().min(127);
        buf[..len].copy_from_slice(&encoded[..len]);

        buf
    }

    unsafe fn to_win_str(&self) -> WinStr {
        WinStr::new(self)
    }
}

pub(crate) fn attempt<T>(mut f: impl FnMut() -> Res<T>, attempt_count: u32, sleep_dur: Duration) -> Res<T> {
    for _ in 0..attempt_count.saturating_sub(1) {
        if let Ok(t) = f() {
            return Ok(t)
        }

        thread::sleep(sleep_dur);
    }

    f()
}

fn find_app(name: &'static str) -> ResVar<PathBuf> {
    which::which(name).map_err(|_| ErrVar::MissingFile { path: name.as_static_cow_path() })
}

pub(crate) fn confirm_or_find_app<P>(name: &'static str, path: Option<P>) -> ResVar<PathBuf> where
    P: AsRef<Path>
{
    fn inner(name: &'static str, confirm: Option<&Path>) -> ResVar<PathBuf> {
        confirm.map(|path| match path.confirm() {
            Ok(path) => Ok(path.to_owned()),
            Err(err) => match err {
                ErrVar::MissingFile { path } => {
                    error!("{}: invalid app path: {} - searching for {}", module_path!(), path.display(), name);

                    find_app(name).inspect(|path| info!("{}: found app: {}", module_path!(), path.display()))
                },
                _ => Err(err)
            }
        })
        .unwrap_or_else(|| find_app(name))
    }

    inner(name, path.as_ref().map(|path| path.as_ref()))
}

pub(crate) fn get_file_kind(ext: &str) -> FileKind {
    match ext {
        "jpg" |
        "jpeg" |
        "png" |
        "webp" => FileKind::Image,
        "m2ts" |
        "mkv" |
        "mp4" |
        "mts" |
        "ts" |
        "webm" => FileKind::Vid,
        _ => FileKind::Other
    }
}

pub(crate) fn get_first_process<'a>(proc_name: &str, system: &'a mut System) -> Option<&'a Process> {
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, default!());

    system.processes_by_exact_name(proc_name.as_ref())
        .find(|process| {
            process.name() == proc_name
        })
}

pub(crate) fn get_process_count(proc_name: &str, system: &mut System) -> usize {
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, default!());

    system.processes_by_exact_name(proc_name.as_ref()).count()
}

pub(crate) fn output_command(cmd: &mut Command) -> ResVar<Output> {
    let output = cmd.output()
        .map_err(|err| {
            ErrVar::FailedOutputCommand { inner: err, cmd: cmd.as_display().to_string() }
        })?;

    match output.status.success() {
        true => Ok(output),
        false => Err(ErrVar::UnsuccessfulExitCode { code: output.status.code(), cmd: cmd.as_display().to_string() })
    }
}

pub(crate) fn spawn_command(cmd: &mut Command) -> ResVar<Child> {
    let output = cmd.spawn()
        .map_err(|err| {
            ErrVar::FailedSpawnCommand { inner: err, cmd: cmd.as_display().to_string() }
        })?;

    Ok(output)
}

pub(crate) unsafe fn send_cursor_pos(x: i32, y: i32, screen_extent: Extent2d) -> windows::core::Result<()> {
    const NORM: i64 = 65535;

    let x = i64::from(x);
    let screen_width = i64::from(screen_extent.width - 1);
    let num = x * NORM + screen_width / 2;
    let dx = (num / screen_width) as i32;

    let y = i64::from(y);
    let screen_height = i64::from(screen_extent.height - 1);
    let num = y * NORM + screen_height / 2;
    let dy = (num / screen_height) as i32;

    let mut input_0 = INPUT_0::default();
    input_0.mi = MOUSEINPUT {
        dx,
        dy,
        mouseData: 0,
        dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
        time: 0,
        dwExtraInfo: 0
    };
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: input_0
    };

    SendInput(&[input], size_of::<INPUT>() as i32).win32_core_ok().map(|_| ())
}
