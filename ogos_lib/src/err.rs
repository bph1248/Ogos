use crate::{
    common::*,
    window_watch::*
};

use castaway::*;
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
    pub var: Box<ErrVar>,
    pub trail: Option<Vec<Loc>>,
    pub x: Loc
}
impl<const ID: u32> Display for ErrLoc<ID> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

macro_rules! loc_var {
    ($var:ident) => {
        LocVar::$var as u32
    };
}
pub(crate) use loc_var;

pub type Res<T, const ID: u32 = { LocVar::Default as u32 }> = Result<T, ErrLoc<ID>>;
pub type Res1<T, const ID: u32 = { LocVar::Res1 as u32 }> = Result<T, ErrLoc<ID>>;
pub type Res2<T, const ID: u32 = { LocVar::Res2 as u32 }> = Result<T, ErrLoc<ID>>;
pub type ResVar<T> = Result<T, ErrVar>;

#[derive(AsRefStr, Debug, Error)]
pub enum ErrVar {
    Anyhow(#[from] anyhow::Error),
    Bincode(#[from] bincode::Error),
    Clap(#[from] clap::Error),
    DiscordRichPresence(#[from] Box<dyn std::error::Error>),
    Eframe(#[from] eframe::Error),
    Ffprobe(#[from] ffprobe::FfProbeError),
    FromUtf16(#[from] string::FromUtf16Error),
    FromUtf8(#[from] string::FromUtf8Error),
    Ini(#[from] ini::Error),
    Io(#[from] io::Error),
    Json5(#[from] json5::Error),
    NetCoreHostContainsNul(#[from] netcorehost::pdcstring::ContainsNul),
    NetCoreHostGetManagedFunction(#[from] netcorehost::hostfxr::GetManagedFunctionError),
    NetCoreHostHosting(#[from ] netcorehost::error::HostingError),
    NetCoreHostLoadHostfxr(#[from ] netcorehost::nethost::LoadHostfxrError),
    LogSetLogger(#[from] log::SetLoggerError),
    NvApi(#[from] nvapi::Status),
    Opener(#[from] opener::OpenError),
    ParseBool(#[from] str::ParseBoolError),
    ParseError(#[from] fraction::error::ParseError),
    ParseInt(#[from] num::ParseIntError),
    QuickXml(#[from] quick_xml::Error),
    QuickXmlAttr(#[from] quick_xml::events::attributes::AttrError),
    Recv(#[from] sync::mpsc::RecvError),
    RecvOneshot(#[from] oneshot::error::RecvError),
    RecvTimeout(#[from] sync::mpsc::RecvTimeoutError),
    SendMsg(#[from] sync::mpsc::SendError<Msg>),
    SendReadyMsg(#[from] sync::mpsc::SendError<ReadyMsg>),
    SendWindowForegroundMsg2(#[from] sync::mpsc::SendError<WindowForegroundMsg>),
    SendWindowShiftMsg2(#[from] sync::mpsc::SendError<WindowShiftMsg>),
    SerdeJson(#[from] serde_json::Error),
    SystemTime(#[from] time::SystemTimeError),
    TryFromInt(#[from] num::TryFromIntError),
    Which(#[from] which::Error),
    WidestringContainsNul(#[from] widestring::error::ContainsNul<u16>),
    WinCore(#[from] windows::core::Error),
    WinCore052(#[from] windows_052::core::Error),
    Wmi(#[from] wmi::utils::WMIError),

    VideoSetting(#[from] VideoSettingConvertError),

    FailedAsColorBitDepth { from: u32 },
    FailedAsDitherBitDepth { from: u32 },
    FailedAsHz { from: String },
    FailedAsRequiredStream,
    FailedBuildLoggerConfig,
    FailedContactHookMgr { inner: windows::core::Error },
    FailedGetConfig,
    FailedIniOp { inner: ini::Error, path: String },
    FailedKeycodeAsKey { from: Keycode },
    FailedStrAsKey { from: String },
    FailedStrAsInputEvent { from: String },
    FailedNovideoSrgbApply,
    FailedOutputCommand { inner: io::Error, cmd: String },
    FailedQmkKeyboardInit { vid: u16, pid: u16, usage_page: u16 },
    FailedSetOnceCell,
    FailedSetWinEventHooks { inner: windows::core::Error, ctx: WinEventHookContext },
    FailedSpawnCommand { inner: io::Error, cmd: String },
    FailedToStr,
    FailedWmMouseMouse { inner: windows::core::Error, fg_hwnd: HWND, fg_exe: String },
    FailedWriteFile { inner: io::Error, path: String },
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
    MissingReShadeVkLayerDisableEnvKey,
    MissingStreamMetadata,
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
            Ffprobe(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            Eframe(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            FromUtf16(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            FromUtf8(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            Ini(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            Io(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            Json5(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            NetCoreHostContainsNul(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            NetCoreHostGetManagedFunction(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            NetCoreHostHosting(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            NetCoreHostLoadHostfxr(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            LogSetLogger(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            NvApi(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            Opener(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            ParseBool(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            ParseError(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            ParseInt(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            QuickXml(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            QuickXmlAttr(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            Recv(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            RecvOneshot(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            RecvTimeout(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            SendMsg(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            SendReadyMsg(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            SendWindowForegroundMsg2(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            SendWindowShiftMsg2(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            SerdeJson(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            SystemTime(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            TryFromInt(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            Which(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            WidestringContainsNul(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            WinCore(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            WinCore052(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),
            Wmi(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),

            VideoSetting(inner) => write!(f, "{}: {}", as_ref_str!(self), inner),

            FailedAsColorBitDepth { from } => write!(f, "failed to map color bit depth from: {}", from),
            FailedAsDitherBitDepth { from } => write!(f, "failed to map dither bit depth from: {}", from),
            FailedAsHz { from } => write!(f, "failed to map hz from: {}", from),
            FailedAsRequiredStream => write!(f, "failed to map required stream from stream"),
            FailedBuildLoggerConfig => write!(f, "failed to build logger config"),
            FailedContactHookMgr { inner } => write!(f, "failed to contact hook manager: {}", inner),
            FailedGetConfig => write!(f, "failed to get config"),
            FailedIniOp { inner, path } => write!(f, "failed .ini op: path: {}: {}", path, inner),
            FailedKeycodeAsKey { from } => write!(f, "failed to map key from keycode: {:#06x}", from.clone() as u16),
            FailedStrAsInputEvent { from } => write!(f, "failed to map input event from str: {}", from),
            FailedStrAsKey { from} => write!(f, "failed to map key from str: {}", from),
            FailedNovideoSrgbApply => write!(f, "failed to apply novideo_srgb"),
            FailedOutputCommand { inner, cmd } => write!(f, "failed to output command: {}: {}", cmd, inner),
            FailedQmkKeyboardInit { vid, pid, usage_page } => write!(f, "failed to init qmk keyboard: vid: {}, pid: {}, usage page: {}", vid, pid, usage_page),
            FailedSetOnceCell => write!(f, "failed to set OnceCell"),
            FailedSetWinEventHooks { inner, ctx } => write!(f, "failed to set win event hooks: {}: {}", ctx, inner),
            FailedSpawnCommand { inner, cmd } => write!(f, "failed to spawn command: {}: {}", cmd, inner),
            FailedToStr => write!(f, "failed to convert value to str"),
            FailedWmMouseMouse { inner, fg_hwnd, fg_exe} => write!(f, "failed on wm mouse move: fg_hwnd: {:p}, fg_exe: {}: {}", fg_hwnd.0, fg_exe, inner),
            FailedWriteFile { inner, path } => write!(f, "failed to write file: path: {}: {}", path, inner),
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
            MissingReShadeVkLayerDisableEnvKey => write!(f, "missing reshade vulkan layer disable environment key"),
            MissingStreamMetadata => write!(f, "missing video stream metadata"),
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

pub trait Track<T, E> where
    E: Into<ErrVar> + 'static
{
    fn x(self) -> Res<T>;
}
impl<T, E> Track<T, E> for Result<T, E> where
    E: Into<ErrVar> + 'static
{
    #[track_caller]
    fn x(self) -> Res<T> {
        let x = Loc {
            file: panic::Location::caller().file(),
            line: panic::Location::caller().line()
        };

        self.map_err(|err| {
            match_type!(err, {
                ErrLoc as mut existing => {
                    existing.x = x;

                    existing
                },
                other => ErrLoc {
                    var: Box::new(other.into()),
                    trail: None,
                    x
                }
            })
        })
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

pub(crate) trait Win32GleOk<T> {
    fn win32_gle_ok(self) -> windows::core::Result<T>;
}
impl Win32GleOk<Self> for u32 {
    fn win32_gle_ok(self) -> windows::core::Result<Self> {
        if self == 0 {
            unsafe { GetLastError().ok()? }
        }

        Ok(self)
    }
}
impl Win32GleOk<Self> for HWINEVENTHOOK {
    fn win32_gle_ok(self) -> windows::core::Result<Self> {
        if self.is_invalid() {
            unsafe { GetLastError().ok()? }
        }

        Ok(self)
    }
}
impl Win32GleOk<Self> for WAIT_EVENT {
    fn win32_gle_ok(self) -> windows::core::Result<Self> {
        if self.0 == WAIT_FAILED {
            unsafe { GetLastError().ok()? }
        }

        Ok(self)
    }
}

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
                        unsafe { GetLastError().ok()? }
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
                        unsafe { GetLastError().ok()? }
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
            Err(ErrVar::Win32DispChange { disp_change: self })?;
        }

        Ok(self)
    }
}
impl Win32VarOk<Self> for LRESULT {
    fn win32_var_ok(self) -> ResVar<Self> {
        if self.0 != 0 {
            Err(ErrVar::Win32LResult { lresult: self })?;
        }

        Ok(self)
    }
}
