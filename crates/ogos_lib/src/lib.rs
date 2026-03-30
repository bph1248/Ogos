#![allow(clippy::blocks_in_conditions)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::from_over_into)]
#![allow(clippy::just_underscores_and_digits)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::uninlined_format_args)]
#![allow(clippy::unused_io_amount)]
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

pub(crate) mod binds;
pub(crate) mod cli;
pub(crate) mod config_watch;
pub(crate) mod cursor_watch;
pub(crate) mod games;
pub(crate) mod pipe_client;
pub(crate) mod pipe_server;
pub(crate) mod win32;
pub(crate) mod window_foreground;
pub(crate) mod window_shift;
pub(crate) mod window_watch;

use bitflags::*;
use cli::*;
use ogos_audio::*;
use ogos_common::*;
use ogos_config as config;
use config::*;
use ogos_core::*;
use ogos_display::*;
use ogos_err::*;
use ogos_gui as gui;
use ogos_video as video;
use win32::*;
use window_foreground::*;

use clap::CommandFactory;
use log::*;
use netcorehost::{
    pdcstr,
    hostfxr::*,
    pdcstring::PdCString
};
use once_cell::sync::*;
use pipe_client::*;
use std::{
    convert::*,
    env,
    fs::{self, *},
    path::*,
    sync::{mpsc, *},
    thread::*,
    time::*
};
use subenum::*;
use sysinfo::*;
use windows::{
    core::{w, PCWSTR},
    Win32::{
        Foundation::*,
        System::{
            LibraryLoader::*,
            Threading::*
        },
        UI::{
            Shell::*,
            WindowsAndMessaging::*
        }
    }
};

const ICON_ID: usize = 1;
const OGOS_TRAY_CLASS_NAME: PCWSTR = w!("OgosTray");

static ON_TASKBAR_RECREATE_INFO: OnceCell<OnTaskbarRereateInfo> = OnceCell::new(); // Use sync:: as Windows may call wnd_proc from another thread
static SHUTDOWN_INFO: OnceCell<Mutex<ShutdownInfo>> = OnceCell::new();

bitflags! {
    struct EndSessionFlags: isize {
        const CLOSEAPP = ENDSESSION_CLOSEAPP as isize;
        const CRITICAL = ENDSESSION_CRITICAL as isize;
        const LOGOFF   = ENDSESSION_LOGOFF as isize;
    }
}

#[derive(Debug)]
struct OnTaskbarRereateInfo {
    exe_module: HINSTANCE,
    wm_taskbar_created: u32
}
unsafe impl Send for OnTaskbarRereateInfo {}
unsafe impl Sync for OnTaskbarRereateInfo {}

#[derive(Debug)]
struct ShutdownInfo {
    to_close: Vec<LongLivedTask>,
    thread_hnds: Vec<JoinHandle<()>>
}

#[derive(Debug)]
#[subenum(CanReloadConfig)]
pub(crate) enum LongLivedTask {
    ConfigWatch(HANDLE),
    PipeServer,
    #[subenum(CanReloadConfig)]
    StaticBinds,
    #[subenum(CanReloadConfig)]
    WindowWatch(Tid)
}
unsafe impl Send for LongLivedTask {}
unsafe impl Sync for LongLivedTask {}

