use ogos_common::*;
use ogos_config as config;
use ogos_err::*;

use com_policy_config::*;
use log::*;
use std::{
    fmt::{self, *},
    fs,
    process::*
};
use windows_061::{
    core::*,
    Win32::{
        Devices::FunctionDiscovery::*,
        Media::Audio::*,
        System::Com::*
    }
};

#[derive(Clone, Copy, PartialEq, PartialOrd)]
pub enum Hz {
    N44100,
    N48000,
    N88200,
    N96000
}
impl Hz {
    fn as_str(&self) -> &str {
        match self {
            Self::N44100 => "44100",
            Self::N48000 => "48000",
            Self::N88200 => "88200",
            Self::N96000 => "96000"
        }
    }
}
impl Display for Hz {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}
impl From<Hz> for u32 {
    fn from(value: Hz) -> Self {
        match value {
            Hz::N44100 => 44100,
            Hz::N48000 => 48000,
            Hz::N88200 => 88200,
            Hz::N96000 => 96000
        }
    }
}
impl TryFrom<&str> for Hz {
    type Error = ErrVar;

    fn try_from(value: &str) -> ResVar<Self> {
        Ok(match value {
            "44100" => Self::N44100,
            "48000" => Self::N48000,
            "88200" => Self::N88200,
            "96000" => Self::N96000,
            _ => Err(ErrVar::FailedHzFrom { from: value.into() })?
        })
    }
}
impl TryFrom<u32> for Hz {
    type Error = ErrVar;

    fn try_from(value: u32) -> ResVar<Self> {
        Ok(match value {
            44100 => Self::N44100,
            48000 => Self::N48000,
            88200 => Self::N88200,
            96000 => Self::N96000,
            _ => Err(ErrVar::FailedHzFrom { from: value.to_string() })?
        })
    }
}

pub trait HzExt {
    fn try_as_hz(&self) -> ResVar<Hz>;
}
impl<T> HzExt for T where
    T: AsRef<str>
{
    fn try_as_hz(&self) -> ResVar<Hz> {
        Hz::try_from(self.as_ref())
    }
}

trait PlayNice {
    unsafe fn set_device_format(&self, device_name: impl Param<PCWSTR>, endpoint_format: *mut WAVEFORMATEX, mix_format: *mut WAVEFORMATEX) -> HRESULT;
}
impl PlayNice for IPolicyConfig {
    unsafe fn set_device_format(&self, device_name: impl Param<PCWSTR>, endpoint_format: *mut WAVEFORMATEX, mix_format: *mut WAVEFORMATEX) -> HRESULT { unsafe {
        (Interface::vtable(self).SetDeviceFormat)(Interface::as_raw(self), device_name.param().abi(), endpoint_format, mix_format)
    } }
}

pub fn set_endpoint(name: &str) -> Res<()> { unsafe {
    CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;

    let set_endpoint_res = (|| -> Res<()> {
        let device_enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device_collection = device_enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;

        for i in 0..device_collection.GetCount()? {
            let device = device_collection.Item(i)?;
            let property_store = device.OpenPropertyStore(STGM_READ)?;
            let device_desc_prop_var = property_store.GetValue(&PKEY_Device_DeviceDesc)?;
            let device_desc = device_desc_prop_var.Anonymous.Anonymous.Anonymous.pwszVal.to_string()?;

            if device_desc == name {
                let device_id = device.GetId()?;

                let policy_config: IPolicyConfig = CoCreateInstance(&PolicyConfigClient, None, CLSCTX_ALL)?;
                policy_config.SetDefaultEndpoint(device_id, eCommunications)?;
                policy_config.SetDefaultEndpoint(device_id, eConsole)?;
                policy_config.SetDefaultEndpoint(device_id, eMultimedia)?;

                return Ok(())
            }
        }

        Err(ErrVar::UnknownEndpoint)?
    })();

    CoUninitialize();

    set_endpoint_res?;

    info!("{}: set endpoint: {}", module_path!(), name);

    let config = config::get().read()?;
    let apps = config.audio.as_ref().and_then(|audio_config| audio_config.endpoint_apps.as_ref());
    if let Some(apps) = apps &&
        let Some(app) = apps.get(name)
    {
        let mut cmd = Command::new(app.path);
        cmd.args(&app.args);

        spawn_command(&mut cmd)?;

        info!("{}: spawned endpoint proc: {}: {:?}", module_path!(), app.path, app.args);
    }

    Ok(())
} }

pub fn set_eq(name: &str) -> Res<()> {
    let config = config::get().read()?;
    let eq_apo = config.audio.as_ref()
        .and_then(|audio_config| audio_config.eq_apo.as_ref())
        .ok_or(ErrVar::MissingConfigOption { name: config::EqApo::NAME })?;

    let custom_config_path = eq_apo.custom_config_paths.get(name).ok_or(ErrVar::UnknownEqApoConfigName)?;

    fs::copy(custom_config_path, eq_apo.master_config_path)?;

    info!("{}: set eq: {}", module_path!(), name);

    Ok(())
}

pub fn set_sample_rate(hz: Hz) -> Res<Option<Hz>> { unsafe {
    CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;

    let prev_hz = (|| -> Res<Option<Hz>> {
        let device_enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = device_enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
        let device_id = PCWSTR(device.GetId()?.0);

        let policy_config: IPolicyConfig = CoCreateInstance(&PolicyConfigClient, None, CLSCTX_ALL)?;
        let device_format = policy_config.GetDeviceFormat(device_id, false)? as *mut WAVEFORMATEXTENSIBLE;
        let mut device_format = *device_format;

        let sample_rate = u32::from(hz);
        let prev_sample_rate = device_format.Format.nSamplesPerSec;

        if sample_rate != prev_sample_rate {
            let prev_hz = Hz::try_from(prev_sample_rate)?;

            device_format.Format.nSamplesPerSec = sample_rate;
            device_format.Format.nAvgBytesPerSec = u32::from(device_format.Format.nChannels) * u32::from(hz) * u32::from(device_format.Format.wBitsPerSample) / 8;

            let device_format_ptr = &mut device_format as *mut _ as *mut WAVEFORMATEX;
            let mix_format_ptr = device_format_ptr;
            policy_config.set_device_format(device_id, device_format_ptr, mix_format_ptr).ok()?;

            info!("{}: set sample rate: {}", module_path!(), hz);

            return Ok(Some(prev_hz))
        }

        Ok(None)
    })();

    CoUninitialize();

    prev_hz
} }
