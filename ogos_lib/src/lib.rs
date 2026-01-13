#![allow(unsafe_op_in_unsafe_fn)]

#![allow(clippy::blocks_in_conditions)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::from_over_into)]
#![allow(clippy::just_underscores_and_digits)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::uninlined_format_args)]
// #![allow(clippy::struct_excessive_bools)]
// #![allow(clippy::too_many_lines)]
// #![allow(clippy::uninlined_format_args)]
// #![allow(clippy::unreadable_literal)]
// #![allow(clippy::wildcard_imports)]

#![warn(clippy::case_sensitive_file_extension_comparisons)]
#![warn(clippy::cast_lossless)]
#![warn(clippy::cast_precision_loss)]
#![warn(clippy::redundant_else)]
#![warn(clippy::semicolon_if_nothing_returned)]
#![warn(clippy::unseparated_literal_suffix)]
#![warn(clippy::unused_self)]
#![warn(clippy::use_self)]
#![warn(clippy::wrong_self_convention)]
// #![warn(clippy::cast_possible_wrap)]
// #![warn(clippy::cast_sign_loss)]
// #![warn(clippy::pedantic)]

pub(crate) mod audio;
pub(crate) mod binds;
pub(crate) mod cli;
pub mod common;
pub(crate) mod config;
pub(crate) mod config_watch;
pub(crate) mod discord;
pub(crate) mod display;
pub mod err;
pub(crate) mod games;
pub(crate) mod gui;
pub(crate) mod nvapi_shadow;
pub(crate) mod pipe_client;
pub(crate) mod pipe_server;
pub(crate) mod video;
pub mod win32;
pub(crate) mod window_foreground;
pub(crate) mod window_shift;
pub(crate) mod window_watch;

use audio::*;
use cli::*;
use common::*;
use config::*;
use display::*;
use err::*;
use win32::*;
use window_foreground::*;

use clap::CommandFactory;
use log::*;
use netcorehost::{
    pdcstr,
    hostfxr::*,
    pdcstring::PdCString
};
use pipe_client::*;
use windows::Win32::{
    Foundation::*,
    System::{
        Console::*,
        LibraryLoader::*,
        Threading::*
    },
    UI::{
        Shell::*,
        WindowsAndMessaging::*
    }
};
use std::{
    convert::*,
    env,
    fs::{self, *},
    path::*,
    sync::{mpsc, *}
};
use subenum::*;
use sysinfo::*;

#[subenum(CanReloadConfig)]
pub(crate) enum LongLivedTask {
    ConfigWatch(HANDLE),
    PipeServer,
    #[subenum(CanReloadConfig)]
    StaticBinds,
    #[subenum(CanReloadConfig)]
    WindowWatch(Tid)
}

unsafe extern "system" fn wnd_proc_tray(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CLOSE => {
            DestroyWindow(hwnd).x()
                .unwrap_or_else(|err| {
                    panic!("{}: failed to destroy window: {:p}: {}", module_path!(), hwnd.0, err)
                });

            return LRESULT(0)
        },
        WM_CREATE => return LRESULT(0),
        WM_DESTROY => {
            PostQuitMessage(0);

            return LRESULT(0)
        },
        WM_NCCREATE => return LRESULT(1),
        _ if msg == WmOgos::Tray as u32 => {
            (|| -> Res<()> {
                if lparam.0 as u32 == WM_RBUTTONUP {
                    let menu_hnd = CreatePopupMenu()?;

                    SetForegroundWindow(hwnd).ok()?;

                    const RELOAD_CONFIG: usize = 1;
                    const QUIT: usize = 2;
                    let menu_entry_reload_config = "Reload config".to_win_str();
                    let menu_entry_quit = "Quit".to_win_str();
                    AppendMenuW(menu_hnd, MF_STRING, RELOAD_CONFIG, *menu_entry_reload_config)?;
                    AppendMenuW(menu_hnd, MF_STRING, QUIT, *menu_entry_quit)?;

                    let mut cursor_pos = POINT::default();
                    GetCursorPos(&mut cursor_pos)?;
                    let selected = TrackPopupMenu(menu_hnd, TPM_BOTTOMALIGN | TPM_LEFTALIGN | TPM_RETURNCMD, cursor_pos.x, cursor_pos.y, 0, hwnd, None);

                    match selected.0 as usize {
                        RELOAD_CONFIG => (),
                        QUIT => PostQuitMessage(0),
                        _ => ()
                    }
                }

                Ok(())
            })()
            .unwrap_or_else(|err| {
                error!("{}: failed to handle {}: {}", module_path!(), msg.to_wm_string(), err);
            });
        },
        _ => ()
    }

    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe fn find_novideo_srgb(config: RwLockReadGuard<'_, Config>) -> Res1<PathBuf> {
    let confirm_deps = |path: &Path| -> ResVar<()>{
        path.with_file_name("EDIDParser.dll").confirm()?;
        path.with_file_name("NvAPIWrapper.dll").confirm()?;
        path.with_file_name("WindowsDisplayAPI.dll").confirm()?;

        Ok(())
    };

    let path = find_app(App::NOVIDEO_SRGB, config.app_paths.novideo_srgb.as_ref())?;
    confirm_deps(path.as_ref())?;

    Ok(path)
}