unsafe extern "system" fn tray_notify_icon_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT { unsafe {
    match msg {
        WM_CLOSE => DefWindowProcW(hwnd, msg, wparam, lparam),
        WM_CREATE => LRESULT(0),
        WM_DESTROY => {
            PostQuitMessage(0);

            LRESULT(0)
        },
        WM_ENDSESSION => {
            if wparam.0 != 0 { // Session is actually ending
                let end_session_reason = EndSessionFlags::from_bits_retain(lparam.0);

                if end_session_reason.contains(EndSessionFlags::CLOSEAPP) {
                    info!("{}: end session: system has requested shutdown due to service/updates", module_path!());
                } else if end_session_reason.contains(EndSessionFlags::CRITICAL) {
                    info!("{}: end session: system is forcing shutdown", module_path!());
                } else if end_session_reason.contains(EndSessionFlags::LOGOFF) {
                    info!("{}: end session: user is logging off", module_path!());
                }

                shutdown();
            }

            LRESULT(0)
        },
        WM_NCCREATE => LRESULT(1),
        WM_QUERYENDSESSION => LRESULT(1), // Acquiesce
        WM_OGOS_TRAY => {
            (|| -> Res<()> {
                if lparam.0 as u32 == WM_RBUTTONUP {
                    let menu_hnd = CreatePopupMenu()?;

                    SetForegroundWindow(hwnd).ok()?;

                    const RELOAD_CONFIG: usize = 1;
                    const QUIT: usize = 2;
                    // let menu_entry_reload_config = "Reload config".to_win_str();
                    let menu_entry_quit = "Quit".to_win_str();
                    // AppendMenuW(menu_hnd, MF_STRING, RELOAD_CONFIG, *menu_entry_reload_config)?;
                    AppendMenuW(menu_hnd, MF_STRING, QUIT, *menu_entry_quit)?;

                    let mut cursor_pos = POINT::default();
                    GetCursorPos(&mut cursor_pos)?;
                    let selected = TrackPopupMenu(menu_hnd, TPM_BOTTOMALIGN | TPM_LEFTALIGN | TPM_RETURNCMD, cursor_pos.x, cursor_pos.y, None, hwnd, None);

                    match selected.0 as usize {
                        RELOAD_CONFIG => (),
                        QUIT => {
                            shutdown();
                            PostQuitMessage(0);
                        },
                        _ => ()
                    }
                }

                Ok(())
            })()
            .unwrap_or_else(|err| {
                error!("{}: failed to handle {}: {}", module_path!(), msg.to_wm_string(), err);
            });

            LRESULT(0)
        },
        _ => {
            let info = ON_TASKBAR_RECREATE_INFO.get_unchecked();
            if msg == info.wm_taskbar_created {
                add_tray_notify_icon(info.exe_module, OGOS_TRAY_CLASS_NAME, None).unwrap_or_else(|err| {
                    error!("{}: failed to recreate tray notify icon: {}", module_path!(), err);
                });
            }

            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }
} }

pub(crate) fn add_tray_notify_icon(exe_module: HINSTANCE, class_name: PCWSTR, register_class: Option<WNDCLASSEXW>) -> Res1<()> { unsafe {
    if let Some(wnd_class) = register_class {
        RegisterClassExW(&wnd_class).win32_core_ok()?;
    }

    let hidden_tray_hwnd = CreateWindowExW(
        default!(),
        class_name,
        class_name,
        WS_OVERLAPPED,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        CW_USEDEFAULT,
        None,
        None,
        Some(exe_module),
        None
    )?;

    let icon_hnd = LoadImageW(Some(exe_module), PCWSTR(ICON_ID as *const u16), IMAGE_ICON, 0, 0, LR_DEFAULTSIZE | LR_SHARED)?;
    let notify_icon_data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hidden_tray_hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_OGOS_TRAY,
        hIcon: HICON(icon_hnd.0),
        szTip: "Ogos".to_wide_128(),
        ..default!()
    };

    Shell_NotifyIconW(NIM_ADD, &notify_icon_data).ok()?;

    Ok(())
} }

fn error_alert(msg: String) {
    error!("{}", &msg);

    _ = gui::begin(gui::Kind::Info { msg });
}

fn find_novideo_srgb(config: RwLockReadGuard<'_, Config>) -> Res1<PathBuf> {
    let confirm_deps = |path: &Path| -> ResVar<()>{
        path.with_file_name("EDIDParser.dll").confirm()?;
        path.with_file_name("NvAPIWrapper.dll").confirm()?;
        path.with_file_name("WindowsDisplayAPI.dll").confirm()?;

        Ok(())
    };

    let path = confirm_or_find_app(App::NOVIDEO_SRGB, config.app_paths.novideo_srgb.as_ref())?;
    confirm_deps(path.as_path())?;

    Ok(path)
}

fn get_long_lived_channels(enable_window_foreground: bool, enable_window_shift: bool) -> LongLivedChannels {
    let mut llc = LongLivedChannels::default();

    if enable_window_foreground {
        let channels = mpsc::channel::<window_foreground::Msg>();

        llc.with_window_foreground(channels);
    }

    if enable_window_shift {
        let channels = mpsc::channel::<window_shift::Msg>();

        llc.with_window_shift(channels);
    }

    llc
}

