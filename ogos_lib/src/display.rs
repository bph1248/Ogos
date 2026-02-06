use crate::{
    common::*,
    config::{self, *},
    nvapi_shadow::*,
    window_foreground
};
use ogos_binds::*;
use ogos_err::*;

use ddc::Ddc;
use log::*;
use netcorehost::hostfxr::*;
use nvapi_sys as nvapi;
use nvapi::{
    NVAPI_MAX_PHYSICAL_GPUS,
    nvapi_QueryInterface,
    Api::*,
    gpu::{
        NvAPI_EnumPhysicalGPUs,
        display::{
            NV_GPU_DISPLAYIDS_VER,
            NV_GPU_DISPLAYIDS,
            NvAPI_GPU_GetConnectedDisplayIds
        }
    },
    handles::NvPhysicalGpuHandle
};
use nvapi_sys_new as nvapi_530;
use nvapi_530::*;
use once_cell::sync::*;
use serde::*;
use std::{
    fmt::{self, Display},
    ops::*,
    ptr,
    process::*,
    thread,
    time::*
};
use sysinfo::*;
use widestring::*;
use windows::Win32::{
    Devices::Display::*,
    Foundation::*,
    Graphics::Gdi::*,
    UI::WindowsAndMessaging::*
};

pub(crate) const DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO_2: DISPLAYCONFIG_DEVICE_INFO_TYPE = DISPLAYCONFIG_DEVICE_INFO_TYPE(15_i32);
pub(crate) const DISPLAYCONFIG_DEVICE_INFO_SET_HDR_STATE: DISPLAYCONFIG_DEVICE_INFO_TYPE = DISPLAYCONFIG_DEVICE_INFO_TYPE(16_i32);
pub(crate) const MINIMIZE_ALL: usize = 419;
pub(crate) const UNDO_MINIMIZE_ALL: usize = 416;
pub(crate) const NV_COLOR_DATA_VER: NvU32 = make_nvapi_version::<NV_COLOR_DATA>(5);
pub(crate) const NV_DITHER_CONTROL_VER: NvU32 = make_nvapi_version::<NV_GPU_DITHER_CONTROL>(1);
           const VCP_FEATURE_PIXEL_CLEANING: u8 = 0xfd;
           const VCP_VALUE_PIXEL_CLEANING_IGNITION: u16 = 0x10;
           const VCP_VALUE_PIXEL_CLEANING_OFF_SDR: u16 = 0x41;
           const VCP_VALUE_PIXEL_CLEANING_OFF_HDR: u16 = 0x01;

#[derive(Default, PartialEq)]
#[repr(transparent)]
#[allow(nonstandard_style)]
pub(crate) struct DISPLAYCONFIG_ADVANCED_COLOR_MODE(pub(crate) i32);
impl DISPLAYCONFIG_ADVANCED_COLOR_MODE {
    pub(crate) const SDR: Self = Self(0);
    pub(crate) const WCG: Self = Self(1);
    pub(crate) const HDR: Self = Self(2);
}

#[derive(Default)]
#[repr(C)]
#[allow(nonstandard_style)]
pub(crate) struct DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO_2 {
    pub(crate) header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    pub(crate) Anonymous: DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO_0,
    pub(crate) colorEncoding: DISPLAYCONFIG_COLOR_ENCODING,
    pub(crate) bitsPerColorChannel: u32,
    pub(crate) activeColorMode: DISPLAYCONFIG_ADVANCED_COLOR_MODE
}

#[derive(Clone, Copy)]
#[repr(C)]
#[allow(nonstandard_style)]
pub(crate) struct DISPLAYCONFIG_SET_HDR_STATE_0_0 {
    pub(crate) _bitfield: u32
}

#[derive(Clone, Copy)]
#[repr(C)]
#[allow(nonstandard_style)]
pub(crate) union DISPLAYCONFIG_SET_HDR_STATE_0 {
    pub(crate) Anonymous: DISPLAYCONFIG_SET_HDR_STATE_0_0,
    pub(crate) value: u32
}

