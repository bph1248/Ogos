use crate::{
    common::*,
    win32::*,
    window_watch::*
};

use mki::*;
use nvapi_sys as nvapi;
use nvapi::NvAPI_Status;
use ogos_proc_macros::*;
use qmk_via_api::keycodes::*;
use std::{
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
    core::HRESULT,
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        UI::Accessibility::*
    }
};

const WAIT_FAILED: u32 = 0xFFFFFFFF;

#[derive(Default)]
pub(crate) struct Errored {
    pub(crate) hresults: HashSet<HRESULT>,
    pub(crate) others: HashSet<Discriminant<ErrVar>>
}

#[derive(Debug, Error)]
pub struct ErrLoc<const ID: u32 = { LocVar::Default as u32 }> {
    pub(crate) var: Box<ErrVar>,
    pub(crate) msg: &'static str,
    pub(crate) trail: Option<Vec<Loc>>,
    pub(crate) x: Loc
}
impl<const ID: u32> ErrLoc<ID> {
    pub(crate) fn msg(mut self, msg: &'static str) -> Self {
        self.msg = msg;

        self
    }
}
impl<const ID: u32> Display for ErrLoc<ID> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}, {}", self.msg, self.var, self.x)?;

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
pub(crate) struct Loc {
    pub(crate) file: &'static str,
    pub(crate) line: u32
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

macro_rules! loc_var {
    ($var:ident) => {
        LocVar::$var as u32
    };
}
pub(crate) use loc_var;

pub        type Res<T, const ID: u32 = { LocVar::Default as u32 }> = Result<T, ErrLoc<ID>>;
pub(crate) type Res1<T, const ID: u32 = { LocVar::Res1 as u32 }> = Result<T, ErrLoc<ID>>;
pub(crate) type Res2<T, const ID: u32 = { LocVar::Res2 as u32 }> = Result<T, ErrLoc<ID>>;
pub(crate) type ResVar<T> = Result<T, ErrVar>;

#[derive(AsRefStr, Debug, Error)]
pub(crate) enum ErrVar {
    Anyhow(#[from] anyhow::Error),
    Bincode(#[from] bincode::Error),
    Clap(#[from] clap::Error),
    DiscordRichPresence(#[from] Box<dyn std::error::Error>),
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
    Opener(#[from] opener::OpenError),
    Recv(#[from] sync::mpsc::RecvError),
    RecvOneshot(#[from] oneshot::error::RecvError),
    RecvTimeout(#[from] sync::mpsc::RecvTimeoutError),
    Resize(#[from] resize::Error),
    SendMsg(#[from] sync::mpsc::SendError<Msg>),
    SendReadyMsg(#[from] sync::mpsc::SendError<ReadyMsg>),
    SendWindowForegroundMsg2(#[from] sync::mpsc::SendError<WindowForegroundMsg>),
    SendWindowShiftMsg2(#[from] sync::mpsc::SendError<WindowShiftMsg>),
    SerdeJson(#[from] serde_json::Error),
    SerdeJson5(#[from] serde_json5::Error),
    SystemTime(#[from] time::SystemTimeError),
    ThreadPoolBuild(#[from] rayon::ThreadPoolBuildError),
    TryFromInt(#[from] num::TryFromIntError),
    Which(#[from] which::Error),
    WidestringContainsNul(#[from] widestring::error::ContainsNul<u16>),
    WinCore(#[from] windows::core::Error),
    WinCore061(#[from] windows_061::core::Error),

    FailedBuildLoggerConfig,
    FailedColorBitDepthFrom { from: u32 },
    FailedContactHookMgr { inner: windows::core::Error },
    FailedDitherBitDepthFrom { from: u32 },
    FailedHzFrom { from: String },
    FailedIniOp { inner: ini::Error, path: String },
    FailedInputEventFrom { from: String },
    FailedKeyFromKeycode { from: Keycode },
    FailedKeyFromStr { from: String },
    FailedNovideoSrgbApply,
    FailedOutputCommand { inner: io::Error, cmd: String },
    FailedQmkKeyboardInit { vid: u16, pid: u16, usage_page: u16 },
    FailedSetConfig,
    FailedSetOnceCell,
    FailedSetWinEventHooks { inner: windows::core::Error, ctx: WinEventHookContext },
    FailedSpawnCommand { inner: io::Error, cmd: String },
    FailedToStr,
    FailedWmMouseMouse { inner: windows::core::Error, fg_hwnd: SafeHwnd, fg_exe: String },
    FailedWriteFile { inner: io::Error, path: String },
    InvalidDisplayMode,
    InvalidDisplayCount,
    InvalidDisplayModelName,
    InvalidFileExt,
    InvalidFileName,
    InvalidFileStem,
    InvalidInputEventMap { from: InputEvent, to: InputEvent},
    InvalidPathParent,
    InvalidPixelCleaningVcpValue { vcp_value: u16 },
    InvalidUrl,
    InvalidQmkLayer { index: u8 },
    MissingClickMap,
    MissingConfigKey { name: &'static str },
    MissingFile { path: PathBuf },
    MissingNovideoSrgbFfi,
    MissingNvApiBackedDisplay,
    MissingProcess { name: String },
    MissingUsername,
    MissingTaskbarRelatedInfo,
    PoisonedLock,
    ReloadConfig,
    UnknownEq { name: String },
    UnknownGame { name: String },
    UnsuccessfulExitCode { code: Option<i32>, cmd: String },

    Win32DispChange { disp_change: DISP_CHANGE },
    Win32LResult { lresult: LRESULT }
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
            DiscordRichPresence(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
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
            Opener(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            Recv(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            RecvOneshot(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            RecvTimeout(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            Resize(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            SendMsg(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            SendReadyMsg(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            SendWindowForegroundMsg2(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            SendWindowShiftMsg2(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            SerdeJson(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            SerdeJson5(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            SystemTime(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            ThreadPoolBuild(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            TryFromInt(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            Which(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            WidestringContainsNul(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            WinCore(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            WinCore061(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),

            FailedBuildLoggerConfig => write!(f, "failed to build logger config"),
            FailedColorBitDepthFrom { from } => write!(f, "failed to map color bit depth from: {}", from),
            FailedContactHookMgr { inner } => write!(f, "failed to contact hook manager: {}", inner),
            FailedDitherBitDepthFrom { from } => write!(f, "failed to map dither bit depth from: {}", from),
            FailedHzFrom { from } => write!(f, "failed to map hz from: {}", from),
            FailedIniOp { inner, path } => write!(f, "failed .ini op: path: {}: {}", path, inner),
            FailedInputEventFrom { from } => write!(f, "failed to map input event from: {}", from),
            FailedKeyFromKeycode { from } => write!(f, "failed to map key from: {:#06x}", from.clone() as u16),
            FailedKeyFromStr { from} => write!(f, "failed to map key from: {}", from),
            FailedNovideoSrgbApply => write!(f, "failed to apply novideo_srgb"),
            FailedOutputCommand { inner, cmd } => write!(f, "failed to output command: {}: {}", cmd, inner),
            FailedQmkKeyboardInit { vid, pid, usage_page } => write!(f, "failed to init qmk keyboard: vid: {}, pid: {}, usage page: {}", vid, pid, usage_page),
            FailedSetConfig => write!(f, "failed to set config"),
            FailedSetOnceCell => write!(f, "failed to set OnceCell"),
            FailedSetWinEventHooks { inner, ctx } => write!(f, "failed to set win event hooks: {}: {}", ctx, inner),
            FailedSpawnCommand { inner, cmd } => write!(f, "failed to spawn command: {}: {}", cmd, inner),
            FailedToStr => write!(f, "failed to convert value to str"),
            FailedWmMouseMouse { inner, fg_hwnd, fg_exe} => write!(f, "failed on wm mouse move: fg_hwnd: {:p}, fg_exe: {}: {}", **fg_hwnd, fg_exe, inner),
            FailedWriteFile { inner, path } => write!(f, "failed to write file: path: {}: {}", path, inner),
            InvalidDisplayMode => write!(f, "invalid display mode"),
            InvalidDisplayCount => write!(f, "invalid display count - only 1 display is supported"),
            InvalidDisplayModelName => write!(f, "invalid display model name"),
            InvalidFileExt => write!(f, "invalid file extension"),
            InvalidFileName => write!(f, "invalid file name"),
            InvalidFileStem => write!(f, "invalid file stem"),
            InvalidInputEventMap { from, to} => write!(f, "invalid input event map: from: {}, to: {}", from, to),
            InvalidPathParent => write!(f, "invalid path parent"),
            InvalidPixelCleaningVcpValue { vcp_value: value } => write!(f, "invalid pixel cleaning vcp value: {}", value),
            InvalidUrl => write!(f, "unknown url"),
            InvalidQmkLayer { index } => write!(f, "invalid qmk layer: {}", index),
            MissingClickMap => write!(f, "missing click map"),
            MissingConfigKey { name } => write!(f, "missing config key: {}", name),
            MissingFile { path } => write!(f, "missing file: {}", path.display()),
            MissingNovideoSrgbFfi => write!(f, "missing novideo_srgb ffi"),
            MissingNvApiBackedDisplay => write!(f, "missing NvApi backed display"),
            MissingProcess { name } => write!(f, "missing process: {}", name),
            MissingUsername => write!(f, "missing username"),
            MissingTaskbarRelatedInfo => write!(f, "missing taskbar related init info"),
            PoisonedLock => write!(f, "poisoned lock"),
            ReloadConfig => write!(f, "reload config"),
            UnknownEq { name } => write!(f, "unknown eq: {}", name),
            UnknownGame { name } => write!(f, "unknown game: {}", name),
            UnsuccessfulExitCode { code, cmd } => write!(f, "unsuccessful exit code: {:?}, command: {}", code, cmd),

            Win32DispChange { disp_change } => write!(f, "Win32DispChange: {:?}", disp_change),
            Win32LResult { lresult } => write!(f, "Win32LResult: {}", lresult.0)
        }
    }
}

unsafe impl Send for ErrVar {}
unsafe impl Sync for ErrVar {}

impl From<&mut simplelog::ConfigBuilder> for ErrVar {
    fn from(_: &mut simplelog::ConfigBuilder) -> Self {
        Self::FailedBuildLoggerConfig
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

pub(crate) trait Track<T, E> where
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

pub(crate) trait NvApiOk {
    fn nvapi_ok(self) -> nvapi::Result<()>;
}
impl NvApiOk for NvAPI_Status {
    fn nvapi_ok(self) -> nvapi::Result<()> {
        nvapi::status_result(self)
    }
}

pub(crate) trait Win32CoreOk<T> {
    fn win32_core_ok(self) -> windows::core::Result<T>;
}
macro_rules! impl_Win32CoreOk {
    (if self == 0 = { $($ty:ty),+ }) => {
        $(
            impl Win32CoreOk<Self> for $ty {
                fn win32_core_ok(self) -> windows::core::Result<Self> {
                    if self == 0 {
                        unsafe { return Err(GetLastError().into()) }
                    }

                    Ok(self)
                }
            }
        )+
    };
    (if self.is_invalid() = { $($ty:ty),+ }) => {
        $(
            impl Win32CoreOk<Self> for $ty {
                fn win32_core_ok(self) -> windows::core::Result<Self> {
                    if self.is_invalid() {
                        unsafe { return Err(GetLastError().into()) }
                    }

                    Ok(self)
                }
            }
        )+
    };
    (if self.0 == WAIT_FAILED = { $($ty:ty),+ }) => {
        $(
            impl Win32CoreOk<Self> for $ty {
                fn win32_core_ok(self) -> windows::core::Result<Self> {
                    if self.0 == WAIT_FAILED {
                        unsafe { return Err(GetLastError().into()) }
                    }

                    Ok(self)
                }
            }
        )+
    };
}
impl_Win32CoreOk!(if self == 0             = { u32 });
impl_Win32CoreOk!(if self.is_invalid()     = { HWINEVENTHOOK });
impl_Win32CoreOk!(if self.0 == WAIT_FAILED = { WAIT_EVENT });

pub(crate) trait Win32ErrorOk<T> {
    fn win32_err_ok(self) -> windows::core::Result<()>;
}
impl Win32ErrorOk<()> for i32 {
    fn win32_err_ok(self) -> windows::core::Result<()> {
        WIN32_ERROR(u32::try_from(self).unwrap()).ok()
    }
}

pub(crate) trait Win32VarOk<T> {
    fn win32_var_ok(self) -> ResVar<T>;
}
macro_rules! impl_Win32VarOk {
    (if self == 0 = { $($ty:ty),+ }) => {
        $(
            impl Win32VarOk<Self> for $ty {
                fn win32_var_ok(self) -> ResVar<Self> {
                    if self == 0 {
                        unsafe { return Err(GetLastError().into()) }
                    }

                    Ok(self)
                }
            }
        )+
    };
    (if self.is_invalid() = { $($ty:ty),+ }) => {
        $(
            impl Win32VarOk<Self> for $ty {
                fn win32_var_ok(self) -> ResVar<Self> {
                    if self.is_invalid() {
                        unsafe { return Err(GetLastError().into()) }
                    }

                    Ok(self)
                }
            }
        )+
    };
}
impl_Win32VarOk!(if self == 0         = { i32, u16, u32 });
impl_Win32VarOk!(if self.is_invalid() = { HANDLE, HWINEVENTHOOK, HWND });

impl Win32VarOk<Self> for DISP_CHANGE {
    fn win32_var_ok(self) -> ResVar<Self> {
        if self != DISP_CHANGE_SUCCESSFUL {
            return Err(ErrVar::Win32DispChange { disp_change: self });
        }

        Ok(self)
    }
}
impl Win32VarOk<Self> for LRESULT {
    fn win32_var_ok(self) -> ResVar<Self> {
        if self.0 != 0 {
            return Err(ErrVar::Win32LResult { lresult: self });
        }

        Ok(self)
    }
}
