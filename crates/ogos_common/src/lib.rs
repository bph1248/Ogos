#![allow(clippy::missing_safety_doc)]

pub mod binds;
pub mod display;
pub mod window_foreground;
pub mod window_shift;

pub use binds::*;
pub use display::*;
pub use window_foreground::*;
pub use window_shift::*;

use ogos_core::*;
use ogos_err::*;

use log::*;
use mime_guess::*;
use nvapi_sys_new as nvapi_530;
use nvapi_530::*;
use paste::*;
use serde::*;
use std::{
    borrow::*,
    fmt::{self, Display},
    ops::*,
    path::*,
    process::*,
    thread,
    time::*
};
use strum::*;
use sysinfo::*;
use widestring::*;
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

use std::result::Result;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Tid(pub u32);
impl From<u32> for Tid {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Tpids {
    pub thread: u32,
    pub proc: u32
}
impl Display for Tpids {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}, {}", self.thread, self.proc)
    }
}

pub struct WinStr {
    _wide: U16CString,
    pcwstr: PCWSTR
}
impl WinStr {
    unsafe fn new(s: &str) -> Self { unsafe {
        let _wide = U16CString::from_str_unchecked(s);
        let pcwstr = PCWSTR(_wide.as_ptr());

        Self {
            _wide,
            pcwstr
        }
    } }
}
impl Default for WinStr {
    fn default() -> Self {
        let _wide: U16CString = default!();
        let pcwstr = PCWSTR(_wide.as_ptr());

        Self {
            _wide,
            pcwstr
        }
    }
}
impl Deref for WinStr {
    type Target = PCWSTR;

    fn deref(&self) -> &Self::Target {
        &self.pcwstr
    }
}
impl From<&PathBuf> for WinStr {
    fn from(value: &PathBuf) -> Self {
        unsafe {
            let _wide = U16CString::from_os_str_unchecked(value.as_os_str());
            let pcwstr = PCWSTR(_wide.as_ptr());

            Self {
                _wide,
                pcwstr
            }
        }
    }
}

#[derive(Clone, Copy, IntoStaticStr)]
pub enum BroadcastMsg {
    Close,
    WmDisplayChange(LPARAM),
    WmReloadConfig
}
impl VarName for BroadcastMsg {
    fn var_name(&self) -> &'static str {
        self.into()
    }
}

#[derive(Display)]
pub enum ReadyMsg {
    PipeServer,
    WindowWatch(Tid)
}

macro_rules! impl_WmOgos {
    ($first:ident, $($rest:ident),*) => {
        #[repr(u32)]
        enum WmOgos {
            $first = WM_USER + 1,
            $($rest,)*
        }

        paste! {
            pub const [<WM_OGOS_ $first:snake:upper>]: u32 = WmOgos::$first as u32;
            $(
                pub const [<WM_OGOS_ $rest:snake:upper>]: u32 = WmOgos::$rest as u32;
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

pub trait BoolExt {
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

pub trait PathExt {
    fn confirm(&self) -> ResVar<&Self>;
    fn confirm_static(&'static self) -> ResVar<&'static Self>;
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

    fn confirm_static(&'static self) -> ResVar<&'static Self> {
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

pub trait PathBufExt {
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

pub trait StrExt {
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

    unsafe fn to_win_str(&self) -> WinStr { unsafe {
        WinStr::new(self)
    } }
}

pub trait VarName {
    fn var_name(&self) -> &'static str;
}

pub fn attempt<T>(mut f: impl FnMut() -> Res<T>, attempt_count: u32, sleep_dur: Duration) -> Res<T> {
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

pub fn confirm_or_find_app<'a, P>(confirm: Option<&'a P>, app: &'static str) -> ResVar<Cow<'a, Path>> where
    P: AsRef<Path>
{
    fn inner<'a>(confirm: Option<&'a Path>, app: &'static str) -> ResVar<Cow<'a, Path>> {
        confirm.map(|path| {
            match path.try_exists()? {
                true => Ok(path.into()),
                false => {
                    error!("{}: invalid app path: {} - searching for {}", module_path!(), path.display(), app);

                    find_app(app).inspect(|path| info!("{}: found app: {}", module_path!(), path.display())).map(into!())
                }
            }
        })
        .unwrap_or_else(|| find_app(app).map(into!()))
    }

    inner(confirm.map(|v| v.as_ref()), app)
}

pub fn get_file_kind(ext: &str) -> FileKind {
    let guess = mime_guess::from_ext(ext).first();

    guess.as_ref().map(|mime| match mime.type_() {
        mime::IMAGE => FileKind::Image,
        mime::VIDEO => FileKind::Vid,
        _ => FileKind::Other
    })
    .unwrap_or(FileKind::Unknown)
}

pub fn get_first_process<'a>(proc_name: &str, system: &'a mut System) -> Option<&'a Process> {
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, default!());

    system.processes_by_exact_name(proc_name.as_ref())
        .find(|process| {
            process.name() == proc_name
        })
}

pub fn get_process_count(proc_name: &str, system: &mut System) -> usize {
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, default!());

    system.processes_by_exact_name(proc_name.as_ref()).count()
}

pub fn output_command(cmd: &mut Command) -> ResVar<Output> {
    let output = cmd.output()
        .map_err(|err| {
            ErrVar::FailedOutputCommand { inner: err, cmd: cmd.as_display().to_string() }
        })?;

    match output.status.success() {
        true => Ok(output),
        false => Err(ErrVar::UnsuccessfulExitCode { code: output.status.code(), cmd: cmd.as_display().to_string(), stdout: output.stdout })
    }
}

pub fn send_cursor_pos(x: i32, y: i32, screen_extent: Extent2d) -> windows::core::Result<()> { unsafe {
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
} }

pub fn spawn_command(cmd: &mut Command) -> ResVar<Child> {
    let output = cmd.spawn()
        .map_err(|err| {
            ErrVar::FailedSpawnCommand { inner: err, cmd: cmd.as_display().to_string() }
        })?;

    Ok(output)
}