#[repr(C)]
#[allow(nonstandard_style)]
pub(crate) struct DISPLAYCONFIG_SET_HDR_STATE {
    pub(crate) header: DISPLAYCONFIG_DEVICE_INFO_HEADER,
    pub(crate) Anonymous: DISPLAYCONFIG_SET_HDR_STATE_0,
}

#[repr(C)]
pub(crate) struct NovideoSrgbApplyInfo {
    pub(crate) enable_clamp: bool,
    pub(crate) color_space_target: i32,
    pub(crate) primaries_source: i32,
    pub(crate) profile_path: *const u16,
    pub(crate) calibrate_gamma: bool,
    pub(crate) gamma_target: i32,
    pub(crate) gamma_value: f64,
    pub(crate) black_output_offset: f64,
    pub(crate) disable_optimization: bool
}
impl Default for NovideoSrgbApplyInfo {
    fn default() -> Self {
        Self {
            enable_clamp: false,
            color_space_target: 0,
            primaries_source: 0,
            profile_path: ptr::null(),
            calibrate_gamma: false,
            gamma_target: 0,
            gamma_value: 0.0,
            black_output_offset: 0.0,
            disable_optimization: false
        }
    }
}

pub(crate) type NovideoSrgbApplyFn = unsafe extern "system" fn(*const NovideoSrgbApplyInfo) -> i32;
pub(crate) struct NovideoSrgbFfi {
    pub(crate) _hostfxr: Hostfxr,
    pub(crate) novideo_srgb_apply_fn: ManagedFunction<NovideoSrgbApplyFn>
}

#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(try_from = "BindVar")]
pub(crate) enum ColorBitDepth {
    Default,
    N6,
    N8,
    N10,
    N12,
    N16
}
impl Deref for ColorBitDepth {
    type Target = NV_BPC;

    fn deref(&self) -> &Self::Target {
        use nvapi_530::*;

        match self {
            Self::Default => &_NV_BPC_NV_BPC_DEFAULT,
            Self::N6 => &_NV_BPC_NV_BPC_6,
            Self::N8 => &_NV_BPC_NV_BPC_8,
            Self::N10 => &_NV_BPC_NV_BPC_10,
            Self::N12 => &_NV_BPC_NV_BPC_12,
            Self::N16 => &_NV_BPC_NV_BPC_16
        }
    }
}
impl Display for ColorBitDepth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => write!(f, "default"),
            Self::N6 => write!(f, "6"),
            Self::N8 => write!(f, "8"),
            Self::N10 => write!(f, "10"),
            Self::N12 => write!(f, "12"),
            Self::N16 => write!(f, "16")
        }
    }
}
impl TryFrom<BindVar> for ColorBitDepth {
    type Error = ErrVar;

    fn try_from(value: BindVar) -> Result<Self, Self::Error> {
        Ok(match value {
            BindVar::Default => Self::Default,
            BindVar::N6 => Self::N6,
            BindVar::N8 => Self::N8,
            BindVar::N10 => Self::N10,
            BindVar::N12 => Self::N12,
            BindVar::N16 => Self::N16,
            _ => Err(ErrVar::FailedColorBitDepthFrom { from: value.as_str().into() })?
        })
    }
}

pub(crate) enum ControlWindowsArg {
    MinimizeAll,
    _UndoMinimizeAll
}
impl Display for ControlWindowsArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MinimizeAll => write!(f, "minimize all"),
            Self::_UndoMinimizeAll => write!(f, "undo minimize all")
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum DisplayMode {
    Sdr,
    Hdr
}
impl Display for DisplayMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sdr => write!(f, "sdr"),
            Self::Hdr => write!(f, "hdr")
        }
    }
}
impl Not for DisplayMode {
    type Output = Self;

    fn not(self) -> Self::Output {
        match self {
            Self::Sdr => Self::Hdr,
            Self::Hdr => Self::Sdr
        }
    }
}