fn get_enabled_channels(window_foreground_enable: bool, window_shift_enable: bool) -> EnabledChannels {
    let mut enabled = EnabledChannels::default();

    if window_foreground_enable {
        let channels = mpsc::channel::<WindowForegroundMsg>();

        enabled = enabled.with_window_foreground(channels);
    }

    if window_shift_enable {
        let channels = mpsc::channel::<WindowShiftMsg>();

        enabled = enabled.with_window_shift(channels);
    }

    enabled
}

fn receive_ready(to_close: &mut Vec<LongLivedTask>, ready_receiver: &mpsc::Receiver<ReadyMsg>) -> Option<Tid> {
    let mut window_watch_tid = None;

    for msg in ready_receiver.iter() {
        match msg {
            ReadyMsg::PipeServer => to_close.push(LongLivedTask::PipeServer),
            ReadyMsg::WindowWatch(tid) => {
                to_close.push(LongLivedTask::WindowWatch(tid));
                window_watch_tid = Some(tid);
            }
        }
    }

    window_watch_tid
}

unsafe fn shutdown(to_close: Vec<LongLivedTask>) {
    for long_lived_task in to_close.into_iter().rev() {
        (|| -> Res<()> {
            match long_lived_task {
                LongLivedTask::ConfigWatch(event_close) => SetEvent(event_close)?,
                LongLivedTask::PipeServer => pipe_msg(PipeMsg::Close)?,
                LongLivedTask::WindowWatch(tid) => PostThreadMessageW(tid.0, WmOgos::Close as u32, WPARAM(0), LPARAM(0))?,
                _ => ()
            }

            Ok(())
        })()
        .unwrap_or_else(|err| {
            error!("{}: failed to close long-lived task: {}", module_path!(), err);
        });
    }
}

