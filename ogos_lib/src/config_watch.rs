use crate::{
    CanReloadConfig,

    binds,
    common::*,
    config::{self, *},
    err::*,
    win32::*,
    window_watch
};

use log::*;
use windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;
use std::{
    ffi::*,
    os::windows::ffi::*,
    slice,
    thread::{self, *}
};
use windows::Win32::{
    Foundation::*,
    Storage::FileSystem::*,
    System::{
        IO::*,
        Threading::*
    }
};

const DEBOUNCE_INTERVAL_MS: u32 = 200;

unsafe fn begin(can_reload_config: Vec<CanReloadConfig>, event_close: usize) -> Res<()> {
    info!("{}: begin", module_path!());

    let watch_dir_name = WinStr::from(CURRENT_EXE_PARENT_PATH.get_unchecked());
    let watch_dir_hnd = CreateFileW(
         *watch_dir_name,
        FILE_LIST_DIRECTORY.0,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        None,
        OPEN_EXISTING,
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
        None
    )?;

    let event_close = HANDLE(event_close as *mut c_void);
    let event_io = CreateEventW(None, true, false, None)?;

    let mut buf = [0_u8; 1024];
    let mut overlapped = OVERLAPPED {
        hEvent: event_io,
        ..default!()
    };

    let mut timeout = INFINITE;
    loop {
        ReadDirectoryChangesW(
            watch_dir_hnd,
            buf.as_mut_ptr() as *mut _,
            buf.len() as u32,
            false,
            FILE_NOTIFY_CHANGE_LAST_WRITE,
            None,
            Some(&mut overlapped),
            None,
        )?;

        const WAIT_CLOSE: WAIT_EVENT = WAIT_EVENT(WAIT_OBJECT_0.0 + 1);
        match WaitForMultipleObjects(&[event_io, event_close], false, timeout) {
            WAIT_OBJECT_0 => {
                let mut bytes_transferred = 0;
                GetOverlappedResult(watch_dir_hnd, &overlapped, &mut bytes_transferred, false)?;

                timeout = DEBOUNCE_INTERVAL_MS;
            },
            WAIT_CLOSE => {
                CancelIoEx(watch_dir_hnd, Some(&overlapped))?;

                break
            },
            WAIT_TIMEOUT => {
                let info = &*(buf[0..].as_ptr() as *const FILE_NOTIFY_INFORMATION);

                let file_name_len = (info.FileNameLength / 2) as usize; // FileNameLength is in bytes, not wide chars
                let file_name_slc = slice::from_raw_parts(info.FileName.as_ptr(), file_name_len);
                let file_name = OsString::from_wide(file_name_slc);

                if info.Action == FILE_ACTION_MODIFIED && file_name == CONFIG_FILE_NAME &&
                    let Some(current) = CONFIG.get_unchecked()
                {
                    match config::load() {
                        Ok(new) => {
                            *current.write()? = new;

                            info!("{}: loaded new config", module_path!());

                            for can_reload_config in can_reload_config.iter() {
                                match can_reload_config {
                                    CanReloadConfig::StaticBinds => {
                                        thread::Builder::new()
                                            .spawn(|| {
                                                mki::clear();

                                                binds::configure_static_binds().unwrap_or_else(|err| {
                                                    error!("{}: failed to configure static binds: {}", module_path!(), err);
                                                });
                                            })?;
                                    },
                                    CanReloadConfig::WindowWatch(hook_mgr_tid) => PostThreadMessageW(hook_mgr_tid.0, window_watch::WM_OGOS_RELOAD_CONFIG, WPARAM(0), LPARAM(0))?
                                }
                            }
                        },
                        Err(err) => error!("{}: failed to load config: {} ", module_path!(), err)
                    }
                }

                timeout = INFINITE;
            },
            WAIT_FAILED => error!("{}: wait on config write failed", module_path!()),
            _ => () // WAIT_ABANDONED
        }
    }

    info!("{}: closed", module_path!());

    Ok(())
}

pub(crate) unsafe fn spawn(can_reload_config: Vec<CanReloadConfig>, event_close: usize) -> JoinHandle<()> {
    thread::Builder::new()
        .spawn(move || {
            begin(can_reload_config, event_close).unwrap_or_else(|err| {
                error!("{}: terminated: {}", module_path!(), err);
            });
        })
        .unwrap()
}