#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(try_from = "BindVar")]
pub(crate) enum DitherBitDepth {
    N6,
    N8,
    N10
}
impl Deref for DitherBitDepth {
    type Target = NV_DITHER_BITS;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::N6 => &_NV_DITHER_BITS_NV_DITHER_BITS_6,
            Self::N8 => &_NV_DITHER_BITS_NV_DITHER_BITS_8,
            Self::N10 => &_NV_DITHER_BITS_NV_DITHER_BITS_10
        }
    }
}
impl Display for DitherBitDepth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::N6 => write!(f, "6"),
            Self::N8 => write!(f, "8"),
            Self::N10 => write!(f, "10")
        }
    }
}
impl TryFrom<BindVar> for DitherBitDepth {
    type Error = ErrVar;

    fn try_from(value: BindVar) -> Result<Self, Self::Error> {
        Ok(match value {
            BindVar::N6 => Self::N6,
            BindVar::N8 => Self::N8,
            BindVar::N10 => Self::N10,
            _ => Err(ErrVar::FailedDitherBitDepthFrom { from: value.as_str().into() })?
        })
    }
}

#[derive(Clone, Copy,Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DitherMode {
    SpatialStatic,
    SpatialStatic2x2,
    SpatialDynamic,
    SpatialDynamic2x2,
    Temporal
}
impl Deref for DitherMode {
    type Target = _NV_DITHER_MODE;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::SpatialStatic => &_NV_DITHER_MODE_NV_DITHER_MODE_SPATIAL_STATIC,
            Self::SpatialStatic2x2 => &_NV_DITHER_MODE_NV_DITHER_MODE_SPATIAL_STATIC_2x2,
            Self::SpatialDynamic => &_NV_DITHER_MODE_NV_DITHER_MODE_SPATIAL_DYNAMIC,
            Self::SpatialDynamic2x2 => &_NV_DITHER_MODE_NV_DITHER_MODE_SPATIAL_DYNAMIC_2x2,
            Self::Temporal => &_NV_DITHER_MODE_NV_DITHER_MODE_TEMPORAL
        }
    }
}
impl Display for DitherMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SpatialStatic => write!(f, "spatial_static"),
            Self::SpatialStatic2x2 => write!(f, "spatial_static2x2"),
            Self::SpatialDynamic => write!(f, "spatial_dynamic"),
            Self::SpatialDynamic2x2 => write!(f, "spatial_dynamic2x2"),
            Self::Temporal => write!(f, "temporal")
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DitherState {
    Default,
    Enabled,
    Disabled
}
impl Deref for DitherState {
    type Target = _NV_DITHER_STATE;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Default => &_NV_DITHER_STATE_NV_DITHER_STATE_DEFAULT,
            Self::Enabled => &_NV_DITHER_STATE_NV_DITHER_STATE_ENABLED,
            Self::Disabled => &_NV_DITHER_STATE_NV_DITHER_STATE_DISABLED
        }
    }
}
impl Display for DitherState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Default => write!(f, "default"),
            Self::Enabled => write!(f, "enabled"),
            Self::Disabled => write!(f, "disabled")
        }
    }
}

pub(crate) enum SetDisplayModeOp {
    Set(DisplayMode),
    Toggle
}

pub(crate) enum WallpaperEngineArg {
    Play,
    Stop
}
impl WallpaperEngineArg {
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Play => "play",
            Self::Stop => "stop"
        }
    }
}
impl Display for WallpaperEngineArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

pub(crate) static NOVIDEO_SRGB_FFI: OnceCell<Option<NovideoSrgbFfi>> = OnceCell::new();

