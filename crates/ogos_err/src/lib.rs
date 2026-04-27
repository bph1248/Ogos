#![allow(clippy::missing_safety_doc)]

use ogos_core::*;
use ogos_mki::*;
use ogos_proc_macros::*;

use nvapi_sys as nvapi;
use nvapi::NvAPI_Status;
use qmk_via_api::keycodes::*;
use std::{
    borrow::*,
    collections::*,
    fmt::{self, Display},
    io,
    mem::*,
    num,
    panic,
    path::*,
    str,
    string,
    sync,
    time
};
use strum::*;
use thiserror::*;
use tokio::sync::oneshot;
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        UI::Accessibility::*
    }
};

use std::result::Result;

const WAIT_FAILED: u32 = 0xFFFFFFFF;

#[derive(Default)]
pub struct Errored {
    pub hresults: HashSet<HRESULT>,
    pub others: HashSet<Discriminant<ErrVar>>
}

#[derive(Debug, Error)]
pub struct ErrLoc<const ID: u32 = { LocVar::Default as u32 }> {
    pub var: Box<ErrVar>,
    pub msg: &'static str,
    pub trail: Option<Vec<Loc>>,
    pub x: Loc
}
impl<const ID: u32> ErrLoc<ID> {
    pub fn msg(mut self, msg: &'static str) -> Self {
        self.msg = msg;

        self
    }
}
impl<const ID: u32> Display for ErrLoc<ID> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.msg.is_empty() {
            write!(f, "{}: ", self.msg)?;
        }

        write!(f, "{}, {}", self.var, self.x)?;

        if let Some(trail) = self.trail.as_ref() {
            for loc in trail.iter() {
                write!(f, ", {}", loc)?;
            }
        }

        Ok(())
    }
}
impl<E, const ID: u32> From<E> for ErrLoc<ID> where
    ErrVar: From<E>
{
    #[track_caller]
    fn from(err: E) -> Self {
        Self {
            var: Box::new(err.into()),
            msg: "",
            trail: None,
            x: Loc {
                file: panic::Location::caller().file(),
                line: panic::Location::caller().line()
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct Loc {
    pub file: &'static str,
    pub line: u32
}
impl Display for Loc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}, {}", self.file, self.line)
    }
}

err_loc_sets! {
    ErrLoc,
    LocVar,
    Default,

    Discord,
    Gui,
    Res1,

    DisplayMode = { Res2 },
    Games = { DisplayMode, Gui },
    Mpv = { DisplayMode },
    Res2 = { Res1 }
}

macro_rules! into {
    () => {
        |x| x.into()
    };
}
#[macro_export]
macro_rules! loc_var {
    ($var:ident) => {
        ogos_err::LocVar::$var as u32
    };
}

pub type Res<T, const ID: u32 = { LocVar::Default as u32 }> = Result<T, ErrLoc<ID>>;
pub type Res1<T, const ID: u32 = { LocVar::Res1 as u32 }> = Result<T, ErrLoc<ID>>;
pub type Res2<T, const ID: u32 = { LocVar::Res2 as u32 }> = Result<T, ErrLoc<ID>>;
pub type ResVar<T> = Result<T, ErrVar>;

#[derive(AsRefStr, Debug, Error)]
pub enum ErrVar {
    Anyhow(#[from] anyhow::Error),
    Bincode(#[from] bincode::Error),
    Clap(#[from] clap::Error),
    Discord(#[from] discord_rich_presence::error::Error),
    Eframe(#[from] eframe::Error),
    FromUtf16(#[from] string::FromUtf16Error),
    FromUtf8(#[from] string::FromUtf8Error),
    Image(#[from] image::error::ImageError),
    Io(#[from] io::Error),
    NetCoreHostContainsNul(#[from] netcorehost::pdcstring::ContainsNul),
    NetCoreHostGetManagedFunction(#[from] netcorehost::hostfxr::GetManagedFunctionError),
    NetCoreHostHosting(#[from ] netcorehost::error::HostingError),
    NetCoreHostLoadHostfxr(#[from ] netcorehost::nethost::LoadHostfxrError),
    LogSetLogger(#[from] log::SetLoggerError),
    NvApi(#[from] nvapi::Status),
    Recv(#[from] sync::mpsc::RecvError),
    RecvOneshot(#[from] oneshot::error::RecvError),
    RecvTimeout(#[from] sync::mpsc::RecvTimeoutError),
    Resize(#[from] resize::Error),
    Ron(#[from] ron::de::SpannedError),
    SerdeJson(#[from] serde_json::Error),
    SystemTime(#[from] time::SystemTimeError),
    ThreadPoolBuild(#[from] rayon::ThreadPoolBuildError),
    TryFromInt(#[from] num::TryFromIntError),
    Which(#[from] which::Error),
    WidestringContainsNul(#[from] widestring::error::ContainsNul<u16>),
    WinCore(#[from] windows::core::Error),
    WinCore061(#[from] windows_061::core::Error),

    Close,
    FailedBindVarFrom { from: String },
    FailedBuildLoggerConfig,
    FailedColorBitDepthFrom { from: String },
    FailedContactHookMgr { inner: windows::core::Error },
    FailedDitherBitDepthFrom { from: String },
    FailedHzFrom { from: String },
    FailedIniOp { inner: ini::Error, path: Cow<'static, Path> },
    FailedInputEventFrom { from: String },
    FailedKeyFrom { from: String },
    FailedKeyFromKeycode { from: Keycode },
    FailedNovideoSrgbApply,
    FailedOutputCommand { inner: io::Error, cmd: String },
    FailedQmkKeyboardInit { vid: u16, pid: u16, usage_page: u16 },
    FailedReadFile { inner: io::Error, path: Cow<'static, Path> },
    FailedSend { of: String },
    FailedSetConfig,
    FailedSetOnceCell,
    FailedSetWinEventHooks { ctx: String },
    FailedSpawnCommand { inner: io::Error, cmd: String },
    FailedToStr,
    FailedWmMouseMouse { inner: windows::core::Error, fg_hwnd: HWND, fg_exe: String },
    FailedWriteFile { inner: io::Error, path: Cow<'static, Path> },
    InvalidDisplayMode,
    InvalidDisplayName,
    InvalidFileExt,
    InvalidFileName,
    InvalidFileStem,
    InvalidHotkeyPrefix { key: &'static str },
    InvalidInputEventMap { from: InputEvent, to: InputEvent},
    InvalidLookahead(usize),
    InvalidPathParent,
    InvalidPixelCleaningVcpValue { vcp_value: u16 },
    InvalidProximity(usize),
    InvalidUrl,
    InvalidQmkLayer { index: u8 },
    MissingClickParams,
    MissingConfigKey { name: &'static str },
    MissingDirs,
    MissingFile { path: Cow<'static, Path> },
    MissingHotkeyPrefix,
    MissingNovideoSrgbFfi,
    MissingProcess { name: &'static str },
    MissingUsername,
    MissingTaskbarRelatedInfo,
    NegativeFloat,
    PoisonedLock,
    ReloadConfig,
    UnknownEndpoint,
    UnknownEqApoConfigName,
    UnknownGame { name: String },
    UnsuccessfulExitCode { code: Option<i32>, cmd: String, stdout: Vec<u8> },

    Win32InvalidHandle,
    Win32NonZeroLResult { lresult: LRESULT }
}
impl AsRef<Self> for ErrVar {
    fn as_ref(&self) -> &Self {
        self
    }
}
impl Display for ErrVar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use ErrVar::*;

        macro_rules! as_ref_str {
            ($i:ident) => {
                <Self as AsRef<str>>::as_ref($i)
            };
        }

        match self {
            Anyhow(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            Bincode(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            Clap(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            Discord(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            Eframe(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            FromUtf16(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            FromUtf8(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            Image(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            Io(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            NetCoreHostContainsNul(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            NetCoreHostGetManagedFunction(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            NetCoreHostHosting(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            NetCoreHostLoadHostfxr(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            LogSetLogger(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            NvApi(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            Recv(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            RecvOneshot(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            RecvTimeout(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            Resize(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            Ron(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            SerdeJson(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            SystemTime(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            ThreadPoolBuild(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            TryFromInt(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            Which(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            WidestringContainsNul(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            WinCore(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            WinCore061(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),

            Close => write!(f, "close"),
            FailedBindVarFrom { from } => write!(f, "failed bind var from: {}", from),
            FailedBuildLoggerConfig => write!(f, "failed to build logger config"),
            FailedColorBitDepthFrom { from } => write!(f, "failed color bit depth from: {}", from),
            FailedContactHookMgr { inner } => write!(f, "failed to contact hook manager: {}", inner),
            FailedDitherBitDepthFrom { from } => write!(f, "failed dither bit depth from: {}", from),
            FailedHzFrom { from } => write!(f, "failed hz from: {}", from),
            FailedIniOp { inner, path } => write!(f, "failed .ini op: path: {}: {}", path.display(), inner),
            FailedInputEventFrom { from } => write!(f, "failed input event from: {}", from),
            FailedKeyFrom { from} => write!(f, "failed key from: {}", from),
            FailedKeyFromKeycode { from } => write!(f, "failed key from: {:#06x}", from.clone() as u16),
            FailedNovideoSrgbApply => write!(f, "failed to apply novideo_srgb"),
            FailedOutputCommand { inner, cmd } => write!(f, "failed to output command: {}: {}", cmd, inner),
            FailedQmkKeyboardInit { vid, pid, usage_page } => write!(f, "failed to init qmk keyboard: vid: {}, pid: {}, usage page: {}", vid, pid, usage_page),
            FailedReadFile { inner, path } => write!(f, "failed to read file: {}: {}", path.display(), inner),
            FailedSend { of } => write!(f, "failed to send: {}", of),
            FailedSetConfig => write!(f, "failed to set config"),
            FailedSetOnceCell => write!(f, "failed to set OnceCell"),
            FailedSetWinEventHooks { ctx } => write!(f, "failed to set win event hooks: {}", ctx),
            FailedSpawnCommand { inner, cmd } => write!(f, "failed to spawn command: {}: {}", cmd, inner),
            FailedToStr => write!(f, "failed to convert value to str"),
            FailedWmMouseMouse { inner, fg_hwnd, fg_exe} => write!(f, "failed on wm mouse move: fg_hwnd: {}, fg_exe: {}: {}", fg_hwnd.as_display(), fg_exe, inner),
            FailedWriteFile { inner, path } => write!(f, "failed to write file: {}: {}", path.display(), inner),
            InvalidDisplayMode => write!(f, "invalid display mode"),
            InvalidDisplayName => write!(f, "invalid display name"),
            InvalidFileExt => write!(f, "invalid file extension"),
            InvalidFileName => write!(f, "invalid file name"),
            InvalidFileStem => write!(f, "invalid file stem"),
            InvalidHotkeyPrefix { key } => write!(f, "invalid hotkey prefix: {}", key),
            InvalidInputEventMap { from, to} => write!(f, "invalid input event map: from: {}, to: {}", from, to),
            InvalidLookahead(val) => write!(f, "invalid lookahead: {}", val),
            InvalidPathParent => write!(f, "invalid path parent"),
            InvalidPixelCleaningVcpValue { vcp_value: value } => write!(f, "invalid pixel cleaning vcp value: {}", value),
            InvalidProximity(val) => write!(f, "invalid proximity: {}", val),
            InvalidUrl => write!(f, "unknown url"),
            InvalidQmkLayer { index } => write!(f, "invalid qmk layer: {}", index),
            MissingClickParams => write!(f, "missing click parameters"),
            MissingConfigKey { name } => write!(f, "missing config key: {}", name),
            MissingDirs => write!(f, "missing dirs"),
            MissingFile { path } => write!(f, "missing file: {}", path.display()),
            MissingHotkeyPrefix => write!(f, "missing hotkey prefix"),
            MissingNovideoSrgbFfi => write!(f, "missing novideo_srgb ffi"),
            MissingProcess { name } => write!(f, "missing process: {}", name),
            MissingUsername => write!(f, "missing username"),
            MissingTaskbarRelatedInfo => write!(f, "missing taskbar related init info"),
            NegativeFloat => write!(f, "float must not be negative"),
            PoisonedLock => write!(f, "poisoned lock"),
            ReloadConfig => write!(f, "reload config"),
            UnknownEndpoint => write!(f, "unknown endpoint"),
            UnknownEqApoConfigName => write!(f, "unknown eq apo config name"),
            UnknownGame { name } => write!(f, "unknown game: {}", name),
            UnsuccessfulExitCode { code, cmd, stdout } => write!(f, "unsuccessful exit code: {}, command: {}, stdout: \n{}", code.as_display(), cmd, String::from_utf8_lossy(stdout)),
            Win32InvalidHandle => write!(f, "invalid win32 handle"),
            Win32NonZeroLResult { lresult } => write!(f, "non-zero win32 lresult: {}", lresult.0)
        }
    }
}
impl From<&mut simplelog::ConfigBuilder> for ErrVar {
    fn from(_: &mut simplelog::ConfigBuilder) -> Self {
        Self::FailedBuildLoggerConfig
    }
}
impl<T> From<sync::mpsc::SendError<T>> for ErrVar where
    T: Display
{
    fn from(value: sync::mpsc::SendError<T>) -> Self {
        Self::FailedSend { of: value.to_string() }
    }
}
impl<T> From<sync::PoisonError<sync::RwLockReadGuard<'_, T>>> for ErrVar {
    fn from(_: sync::PoisonError<sync::RwLockReadGuard<'_, T>>) -> Self {
        Self::PoisonedLock
    }
}
impl<T> From<sync::PoisonError<sync::RwLockWriteGuard<'_, T>>> for ErrVar {
    fn from(_: sync::PoisonError<sync::RwLockWriteGuard<'_, T>>) -> Self {
        Self::PoisonedLock
    }
}
impl From<WIN32_ERROR> for ErrVar {
    fn from(value: WIN32_ERROR) -> Self {
        Self::WinCore(value.into())
    }
}
unsafe impl Send for ErrVar {}
unsafe impl Sync for ErrVar {}

pub trait Track<T, E> where
    ErrVar: From<E>
{
    fn x(self) -> Res<T>;
}
impl<T, E> Track<T, E> for Result<T, E> where
    ErrVar: From<E>
{
    #[track_caller]
    fn x(self) -> Res<T> {
        self.map_err(into!())
    }
}

pub trait NvApiOk {
    fn nvapi_ok(self) -> nvapi::Result<()>;
}
impl NvApiOk for NvAPI_Status {
    fn nvapi_ok(self) -> nvapi::Result<()> {
        nvapi::status_result(self)
    }
}

pub trait Win32CoreOk<T> {
    fn win32_core_ok(self) -> windows::core::Result<T>;
}
impl Win32CoreOk<Self> for bool {
    fn win32_core_ok(self) -> windows::core::Result<Self> {
        if !self {
            Err(unsafe { GetLastError() })?;
        }

        Ok(self)
    }
}
impl Win32CoreOk<Self> for HANDLE {
    fn win32_core_ok(self) -> windows::core::Result<Self> {
        if self.is_invalid() {
            Err(unsafe { GetLastError() })?;
        }

        Ok(self)
    }
}
impl Win32CoreOk<Self> for WAIT_EVENT {
    fn win32_core_ok(self) -> windows::core::Result<Self> {
        if self.0 == WAIT_FAILED {
            Err(unsafe { GetLastError() })?;
        }

        Ok(self)
    }
}
macro_rules! impl_Win32CoreOk {
    (if self == 0 = { $($ty:ty),+ }) => {
        $(
            impl Win32CoreOk<Self> for $ty {
                fn win32_core_ok(self) -> windows::core::Result<Self> {
                    if self == 0 {
                        Err(unsafe { GetLastError() })?;
                    }

                    Ok(self)
                }
            }
        )+
    };
}
impl_Win32CoreOk!(if self == 0 = { i32, u16, u32, usize });

pub trait Win32ErrorOk<T> {
    fn win32_err_ok(self) -> windows::core::Result<()>;
}
impl Win32ErrorOk<()> for i32 {
    fn win32_err_ok(self) -> windows::core::Result<()> {
        WIN32_ERROR(u32::try_from(self).unwrap()).ok()
    }
}

pub trait Win32VarOk<T> {
    fn win32_var_ok(self) -> ResVar<T>;
}
impl Win32VarOk<Self> for HWINEVENTHOOK {
    fn win32_var_ok(self) -> ResVar<Self> {
        if self.is_invalid() {
            Err(ErrVar::Win32InvalidHandle)?;
        }

        Ok(self)
    }
}
impl Win32VarOk<Self> for LRESULT {
    fn win32_var_ok(self) -> ResVar<Self> {
        if self.0 != 0 {
            Err(ErrVar::Win32NonZeroLResult { lresult: self })?;
        }

        Ok(self)
    }
}