unsafe fn begin() -> Res<()> {
    // Parse Cli
    let (cli, cli_path_kind) = parse_cli()?;

    // Help
    if cli.help {
        Err(ErrVar::Clap(clap::error::Error::new(clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand)))?;
    }

    // Audio
    if let Some(name) = cli.set_endpoint.as_ref() &&
        pipe_msg(PipeMsg::Endpoint(name.clone())).is_err()
    {
        audio::set_endpoint(name.as_str()).unwrap_or_else(|err| {
            error!("{}: failed to set endpoint: {}: {}", module_path!(), name, err);
        });
    }

    if let Some(name) = cli.set_eq.as_ref() {
        set_eq(name).unwrap_or_else(|err| {
            error!("{}: failed to set eq: {}: {}", module_path!(), name, err);
        });
    }

    // Display
    if cli.toggle_display_mode {
        _ = set_display_mode(SetDisplayModeOp::Toggle).inspect_err(|err| {
            error!("{}: failed to toggle display mode: {}", module_path!(), err);
        });
    }

    // Games
    if let Some(name) = cli.launch_game.as_ref() {
        games::launch(name, &cli).unwrap_or_else(|err| {
            error!("{}: failure launching game: {}: {}", module_path!(), name, err);
        });
    }

    // Media
    if cli.maintain_sample_rate {
        video::create_maintain_sample_rate_guard().unwrap_or_else(|err| {
            error!("{}: failed to create maintain-sample-rate guard: {}", module_path!(), err);
        });
    }

    if let Some(path_string) = cli.path.as_ref() {
        let path = Path::new(path_string);

        if let CliPathKind::Media = cli_path_kind {
            (|| -> Res<()> {
                if path.is_file() {
                    let ext = path.extension()
                        .and_then(|ext| ext.to_str())
                        .ok_or(ErrVar::InvalidFileExt)?;

                    match get_file_kind(ext) {
                        FileKind::Vid => video::launch_mpv(path, video::MaintainSampleRate::CheckGuard, false)?,
                        _ => opener::open(path)?
                    }
                }

                if path.is_dir() {
                    let dir_name = path.get_file_name()?;
                    let dir_entries = path.read_dir()?.collect::<Result<Vec<_>, _>>()?;

                    let config = config::get()?.read()?;
                    let discord_rp_client_ids = config.discord_app_ids.clone().unwrap_or_default();

                    drop(config);

                    gui::begin(gui::Kind::Dir { name: dir_name.into(), entries: dir_entries, discord_app_ids: discord_rp_client_ids })?;
                }

                Ok(())
            })()
            .unwrap_or_else(|err| {
                error!("{}: failed to handle media path: {}", module_path!(), err);
            });
        }
    }

    // Long-lived tasks
    if cli.binds || cli.taskbar || cli.window_shift {
        let mut thread_hnds = Vec::new();
        let mut to_close = Vec::new();

        let (ready_sender, ready_receiver) = mpsc::channel::<ReadyMsg>();

        let init_long_lived_tasks = || -> Res<()> {
            let mut can_reload_config = Vec::new();
            let mut window_foreground_enable = WindowForegroundEnable::empty();

            // Binds
            if cli.binds {
                can_reload_config.push(CanReloadConfig::StaticBinds);

                binds::configure_static_binds().unwrap_or_else(|err| {
                    error!("{}: failed to configure static binds: {}", module_path!(), err);
                });

                thread_hnds.push(pipe_server::spawn(ready_sender.clone()));
            }

            let sxrx = get_enabled_channels(cli.binds || cli.taskbar, cli.window_shift);

            // Window watch
            if !sxrx.enable.is_empty() {
                if cli.binds { window_foreground_enable |= WindowForegroundEnable::DYNAMIC_BINDS };
                if cli.taskbar { window_foreground_enable |= WindowForegroundEnable::TASKBAR; }

                thread_hnds.push(window_watch::spawn(window_foreground_enable, sxrx.senders, ready_sender));
            }
            let hook_mgr_tid = receive_ready(&mut to_close, &ready_receiver);

            match sxrx.enable {
                ChannelEnable::WINDOW_FOREGROUND => {
                    thread_hnds.push(window_foreground::spawn(window_foreground_enable, sxrx.receivers.window_foreground.unwrap(), hook_mgr_tid.unwrap()));
                },
                ChannelEnable::WINDOW_SHIFT => {
                    thread_hnds.push(window_shift::spawn(sxrx.receivers.window_shift.unwrap()));
                },
                _ if sxrx.enable == ChannelEnable::WINDOW_FOREGROUND | ChannelEnable::WINDOW_SHIFT => {
                    thread_hnds.push(window_foreground::spawn(window_foreground_enable, sxrx.receivers.window_foreground.unwrap(), hook_mgr_tid.unwrap()));
                    thread_hnds.push(window_shift::spawn(sxrx.receivers.window_shift.unwrap()));
                },
                _ => ()
            }

            // Config watch
            let event_close = CreateEventW(None, true, false, None)?;
            if let Some(hook_mgr_tid) = hook_mgr_tid {
                can_reload_config.push(CanReloadConfig::WindowWatch(hook_mgr_tid));

                thread_hnds.push(config_watch::spawn(can_reload_config, event_close.0 as usize));
                to_close.push(LongLivedTask::ConfigWatch(event_close));
            }

            // Tray
            let tray_class_name = "OgosTray".to_win_str();
            let exe_module = GetModuleHandleW(None)?;
            let wnd_class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wnd_proc_tray),
                hInstance: exe_module.into(),
                lpszClassName: *tray_class_name,
                ..default!()
            };
            RegisterClassExW(&wnd_class).win32_var_ok()?;

            let hidden_tray_hwnd = CreateWindowExW(
                default!(),
                *tray_class_name,
                *tray_class_name,
                WS_OVERLAPPED,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                None,
                None,
                exe_module,
                None
            )?;

            let icon_hnd = LoadIconW(None, IDI_APPLICATION)?;
            let notify_icon_data = NOTIFYICONDATAW {
                cbSize: size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hidden_tray_hwnd,
                uID: 1,
                uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
                uCallbackMessage: WmOgos::Tray as u32,
                hIcon: icon_hnd,
                szTip: "Ogos".to_wide_128(),
                ..default!()
            };

            Shell_NotifyIconW(NIM_ADD, &notify_icon_data).ok()?;

            Ok(())
        };

        match init_long_lived_tasks() {
            Ok(_) => {
                // Message loop
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, WM_CREATE, WmOgos::Close as u32).as_bool() {
                    DispatchMessageW(&msg);
                }
            }
            Err(err) => error!("{}: failure initializing long-lived tasks: {}", module_path!(), err)
        }

        shutdown(to_close);

        for hnd in thread_hnds {
            _ = hnd.join();
        }
    }

    info!("{}: o/", module_path!());

    Ok(())
}