pub(crate) static NVAPI_GPU_GET_DITHER_CONTROL_FN: Lazy<NvAPI_GPU_GetDitherControl_fn> = Lazy::new(|| unsafe {
    let interface = nvapi_QueryInterface(NvAPI_GPU_GetDitherControl.id()).x().unwrap_or_else(|err| {
        panic!("{}: failed to query nvapi interface: {}: {}", module_path!(), stringify!(NvAPI_GPU_GetDitherControl), err)
    });

    std::mem::transmute(interface)
});
pub(crate) static NVAPI_GPU_SET_DITHER_CONTROL_FN: Lazy<NvAPI_GPU_SetDitherControl_fn> = Lazy::new(|| unsafe {
    let interface = nvapi_QueryInterface(NvAPI_GPU_SetDitherControl.id()).x().unwrap_or_else(|err| {
        panic!("{}: failed to query nvapi interface: {}: {}", module_path!(), stringify!(NvAPI_GPU_SetDitherControl), err)
    });

    std::mem::transmute(interface)
});

fn make_novideo_srgb_apply_info(info: &NovideoSrgbInfo) -> Res1<(NovideoSrgbApplyInfo, U16CString)> {
    let profile_path = match info.primaries_source {
        PrimariesSource::Edid => U16CString::new(),
        PrimariesSource::Profile { ref path } => U16CString::from_str(path)?
    };

    let GammaFfi {
        calibrate_gamma,
        gamma_target,
        gamma_value,
        black_output_offset,
    } = info.gamma.as_ffi();

    Ok((
        NovideoSrgbApplyInfo {
            enable_clamp: info.enable_clamp,
            color_space_target: info.color_space_target as i32,
            primaries_source: info.primaries_source.as_i32(),
            profile_path: profile_path.as_ptr(),
            calibrate_gamma,
            gamma_target,
            gamma_value,
            black_output_offset,
            disable_optimization: !info.enable_optimization
        },
        profile_path // Keep alive
    ))
}

pub(crate) fn control_novideo_srgb(info: &NovideoSrgbInfo) -> Res2<()> { unsafe {
    match NOVIDEO_SRGB_FFI.get_unchecked() {
        Some(ffi) => {
            let apply_info = make_novideo_srgb_apply_info(info)?;

            match (ffi.novideo_srgb_apply_fn)(&apply_info.0) {
                42 => Ok(()),
                _ => Err(ErrVar::FailedNovideoSrgbApply)?
            }
        },
        None => Err(ErrVar::MissingNovideoSrgbFfi)?
    }
} }

pub(crate) fn control_wallpaper_engine(arg: WallpaperEngineArg, system: &mut System) -> Res1<()> {
    if get_first_process(App::WALLPAPER_ENGINE, system).is_some() {
        let config = config::get().read()?;
        let wallpaper_engine_path = confirm_or_find_app(App::WALLPAPER_ENGINE, config.app_paths.wallpaper_engine.as_ref())?;

        drop(config);

        let mut cmd = Command::new(wallpaper_engine_path);
        cmd.args(["-control", arg.as_str()]);

        output_command(&mut cmd)?;
        info!("{}: wallpaper engine: {}", module_path!(), arg);
    }

    Ok(())
}

pub(crate) fn control_windows(arg: ControlWindowsArg) -> Res1<()> { unsafe {
    let taskbar_class_name = window_foreground::TASKBAR_CLASS_NAME.to_win_str();
    let taskbar_hwnd = FindWindowW(Some(&*taskbar_class_name), None)?;

    let wparam = WPARAM(
        match arg {
            ControlWindowsArg::MinimizeAll => MINIMIZE_ALL,
            ControlWindowsArg::_UndoMinimizeAll => UNDO_MINIMIZE_ALL
        }
    );
    SendMessageW(taskbar_hwnd, WM_COMMAND, Some(wparam), None).win32_var_ok()?;

    info!("{}: control windows: {}", module_path!(), arg);

    Ok(())
} }

