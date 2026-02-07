use crate::{
    common::*,
    cursor_watch,
    display::*,
    win32::*,
    window_foreground::*
};
use ogos_config as config;
use ogos_core::*;
use ogos_err::*;

use log::*;
use std::{
    cell::*,
    fmt::{self, *},
    sync::mpsc::*,
    thread::{self, *}
};
use strum::*;
use tokio::sync::oneshot;
use windows::Win32::{
    Foundation::*,
    System::{
        LibraryLoader::*,
        Threading::*
    },
    UI::{
        Accessibility::*,
        WindowsAndMessaging::*
    }
};

struct ThreadState {
    sxs: Senders
}

thread_local! {
    static THREAD_STATE: OnceCell<ThreadState> = const { OnceCell::new() }
}

trait Dispatch {
    fn dispatch(self, sxs: &Senders) -> Res1<()>;
}
impl Dispatch for BroadcastMsg {
    fn dispatch(self, sxs: &Senders) -> Res1<()> {
        sxs.window_foreground.as_ref().map(|sx| { sx.send(WindowForegroundMsg::BroadcastMsg(self)) }).transpose()?;
        sxs.window_shift.as_ref().map(|sx| { sx.send(WindowShiftMsg::BroadcastMsg(self)) }).transpose()?;

        Ok(())
    }
}
impl Dispatch for WindowForegroundMsg {
    fn dispatch(self, sxs: &Senders) -> Res1<()> {
        sxs.window_foreground.as_ref().map(|sx| { sx.send(self) }).transpose()?;

        Ok(())
    }
}
impl Dispatch for WindowShiftMsg {
    fn dispatch(self, sxs: &Senders) -> Res1<()> {
        sxs.window_shift.as_ref().map(|sx| { sx.send(self) }).transpose()?;

        Ok(())
    }
}

pub(crate) struct WinEventHookInfo {
    pub(crate) eventmin: u32,
    pub(crate) eventmax: u32,
    pub(crate) idprocess: u32,
    pub(crate) idthread: u32,
    pub(crate) ctx: WinEventHookContext
}

pub(crate) struct WinEventHookRequest {
    pub(crate) infos: Vec<WinEventHookInfo>
}

pub(crate) struct WinEventUnhookRequest {
    pub(crate) hooks: Vec<HWINEVENTHOOK>
}

#[derive(Clone, Copy, Debug, IntoStaticStr)]
pub(crate) enum WinEventHookContext {
    AllOtherForegroundDestroy { hwnd: usize },
    ExplorerDestroy,
    ForegroundLocationChange,
    ShellExperienceHostDestroy,
    ShellExperienceHostLocationChange,
    TaskbarLocationChange
}
impl WinEventHookContext {
    pub(crate) fn get_hwnd(&self) -> Option<HWND> {
        match self {
            Self::AllOtherForegroundDestroy { hwnd } => Some(hwnd.as_hwnd()),
            Self::ExplorerDestroy |
            Self::ForegroundLocationChange |
            Self::ShellExperienceHostDestroy |
            Self::ShellExperienceHostLocationChange |
            Self::TaskbarLocationChange => {
                None
            }
        }
    }
}
impl Display for WinEventHookContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.get_hwnd() {
            Some(hwnd) => write!(f, "{}: hwnd: {:p}, exe: {}, caption: {}", self.name(), hwnd.0, hwnd.get_exe_or_err(), hwnd.get_caption_or_err()),
            None => write!(f, "{}", self.name())
        }
    }
}
impl Name for WinEventHookContext {
    fn name(&self) -> &'static str{
        self.into()
    }
}

fn cleanup_hooks(hooks: &[HWINEVENTHOOK]) { unsafe {
    for hook in hooks {
        if let Err(err) = UnhookWinEvent(*hook).ok() &&
            err.as_win32_error() != ERROR_INVALID_HANDLE
        {
            error!("{}: failed to unhook win event hooks - closing: {}", module_path!(), err);

            PostQuitMessage(1);
        }
    }
} }

fn error_and_close(msg_name: &str, err: ErrLoc) { unsafe {
    error!("{}: failed to dispatch message - closing: {}: {}", module_path!(), msg_name, err);

    PostQuitMessage(1);
} }

