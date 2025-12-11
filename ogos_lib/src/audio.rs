use crate::{
    common::*,
    config,
    err::*
};

use com_policy_config::*;
use log::*;
use std::{
    fmt::{self, *},
    fs,
    process::Command
};
use windows_052::{
    core::*,
    Win32::{
        Devices::FunctionDiscovery::*,
        Media::Audio::*,
        System::Com::*
    }
};

#[derive(Clone, Copy, PartialEq, PartialOrd)]
pub(crate) enum Hz {
    N44100,
    N48000,
    N88200,
    N96000
}
impl Hz {
    pub(crate) fn as_str(&self) -> &str {
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
        Ok(
            match value {
                "44100" => Self::N44100,
                "48000" => Self::N48000,
                "88200" => Self::N88200,
                "96000" => Self::N96000,
                _ => Err(ErrVar::FailedAsHz { from: value.into() })?
            }
        )
    }
}
impl TryFrom<u32> for Hz {
    type Error = ErrVar;

    fn try_from(value: u32) -> ResVar<Self> {
        Ok(
            match value {
                44100 => Self::N44100,
                48000 => Self::N48000,
                88200 => Self::N88200,
                96000 => Self::N96000,
                _ => Err(ErrVar::FailedAsHz { from: value.to_string() })?
            }
        )
    }
}

pub(crate) trait HzExt {
    fn try_as_hz(&self) -> ResVar<Hz>;
}
impl<T> HzExt for T where
    T: AsRef<str>
{
    fn try_as_hz(&self) -> ResVar<Hz> {
        Hz::try_from(self.as_ref())
    }
}

pub(crate) unsafe fn set_endpoint(name: &str) -> Res1<()> {
    let config = config::get().read()?;
    let audio_config = config.audio.as_ref().ok_or(ErrVar::MissingConfigKey { name: config::Audio::NAME })?;

    let endpoints = audio_config.endpoints.as_ref().ok_or(ErrVar::MissingConfigKey { name: config::Endpoints::NAME })?;
    let prog = endpoints.0.get(name);

    CoInitializeEx(None, COINIT_MULTITHREADED)?;

    {
        let device_enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device_collection = device_enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;

        for i in 0..device_collection.GetCount()? {
            let device = device_collection.Item(i)?;
            let property_store = device.OpenPropertyStore(STGM_READ)?;
            let device_desc_prop_var = property_store.GetValue(&PKEY_Device_DeviceDesc)?;
            let device_desc = device_desc_prop_var.Anonymous.Anonymous.Anonymous.pwszVal.to_string()?;

            if device_desc == name {
                let device_id = PCWSTR(device.GetId()?.0);

                let policy_config: IPolicyConfig = CoCreateInstance(&PolicyConfigClient, None, CLSCTX_ALL)?;
                policy_config.SetDefaultEndpoint(device_id, eMultimedia)?;

                break
            }
        }
    }

    CoUninitialize();

    info!("{}: set endpoint: {}", module_path!(), name);

    if let Some(app) = prog {
        let mut cmd = Command::new(&app.path);
        cmd.args(&app.args);

        spawn_command(&mut cmd)?;

        info!("{}: spawned endpoint proc: {}: {:?}", module_path!(), app.path, app.args);
    }

    Ok(())
}

pub(crate) unsafe fn set_sample_rate(hz: Hz) -> Res1<Option<Hz>> {
    CoInitializeEx(None, COINIT_MULTITHREADED)?;

    let prev_sample_rate = (|| -> Res<Option<Hz>> {
        let device_enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = device_enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia)?;
        let device_id = PCWSTR(device.GetId()?.0);

        let policy_config: IPolicyConfig = CoCreateInstance(&PolicyConfigClient, None, CLSCTX_ALL)?;
        let device_format = policy_config.GetDeviceFormat(device_id, false)? as *mut WAVEFORMATEXTENSIBLE;
        let mut device_format = *device_format;

        let sample_rate = u32::from(hz);
        let prev_sample_rate = device_format.Format.nSamplesPerSec;

        match sample_rate == prev_sample_rate {
            true => Ok(None),
            false => {
                device_format.Format.nSamplesPerSec = sample_rate;
                device_format.Format.nAvgBytesPerSec = u32::from(device_format.Format.nChannels) * u32::from(hz) * u32::from(device_format.Format.wBitsPerSample) / 8;

                let mix_format_ptr = &mut device_format as *mut _ as *mut WAVEFORMATEX;
                (Interface::vtable(&policy_config).SetDeviceFormat)(Interface::as_raw(&policy_config), device_id.into_param().abi(), mix_format_ptr, mix_format_ptr).ok()?;

                info!("{}: set sample rate: {}", module_path!(), hz);

                Ok(Some(Hz::try_from(prev_sample_rate)?))
            }
        }
    })();

    CoUninitialize();

    Ok(prev_sample_rate?)
}

pub(crate) fn set_eq(name: &str) -> Res1<()> {
    let config = config::get().read()?;
    let audio_config = config.audio.as_ref().ok_or(ErrVar::MissingConfigKey { name: config::Audio::NAME })?;
    let eq_apo = audio_config.eq_apo.as_ref().ok_or(ErrVar::MissingConfigKey { name: config::EqApo::NAME })?;

    let custom_config_path = eq_apo.custom_config_paths.get(name).ok_or_else(|| { ErrVar::UnknownEq { name: name.into() } })?;

    fs::copy(custom_config_path, eq_apo.master_config_path.as_str())?;

    info!("{}: set eq: {}", module_path!(), name);

    Ok(())
}