pub(crate) fn begin_pixel_cleaning(prelude: Option<config::PixelCleaning>) -> Res2<()> { unsafe {
    let system = (|| -> Res<Option<System>> {
        if let Some(prelude) = prelude {
            if prelude.let_walk_away { let_walk_away()?; }
            if prelude.pause_wallpaper_engine {
                let mut system = System::new();
                control_wallpaper_engine(WallpaperEngineArg::Stop, &mut system)?;

                return Ok(Some(system))
            }
        }

        Ok(None)
    })()?;

    let path = get_first_display_path()?;
    let friendly_name = get_display_friendly_name(path)?;
    if friendly_name != u16cstr!("PG32UCDM") {
        Err(ErrVar::InvalidDisplayName)?;
    }
    let gdi_name = get_display_gdi_name(path)?;

    let monitor_hnds = ddc_winapi::enumerate_monitors()?;
    let mut monitor_info = MONITORINFOEXW::default();
    monitor_info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;

    let droid = (|| -> windows::core::Result<_> {
        for hnd in monitor_hnds {
            let hnd = HMONITOR(hnd as *mut _); // Different Windows APIs
            GetMonitorInfoW(hnd, &mut monitor_info as *mut _ as _).ok()?;
            let gdi_name_ = U16CString::from_ptr_truncate(&monitor_info.szDevice as _, monitor_info.szDevice.len());

            if gdi_name_ == gdi_name {
                return Ok(Some(hnd));
            }
        }

        Ok(None)
    })()?;

    let phys_monitors = ddc_winapi::get_physical_monitors_from_hmonitor(droid.unwrap().0 as *mut _)?;
    let mut monitor = ddc_winapi::Monitor::new(phys_monitors[0]);

    let vcp_value = monitor.get_vcp_feature(VCP_FEATURE_PIXEL_CLEANING)?.value();
    match vcp_value {
        VCP_VALUE_PIXEL_CLEANING_OFF_SDR | VCP_VALUE_PIXEL_CLEANING_OFF_HDR => {
            let vcp_value = vcp_value + VCP_VALUE_PIXEL_CLEANING_IGNITION;
            monitor.set_vcp_feature(VCP_FEATURE_PIXEL_CLEANING, vcp_value)?;

            info!("{}: enable pixel cleaning: vcp {:#x}: {:#x}", module_path!(), VCP_FEATURE_PIXEL_CLEANING, vcp_value);

            if let Some(mut system) = system {
                thread::spawn(move || {
                    thread::sleep(Duration::from_secs(420));

                    control_wallpaper_engine(WallpaperEngineArg::Play, &mut system).unwrap_or_else(|err| {
                        error!("{}: failed to resume wallpaper engine after pixel cleaning: {}", module_path!(), err);
                    });
                });
            }
        },
        _ => Err(ErrVar::InvalidPixelCleaningVcpValue { vcp_value })?
    }

    Ok(())
} }

pub(crate) fn enable_screensaver() -> ResVar<()> { unsafe {
    SendMessageW(GetDesktopWindow(), WM_SYSCOMMAND, Some(WPARAM(SC_SCREENSAVE as usize)), None).win32_var_ok()?;

    info!("{}: screensaver: enabled", module_path!());

    Ok(())
} }

pub(crate) fn let_walk_away() -> Res2<()> {
    control_windows(ControlWindowsArg::MinimizeAll)?;
    enable_screensaver()?;

    Ok(())
}

fn get_color_bit_depth(display_id: NvU32) -> nvapi::Result<ColorBitDepth> { unsafe {
    use nvapi_530::*;

    let mut color_data = NV_COLOR_DATA {
        version: NV_COLOR_DATA_VER,
        size: size_of::<NV_COLOR_DATA>() as NvU16,
        cmd: NV_COLOR_CMD_NV_COLOR_CMD_GET as NvU8,
        ..default!()
    };
    NvAPI_Disp_ColorControl(display_id, &mut color_data).nvapi_ok()?;

    let color_bit_depth = match color_data.data.bpc {
        _NV_BPC_NV_BPC_DEFAULT => ColorBitDepth::Default,
        _NV_BPC_NV_BPC_6 => ColorBitDepth::N6,
        _NV_BPC_NV_BPC_8 => ColorBitDepth::N8,
        _NV_BPC_NV_BPC_10 => ColorBitDepth::N10,
        _NV_BPC_NV_BPC_12 => ColorBitDepth::N12,
        _NV_BPC_NV_BPC_16 => ColorBitDepth::N16,
        _ => unreachable!()
    };

    Ok(color_bit_depth)
} }