fn dispatch_msg<T>(msg: T) where
    T: Dispatch + Name
{
    let msg_name = msg.name();
    THREAD_STATE.with(|ts| -> Res<()> {
        let sxs = &ts.get().unwrap().sxs;

        msg.dispatch(sxs)?;

        Ok(())
    })
    .unwrap_or_else(|err| error_and_close(msg_name, err));
}

unsafe extern "system" fn all_other_foreground_destroy_proc(hook: HWINEVENTHOOK, _: u32, hwnd: HWND, id_obj: i32, _: i32, _: u32, _: u32) {
    if id_obj == OBJID_WINDOW.0 {
        dispatch_msg(WindowForegroundMsg::WinEventHookAllOtherForegroundDestroy { hook: hook.0 as usize, hwnd: hwnd.as_usize() });
    }
}

unsafe extern "system" fn explorer_destroy_proc(_: HWINEVENTHOOK, _: u32, hwnd: HWND, id_obj: i32, _: i32, _: u32, _: u32) {
    if id_obj == OBJID_WINDOW.0 {
        dispatch_msg(WindowForegroundMsg::WinEventHookExplorerDestroy { hwnd: hwnd.as_usize() });
    }
}

unsafe extern "system" fn shell_experience_host_destroy_proc(hook: HWINEVENTHOOK, _: u32, hwnd: HWND, id_obj: i32, _: i32, _: u32, _: u32) {
    if id_obj == OBJID_WINDOW.0 {
        dispatch_msg(WindowForegroundMsg::WinEventHookShellExperienceHostDestroy { hook: hook.0 as usize, hwnd: hwnd.as_usize() });
    }
}

unsafe extern "system" fn foreground_location_change_proc(_: HWINEVENTHOOK, _: u32, hwnd: HWND, id_obj: i32, _: i32, _: u32, _: u32) {
    if id_obj == OBJID_WINDOW.0 {
        dispatch_msg(WindowForegroundMsg::WinEventHookForegroundLocationChange { hwnd: hwnd.as_usize() });
    }
}

unsafe extern "system" fn shell_experience_host_location_change_proc(hook: HWINEVENTHOOK, _: u32, hwnd: HWND, id_obj: i32, _: i32, _: u32, _: u32) {
    if id_obj == OBJID_WINDOW.0 {
        dispatch_msg(WindowForegroundMsg::WinEventHookShellExperienceHostLocationChange { hook: hook.0 as usize, hwnd: hwnd.as_usize() });
    }
}

unsafe extern "system" fn taskbar_location_change_proc(_: HWINEVENTHOOK, _: u32, hwnd: HWND, id_obj: i32, _: i32, _: u32, _: u32) {
    if id_obj == OBJID_WINDOW.0 {
        dispatch_msg(WindowForegroundMsg::WinEventHookTaskbarLocationChange { hwnd: hwnd.as_usize() });
    }
}

unsafe extern "system" fn all_foreground_proc(_: HWINEVENTHOOK, _: u32, hwnd: HWND, _: i32, _: i32, _: u32, _: u32) {
    if !hwnd.is_invalid() {
        dispatch_msg(WindowForegroundMsg::WinEventHookAllForeground { hwnd: hwnd.as_usize() });
    }
}

unsafe extern "system" fn window_shift_proc(_: HWINEVENTHOOK, event: u32, hwnd: HWND, id_obj: i32, _id_child: i32, _: u32, _: u32) {
    #[cfg(feature = "dbg_window_watch_win_events")]
    info!("{}: {} ({:#06x}): hwnd: {:p}, id_obj: {:#x}, id_child: {:#x}", module_path!(), event._to_event_string(), event, hwnd.0, id_obj, _id_child);

    let msg = match event {
        EVENT_OBJECT_DESTROY if id_obj == OBJID_WINDOW.0 => WindowShiftMsg::Destroy(hwnd.as_usize()),
        EVENT_SYSTEM_MENUSTART => WindowShiftMsg::MenuStart,
        EVENT_SYSTEM_MENUEND => WindowShiftMsg::MenuEnd,
        _ => return
    };
    dispatch_msg(msg);
}

