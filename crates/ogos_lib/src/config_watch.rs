use ogos_common::*;
use ogos_config as config;
use ogos_err::*;

use crossbeam::channel as mpmc;
use log::*;
use notify_debouncer_full::{
    self as notify_db,
    notify
};
use std::{
    sync::mpsc,
    thread::{self, *}
};
use windows::Win32::{
    Foundation::*,
    UI::WindowsAndMessaging::*
};

fn begin(hook_mgr_tid: Tid, rx: mpmc::Receiver<notify_db::DebounceEventResult>, error_sx: &mpsc::Sender<String>) -> Res<()> { unsafe {
    info!("{}: begin", module_path!());

    let handle_debounce = |res: notify_db::DebounceEventResult| -> Res<_> {
        let events = res.map_err(ErrVar::NotifyDebounced)?;

        for event in events {
            if let notify::EventKind::Modify(_) = event.event.kind {
                let new_config = config::load()?;
                *config::get().write()? = new_config;

                info!("{}: reloaded config", module_path!());

                PostThreadMessageW(hook_mgr_tid.0, WM_OGOS_RELOAD_CONFIG, WPARAM(0), LPARAM(0))?;

                break
            }
        }

        Ok(())
    };

    for res in rx {
        if let Err(err) = handle_debounce(res) {
            error_sx.send(format!("{}: failure watching config: {}", module_path!(), err)).unwrap();
        }
    }

    info!("{}: closed", module_path!());

    Ok(())
} }

pub(crate) fn spawn(hook_mgr_tid: Tid, rx: mpmc::Receiver<notify_db::DebounceEventResult>, error_sx: mpsc::Sender<String>) -> JoinHandle<()> {
    thread::spawn(move || {
        begin(hook_mgr_tid, rx, &error_sx).unwrap_or_else(|err| {
            error_sx.send(format!("{}: terminated: {}", module_path!(), err)).unwrap();
        });
    })
}
