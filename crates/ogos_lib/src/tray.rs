use crate::*;
use ogos_common::*;
use ogos_core::*;
use ogos_err::*;

use bitflags::*;
use log::*;
use std::thread::{self, *};
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

bitflags! {
    struct EndSessionFlags: isize {
        const CLOSEAPP = ENDSESSION_CLOSEAPP as isize;
        const CRITICAL = ENDSESSION_CRITICAL as isize;
        const LOGOFF   = ENDSESSION_LOGOFF as isize;
    }
}

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

                    shutdown();
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

fn shutdown() { unsafe {
    info!("{}: shutdown", module_path!());

    let info = SHUTDOWN_INFO.get_unchecked();
    for long_lived_task in info.to_close.iter() {
        (|| -> Res<()> {
            match long_lived_task {
                LongLivedTask::_ConfigWatch(event_close) => SetEvent(*event_close)?,
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

    PostQuitMessage(0);
} }

fn begin() -> Res<()> { unsafe {
    info!("{}: begin", module_path!());

    let on_taskbar_recreate_info = OnTaskbarRereateInfo {
        exe_module: GetModuleHandleW(None)?.into(),
        wm_taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated"))
    };
    ON_TASKBAR_RECREATE_INFO.set(on_taskbar_recreate_info).unwrap();

    let exe_module: HINSTANCE = GetModuleHandleW(None)?.into();
    let wnd_class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(tray_notify_icon_proc),
        hInstance: exe_module,
        lpszClassName: OGOS_TRAY_CLASS_NAME,
        ..default!()
    };

    add_tray_notify_icon(exe_module, OGOS_TRAY_CLASS_NAME, Some(wnd_class))?;

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {}

    Ok(())
} }

pub(crate) fn spawn() -> JoinHandle<()> {
    thread::spawn(move || {
        begin().unwrap_or_else(|err| {
            error!("{}: terminated: {}", module_path!(), err);

            shutdown();
        });
    })
}