pub(crate) fn get_display_friendly_name(path: DISPLAYCONFIG_PATH_INFO) -> Res1<U16CString> { unsafe {
    let mut target_device_name = DISPLAYCONFIG_TARGET_DEVICE_NAME {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
            size: size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
            adapterId: path.targetInfo.adapterId,
            id: path.targetInfo.id
        },
        ..default!()
    };
    DisplayConfigGetDeviceInfo(&mut target_device_name.header).win32_err_ok()?;
    let friendly_name = U16CString::from_ptr_truncate(target_device_name.monitorFriendlyDeviceName.as_ptr(), target_device_name.monitorFriendlyDeviceName.len());

    Ok(friendly_name)
} }

pub(crate) fn get_display_gdi_name(path: DISPLAYCONFIG_PATH_INFO) -> Res1<U16CString> { unsafe {
    let mut source_device_name = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
            size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
            adapterId: path.sourceInfo.adapterId,
            id: path.sourceInfo.id
        },
        ..default!()
    };
    DisplayConfigGetDeviceInfo(&mut source_device_name.header).win32_err_ok()?;
    let gdi_name = U16CString::from_ptr_truncate(source_device_name.viewGdiDeviceName.as_ptr(), source_device_name.viewGdiDeviceName.len());

    Ok(gdi_name)
} }

pub(crate) fn get_display_mode(path: DISPLAYCONFIG_PATH_INFO) -> Res1<DisplayMode> { unsafe {
    let mut advanced_color_info = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO_2 {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO_2,
            size: size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO_2>() as u32,
            adapterId: path.targetInfo.adapterId,
            id: path.targetInfo.id
        },
        ..default!()
    };
    DisplayConfigGetDeviceInfo(&mut advanced_color_info.header).win32_err_ok()?;

    // let acm_enabled = advanced_color_info.Anonymous.value & 0x80 == 0x80;
    let display_mode = match advanced_color_info.activeColorMode {
        DISPLAYCONFIG_ADVANCED_COLOR_MODE::SDR => DisplayMode::Sdr,
        DISPLAYCONFIG_ADVANCED_COLOR_MODE::WCG => DisplayMode::Sdr,
        DISPLAYCONFIG_ADVANCED_COLOR_MODE::HDR => DisplayMode::Hdr,
        _ => unreachable!()
    };

    Ok(display_mode)
} }

fn get_dither_control(display_id: NvU32) -> nvapi::Result<NV_GPU_DITHER_CONTROL> { unsafe {
    let mut dither_control = NV_GPU_DITHER_CONTROL {
        version: NV_DITHER_CONTROL_VER,
        ..default!()
    };
    (*NVAPI_GPU_GET_DITHER_CONTROL_FN)(display_id, &mut dither_control).nvapi_ok()?;

    Ok(dither_control)
} }

pub(crate) fn get_first_display_path() -> Res1<DISPLAYCONFIG_PATH_INFO> { unsafe {
    let mut path_count = 0;
    let mut mode_count = 0;
    GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count).ok()?;

    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
    QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut path_count, paths.as_mut_ptr(), &mut mode_count, modes.as_mut_ptr(), None).ok()?;

    Ok(paths[0])
} }

pub(crate) fn get_first_gpu_display_ids() -> Res1<(NvPhysicalGpuHandle, NV_GPU_DISPLAYIDS)> { unsafe {
    let mut gpu_hnds = [NvPhysicalGpuHandle::default(); NVAPI_MAX_PHYSICAL_GPUS];
    let mut gpu_count = 0;
    NvAPI_EnumPhysicalGPUs(&mut gpu_hnds, &mut gpu_count).nvapi_ok()?;

    let mut display_ids = [NV_GPU_DISPLAYIDS::zeroed()];
    let mut display_ids_count = display_ids.len() as u32;
    display_ids[0].version = NV_GPU_DISPLAYIDS_VER;
    NvAPI_GPU_GetConnectedDisplayIds(gpu_hnds[0], display_ids.as_mut_ptr(), &mut display_ids_count, 0).nvapi_ok()?; // Just use GPU 0

    Ok((gpu_hnds[0], display_ids[0]))
} }

