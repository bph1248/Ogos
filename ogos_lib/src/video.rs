use crate::{
    audio::*,
    common::*,
    config::{self, *},
    display::*,
    err::*
};

use log::*;
use ini::*;
use serde::{
    de::*,
    *
};
use std::{
    fmt,
    fs::{self, *},
    io,
    os::{self, windows::process::CommandExt},
    path::*,
    process::*,
    string::*
};
use windows::Win32::Foundation::*;

const MAINTAIN_SAMPLE_RATE_GUARD_FILE_NAME: &str = "maintain_sample_rate.guard";
const NA_STR: &str = "<n/a>";

fn deserialize_side_data_list<'de, D>(deserializer: D) -> Result<SideData, D::Error> where
    D: Deserializer<'de>
{
    struct SideDataListVisitor;

    impl<'de> Visitor<'de> for SideDataListVisitor {
        type Value = SideData;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("side_data_list")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error> where
            A: SeqAccess<'de>
        {
            let mut side_data = SideData::default();
            loop {
                match seq.next_element::<SideDataListElement>() {
                    Ok(Some(SideDataListElement::ContentLightLevel { max_content })) => side_data.max_content = Some(max_content),
                    Ok(Some(SideDataListElement::DolbyVision)) => side_data.is_dolby_vision = true,
                    Ok(None) => break,
                    _ => ()
                }
            }

            Ok(side_data)
        }
    }

    let side_data = deserializer.deserialize_seq(SideDataListVisitor {})?;

    Ok(side_data)
}

fn deserialize_packets_and_frames<'de, D>(deserializer: D) -> Result<SideData, D::Error> where
    D: Deserializer<'de>
{
    struct PacketsAndFramesVisitor;

    impl<'de> Visitor<'de> for PacketsAndFramesVisitor {
        type Value = SideData;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("packets_and_frames")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error> where
            A: SeqAccess<'de>
        {
            let mut side_data = SideData::default();
            loop {
                match seq.next_element::<PacketFrame>() {
                    Ok(Some(PacketFrame::Frame { side_data: side_data_ })) => side_data = side_data_,
                    Ok(None) => break,
                    _ => ()
                }
            }

            Ok(side_data)
        }
    }

    let side_data = deserializer.deserialize_seq(PacketsAndFramesVisitor {})?;

    Ok(side_data)
}

fn deserialize_streams<'de, D>(deserializer: D) -> Result<Streams, D::Error> where
    D: Deserializer<'de>
{
    struct StreamsVisitor;

    impl<'de> Visitor<'de> for StreamsVisitor {
        type Value = Streams;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("streams")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error> where
            A: SeqAccess<'de>
        {
            let mut streams = Streams::default();
            loop {
                match seq.next_element::<Stream>() {
                    Ok(Some(Stream::Video(video_stream))) => streams.video = video_stream,
                    Ok(Some(Stream::Audio(audio_stream))) => streams.audio = audio_stream,
                    Ok(None) => break,
                    _ => ()
                }
            }

            Ok(streams)
        }
    }

    let streams = deserializer.deserialize_seq(StreamsVisitor {})?;

    Ok(streams)
}

#[derive(Default, Deserialize)]
struct SideData {
    max_content: Option<u32>,
    is_dolby_vision: bool
}

fn color_transfer() -> String { "bt.709".into() }

#[derive(Clone, Default, Deserialize)]
struct VideoStream {
    #[serde(default = "color_transfer")]
    color_transfer: String,
    bits_per_raw_sample: Option<String>
}

#[derive(Clone, Default, Deserialize)]
struct AudioStream {
    sample_rate: Option<String>
}

#[derive(Default, Deserialize)]
struct Streams {
    video: VideoStream,
    audio: AudioStream
}

