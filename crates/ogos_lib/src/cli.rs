use ogos_err::*;

use clap::*;
use const_format::*;
use std::{
    env,
    path::*
};

const ENDPOINT: &str = "endpoint";
const EQ: &str = "eq";
const GAME: &str = "game";
const STATE: &str = "state";

#[derive(Clone, ValueEnum)]
pub(crate) enum NovideoSrgbOp {
    On,
    Off
}
#[derive(Args)]
#[group(requires = GAME, multiple = true)]
pub(crate) struct Gaming {
    #[arg(long = "cursor-size", visible_alias = "cursor", help = concatcp!("Set the cursor size on game launch. Requires --", GAME, ". Reverts on exit."))]
    pub(crate) set_cursor_size: bool,
    #[arg(long = "hdr", help = concatcp!("Switch to HDR mode on game launch. Requires --", GAME, ". Reverts on exit."))]
    pub(crate) set_display_mode_hdr: bool,
    #[arg(long = "res", help = concatcp!("Set the desktop resolution on game launch. Requires --", GAME, ". Reverts on exit."))]
    pub(crate) set_res: bool,
    #[arg(long, help = concatcp!("Affinitize a game process to run on every other hardware thread (i.e. soft-disable SMT/Hyperthreading). Requires --", GAME, "."))]
    pub(crate) stagger: bool,
    #[arg(long = "special-k", visible_alias = "sk", help = concatcp!("Launch a game via SKIF (Special K Injection Frontend). Requires --", GAME, "."))]
    pub(crate) use_special_k: bool
}

#[derive(Parser)]
#[command(name = "ogos", arg_required_else_help = true, disable_help_flag = true)]
pub(crate) struct Cli {
    #[arg(short, long, hide = true)]
    pub(crate) help: bool,

    #[arg(help =
        "Parse an arg file or launch a video with mpv.\n\
        \n\
        Arg files follow Python's fromfile format and use the .ogos extension.
        \n\
        Valid video file extensions are m2ts, mkv, mp4, mts, ts, and webm. On launch, set display mode, default audio endpoint sample rate, and tone mapping parameters to match video metadata."
    )]
    pub(crate) path: Option<String>,

    #[arg(long, help = "Launch the media browser.")]
    pub(crate) media_browser: bool,

    #[arg(long, help = "Temporarily prevent setting the default audio endpoint sample rate to match video metadata.")]
    pub(crate) maintain_sample_rate: bool,
    #[arg(long = ENDPOINT, name = "device", help = concatcp!("Set the default audio endpoint device, where <device> is listed in System > Sound > Output."))]
    pub(crate) set_endpoint: Option<String>,
    #[arg(long = EQ, name = EQ, help = concatcp!("Overwrite the master Equalizer APO config file, where <", EQ, "> is a config-defined custom config path."))]
    pub(crate) set_eq: Option<String>,
    #[arg(long, help = "Enable/disable HDR mode and set color bit depth, dither state, and novideo_srgb state.")]
    pub(crate) toggle_display_mode: bool,
    #[arg(long, name = STATE, visible_alias = "clamp", hide_possible_values = true, help = concatcp!("Set novideo_srgb's color space clamp, where <", STATE, "> is either on or off."))]
    pub(crate) novideo_srgb: Option<NovideoSrgbOp>,

    #[arg(long = GAME, name = GAME, help = concatcp!("Launch a game, where <", GAME, "> is a config-defined set of launch parameters and additional settings."))]
    pub(crate) launch_game: Option<String>,
    #[command(flatten)]
    pub(crate) gaming: Gaming,
    //
    #[arg(long, help = "Enable global hotkeys and dynamic keymaps.")]
    pub(crate) binds: bool,

    #[arg(long, help =
        "Manage taskbar visibility by monitoring cursor collisions against an invisible window or 'hitbox'.\n\
        The hitbox is disabled if the foreground window is full screen."
    )]
    pub(crate) taskbar: bool,
    #[arg(long, help = "Periodically 'pixel-shift' windows about the desktop. Shifting is disabled if the foreground window is full screen, or the left mouse button is held down.")]
    pub(crate) window_shift: bool
}

pub(crate) enum CliPathKind {
    ArgFile,
    Media
}

pub(crate) fn parse_cli() -> Res<(Cli, CliPathKind)> {
    let mut args = env::args();

    if args.len() > 1 {
        let probably_current_exe_path = args.next().unwrap();
        let maybe_arg_file_path = args.next().unwrap();

        if let Some(ext) = Path::new(&maybe_arg_file_path).extension() && ext == "ogos" {
            let arg_file_path = maybe_arg_file_path;
            let prefixed_arg_file_path = format!("{}{}", argfile::PREFIX, arg_file_path); // Let the expander know this is an arg file
            let head = [probably_current_exe_path, arg_file_path, prefixed_arg_file_path];
            let unexpanded_args = head.iter().cloned().chain(args); // Chain remaining args onto head
            let expanded_args = argfile::expand_args_from(unexpanded_args.map(|arg| arg.into()), argfile::parse_fromfile, argfile::PREFIX).unwrap();

            let cli = Cli::try_parse_from(expanded_args)?;

            return Ok((cli, CliPathKind::ArgFile))
        }
    }

    let cli = Cli::try_parse()?;

    Ok((cli, CliPathKind::Media))
}
