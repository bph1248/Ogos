use crate::common::*;
use ogos_err::*;

use log::*;
use std::{
    sync::{atomic::*, *},
    thread,
    time::*
};
use windows::Win32::{
    Foundation::POINT,
    UI::WindowsAndMessaging::*
};

pub(crate) struct CursorWatch {
    pub(crate) sx: mpsc::Sender<CursorWatchMsg>,
    pub(crate) working: AtomicBool,
    pub(crate) request_stop: AtomicBool
}

pub(crate) fn begin(snap_ordinate: i32, screen_extent: Extent2d) -> Arc<CursorWatch> { unsafe {
    let (sx, rx) = mpsc::channel();
    let cursor_watch = Arc::new(CursorWatch {
        sx,
        working: AtomicBool::new(false),
        request_stop: AtomicBool::new(false)
    });

    let inner = move |cursor_watch_: &Arc<CursorWatch>, screen_extent| -> Res<()> {
        let mut cursor_pos = POINT::default();

        for _i in 0..20 {
            GetCursorPos(&mut cursor_pos)?;
            if cursor_pos.y <= snap_ordinate {
                break
            }

            #[cfg(feature = "dbg_cursor_watch")]
            info!("{}: cursor hasn't snapped: {}", module_path!(), _i);

            send_cursor_pos(cursor_pos.x, snap_ordinate, screen_extent)?;

            thread::sleep(Duration::from_millis(1));
            if cursor_watch_.request_stop.load(Ordering::Relaxed) {
                break
            }
        }

        Ok(())
    };

    let cursor_watch_ = cursor_watch.clone();
    let mut screen_extent_ = screen_extent;
    thread::spawn(move || {
        for msg in rx.iter() {
            match msg {
                CursorWatchMsg::Begin => inner(&cursor_watch_, screen_extent_).unwrap_or_else(|err| {
                    error!("{}: failed to monitor cursor pos: {}", module_path!(), err);
                }),
                CursorWatchMsg::DisplayChange(new_extent) => {
                    screen_extent_ = new_extent;
                }
            }

            cursor_watch_.working.store(false, Ordering::Relaxed);
        }

        info!("{}: closed", module_path!());
    });

    cursor_watch
} }