#[derive(Deserialize)]
struct Ffprobe {
    #[serde(rename = "packets_and_frames", deserialize_with = "deserialize_packets_and_frames")]
    side_data: SideData,
    #[serde(deserialize_with = "deserialize_streams")]
    streams: Streams
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase", tag = "type")]
enum PacketFrame {
    Packet,
    Frame {
        #[serde(rename = "side_data_list", deserialize_with = "deserialize_side_data_list")]
        side_data: SideData
    }
}

#[derive(Deserialize)]
#[serde(tag = "side_data_type")]
enum SideDataListElement {
    #[serde(rename = "Content light level metadata")]
    ContentLightLevel { max_content: u32 },
    #[serde(rename = "Dolby Vision Metadata")]
    DolbyVision
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase", tag = "codec_type")]
enum Stream {
    Video(VideoStream),
    Audio(AudioStream)
}

#[derive(PartialEq)]
pub(crate) enum MaintainSampleRate {
    #[allow(dead_code)]
    No,
    Yes,
    CheckGuard
}
impl From<bool> for MaintainSampleRate {
    fn from(value: bool) -> Self {
        match value {
            true => Self::Yes,
            false => Self::CheckGuard
        }
    }
}

enum MpvArg<'a> {
    GlslShaders(&'a String),
    Profile(&'a String)
}
impl MpvArg<'_> {
    fn to_arg_string(&self) -> String {
        let mut arg;

        match self {
            Self::GlslShaders(shaders) => {
                arg = "--glsl-shaders=".to_string();
                arg.push_str(shaders);
            },
            Self::Profile(profile) => {
                arg = "--profile=".to_string();
                arg.push_str(profile);
            }
        }

        arg
    }
}

pub(crate) unsafe fn create_maintain_sample_rate_guard() -> io::Result<()> {
    let guard_path = CURRENT_EXE_PARENT_PATH.get_unchecked().join(MAINTAIN_SAMPLE_RATE_GUARD_FILE_NAME);

    fs::write(&guard_path, "")?;
    info!("{}: created maintain-sample-rate guard: {:?}", module_path!(), guard_path);

    Ok(())
}

pub(crate) unsafe fn launch_mpv(vid_path: &Path, maintain_sample_rate: MaintainSampleRate, override_glsl_shaders: bool) -> Res<(), { loc_var!(Mpv) }> {
    let inner = |revert_to: &mut Vec<VideoSetting>| -> Res<(), { loc_var!(Mpv) }> {
        let config = config::get().read()?;
        let mpv_config = config.mpv.as_ref().ok_or(ErrVar::MissingConfigKey { name: config::Mpv::NAME })?;

        let ffprobe_path = find_or_confirm_app(App::FFPROBE, config.app_paths.ffprobe.as_ref())?;
        let mpv_path = find_or_confirm_app(App::MPV, config.app_paths.mpv.as_ref())?;

        let mut cmd = Command::new(mpv_path.as_path());
        let mut args = vec![];

        let mut ffprobe_cmd = Command::new(&ffprobe_path);
        ffprobe_cmd.args(["-v", "quiet", "-read_intervals", "%+#1", "-show_entries", "stream=codec_type,bits_per_raw_sample,sample_rate,color_transfer:side_data=side_data_type,max_content", "-of", "json"])
            .arg(vid_path)
            .creation_flags(CREATE_NO_WINDOW);
        let output = output_command(&mut ffprobe_cmd)?;
        let output = String::from_utf8(output.stdout)?;
        let ffprobe = serde_json::from_str::<Ffprobe>(output.as_ref())?;

        // Sample rate
        let guard_path = CURRENT_EXE_PARENT_PATH.get_unchecked().join(MAINTAIN_SAMPLE_RATE_GUARD_FILE_NAME);
        let maintain_sample_rate = match maintain_sample_rate {
            MaintainSampleRate::No => false,
            MaintainSampleRate::Yes => true,
            MaintainSampleRate::CheckGuard => File::open(&guard_path).is_ok()
        };

        let vid_sample_rate = match ffprobe.streams.audio.sample_rate.as_ref() {
            Some(vid_sample_rate) => {
                if !maintain_sample_rate  {
                    set_sample_rate(vid_sample_rate.try_as_hz()?)
                        .inspect(|prev| {
                            if let Some(prev) = prev { revert_to.push(VideoSetting::SampleRate(*prev)); }
                        })?;
                }

                vid_sample_rate.as_str()
            },
            None => NA_STR
        };
        info!("{}: sample rate: {}", module_path!(), vid_sample_rate);

        // Color transfer
        let vid_color_transfer = &ffprobe.streams.video.color_transfer;
        info!("{}: color transfer: {}", module_path!(), vid_color_transfer);

        // GLSL shaders
        if override_glsl_shaders && let Some(glsl_shaders) = mpv_config.override_glsl_shaders.as_ref() {
            cmd.arg(MpvArg::GlslShaders(glsl_shaders).to_arg_string());
        } else if let Some(glsl_shaders) = mpv_config.default_glsl_shaders.as_ref() {
            cmd.arg(MpvArg::GlslShaders(glsl_shaders).to_arg_string());
        }

        // Display mode
        let reshade_config = mpv_config.reshade.as_ref();
        let profile_arg;
        let set_display_mode_op;

        let mut disable_reshade = || -> Res1<()> {
            if let Some(reshade_config) = reshade_config {
                let layer_file_string = fs::read_to_string(reshade_config.layer_path.as_str())?;
                let root_value = serde_json::from_str::<serde_json::Value>(&layer_file_string)?;

                let reshade_vk_layer_disable_env_key = root_value.get("layer")
                    .and_then(|value| {
                        value.get("disable_environment")
                    })
                    .and_then(|value| {
                        value.as_object()
                    })
                    .and_then(|obj| {
                        obj.keys().find(|key| key.starts_with("DISABLE_"))
                    })
                    .ok_or(ErrVar::MissingReShadeVkLayerDisableEnvKey)?;

                cmd.env(reshade_vk_layer_disable_env_key, "1");
            }

            Ok(())
        };

        match vid_color_transfer == "smpte2084" || ffprobe.side_data.is_dolby_vision {
            true => {
                match (reshade_config, ffprobe.side_data.max_content) {
                    (Some(reshade_config), Some(max_content)) if max_content > 0 => { // Statically tone map with ReShade
                        // Check ReShade.ini exists as symlink in mpv dir. Link from ProgramData if it's missing (ie. due to scoop update)
                        let mpv_parent_path = mpv_path.get_parent()?;
                        let reshade_settings_sym_link_path = mpv_parent_path.join("ReShade.ini");

                        if reshade_settings_sym_link_path.as_path().confirm().is_err() {
                            // Either the symlink doesn't exist or its target doesn't exist. In case the symlink exists but is broken, remove it
                            if let Err(err) = fs::remove_file(&reshade_settings_sym_link_path) &&
                                err.kind() != io::ErrorKind::NotFound
                            {
                                Err(err)?;
                            }

                            // Check ReShade.ini exists before symlinking
                            Path::new(&reshade_config.settings_path).confirm()?;

                            if let Err(err) = os::windows::fs::symlink_file(&reshade_config.settings_path, &reshade_settings_sym_link_path) {
                                let mut question = String::new();
                                if let Some(ERROR_PRIVILEGE_NOT_HELD) = err.raw_os_error().map(|code| WIN32_ERROR(code as u32)) {
                                    question.push_str(" Is developer mode enabled?");
                                }

                                warn!("{}: failed to symlink {} to {}.{} Copying file instead", module_path!(), Path::new(&reshade_config.settings_path).to_string_lossy(), reshade_settings_sym_link_path.to_string_lossy(), question);

                                fs::copy(&reshade_config.settings_path, &reshade_settings_sym_link_path)?;
                            }
                        }

                        // Max luminance
                        info!("{}: max luminance: {}", module_path!(), max_content);

                        // Write max luminance to preset
                        let reshade_preset_path = Path::new(&reshade_config.preset_path);
                        let mut reshade_preset_ini = Ini::load_from_file(reshade_preset_path).map_err(|err| ErrVar::FailedIniOp { inner: err, path: reshade_config.preset_path.clone() })?;
                        reshade_preset_ini.with_section(Some("lilium__tone_mapping.fx")).set("InputLuminanceMax", max_content.to_string());
                        reshade_preset_ini.write_to_file(reshade_preset_path).map_err(|err| ErrVar::FailedWriteFile { inner: err, path: reshade_config.preset_path.clone() })?;

                        profile_arg = MpvArg::Profile(&reshade_config.profile).to_arg_string();
                    },
                    _ => { // Let mpv handle tone mapping
                        disable_reshade()?;

                        profile_arg = MpvArg::Profile(&mpv_config.hdr_profile).to_arg_string();
                    }
                }

                set_display_mode_op = SetDisplayModeOp::Set(DisplayMode::Hdr);
            },
            false => {
                disable_reshade()?;

                match vid_color_transfer.as_ref() {
                    "arib-std-b67" => { // HLG
                        profile_arg = MpvArg::Profile(&mpv_config.hdr_profile).to_arg_string();
                        set_display_mode_op = SetDisplayModeOp::Set(DisplayMode::Hdr);
                    },
                    _ => { // SDR
                        profile_arg = MpvArg::Profile(&mpv_config.sdr_profile).to_arg_string();
                        set_display_mode_op = SetDisplayModeOp::Set(DisplayMode::Sdr);

                        // Bit depth / novideo_srgb optimization
                        if let Some(vid_bit_depth) = ffprobe.streams.video.bits_per_raw_sample.filter(|depth| depth == "10").as_ref() {
                            info!("{}: bit depth: {}", module_path!(), vid_bit_depth);

                            if let Some(info) = config.display_modes.as_ref().and_then(|display_modes| display_modes.sdr.novideo_srgb.as_ref()) {
                                let info = NovideoSrgbInfo {
                                    enable_optimization: false,
                                    ..info.clone()
                                };

                                control_novideo_srgb(&info).map(|_| {
                                    let prev = info.clone();

                                    revert_to.push(VideoSetting::NovideoSrgb(prev));
                                })?;
                            }
                        }
                    }
                }
            }
        }

        drop(config);

        set_display_mode(set_display_mode_op)
            .map(|prev| {
                if let Some(prev) = prev { revert_to.push(VideoSetting::DisplayMode(prev)) }
            })?;

        // Build cmd and launch
        args.push(profile_arg.as_str());
        cmd.args(args).arg(vid_path);

        info!("{}: launching: {}", module_path!(), cmd.to_string());
        output_command(&mut cmd)?;

        if fs::remove_file(guard_path).is_ok() {
            info!("{}: removed: {:?}", module_path!(), MAINTAIN_SAMPLE_RATE_GUARD_FILE_NAME);
        }

        Ok(())
    };

    let mut revert_to: Vec<VideoSetting> = Vec::new();
    let res = inner(&mut revert_to);

    for setting in revert_to.into_iter().rev() {
        (|| -> Res<()> {
            match setting {
                VideoSetting::DisplayMode(display_mode) => {
                    set_display_mode(SetDisplayModeOp::Set(display_mode))?;
                },
                VideoSetting::NovideoSrgb(info) => {
                    control_novideo_srgb(&info)?;
                },
                VideoSetting::SampleRate(hz) => {
                    set_sample_rate(hz)?;
                }
            }

            Ok(())
        })()
        .unwrap_or_else(|err| {
            error!("{}: failed to revert setting: {}", module_path!(), err);
        });
    }

    res
}