unsafe extern "system" fn hitbox_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT { unsafe {
    match msg {
        WM_CLOSE => {
            info!("{}: closing hitbox", module_path!());

            DefWindowProcW(hwnd, msg, wparam, lparam)
        },
        WM_CREATE => LRESULT(0),
        WM_DESTROY => {
            info!("{}: destroying hitbox", module_path!());

            PostQuitMessage(0);

            LRESULT(0)
        },
        WM_DISPLAYCHANGE => {
            dispatch_msg(BroadcastMsg::WmDisplayChange(lparam));

            DefWindowProcW(hwnd, msg, wparam, lparam)
        },
        WM_MOUSEMOVE => {
            let now = now!();

            dispatch_msg(WindowForegroundMsg::WmMouseMove(lparam, now));

            LRESULT(0)
        },
        WM_NCCREATE => LRESULT(1),
        _ => DefWindowProcW(hwnd, msg, wparam, lparam)
    }
} }

unsafe extern "system" fn message_only_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT { unsafe {
    if msg == WM_DISPLAYCHANGE {
        dispatch_msg(BroadcastMsg::WmDisplayChange(lparam));
    }

    DefWindowProcW(hwnd, msg, wparam, lparam)
} }

fn init_hitbox(sxs: &Senders) -> Res1<HWND> { unsafe {
    SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(all_foreground_proc), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS | WINEVENT_SKIPOWNTHREAD).win32_var_ok()?;

    let config = config::get().read()?;
    let taskbar_config = config.taskbar.as_ref().ok_or(ErrVar::MissingConfigKey { name: config::Taskbar::NAME })?;

    // Progman
    let progman_class_name = PROGMAN_CLASS_NAME.to_win_str();
    let progman_hwnd = FindWindowW(Some(&*progman_class_name), None)?;
    let progman_tpids = progman_hwnd.get_thread_proc_ids()?;
    let (sx, progman_hook) = oneshot::channel();
    sx.send(Ok(vec![SetWinEventHook(EVENT_OBJECT_DESTROY, EVENT_OBJECT_DESTROY, None, Some(explorer_destroy_proc), progman_tpids.proc, progman_tpids.thread, WINEVENT_OUTOFCONTEXT).win32_var_ok()?])).unwrap();

    // Taskbar
    let taskbar_class_name = TASKBAR_CLASS_NAME.to_win_str();
    let taskbar_hwnd = FindWindowW(Some(&*taskbar_class_name), None)?;
    let taskbar_tpids = taskbar_hwnd.get_thread_proc_ids()?;
    let (sx, taskbar_hooks) = oneshot::channel();
    sx.send(Ok(vec![
        SetWinEventHook(EVENT_OBJECT_DESTROY, EVENT_OBJECT_DESTROY, None, Some(explorer_destroy_proc), taskbar_tpids.proc, taskbar_tpids.thread, WINEVENT_OUTOFCONTEXT).win32_var_ok()?,
        SetWinEventHook(EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE, None, Some(taskbar_location_change_proc), taskbar_tpids.proc, taskbar_tpids.thread, WINEVENT_OUTOFCONTEXT).win32_var_ok()?
    ]))
    .unwrap();

    let start_menu_class_name = TASKBAR_START_MENU_CLASS_NAME.to_win_str();
    let start_menu_hwnd = FindWindowExW(Some(taskbar_hwnd), None, *start_menu_class_name, None)?;
    let start_menu_rect = start_menu_hwnd.get_rect()?;

    let screen_extent = get_screen_extent()?;
    let taskbar_rect = taskbar_hwnd.get_rect()?;
    let taskbar_side = get_taskbar_side(taskbar_rect, screen_extent);
    let (hitbox_exit_snap_ordinate,
        cursor_watch
    ) = taskbar_config.hitbox_exit.cursor_snap_offset_pc.map(|pc| {
        let snap_ordinate = get_hitbox_exit_snap_ordinate(taskbar_side, screen_extent, pc);
        let cursor_watch = cursor_watch::begin(snap_ordinate, screen_extent);

        (snap_ordinate, cursor_watch)
    })
    .unzip();
    let hitbox_entry_side = taskbar_config.hitbox_entry.side.unwrap_or(taskbar_side);
    let hitbox_pos = get_hitbox_pos(taskbar_rect, taskbar_side, hitbox_entry_side, i32::from(taskbar_config.hitbox_entry.inset_px), i32::from(taskbar_config.hitbox_exit.taskbar_offset_px), screen_extent);

    let class_name = WINDOW_WATCH_CLASS_NAME.to_win_str();
    let exe_module = GetModuleHandleW(None)?;
    let cursor_hnd = LoadCursorW(None, IDC_ARROW)?;
    let wnd_class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(hitbox_proc),
        hInstance: exe_module.into(),
        hCursor: cursor_hnd,
        lpszClassName: *class_name,
        ..default!()
    };
    _ = RegisterClassExW(&wnd_class).win32_core_ok()?;

    let hitbox_hwnd = CreateWindowExW(
        WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_TRANSPARENT,
        *class_name,
        *class_name,
        WS_POPUP,
        hitbox_pos.entry.x,
        hitbox_pos.entry.y,
        screen_extent.width,
        screen_extent.height,
        None,
        None,
        Some(exe_module.into()),
        None
    )?;

    let tb = Taskbar {
        progman_class_name,
        progman_hwnd,
        progman_tpids,
        progman_hook: Some(progman_hook),
        taskbar_class_name,
        taskbar_hwnd,
        taskbar_tpids,
        taskbar_hooks: Some(taskbar_hooks),
        taskbar_side,
        taskbar_rect,
        start_menu_class_name,
        start_menu_hwnd,
        start_menu_rect,
        hitbox_hwnd,
        hitbox_pos,
        hitbox_entry_side,
        hitbox_entry_inset_px: i32::from(taskbar_config.hitbox_entry.inset_px),
        hitbox_entry_cursor_snap_offset_px: taskbar_config.hitbox_entry.cursor_snap_offset_px.unwrap_or(0),
        hitbox_exit_taskbar_offset_px: i32::from(taskbar_config.hitbox_exit.taskbar_offset_px),
        hitbox_exit_jump_list_offset_px: i32::from(taskbar_config.hitbox_exit.jump_list_offset_px),
        hitbox_exit_cursor_snap_offset_pc: taskbar_config.hitbox_exit.cursor_snap_offset_pc,
        hitbox_exit_snap_ordinate,
        cursor_watch,
        screen_extent,
        ..default!()
    };

    sxs.window_foreground.as_ref().unwrap().send(WindowForegroundMsg::Taskbar(Box::new(tb)))?;

    Ok(hitbox_hwnd)
} }

