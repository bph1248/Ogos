use ogos_audio::*;
use ogos_common::*;
use ogos_config as config;
use config::*;
use ogos_core::*;
use ogos_display::*;
use ogos_err::*;

use concat_string::*;
use log::*;
use serde::{
    de::*,
    *
};
use std::{
    fmt,
    fs::{self, *},
    io,
    os::{windows::process::CommandExt},
    path::*,
    process::*,
    string::*
};
use windows::Win32::System::Threading::*;

const MAINTAIN_SAMPLE_RATE_GUARD_FILE_NAME: &str = "maintain_sample_rate.guard";
const NA_STR: &str = "<n/a>";

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
            let mut video_stream = None;
            let mut audio_stream = None;
            loop {
                match seq.next_element::<Stream>() {
                    Ok(Some(Stream::Video(v))) if video_stream.is_none() => video_stream = Some(v),
                    Ok(Some(Stream::Audio(a))) if audio_stream.is_none() => audio_stream = Some(a),
                    Ok(None) => break,
                    _ => ()
                }
            }

            let streams = Streams {
                video: video_stream.unwrap_or_default(),
                audio: audio_stream.unwrap_or_default()
            };

            Ok(streams)
        }
    }

    let streams = deserializer.deserialize_seq(StreamsVisitor {})?;

    Ok(streams)
}

fn color_transfer() -> String { "bt.709".into() }

#[derive(Clone, Default, Deserialize)]
struct VideoStream {
    #[serde(default = "color_transfer")]
    color_transfer: String
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
    #[serde(deserialize_with = "deserialize_streams")]
    streams: Streams
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase", tag = "codec_type")]
enum Stream {
    Video(VideoStream),
    Audio(AudioStream)
}

#[derive(PartialEq)]
pub enum MaintainSampleRate {
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
    GlslShaders(&'a str),
    Profile(&'a str)
}
impl MpvArg<'_> {
    fn to_arg_string(&self) -> String {
        match self {
            Self::GlslShaders(shaders) => {
                concat_string!("--glsl-shaders=", shaders)
            },
            Self::Profile(profile) => {
                concat_string!("--profile=", profile)
            }
        }
    }
}

enum Setting {
    DisplayMode(DisplayMode),
    SampleRate(Hz)
}

pub fn create_maintain_sample_rate_guard() -> io::Result<()> {
    let guard_path = unsafe { CURRENT_EXE_DIR.get_unchecked().join(MAINTAIN_SAMPLE_RATE_GUARD_FILE_NAME) };

    fs::write(&guard_path, "")?;
    info!("{}: created maintain-sample-rate guard: {}", module_path!(), guard_path.display());

    Ok(())
}

pub fn launch_mpv(vid_path: &Path, maintain_sample_rate: MaintainSampleRate, override_glsl_shaders: bool) -> Res<(), { loc_var!(Mpv) }> {
    let inner = |revert_to: &mut Vec<Setting>| -> Res<(), { loc_var!(Mpv) }> {
        let config = config::get().read()?;
        let mpv_config = config.mpv.as_ref().ok_or(ErrVar::MissingConfigOption { name: config::Mpv::NAME })?;

        let ffprobe_path = confirm_or_find_app(config.app_paths.ffprobe.as_ref(), App::FFPROBE)?;
        let mpv_path = confirm_or_find_app(config.app_paths.mpv.as_ref(), App::MPV)?;

        let mut cmd = Command::new(mpv_path.as_ref());
        let mut args = vec![];

        let mut ffprobe_cmd = Command::new(ffprobe_path.as_ref());
        ffprobe_cmd.args(["-v", "quiet", "-read_intervals", "%+#1", "-show_entries", "stream=codec_type,sample_rate,color_transfer:side_data=side_data_type,max_content", "-of", "json"])
            .arg(vid_path)
            .creation_flags(CREATE_NO_WINDOW.0);
        let output = output_command(&mut ffprobe_cmd)?;
        let output = String::from_utf8(output.stdout)?;
        let ffprobe = serde_json::from_str::<Ffprobe>(output.as_str())?;

        // Sample rate
        let guard_path = unsafe { CURRENT_EXE_DIR.get_unchecked().join(MAINTAIN_SAMPLE_RATE_GUARD_FILE_NAME) };
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
                            if let Some(prev) = prev { revert_to.push(Setting::SampleRate(*prev)); }
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
        let profile_arg;
        let set_display_mode_op;

        match vid_color_transfer.as_str() {
            "smpte2084" | "arib-std-b67" => {
                profile_arg = MpvArg::Profile(mpv_config.hdr_profile).to_arg_string();
                set_display_mode_op = SetDisplayModeOp::Set(DisplayMode::Hdr);
            },
            _ => {
                profile_arg = MpvArg::Profile(mpv_config.sdr_profile).to_arg_string();
                set_display_mode_op = SetDisplayModeOp::Set(DisplayMode::Sdr);
            }
        }

        drop(config);

        set_display_mode(set_display_mode_op)
            .map(|prev| {
                if let Some(prev) = prev { revert_to.push(Setting::DisplayMode(prev)) }
            })?;

        // Build cmd and launch
        args.push(profile_arg.as_str());
        cmd.args(args).arg(vid_path);

        info!("{}: launching: {}", module_path!(), cmd.as_display());
        output_command(&mut cmd)?;

        if fs::remove_file(guard_path).is_ok() {
            info!("{}: removed: {}", module_path!(), MAINTAIN_SAMPLE_RATE_GUARD_FILE_NAME);
        }

        Ok(())
    };

    let mut revert_to: Vec<Setting> = Vec::new();
    let res = inner(&mut revert_to);

    while let Some(setting) = revert_to.pop() {
        (|| -> Res<()> {
            match setting {
                Setting::DisplayMode(display_mode) => {
                    set_display_mode(SetDisplayModeOp::Set(display_mode))?;
                },
                Setting::SampleRate(hz) => {
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
