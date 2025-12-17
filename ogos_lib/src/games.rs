use crate::{
    cli::*,
    common::*,
    config::{self, *},
    discord,
    display::*,
    err::*,
    gui,
    pipe_client::*,
    win32::*
};

use concat_string::*;
use discord_rich_presence::*;
use log::*;
use sysinfo::*;
use std::{
    process::*,
    sync::mpsc,
    thread,
    time::*
};
use strum::*;
use windows::Win32::{
    Foundation::*,
    System::{
        Threading::*,
        SystemInformation::*
    }
};

#[derive(Clone, Copy, EnumIter)]
enum Launcher {
    Epic,
    Gog,
    Steam
}
impl Launcher {
    fn as_url_prefix(&self) -> &str {
        match self {
            Self::Epic => "com.epicgames.launcher://",
            Self::Gog => "gog://",
            Self::Steam => "steam://"
        }
    }
}

pub(crate) unsafe fn launch(name: &str, cli: &Cli) -> Res<(), { loc_var!(Games) }> {
    let mut revert_to: Vec<GamesSetting> = Vec::new();

    let res = (|| -> Res<(), { loc_var!(Games) }> {
        let config = config::get().read()?;
        let games_config = config.games.as_ref().ok_or(ErrVar::MissingConfigKey { name: config::Games::NAME })?;
        let game_info = games_config.0.get(name).ok_or_else(|| ErrVar::UnknownGame { name: name.into() })?;
        let discord_info = game_info.discord.clone();

        if let Some(name) = game_info.unbind {
            pipe_msg(PipeMsg::BindMsg(BindMsg::Unbind(BindName::Underscore)))?;

            revert_to.push(GamesSetting::Bind(name));
        }

        if let Some(cursor_size) = cli.gaming.cursor_size {
            set_cursor_size(cursor_size)?;

            revert_to.push(GamesSetting::CursorSize(32));

        }

        if cli.gaming.set_res &&
            let Some(res) = game_info.res
        {
            let prev = set_screen_extent(res)?;

            if let Some(prev) = prev {
                revert_to.push(GamesSetting::ScreenExtent(prev));
            }
        }

        if cli.gaming.set_display_mode_hdr {
            let prev = set_display_mode(SetDisplayModeOp::Set(DisplayMode::Hdr))?;

            if let Some(prev) = prev {
                revert_to.push(GamesSetting::DisplayMode(prev));
            }
        }

        let using_launcher = match game_info.url.as_ref() {
            Some(url) => {
                let launcher = Launcher::iter()
                    .find(|launcher| {
                        url.starts_with(launcher.as_url_prefix())
                    })
                    .ok_or(ErrVar::InvalidUrl)?;

                Some(launcher)
            },
            None => None
        };

        let launcher_or_game_path = match using_launcher {
            Some(launcher) => {
                match launcher {
                    Launcher::Epic => config.app_paths.epic.as_str(),
                    Launcher::Gog => config.app_paths.gog.as_str(),
                    Launcher::Steam => config.app_paths.steam.as_str()
                }
            },
            None => game_info.proc.as_str()
        };

        let mut cmd;
        match cli.gaming.use_special_k {
            true => {
                let skif_path = confirm_or_find_app(App::SKIF, config.app_paths.skif.as_ref())?;

                cmd = Command::new(skif_path);
                cmd.arg(launcher_or_game_path);
            },
            false => {
                cmd = Command::new(launcher_or_game_path);
            }
        }

        match game_info.url.as_ref() {
            Some(url) => {
                match using_launcher.as_ref().unwrap() {
                    Launcher::Gog => {
                        let game_id = url.trim_start_matches(Launcher::Gog.as_url_prefix());
                        let game_id_arg = concat_string!("/gameId=", game_id);

                        cmd.args(["/command=runGame", game_id_arg.as_str()]);
                    },
                    _ => { // Epic/Steam
                        cmd.arg(url);
                    }

                }
            },
            None => {
                if let Some(args) = game_info.args.as_ref() {
                    cmd.args(args);
                }
            }

        }

        spawn_command(&mut cmd)?;
        info!("{}: spawned {}", module_path!(), cmd.display());

        let mut system = System::new();
        let pid = (0..30).find_map(|_| {
            get_first_process(&game_info.proc, &mut system)
                .map(|proc| proc.pid())
                .or_else(|| {
                    thread::sleep(Duration::from_secs(1));

                    None
                })
        })
        .ok_or_else(|| ErrVar::MissingProcess { name: game_info.proc.clone() })?;
        info!("{}: process: {}, pid: {}", module_path!(), game_info.proc, pid);

        drop(config);

        if cli.gaming.stagger {
            thread::spawn(move || {
                (|| -> Res<()> {
                    let proc_hnd = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_LIMITED_INFORMATION, false, pid.as_u32())?;

                    let mut required_cpu_set_infos_size = 0;
                    _ = GetSystemCpuSetInformation(None, 0, &mut required_cpu_set_infos_size, Some(proc_hnd), None); // Ignore error thrown when querying required buffer size

                    let cpu_set_info_size = size_of::<SYSTEM_CPU_SET_INFORMATION>() as u32;
                    let cpu_set_infos_len = required_cpu_set_infos_size / cpu_set_info_size;
                    let mut cpu_set_infos = vec![SYSTEM_CPU_SET_INFORMATION::default(); cpu_set_infos_len as usize];
                    let cpu_set_infos_size = size_of_val(&*cpu_set_infos) as u32;
                    GetSystemCpuSetInformation(Some(cpu_set_infos.as_mut_ptr()), cpu_set_infos_size, &mut required_cpu_set_infos_size, Some(proc_hnd), None).ok()?;

                    let valid_cpu_set_infos_len = required_cpu_set_infos_size / cpu_set_info_size;
                    let staggered_cpu_set_ids = cpu_set_infos[0..valid_cpu_set_infos_len as usize].iter()
                        .filter_map(|cpu_set_info| {
                            #[cfg(feature = "dbg_cpu_sets")] {
                                info!("GetSystemCpuSetInformation:");
                                info!("\tinfo size: {:?}", cpu_set_info.Size);
                                info!("\tinfo type: {:?}", cpu_set_info.Type);
                                info!("\tid: {}", cpu_set_info.Anonymous.CpuSet.Id);
                                info!("\tgroup: {}", cpu_set_info.Anonymous.CpuSet.Group);
                                info!("\tlogical processor index: {}", cpu_set_info.Anonymous.CpuSet.LogicalProcessorIndex);
                                info!("\tcore index: {}", cpu_set_info.Anonymous.CpuSet.CoreIndex);
                                info!("\tlast level cache index: {}", cpu_set_info.Anonymous.CpuSet.LastLevelCacheIndex);
                                info!("\tnuma node index: {}", cpu_set_info.Anonymous.CpuSet.NumaNodeIndex);
                                info!("\tefficiency class: {}", cpu_set_info.Anonymous.CpuSet.EfficiencyClass);
                                info!("\tallocation tag: {}", cpu_set_info.Anonymous.CpuSet.AllocationTag);
                                info!("\tall flags: {:0b}", cpu_set_info.Anonymous.CpuSet.Anonymous1.AllFlags);
                                info!("\tbitfield: {:0b}", cpu_set_info.Anonymous.CpuSet.Anonymous1.Anonymous._bitfield);
                                info!("\tscheduling class: {}", cpu_set_info.Anonymous.CpuSet.Anonymous2.SchedulingClass);
                                info!("\tscheduling reserved: {}", cpu_set_info.Anonymous.CpuSet.Anonymous2.Reserved);
                            }

                            if cpu_set_info.Anonymous.CpuSet.Id % 2 == 0 {
                                return Some(cpu_set_info.Anonymous.CpuSet.Id)
                            }

                            None
                        })
                        .collect::<Vec<_>>();

                    SetProcessDefaultCpuSets(proc_hnd, Some(&staggered_cpu_set_ids)).ok()?;
                    info!("{}: staggered cpu set ids: {:?}", module_path!(), staggered_cpu_set_ids);

                    #[cfg(feature = "dbg_cpu_sets")] {
                        let mut required_count = 0_u32;
                        _ = GetProcessDefaultCpuSets(proc_hnd, None, &mut required_count);
                        info!("GetProcessDefaultCpuSets:");
                        info!("\trequired count: {}", required_count);
                        let mut cpu_ids = vec![0; required_count as usize];
                        GetProcessDefaultCpuSets(proc_hnd, Some(&mut cpu_ids), &mut required_count).ok()?;
                        info!("\tcpu ids: {:?}", cpu_ids);

                        let mut required_mask_count = 0_u16;
                        _ = GetProcessDefaultCpuSetMasks(proc_hnd, None, &mut required_mask_count);
                        info!("GetProcessDefaultCpuSetMasks:");
                        info!("\trequired mask count: {}", required_mask_count);
                        let mut cpu_set_masks = vec![GROUP_AFFINITY::default(); required_mask_count as usize];
                        GetProcessDefaultCpuSetMasks(proc_hnd, Some(&mut cpu_set_masks), &mut required_mask_count).ok()?;
                        info!("\tcpu set masks: {:?}", cpu_set_masks);
                    }
                    CloseHandle(proc_hnd)?;

                    Ok(())
                })()
                .unwrap_or_else(|err| {
                    error!("{}: failed to set cpu sets: {}", module_path!(), err);
                });
            });
        }

        match discord_info {
            Some(discord_info) => {
                let mut ipc_client = DiscordIpcClient::new(discord_info.client_id.as_str());

                info!("{}: calling discord and waiting for gui to terminate...", module_path!());
                match name {
                    "Chess" => {
                        thread::scope(|s| -> Res<()> {
                            let (discord_sx, rx) = mpsc::channel::<Msg>();

                            let large_image = discord_info.large_image.clone();
                            let chess_username = discord_info.chess_username.clone().ok_or(ErrVar::MissingUsername)?;
                            discord::spawn_scoped_chess(s, &mut ipc_client, large_image, chess_username, rx);

                            gui::begin(gui::Kind::Discord { name: name.into(), discord_info })?;
                            discord_sx.send(Msg::Close)?;

                            Ok(())
                        })?;
                    },
                    _ => {
                        discord::begin(&mut ipc_client, &discord_info)?;

                        gui::begin(gui::Kind::Discord { name: name.into(), discord_info })?;

                        ipc_client.close()?;
                    }
                }

                revert_to.push(GamesSetting::Discord(ipc_client));
            },
            _ => {
                info!("{}: waiting for process to terminate...", module_path!());

                let proc_hnd = OpenProcess(PROCESS_SYNCHRONIZE, false, pid.as_u32())?;
                WaitForSingleObject(proc_hnd, INFINITE).win32_core_ok()?;

                info!("{}: process no longer exists", module_path!());
                CloseHandle(proc_hnd)?;
            }
        }

        Ok(())
    })();

    while let Some(setting) = revert_to.pop() {
        (|| -> Res<()> {
            match setting {
                GamesSetting::Bind(name) => {
                    pipe_msg(PipeMsg::BindMsg(BindMsg::Bind(name)))?;
                },
                GamesSetting::CursorSize(size) => {
                    set_cursor_size(size)?;
                },
                GamesSetting::Discord(mut ipc_client) => {
                    ipc_client.close()?;
                },
                GamesSetting::DisplayMode(display_mode) => {
                    set_display_mode(SetDisplayModeOp::Set(display_mode))?;
                },
                GamesSetting::ScreenExtent(screen_extent) => {
                    set_screen_extent(screen_extent)?;
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