pub(crate) fn get_screen_extent() -> Res1<Extent2d> { unsafe {
    let mut path_count = 0;
    let mut mode_count = 0;
    GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count).ok()?;

    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
    QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut path_count, paths.as_mut_ptr(), &mut mode_count, modes.as_mut_ptr(), None).ok()?;

    let source_mode = &modes[paths[0].sourceInfo.Anonymous.modeInfoIdx as usize];

    Ok(Extent2d {
        width: source_mode.Anonymous.sourceMode.width as i32,
        height: source_mode.Anonymous.sourceMode.height as i32
    })
} }

pub(crate) fn set_color_bit_depth(display_id: NvU32, bit_depth: ColorBitDepth) -> Res1<Option<ColorBitDepth>> { unsafe {
    use nvapi_530::*;

    let prev_bit_depth = get_color_bit_depth(display_id)?;

    if bit_depth == prev_bit_depth {
        return Ok(None)
    }

    let mut color_data = NV_COLOR_DATA {
        version: NV_COLOR_DATA_VER,
        size: size_of::<NV_COLOR_DATA>() as NvU16,
        cmd: NV_COLOR_CMD_NV_COLOR_CMD_SET as NvU8,
        data: _NV_COLOR_DATA_V5__bindgen_ty_1 {
            colorFormat: NV_COLOR_FORMAT_NV_COLOR_FORMAT_RGB as NvU8,
            colorimetry: NV_COLOR_COLORIMETRY_NV_COLOR_COLORIMETRY_RGB as NvU8,
            dynamicRange: _NV_DYNAMIC_RANGE_NV_DYNAMIC_RANGE_VESA as NvU8,
            bpc: *bit_depth,
            colorSelectionPolicy: _NV_COLOR_SELECTION_POLICY_NV_COLOR_SELECTION_POLICY_USER,
            depth: _NV_DESKTOP_COLOR_DEPTH_NV_DESKTOP_COLOR_DEPTH_DEFAULT
        }
    };
    NvAPI_Disp_ColorControl(display_id, &mut color_data).nvapi_ok()?;

    info!("{}: color bit depth: {}", module_path!(), bit_depth);

    Ok(Some(prev_bit_depth))
} }

fn set_display_mode_unchecked(display_mode: DisplayMode, display_path: DISPLAYCONFIG_PATH_INFO) -> windows::core::Result<()> { unsafe {
    let set_hdr_state = DISPLAYCONFIG_SET_HDR_STATE {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_SET_HDR_STATE,
            size: size_of::<DISPLAYCONFIG_SET_HDR_STATE>() as u32,
            adapterId: display_path.targetInfo.adapterId,
            id: display_path.targetInfo.id
        },
        Anonymous: DISPLAYCONFIG_SET_HDR_STATE_0 {
            value: match display_mode {
                DisplayMode::Sdr => 0,
                DisplayMode::Hdr => 1
            }
        }
    };
    DisplayConfigSetDeviceInfo(&set_hdr_state.header).win32_err_ok()?;

    info!("{}: display mode: {}", module_path!(), display_mode);

    Ok(())
} }

