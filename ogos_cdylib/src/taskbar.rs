use ogos_lib::win32::*;

use log::info;
use windows::Win32::{
    Foundation::*,
    UI::{
        Accessibility::*,
        WindowsAndMessaging::*
    }
};

#[unsafe(no_mangle)]
unsafe extern "system" fn _hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    info!("{}: code: {}, wparam: {}, lparam: {}", module_path!(), code, wparam.0, lparam.0);

    CallNextHookEx(None, code, wparam, lparam)
}

#[unsafe(no_mangle)]
unsafe extern "system" fn _win_event_proc(_: HWINEVENTHOOK, event: u32, hwnd: HWND, id_obj: i32, id_child: i32, id_event_thread: u32, dwms_event_time: u32) {
    info!("{}: {}: hwnd: {:p}, exe: {}, class: {}, id_obj: {}, id_child: {}, id_event_thread: {:#x}, dwms_event_time: {}",
        module_path!(), event.to_event_string(), hwnd.0, hwnd.get_exe_or_err(), hwnd.get_class_or_err(), id_obj, id_child, id_event_thread, dwms_event_time);
}