fn receive_ready(to_close: &mut Vec<LongLivedTask>, rx: mpsc::Receiver<ReadyMsg>) -> Option<Tid> {
    let mut window_watch_tid = None;

    for msg in rx.iter() {
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

fn shutdown() { unsafe {
    info!("{}: shutdown", module_path!());

    let mut info = SHUTDOWN_INFO.get_unchecked().lock().unwrap();

    while let Some(long_lived_task) = info.to_close.pop() {
        (|| -> Res<()> {
            match long_lived_task {
                LongLivedTask::ConfigWatch(event_close) => SetEvent(event_close)?,
                LongLivedTask::PipeServer => pipe_msg(pipe_server::Msg::Close)?,
                LongLivedTask::WindowWatch(tid) => PostThreadMessageW(tid.0, WM_OGOS_CLOSE, WPARAM(0), LPARAM(0))?,
                _ => ()
            }

            Ok(())
        })()
        .unwrap_or_else(|err| {
            error!("{}: failed to close long-lived task: {}", module_path!(), err);
        });
    }

    let thread_hnds = std::mem::take(&mut info.thread_hnds);
    for hnd in thread_hnds {
        _ = hnd.join();
    }
} }

fn begin(cli: Cli, cli_path_kind: CliPathKind) -> Res<()> {
    // Audio
    if let Some(name) = cli.set_endpoint.as_ref() {
        set_endpoint(name.as_str()).unwrap_or_else(|err| {
            let ErrLoc { var, x, .. } = &err;

            match var.as_ref() {
                ErrVar::UnknownEndpoint => error_alert(format!("{}: {var}: {name}, {x}", module_path!())),
                ErrVar::FailedSpawnCommand { inner, cmd } => error_alert(format!("{}: failed to spawn app for endpoint: {}, cmd: {}: {}", module_path!(), name, cmd, inner)),
                _ => error_alert(format!("{}: failed to set endpoint: {}: {}", module_path!(), name, err))
            }
        });
    }

    if let Some(name) = cli.set_eq.as_ref() {
        set_eq(name).unwrap_or_else(|err| {
            let ErrLoc { var, x, .. } = &err;

            match var.as_ref() {
                ErrVar::UnknownEqApoConfigName => error_alert(format!("{}: {var}: {name}, {x}", module_path!())),
                _ => error_alert(format!("{}: failed to set eq: {}: {}", module_path!(), name, err))
            }
        });
    }

    // Display
    if cli.toggle_display_mode {
        _ = set_display_mode(SetDisplayModeOp::Toggle).inspect_err(|err| {
            error_alert(format!("{}: failed to toggle display mode: {}", module_path!(), err));
        });
    }

    if let Some(op) = cli.novideo_srgb.as_ref() {
        (|| -> Res<_> {
            let display_path = get_first_display_path()?;
            let display_mode = get_display_mode(display_path)?;

            if display_mode == DisplayMode::Sdr {
                let config = config::get().read()?;
                let NovideoSrgbInfo {
                    primaries_source,
                    color_space_target,
                    gamma,
                    enable_optimization,
                    ..
                } = config.display_modes.as_ref()
                    .and_then(|display_modes| display_modes.sdr.novideo_srgb.clone())
                    .ok_or(ErrVar::MissingConfigKey { name: NovideoSrgbInfo::NAME })?;

                let enable_clamp = match op {
                    NovideoSrgbOp::On => true,
                    NovideoSrgbOp::Off => false
                };
                let info = NovideoSrgbInfo {
                    enable_clamp,
                    primaries_source,
                    color_space_target,
                    gamma,
                    enable_optimization
                };
                control_novideo_srgb(&info)?;
            } else {
                Err(ErrVar::InvalidDisplayMode)?;
            }

            Ok(())
        })()
        .unwrap_or_else(|err| {
            error_alert(format!("{}: failed to set novideo_srgb clamp: {}", module_path!(), err));
        });
    }

    // Games
    if let Some(name) = cli.launch_game.as_ref() {
        let system = System::new();

        games::launch(name, &cli, system).unwrap_or_else(|err| {
            error_alert(format!("{}: failure launching game: {}: {}", module_path!(), name, err));
        });
    }

    // Media
    if cli.maintain_sample_rate {
        video::create_maintain_sample_rate_guard().unwrap_or_else(|err| {
            error_alert(format!("{}: failed to create maintain-sample-rate guard: {}", module_path!(), err));
        });
    }

    if let Some(path_str) = cli.path.as_ref() &&
        let CliPathKind::Media = cli_path_kind
    {
        (|| -> Res<()> {
            let path = Path::new(path_str).confirm()?;

            if path.is_file() {
                let ext = path.get_file_ext()?;

                match get_file_kind(ext) {
                    FileKind::Vid => video::launch_mpv(path, video::MaintainSampleRate::CheckGuard, false)?,
                    _ => Err(ErrVar::InvalidFileExt)?
                }
            }

            Ok(())
        })()
        .unwrap_or_else(|err| {
            error_alert(format!("{}: failed to handle path: {}: {}", module_path!(), path_str, err));
        });
    }

    if cli.media_browser {
        gui::begin(gui::Kind::MediaBrowser).unwrap_or_else(|err| {
            error_alert(format!("{}: failed to launch media browser: {}", module_path!(), err));
        });
    }

    // Long-lived tasks
    if cli.binds || cli.taskbar || cli.window_shift { unsafe {
        let on_taskbar_recreate_info = OnTaskbarRereateInfo {
            exe_module: GetModuleHandleW(None)?.into(),
            wm_taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated"))
        };
        ON_TASKBAR_RECREATE_INFO.set(on_taskbar_recreate_info).unwrap();

        let mut shutdown_info = ShutdownInfo {
            to_close: Vec::new(),
            thread_hnds: Vec::new()
        };
        let (ready_sx, ready_rx) = mpsc::channel::<ReadyMsg>();

        let init_long_lived_tasks = || -> Res<()> {
            let ShutdownInfo { to_close, thread_hnds } = &mut shutdown_info;

            let mut can_reload_config = Vec::new();
            let long_lived_channels = get_long_lived_channels(cli.binds || cli.taskbar, cli.window_shift);

            // Window watch
            let mut window_foreground_comps = WindowForegroundComponents::empty();
            let window_foreground_sx = match cli.binds {
                true => {
                    window_foreground_comps |= WindowForegroundComponents::DYNAMIC_BINDS;

                    long_lived_channels.sxs.window_foreground.clone()
                },
                false => None
            };
            if cli.taskbar { window_foreground_comps |= WindowForegroundComponents::TASKBAR; }
            thread_hnds.push(window_watch::spawn(window_foreground_comps, long_lived_channels.sxs, ready_sx.clone()));

            // Binds
            if cli.binds {
                can_reload_config.push(CanReloadConfig::StaticBinds);

                binds::configure_static_binds().unwrap_or_else(|err| {
                    error!("{}: failed to configure static binds: {}", module_path!(), err);
                });

                thread_hnds.push(pipe_server::spawn(ready_sx, window_foreground_sx));
            }

            let hook_mgr_tid = receive_ready(to_close, ready_rx);

            match long_lived_channels.enabled {
                EnabledChannels::WINDOW_FOREGROUND => {
                    thread_hnds.push(window_foreground::spawn(window_foreground_comps, long_lived_channels.rxs.window_foreground.unwrap(), hook_mgr_tid.unwrap()));
                },
                EnabledChannels::WINDOW_SHIFT => {
                    thread_hnds.push(window_shift::spawn(long_lived_channels.rxs.window_shift.unwrap()));
                },
                _ if long_lived_channels.enabled == EnabledChannels::WINDOW_FOREGROUND | EnabledChannels::WINDOW_SHIFT => {
                    thread_hnds.push(window_foreground::spawn(window_foreground_comps, long_lived_channels.rxs.window_foreground.unwrap(), hook_mgr_tid.unwrap()));
                    thread_hnds.push(window_shift::spawn(long_lived_channels.rxs.window_shift.unwrap()));
                },
                _ => ()
            }

            // Config watch
            // let event_close = CreateEventW(None, true, false, None)?;
            // if let Some(hook_mgr_tid) = hook_mgr_tid {
            //     can_reload_config.push(CanReloadConfig::WindowWatch(hook_mgr_tid));

            //     thread_hnds.push(config_watch::spawn(can_reload_config, event_close.0 as usize));
            //     to_close.push(LongLivedTask::ConfigWatch(event_close));
            // }

            let exe_module: HINSTANCE = GetModuleHandleW(None)?.into();
            let wnd_class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(tray_notify_icon_proc),
                hInstance: exe_module,
                lpszClassName: OGOS_TRAY_CLASS_NAME,
                ..default!()
            };
            add_tray_notify_icon(exe_module, OGOS_TRAY_CLASS_NAME, Some(wnd_class))?;

            SHUTDOWN_INFO.set(Mutex::new(shutdown_info)).unwrap();

            Ok(())
        };

        // Message loop
        match init_long_lived_tasks() {
            Ok(_) => {
                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
            }
            Err(err) => error_alert(format!("{}: failure initializing long-lived tasks: {}", module_path!(), err))
        }
    } }

    info!("{}: o/", module_path!());

    Ok(())
}

fn init() -> Res<(Cli, CliPathKind)> {
    let current_exe_path = env::current_exe()?;
    let current_exe_dir = current_exe_path.get_dir()?;
    CURRENT_EXE_DIR.set(current_exe_dir.into()).map_err(|_| ErrVar::FailedSetOnceCell)?;

    // Parse Cli
    let (cli, cli_path_kind) = parse_cli()?;

    // Help
    if cli.help {
        Err(ErrVar::Clap(clap::error::Error::new(clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand)))?;
    }

    // Create Log file
    let prefix = if cli.media_browser  {
        "gui"
    } else if cli.binds || cli.taskbar || cli.window_shift {
        "long-lived"
    } else {
        "blip"
    };
    let stamp = chrono::Local::now().format("%d_%m_%Y_%H-%M-%S");

    let log_dir = current_exe_path.with_file_name("logs");
    let log_file_name = format!("{}_{}.log", prefix, stamp);
    let log_file_link_name = format!("{}_current.log", prefix);
    let log_path = log_dir.join(log_file_name);
    let log_link_path = log_dir.join(log_file_link_name);

    {
        use simplelog::*;

        fs::create_dir_all(&log_dir)?;
        let log_file = File::options()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&log_path)?;

        let logger_config = ConfigBuilder::new()
            .add_filter_ignore_str("eframe")
            .add_filter_ignore_str("egui")
            .add_filter_ignore_str("wgpu")
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

    // Relink current log file
    if log_link_path.try_exists().unwrap_or(false) {
        fs::remove_file(&log_link_path)?;
    }
    fs::hard_link(log_path, log_link_path)?;

    // Delete old log files
    let read_dir = log_dir.read_dir()?;
    std::thread::spawn(move || {
        let mut log_dir_entries = read_dir.filter_map(|dir_entry| {
            dir_entry.map_err(into!()).and_then(|dir_entry| -> Res<_> {
                let path = dir_entry.path();
                let file_name = path.get_file_name()?;
                let ext = path.get_file_ext()?;

                match file_name.starts_with(prefix) && ext.eq_ignore_ascii_case("log") {
                    true => Ok(Some(dir_entry)),
                    false => Ok(None)
                }
            })
            .unwrap_or_else(|err| {
                error!("{}: failed to read log file: {}", module_path!(), err);

                None
            })
        })
        .collect::<Vec<_>>();

        log_dir_entries.sort_by_key(|entry| entry.metadata().and_then(|meta| meta.modified()).unwrap_or(SystemTime::UNIX_EPOCH));

        let delete_count = log_dir_entries.len().saturating_sub(6);
        for old in log_dir_entries.iter().take(delete_count) {
            let path = old.path();

            fs::remove_file(&path).unwrap_or_else(|err| error!("{}: failed to delete log file: {}, {}", module_path!(), path.display(), err));
        }
    });

    // Send panic messages to log
    log_panics::init();

    // Config
    let config = config::load().map_err(|err| err.msg("failed to load config"))?;
    CONFIG.set(RwLock::new(config)).map_err(|_| ErrVar::FailedSetConfig)?;

    // NovideoSrgb
    let config = config::get().read()?;
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

                let novideo_srgb_ffi = NovideoSrgbFfi {
                    _hostfxr: hostfxr,
                    novideo_srgb_apply_fn
                };
                NOVIDEO_SRGB_FFI.set(Some(novideo_srgb_ffi))
                    .map_err(|_| ErrVar::FailedSetOnceCell)?;
            },
            false => NOVIDEO_SRGB_FFI.set(None).map_err(|_| ErrVar::FailedSetOnceCell)?
        }
    }

    Ok((cli, cli_path_kind))
}

pub fn entry() -> Res<()> {
    match init() {
        Ok((cli, cli_path_kind)) => begin(cli, cli_path_kind),
        Err(err) => {
            if let ErrVar::Clap(inner) = err.var.as_ref() && inner.kind() == clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand {
                let long_help = cli::Cli::command().render_long_help();
                let long_help = long_help.to_string();

                gui::begin(gui::Kind::Info { msg: long_help })?;

                return Ok(())
            }

            error_alert(format!("{}: failed to init: {}", module_path!(), err));

            Err(err)
        }
    }
}
