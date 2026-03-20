use ogos_err::*;

use clap::*;
use const_format::*;
use std::{
    env,
    path::*
};

const ALIAS: &str = "alias";
const BINDS: &str = "binds";
const CLAMP: &str = "clamp";
const CONFLICTS_WITH: &str = "conflicts with";
const CURSOR: &str = "cursor";
const DEVICE: &str = "device";
const ENDPOINT: &str = "endpoint";
const EQ: &str = "eq";
const GAME: &str = "game";
const LONG_LIVED: [&str; 3] = [BINDS, TASKBAR, WINDOW_SHIFT];
const SK: &str = "sk";
const STATE: &str = "state";
const TASKBAR: &str = "taskbar";
const WINDOW_SHIFT: &str = "window-shift";

const ALIAS_CLAMP: &str = formatcp!("{a}: --{b}", a = ALIAS, b = CLAMP);
const ALIAS_CURSOR: &str = formatcp!("{a}: --{b}", a = ALIAS, b = CURSOR);
const ALIAS_SK: &str = formatcp!("{a}: --{b}", a = ALIAS, b = SK);
const CONFLICTS_WITH_LONG_LIVED: &str = formatcp!("{a}: <{b}, {c}, {d}>", a = CONFLICTS_WITH, b = BINDS, c = TASKBAR, d = WINDOW_SHIFT);
const POSSIBLE_VALUES_CLAMP: &str = "possible values: <on, off>";
const REQUIRES_GAME: &str = formatcp!("requires: --{a}", a = GAME);

#[derive(Clone, ValueEnum)]
pub(crate) enum NovideoSrgbOp {
    On,
    Off
}
#[derive(Args)]
#[group(requires = GAME, multiple = true)]
pub(crate) struct Gaming {
    #[arg(long = "cursor-size", alias = CURSOR, help = formatcp!("Set the cursor size on game launch. Reverts on exit. [{a}, {b}]", a = ALIAS_CURSOR, b = REQUIRES_GAME))]
    pub(crate) set_cursor_size: bool,
    #[arg(long = "hdr", help = formatcp!("Switch to HDR mode on game launch. Reverts on exit. [{a}]", a = REQUIRES_GAME))]
    pub(crate) set_display_mode_hdr: bool,
    #[arg(long = "res", help = formatcp!("Set the desktop resolution on game launch. Reverts on exit. [{a}]", a = REQUIRES_GAME))]
    pub(crate) set_res: bool,
    #[arg(long, help = formatcp!("Affinitize a game process to run on every other hardware thread (i.e. soft-disable SMT/Hyperthreading). [{a}]", a = REQUIRES_GAME))]
    pub(crate) stagger: bool,
    #[arg(long = "special-k", alias = SK, help = formatcp!("Launch a game via SKIF (Special K Injection Frontend). [{a}, {b}]", a = ALIAS_SK, b = REQUIRES_GAME))]
    pub(crate) use_special_k: bool
}

#[derive(Parser)]
#[command(name = "ogos", arg_required_else_help = true, disable_help_flag = true)]
pub(crate) struct Cli {
    #[arg(short, long, hide = true)]
    pub(crate) help: bool,

    #[arg(help =
        "Parse additional arguments from an arg file or launch a video file with mpv.\n\
        Arg files use the .ogos extension and list arguments one per line.\n\
        Video files are inferred from their extension. On launch, set display mode, default audio endpoint sample rate, and tone mapping parameters to match video metadata."
    )]
    pub(crate) path: Option<String>,

    #[arg(long, conflicts_with_all = LONG_LIVED, help = formatcp!("Launch the media browser. [{a}]", a = CONFLICTS_WITH_LONG_LIVED))]
    pub(crate) media_browser: bool,

    #[arg(long, help = "Temporarily prevent setting the default audio endpoint sample rate to match video metadata.")]
    pub(crate) maintain_sample_rate: bool,
    #[arg(long = ENDPOINT, name = DEVICE, help = formatcp!("Set the default audio endpoint device, where <{a}> is listed in System > Sound > Output.", a = DEVICE))]
    pub(crate) set_endpoint: Option<String>,
    #[arg(long = EQ, name = EQ, help = formatcp!("Overwrite the master Equalizer APO config file, where <{a}> is a config-defined custom config path.", a = EQ))]
    pub(crate) set_eq: Option<String>,
    #[arg(long, help = "Enable/disable HDR mode and set color bit depth, dither state, and novideo_srgb state.")]
    pub(crate) toggle_display_mode: bool,
    #[arg(long, name = STATE, alias = CLAMP, hide_possible_values = true, help = formatcp!("Set novideo_srgb's color space clamp. [{a}, {b}]", a = ALIAS_CLAMP, b = POSSIBLE_VALUES_CLAMP))]
    pub(crate) novideo_srgb: Option<NovideoSrgbOp>,

    #[arg(long = GAME, name = GAME, help = formatcp!("Launch a game, where <{a}> is a config-defined set of launch parameters and settings.", a = GAME))]
    pub(crate) launch_game: Option<String>,
    #[command(flatten)]
    pub(crate) gaming: Gaming,
    //
    #[arg(long, help = "Enable global hotkeys and dynamic keymaps.")]
    pub(crate) binds: bool,

    #[arg(long, help =
        "Manage taskbar visibility by monitoring cursor collisions against an invisible window or 'hitbox'."
    )]
    pub(crate) taskbar: bool,
    #[arg(long, help = "Periodically 'pixel-shift' windows about the desktop.")]
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
