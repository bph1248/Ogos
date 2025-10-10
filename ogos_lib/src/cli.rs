use crate::{
    err::*
};

use clap::*;
use const_format::*;
use std::{
    env,
    ffi::*,
    path::*
};

const CURSOR_SIZE: &str = "cursor-size";
const ENDPOINT: &str = "endpoint";
const EQ: &str = "eq";
const GAME: &str = "game";
#[derive(Args)]
#[group(requires = GAME, multiple = true)]
pub(crate) struct Gaming {
    #[arg(long, name = CURSOR_SIZE, help = concatcp!("Set the cursor size before launching a game. Requires --", GAME, ". Reverts on game exit"))]
    pub(crate) cursor_size: Option<usize>,
    #[arg(long = "hdr", help = concatcp!("Switch to HDR mode before launching a game. Requires --", GAME, ". Reverts on game exit"))]
    pub(crate) set_display_mode_hdr: bool,
    #[arg(long = "res", help = concatcp!("Set the desktop resolution before launching a game. Requires --", GAME, ". Reverts on game exit"))]
    pub(crate) set_res: bool,
    #[arg(long, help = concatcp!("Affinitize a game process to run on every other hardware thread (ie. soft-disable SMT/Hyperthreading). Requires --", GAME, ". Reverts on game exit"))]
    pub(crate) stagger: bool,
    #[arg(long = "special-k", alias = "sk", help = concatcp!("Launch a game via SKIF (Special K Injection Frontend). Requires --", GAME))]
    pub(crate) use_special_k: bool
}

#[derive(Parser)]
#[command(name = "ogos", arg_required_else_help = true, disable_help_flag = true)]
pub(crate) struct Cli {
    #[arg(short, long, hide = true)]
    pub(crate) help: bool,

    #[arg(help =
        "Launch a file, open the media launcher, or parse an arg file.\n\
        \n\
        If PATH is a video file, launch it with Mpv, else forward it to its default file handler. Valid video file extensions are m2ts, mkv, mp4, mts, ts, and webm.\n\
        If ffprobe is available, switch display mode and set the sample rate of the default audio endpoint to match video metadata.\n\
        \n\
        If PATH is a directory, open the media launcher on that directory."
    )]
    pub(crate) path: Option<String>,

    #[arg(long)]
    pub(crate) lib: bool,

    #[arg(long, help = "Prevent setting the sample rate of the default audio endpoint to match video metadata")]
    pub(crate) maintain_sample_rate: bool,
    #[arg(long = ENDPOINT, name = ENDPOINT, help = "Set the default audio endpoint")]
    pub(crate) set_endpoint: Option<String>,
    #[arg(long = EQ, name = EQ, help = "Set the current Equalizer APO config")]
    pub(crate) set_eq: Option<String>,
    #[arg(long, help = "Toggle display mode and set color bit depth, dither state, and novideo_srgb state")]
    pub(crate) toggle_display_mode: bool,

    #[arg(long = GAME, name = GAME, help = "Launch a game")]
    pub(crate) launch_game: Option<String>,
    #[command(flatten)]
    pub(crate) gaming: Gaming,
    //
    #[arg(long, help = "Enable global hotkeys and dynamic keymaps")]
    pub(crate) binds: bool,

    #[arg(long, help =
        "Manage taskbar visibility by monitoring collisions between the mouse cursor and an invisible, always-on-top window, or 'hitbox'.\n\
        If the foreground window is full screen, the hitbox is disabled"
    )]
    pub(crate) taskbar: bool,
    #[arg(long, help = "Periodically 'pixel-shift' desktop windows. Shifting is disabled if the foreground window is full screen, or the left mouse button is held down")]
    pub(crate) window_shift: bool
}

pub(crate) enum CliPathKind {
    ArgFile,
    Media
}

pub(crate) fn parse_cli() -> Res<(Cli, CliPathKind)> {
    let mut args_os = env::args_os();

    if args_os.len() > 1 {
        let probably_current_exe_path = args_os.next().unwrap();
        let maybe_arg_file_path = args_os.next().unwrap();

        if Path::new(&maybe_arg_file_path).extension()
            .filter(|ext| {
                *ext == "ogos"
            })
            .is_some()
        {
            let arg_file_path = maybe_arg_file_path; // No longer a maybe
            let prefixed_arg_file_path = OsString::from( // Let the expander know this is an arg file
                format!("{}{}", argfile::PREFIX, arg_file_path.to_str().ok_or(ErrVar::FailedToStr)?)
            );
            let head = [probably_current_exe_path, arg_file_path, prefixed_arg_file_path];
            let unexpanded_args = head.iter().cloned().chain(args_os); // Chain remaining os args onto head
            let expanded_args = argfile::expand_args_from(unexpanded_args, argfile::parse_fromfile, argfile::PREFIX).unwrap();

            let cli = Cli::try_parse_from(expanded_args)?;

            return Ok((cli, CliPathKind::ArgFile))
        }
    }

    let cli = Cli::try_parse()?;

    Ok((cli, CliPathKind::Media))
}