pub(crate) fn set_display_mode(op: SetDisplayModeOp) -> Res<Option<DisplayMode>, { loc_var!(DisplayMode) }> {
    let display_path = get_first_display_path()?;
    let prev_display_mode = get_display_mode(display_path)?;

    let inner = |display_mode: DisplayMode| {
        if display_mode == prev_display_mode {
            return Ok(None)
        }

        let (gpu_hnd, display_ids) = get_first_gpu_display_ids()?;
        let config = config::get().read()?;
        let display_modes_config = config.display_modes.as_ref().ok_or(ErrVar::MissingConfigKey { name: config::DisplayModes::NAME })?;

        match display_mode {
            DisplayMode::Sdr => {
                let display_mode_info = &display_modes_config.sdr;
                let color_bit_depth = display_mode_info.color_bit_depth;
                let dither_bit_depth = display_mode_info.dither.bit_depth;

                set_color_bit_depth(display_ids.displayId, color_bit_depth)?;
                set_dither_control(gpu_hnd, display_ids.displayId, display_mode_info.dither.state, dither_bit_depth, display_mode_info.dither.mode)?;
                set_display_mode_unchecked(display_mode, display_path)?;

                match display_modes_config.sdr.novideo_srgb.as_ref() {
                    Some(info) => control_novideo_srgb(info)?,
                    None => if display_modes_config.hdr.novideo_srgb.is_some() {
                        control_novideo_srgb(&default!())?;
                    }
                }
            },
            DisplayMode::Hdr => {
                let display_mode_info = &display_modes_config.hdr;
                let color_bit_depth = display_mode_info.color_bit_depth;
                let dither_bit_depth = display_mode_info.dither.bit_depth;

                match display_modes_config.hdr.novideo_srgb.as_ref() {
                    Some(info) => control_novideo_srgb(info)?,
                    None => if display_modes_config.sdr.novideo_srgb.is_some() {
                        control_novideo_srgb(&default!())?;
                    }
                }

                set_color_bit_depth(display_ids.displayId, color_bit_depth)?;
                set_dither_control(gpu_hnd, display_ids.displayId, display_mode_info.dither.state, dither_bit_depth, display_mode_info.dither.mode)?;
                set_display_mode_unchecked(display_mode, display_path)?;
            }
        }

        Ok(Some(prev_display_mode))
    };

    match op {
        SetDisplayModeOp::Set(display_mode) => inner(display_mode),
        SetDisplayModeOp::Toggle => inner(!prev_display_mode)
    }
}

pub(crate) fn set_dither_control(gpu_hnd: NvPhysicalGpuHandle, display_id: NvU32, state: DitherState, bit_depth: DitherBitDepth, mode: DitherMode) -> Res1<Option<NV_GPU_DITHER_CONTROL>> { unsafe {
    let prev_dither_control = get_dither_control(display_id)?;
    let dither_control = NV_GPU_DITHER_CONTROL {
        state: *state,
        bits: *bit_depth,
        mode: *mode,
        ..prev_dither_control
    };

    if dither_control == prev_dither_control {
        return Ok(None)
    }

    (*NVAPI_GPU_SET_DITHER_CONTROL_FN)(gpu_hnd, display_id, *state, *bit_depth, *mode).nvapi_ok()?;

    info!("{}: dither: state: {}, bit depth: {}, mode: {}", module_path!(), state, bit_depth, mode);

    Ok(Some(prev_dither_control))
} }

pub(crate) fn set_screen_extent(extent: Extent2dU) -> Res1<Option<Extent2dU>> { unsafe {
    let mut path_count = 0;
    let mut mode_count = 0;
    GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count).ok()?;

    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
    QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut path_count, paths.as_mut_ptr(), &mut mode_count, modes.as_mut_ptr(), None).ok()?;

    let source_mode = &mut modes[paths[0].sourceInfo.Anonymous.modeInfoIdx as usize];
    let prev_extent = Extent2dU {
        width: source_mode.Anonymous.sourceMode.width,
        height: source_mode.Anonymous.sourceMode.height
    };
    if extent == prev_extent {
        return Ok(None)
    }

    source_mode.Anonymous.sourceMode.width = extent.width;
    source_mode.Anonymous.sourceMode.height = extent.height;
    SetDisplayConfig(Some(paths.as_slice()), Some(modes.as_slice()), SDC_APPLY | SDC_USE_SUPPLIED_DISPLAY_CONFIG).win32_err_ok()?;

    Ok(Some(prev_extent))
} }