unsafe fn init() -> Res<()> {
    let current_exe_path = env::current_exe()?;

    let current_exe_file_name = current_exe_path.get_file_name()?;
    let current_exe_parent_path = current_exe_path.get_parent()?;

    CURRENT_EXE_PARENT_PATH.set(current_exe_parent_path.into()).map_err(|_| ErrVar::FailedSetOnceCell)?;

    // Name log file based on the number of instances of Ogos already running
    let mut system = System::new();
    let current_process_count = get_process_count(current_exe_file_name, &mut system);

    let log_file_name = format!("ogos_{}.log", current_process_count);
    let log_parent_path = current_exe_parent_path.join("logs");
    let log_path = log_parent_path.join(log_file_name);

    {
        use simplelog::*;

        fs::create_dir_all(&log_parent_path)?;
        let log_file = File::options()
            .create(true)
            .write(true)
            .truncate(true)
            .open(log_path)?;

        let logger_config = ConfigBuilder::new()
            .set_thread_mode(ThreadLogMode::IDs)
            .set_thread_level(LevelFilter::Error)
            .set_thread_padding(ThreadPadding::Left(2))
            .set_time_offset_to_local()?
            .build();
        CombinedLogger::init(
            vec![
                #[cfg(feature = "dbg_console")]
                TermLogger::new(LevelFilter::Trace, logger_config.clone(), TerminalMode::Mixed, ColorChoice::Never),
                WriteLogger::new(LevelFilter::Info, logger_config, log_file)
            ]
        )?;
    }

    // Send panic messages to log
    log_panics::init();

    // NovideoSrgb
    let config = config::get()?.read()?;
    if let Some(display_modes_config) = config.display_modes.as_ref() {
        match display_modes_config.sdr.novideo_srgb.is_some() || display_modes_config.hdr.novideo_srgb.is_some() {
            true => {
                let novideo_srgb_path = find_novideo_srgb(config)?;
                let runtime_config_path = novideo_srgb_path.with_file_name("novideo_srgb.runtimeconfig.json").confirm()?;
                let novideo_srgb_path = PdCString::from_os_str(novideo_srgb_path)?;
                let runtime_config_path = PdCString::from_os_str(runtime_config_path)?;

                let hostfxr = Hostfxr::load_with_nethost()?;
                let ctx = hostfxr.initialize_for_runtime_config(runtime_config_path)?;

                let delegate_loader = ctx.get_delegate_loader_for_assembly(novideo_srgb_path)?;
                let novideo_srgb_apply_fn = delegate_loader.get_function_with_unmanaged_callers_only::<NovideoSrgbApplyFn>(
                    pdcstr!("novideo_srgb.Interop, novideo_srgb"),
                    pdcstr!("NovideoSrgbApply")
                )?;

                NOVIDEO_SRGB_FFI.set(Some(
                    NovideoSrgbFfi {
                        _hostfxr: hostfxr,
                        novideo_srgb_apply_fn
                    }
                ))
                .map_err(|_| ErrVar::FailedSetOnceCell)?;
            },
            false => NOVIDEO_SRGB_FFI.set(None).map_err(|_| ErrVar::FailedSetOnceCell)?
        }
    }

    Ok(())
}

pub unsafe fn entry() -> Res<()> {
    if let Err(err) = init() {
        display_message_box(&format!("{}: failed to init: {}", module_path!(), err))?;

        return Err(err)
    }

    if let Err(err) = begin() &&
        let ErrVar::Clap(inner) = err.var.as_ref()
    {
        if inner.kind() == clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand {
            let long_help = cli::Cli::command().render_long_help();

            info!("{}", long_help);

            AttachConsole(ATTACH_PARENT_PROCESS).unwrap_or_else(|err| {
                error!("{}: failed to attach console: {}", module_path!(), err);
            });
            println!("{}", long_help);

            return Ok(())
        }

        error!("{}: {}", module_path!(), err);

        return Err(err)
    }

    Ok(())
}