fn begin(enable: WindowForegroundComponents, sxs: Senders, ready_sx: Sender<ReadyMsg>) -> Res<()> { unsafe {
    info!("{}: begin", module_path!());

    let mut msg = MSG::default();

    THREAD_STATE.with(|ts| -> Res<()> {
        if enable.contains(WindowForegroundComponents::TASKBAR) {
            init_hitbox(&sxs)?;
        }

        if enable.contains(WindowForegroundComponents::DYNAMIC_BINDS | !WindowForegroundComponents::TASKBAR) {
            SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(all_foreground_proc), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS | WINEVENT_SKIPOWNTHREAD).win32_var_ok()?;
        }

        if enable.contains(WindowForegroundComponents::WINDOW_SHIFT) {
            SetWinEventHook(EVENT_OBJECT_DESTROY, EVENT_OBJECT_DESTROY, None, Some(window_shift_proc), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS | WINEVENT_SKIPOWNTHREAD).win32_var_ok()?;
            SetWinEventHook(EVENT_SYSTEM_MENUSTART, EVENT_SYSTEM_MENUSTART, None, Some(window_shift_proc), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS | WINEVENT_SKIPOWNTHREAD).win32_var_ok()?;
            SetWinEventHook(EVENT_SYSTEM_MENUEND, EVENT_SYSTEM_MENUEND, None, Some(window_shift_proc), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS | WINEVENT_SKIPOWNTHREAD).win32_var_ok()?;
        }

        if enable == WindowForegroundComponents::WINDOW_SHIFT {
            let class_name = WINDOW_WATCH_CLASS_NAME.to_win_str();
            let exe_module = GetModuleHandleW(None)?;
            let wnd_class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(message_only_proc),
                lpszClassName: *class_name,
                ..default!()
            };
            RegisterClassExW(&wnd_class).win32_core_ok()?;

            CreateWindowExW(
                default!(),
                *class_name,
                *class_name,
                default!(),
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                None,
                None,
                Some(exe_module.into()),
                None
            )?;
        }

        ts.set(ThreadState { sxs }).map_err(|_| ErrVar::FailedSetOnceCell)?;

        PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE).as_bool();

        let tid = GetCurrentThreadId();
        ready_sx.send(ReadyMsg::WindowWatch(Tid(tid)))?;
        drop(ready_sx);

        Ok(())
    })?;

    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        match msg.message {
            WM_OGOS_CLOSE => PostQuitMessage(0),
            WM_OGOS_RELOAD_CONFIG => dispatch_msg(BroadcastMsg::WmReloadConfig),
            WM_OGOS_REQUEST_WIN_EVENT_HOOKS => {
                let (sx, request) = *Box::from_raw(msg.lParam.0 as *mut (Option<WinEventHooksSx>, WinEventHookRequest));
                let mut hooks = Vec::new();

                let mut try_set_all = || -> Res<()> {
                    for info in request.infos.iter() {
                        let callback = match info.ctx {
                            WinEventHookContext::AllOtherForegroundDestroy { .. } => all_other_foreground_destroy_proc,
                            WinEventHookContext::ExplorerDestroy => explorer_destroy_proc,
                            WinEventHookContext::ForegroundLocationChange => foreground_location_change_proc,
                            WinEventHookContext::ShellExperienceHostDestroy => shell_experience_host_destroy_proc,
                            WinEventHookContext::ShellExperienceHostLocationChange => shell_experience_host_location_change_proc,
                            WinEventHookContext::TaskbarLocationChange => taskbar_location_change_proc
                        };

                        let hook = SetWinEventHook(info.eventmin, info.eventmax, None, Some(callback), info.idprocess, info.idthread, WINEVENT_OUTOFCONTEXT)
                            .win32_var_ok()
                            .map_err(|_| {
                                cleanup_hooks(&hooks);

                                // Notify that hook couldn't be set - undo any state that was set on request
                                if let WinEventHookContext::AllOtherForegroundDestroy { hwnd } = info.ctx {
                                    dispatch_msg(WindowForegroundMsg::WinEventHookAllOtherForegroundDestroy { hook: 0, hwnd });
                                }

                                ErrVar::FailedSetWinEventHooks { ctx: info.ctx.to_string() }
                            })?;

                        hooks.push(hook);
                    }

                    Ok(())
                };

                // Bundle hooks on no errors
                let res = try_set_all().map(|_| hooks);

                // If a oneshot channel is available, send result. Else report error, if any
                match sx {
                    Some(sx) => sx.send(res).unwrap_or_else(|_| {
                        error!("{}: failed to send win event hooks - closing", module_path!());

                        PostQuitMessage(1);
                    }),
                    None => if let Err(err) = res {
                        error!("{}: {}", module_path!(), err);
                    }
                }
            },
            WM_OGOS_REQUEST_WIN_EVENT_UNHOOKS => {
                let request = Box::from_raw(msg.lParam.0 as *mut WinEventUnhookRequest);

                cleanup_hooks(&request.hooks);
            },
            _ => { DispatchMessageW(&msg); }
        }
    }

    info!("{}: closed", module_path!());

    Ok(())
} }

pub(crate) fn spawn(enable: WindowForegroundComponents, sxs: Senders, ready_sx: Sender<ReadyMsg>) -> JoinHandle<()> {
    thread::spawn(move || {
        begin(enable, sxs, ready_sx).unwrap_or_else(|err| {
            error!("{}: terminated: {}", module_path!(), err);
        });
    })
}
