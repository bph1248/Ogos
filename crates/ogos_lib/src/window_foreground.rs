use crate::{
    binds::{self, *},
    cursor_watch::*,
    win32::*,
    window_watch::*,
    *
};
use ogos_config as config;
use ogos_core::*;
use ogos_err::*;
use ogos_mki as mki;
use mki::*;

use bitflags::bitflags;
use log::*;
use std::{
    collections::*,
    mem,
    sync::{
        atomic::*,
        mpsc
    },
    thread::{self, *},
    time::*
};
use strum::*;
use tokio::sync::oneshot;
use windows::Win32::{
    Foundation::*,
    UI::{
        Accessibility::*,
        WindowsAndMessaging::*
    }
};

pub(crate) const PROGMAN_CLASS_NAME: &str = "Progman";
pub(crate) const TASKBAR_START_MENU_CLASS_NAME: &str = "Start";
pub(crate) const WORKERW_CLASS_NAME: &str = "WorkerW";
pub(crate) const WINDOW_WATCH_CLASS_NAME: &str = "OgosWindowWatch";

struct Binds<'a> {
    qmk: Option<QmkRuntime>,
    maps: Option<HashMap<&'a str, InputEventMaps>>,
    bound: Vec<InputEventMap>,
    input_hooks_disabled: bool,
    error_sx: mpsc::Sender<String>
}
impl<'a> Binds<'a> {
    fn has_maps(&self, exe: &str) -> HasMaps {
        match self.maps.as_ref().and_then(|maps| maps.get(exe)) {
            Some(input_event_maps) => {
                input_event_maps.iter().find_map(|input_event_map| {
                    if input_event_map.requires_mouse_hook() {
                        return Some(HasMaps::InclMouse)
                    }

                    None
                })
                .unwrap_or(HasMaps::Keyboard)
            },
            None => HasMaps::None
        }
    }
}

impl<'a> Drop for Binds<'a> {
    fn drop(&mut self) {
        unbind_maps(self);
    }
}

bitflags! {
    #[derive(Clone, Copy, Default, PartialEq)]
    pub(crate) struct EnabledChannels: u32 {
        const WINDOW_FOREGROUND = 0b00000001;
        const WINDOW_SHIFT      = 0b00000010;
    }

    #[derive(Clone, Copy, PartialEq)]
    pub(crate) struct WindowForegroundComponents: u32 {
        const DYNAMIC_BINDS     = 0b00000001;
        const TASKBAR           = 0b00000010;
        const WINDOW_SHIFT      = 0b00000100;
    }
}

#[derive(Default)]
pub(crate) struct HitboxPos {
    pub(crate) entry: POINT,
    pub(crate) exit: POINT
}

pub(crate) type WinEventHooksRx = oneshot::Receiver<Res<Vec<HWINEVENTHOOK>>>;
pub(crate) type WinEventHooksSx = oneshot::Sender<Res<Vec<HWINEVENTHOOK>>>;

#[derive(Default)]
pub(crate) struct Senders {
    pub(crate) window_foreground: Option<mpsc::Sender<Msg>>,
    pub(crate) window_shift: Option<mpsc::Sender<window_shift::Msg>>
}

#[derive(Default)]
pub(crate) struct Receivers {
    pub(crate) window_foreground: Option<mpsc::Receiver<Msg>>,
    pub(crate) window_shift: Option<mpsc::Receiver<window_shift::Msg>>
}

#[derive(Default)]
pub(crate) struct LongLivedChannels {
    pub(crate) enabled: EnabledChannels,
    pub(crate) sxs: Senders,
    pub(crate) rxs: Receivers
}
impl LongLivedChannels {
    pub(crate) fn with_window_foreground(&mut self) {
        let channel = mpsc::channel::<window_foreground::Msg>();

        self.enabled |= EnabledChannels::WINDOW_FOREGROUND;
        self.sxs.window_foreground = Some(channel.0);
        self.rxs.window_foreground = Some(channel.1);
    }

    pub(crate) fn with_window_shift(&mut self) {
        let channel = mpsc::channel::<window_shift::Msg>();

        self.enabled |= EnabledChannels::WINDOW_SHIFT;
        self.sxs.window_shift = Some(channel.0);
        self.rxs.window_shift = Some(channel.1);
    }
}

#[derive(Default)]
pub(crate) struct Taskbar {
    pub(crate) progman_class_name: WinStr,
    pub(crate) progman_hwnd: HWND,
    pub(crate) progman_tpids: Tpids,
    pub(crate) progman_hook: Option<WinEventHooksRx>,
    pub(crate) taskbar_class_name: WinStr,
    pub(crate) taskbar_hwnd: HWND,
    pub(crate) taskbar_tpids: Tpids,
    pub(crate) taskbar_hooks: Option<WinEventHooksRx>,
    pub(crate) taskbar_side: Side,
    pub(crate) taskbar_rect: RECT,
    pub(crate) shell_experience_state: Option<(HWND, PendingHooks)>,
    pub(crate) start_menu_class_name: WinStr,
    pub(crate) start_menu_hwnd: HWND,
    pub(crate) start_menu_rect: RECT,
    pub(crate) hitbox_hwnd: HWND,
    pub(crate) hitbox_pos: HitboxPos,
    pub(crate) hitbox_state: HitboxState,
    pub(crate) hitbox_entry_side: Side,
    pub(crate) hitbox_entry_inset_px: i32,
    pub(crate) hitbox_entry_cursor_snap_offset_px: i32,
    pub(crate) hitbox_exit_taskbar_offset_px: i32,
    pub(crate) hitbox_exit_jump_list_offset_px: i32,
    pub(crate) hitbox_exit_cursor_snap_offset_pc: Option<u32>,
    pub(crate) hitbox_exit_snap_ordinate: Option<i32>,
    pub(crate) hitbox_exit_cursor_should_have_snapped: bool,
    pub(crate) hitbox_mouse_move_anchor: Option<Instant>,
    pub(crate) cursor_watch: Option<Arc<CursorWatch>>,
    pub(crate) foreground_hwnd: HWND,
    pub(crate) loc_change_hook: Option<WinEventHooksRx>,
    pub(crate) screen_extent: Extent2d,
    pub(crate) screen_extent_changed: bool
}
unsafe impl Send for Taskbar {}

#[derive(Default)]
struct ThreadState<'a> {
    hook_mgr_tid: u32,
    win_infos: HashMap<usize, WinInfo>,
    win_errored: HashMap<usize, Errored>,
    last_foreground_tpids: Tpids,
    active_game: Option<String>,
    thread_hwnd_counts: HashMap<Tid, u32>,
    binds: Option<Binds<'a>>,
    tb: Option<Taskbar>
}

struct WinInfo {
    tpids: Tpids,
    exe: String,
    may_hook_loc_change: bool,
    has_maps: HasMaps
}

#[derive(Default, PartialEq)]
enum HasMaps {
    #[default]
    None,
    Keyboard,
    InclMouse
}
impl HasMaps {
    fn as_bool(&self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Default, PartialEq)]
pub(crate) enum HitboxState {
    #[default]
    Entry,
    Exit
}

#[derive(Display, IntoStaticStr)]
pub(crate) enum Msg {
    Broadcast(BroadcastMsg),
    Pipe((pipe_server::Msg, oneshot::Sender<()>)),
    Taskbar(Box<Taskbar>),
    WinEventHookAllForeground { hwnd: usize },
    WinEventHookAllOtherForegroundDestroy { hook: usize, hwnd: usize },
    WinEventHookExplorerDestroy { hwnd: usize },
    WinEventHookForegroundLocationChange { hwnd: usize },
    WinEventHookShellExperienceHostDestroy { hook: usize, hwnd: usize },
    WinEventHookShellExperienceHostLocationChange { hook: usize, hwnd: usize },
    WinEventHookTaskbarLocationChange { hwnd: usize },
    WmMouseMove(LPARAM, Instant)
}
impl VarName for Msg {
    fn var_name(&self) -> &'static str {
        self.into()
    }
}

pub(crate) enum PendingHooks {
    Ok(),
    Check(WinEventHooksRx)
}

fn leak_win_event_hooks(hook_mgr_tid: u32, request: WinEventHookRequest) -> ResVar<()> { unsafe {
    let cargo: (Option<WinEventHooksSx>, _) = (None, request);
    let cargo = Box::into_raw(Box::new(cargo));

    if let Err(err) = PostThreadMessageW(hook_mgr_tid, WM_OGOS_REQUEST_WIN_EVENT_HOOKS, WPARAM(0), LPARAM(cargo as isize)) {
        Err(ErrVar::FailedContactHookMgr { inner: err })?;
    }

    Ok(())
} }

fn request_win_event_hooks(hook_mgr_tid: u32, request: WinEventHookRequest) -> windows::core::Result<WinEventHooksRx> { unsafe {
    let (sx, rx) = oneshot::channel::<Res<Vec<HWINEVENTHOOK>>>();
    let cargo = (Some(sx), request);
    let cargo = Box::into_raw(Box::new(cargo));

    PostThreadMessageW(hook_mgr_tid, WM_OGOS_REQUEST_WIN_EVENT_HOOKS, WPARAM(0), LPARAM(cargo as isize))?;

    Ok(rx)
} }

fn request_win_event_unhooks(hook_mgr_tid: u32, request: WinEventUnhookRequest) -> windows::core::Result<()> { unsafe {
    let cargo = Box::into_raw(Box::new(request));

    PostThreadMessageW(hook_mgr_tid, WM_OGOS_REQUEST_WIN_EVENT_UNHOOKS, WPARAM(0), LPARAM(cargo as isize))?;

    Ok(())
} }

fn handle_wm_display_change(tb: &mut Taskbar, lparam: LPARAM) {
    tb.screen_extent.width = (lparam.0 & 0xFFFF) as i32;
    tb.screen_extent.height = ((lparam.0 >> 16) & 0xFFFF) as i32;

    tb.hitbox_pos = get_hitbox_pos(tb.taskbar_rect, tb.taskbar_side, tb.hitbox_entry_side, tb.hitbox_entry_inset_px, tb.hitbox_exit_taskbar_offset_px, tb.screen_extent);
    if let Some(cursor_watch) = tb.cursor_watch.as_ref() {
        cursor_watch.sx.send(cursor_watch::Msg::DisplayChange(tb.screen_extent)).unwrap();
    }

    tb.screen_extent_changed = true;
}

fn handle_wm_mouse_move(tb: &mut Taskbar, _lparam: LPARAM, stamp: Instant) -> Res1<()> { unsafe {
    if let Some(anchor) = tb.hitbox_mouse_move_anchor &&
        stamp.duration_since(anchor).is_zero() // WM_MOUSEMOVE msg is too old
    {
        return  Ok(())
    }

    let mut cursor_pos = POINT::default();
    GetCursorPos(&mut cursor_pos)?;

    match tb.hitbox_state {
        HitboxState::Entry => {
            if let Some(cursor_watch) = tb.cursor_watch.as_ref() &&
                cursor_watch.working.load(Ordering::Relaxed)
            {
                cursor_watch.request_stop.store(true, Ordering::Relaxed);

                return Ok(())
            }

            match tb.hitbox_entry_side {
                Side::Left => {
                    match tb.taskbar_side {
                        Side::Top | Side::Bottom => send_cursor_pos(tb.start_menu_rect.left + tb.hitbox_entry_cursor_snap_offset_px, tb.start_menu_rect.top + tb.start_menu_rect.height() / 2, tb.screen_extent)?,
                        Side::Right => send_cursor_pos(tb.start_menu_rect.right, cursor_pos.y, tb.screen_extent)?,
                        _ => ()
                    }
                },
                Side::Top => {
                    match tb.taskbar_side {
                        Side::Left | Side::Right => send_cursor_pos(tb.start_menu_rect.left + tb.start_menu_rect.width() / 2, tb.start_menu_rect.top + tb.hitbox_entry_cursor_snap_offset_px, tb.screen_extent)?,
                        Side::Bottom => send_cursor_pos(cursor_pos.x, tb.start_menu_rect.bottom, tb.screen_extent)?,
                        _ => ()
                    }
                }
                Side::Right => {
                    match tb.taskbar_side {
                        Side::Top | Side::Bottom => send_cursor_pos(tb.start_menu_rect.left + tb.hitbox_entry_cursor_snap_offset_px, tb.start_menu_rect.top + tb.start_menu_rect.height() / 2, tb.screen_extent)?,
                        Side::Left => send_cursor_pos(tb.start_menu_rect.left, cursor_pos.y, tb.screen_extent)?,
                        _ => ()
                    }
                },
                Side::Bottom => {
                    match tb.taskbar_side {
                        Side::Left | Side::Right => send_cursor_pos(tb.start_menu_rect.left + tb.start_menu_rect.width() / 2, tb.start_menu_rect.top + tb.hitbox_entry_cursor_snap_offset_px, tb.screen_extent)?,
                        Side::Top => send_cursor_pos(cursor_pos.x, tb.start_menu_rect.top, tb.screen_extent)?,
                        _ => ()
                    }
                }
            }

            SetWindowPos(tb.hitbox_hwnd, Some(HWND_TOPMOST), tb.hitbox_pos.exit.x, tb.hitbox_pos.exit.y, 0, 0, SWP_NOACTIVATE | SWP_NOCOPYBITS | SWP_NOSIZE)?;
            tb.hitbox_mouse_move_anchor = Some(now!());
            tb.taskbar_hwnd.show_na(false)?;

            tb.hitbox_state = HitboxState::Exit;
        },
        HitboxState::Exit => {
            match tb.hitbox_exit_snap_ordinate {
                Some(snap_ordinate) => {
                    match tb.hitbox_exit_cursor_should_have_snapped {
                        true => {
                            SetWindowPos(tb.hitbox_hwnd, Some(HWND_TOPMOST), tb.hitbox_pos.entry.x, tb.hitbox_pos.entry.y, 0, 0, SWP_NOACTIVATE | SWP_NOCOPYBITS | SWP_NOSIZE)?;
                            tb.hitbox_mouse_move_anchor = Some(now!());
                            tb.taskbar_hwnd.hide()?;

                            tb.hitbox_exit_cursor_should_have_snapped = false;
                            tb.hitbox_state = HitboxState::Entry;

                            if let Some(cursor_watch) = tb.cursor_watch.as_ref() {
                                cursor_watch.working.store(true, Ordering::Relaxed);
                                cursor_watch.request_stop.store(false, Ordering::Relaxed);

                                cursor_watch.sx.send(cursor_watch::Msg::Begin).unwrap();
                            }
                        },
                        false => { // Snap the cursor. Landing on the hitbox will create another event whereby the hitbox will be moved.
                            match tb.taskbar_side {
                                Side::Top | Side::Bottom => {
                                    tb.hitbox_mouse_move_anchor = Some(now!());
                                    send_cursor_pos(cursor_pos.x, snap_ordinate, tb.screen_extent)?;
                                },
                                Side::Left | Side::Right => send_cursor_pos(snap_ordinate, cursor_pos.y, tb.screen_extent)?
                            };

                            tb.hitbox_exit_cursor_should_have_snapped = true;
                        }
                    }
                },
                None => {
                    SetWindowPos(tb.hitbox_hwnd, Some(HWND_TOPMOST), tb.hitbox_pos.entry.x, tb.hitbox_pos.entry.y, 0, 0, SWP_NOACTIVATE | SWP_NOCOPYBITS | SWP_NOSIZE)?;
                    tb.hitbox_mouse_move_anchor = Some(now!());
                    tb.taskbar_hwnd.hide()?;

                    tb.hitbox_state = HitboxState::Entry;
                }
            }
        }
    }

    Ok(())
} }

fn handle_win_event_hook_all_other_foreground_destroy(ts: &mut ThreadState, hook: HWINEVENTHOOK, hwnd: HWND) -> windows::core::Result<()> {
    if let Some(win_info) = ts.win_infos.remove(&hwnd.as_usize()) {
        if let Some(hwnd_count) = ts.thread_hwnd_counts.get_mut(&win_info.tpids.thread.into()) {
            *hwnd_count -= 1;

            if *hwnd_count == 0 {
                if !hook.is_invalid() {
                    let request = WinEventUnhookRequest { hooks: vec![hook] };
                    request_win_event_unhooks(ts.hook_mgr_tid, request)?;
                }

                ts.thread_hwnd_counts.remove(&win_info.tpids.thread.into());
            }
        }

        ts.win_errored.remove(&hwnd.as_usize());
    }

    Ok(())
}

fn handle_win_event_hook_explorer_destroy(ts: &mut ThreadState, hwnd: HWND) -> Res1<()> { unsafe {
    let tb = ts.tb.as_mut().unwrap();

    match hwnd {
        _ if hwnd == tb.progman_hwnd => {
            if let Some(rx) = tb.progman_hook.take() {
                let request = WinEventUnhookRequest { hooks: rx.blocking_recv()?? };
                request_win_event_unhooks(ts.hook_mgr_tid, request)?;
            }

            let rehook_progman = || -> Res<()> {
                let progman_hwnd = FindWindowW(Some(&*tb.progman_class_name), None)?;
                let progman_tpids = progman_hwnd.get_thread_proc_ids()?;

                let request = WinEventHookRequest {
                    infos: vec![WinEventHookInfo { eventmin: EVENT_OBJECT_DESTROY, eventmax: EVENT_OBJECT_DESTROY, idprocess: progman_tpids.proc, idthread: progman_tpids.thread, ctx: WinEventHookContext::ExplorerDestroy }]
                };
                let rx = request_win_event_hooks(ts.hook_mgr_tid, request)?;

                tb.progman_hwnd = progman_hwnd;
                tb.progman_tpids = progman_tpids;
                tb.progman_hook = Some(rx);

                info!("{}: rehooked progman", module_path!());

                Ok(())
            };

            attempt(rehook_progman, 10, Duration::from_secs(1))
                .unwrap_or_else(|_| {
                    panic!("{}: failed to rehook progman", module_path!())
                });
        },
        _ if hwnd == tb.taskbar_hwnd => {
            if let Some(rx) = tb.taskbar_hooks.take() {
                let request = WinEventUnhookRequest { hooks: rx.blocking_recv()?? };
                request_win_event_unhooks(ts.hook_mgr_tid, request)?;
            }

            let rehook_taskbar = || -> Res<()> {
                let taskbar_hwnd = FindWindowW(Some(&*tb.taskbar_class_name), None)?;
                let start_menu_hwnd = FindWindowExW(Some(taskbar_hwnd), None, *tb.start_menu_class_name, None)?;
                let taskbar_tpids = taskbar_hwnd.get_thread_proc_ids()?;

                let request = WinEventHookRequest {
                    infos: vec![
                        WinEventHookInfo { eventmin: EVENT_OBJECT_DESTROY, eventmax: EVENT_OBJECT_DESTROY, idprocess: taskbar_tpids.proc, idthread: taskbar_tpids.thread, ctx: WinEventHookContext::ExplorerDestroy },
                        WinEventHookInfo { eventmin: EVENT_OBJECT_LOCATIONCHANGE, eventmax: EVENT_OBJECT_LOCATIONCHANGE, idprocess: taskbar_tpids.proc, idthread: taskbar_tpids.thread, ctx: WinEventHookContext::TaskbarLocationChange }
                    ]
                };
                let rx = request_win_event_hooks(ts.hook_mgr_tid, request)?;

                tb.taskbar_hwnd = taskbar_hwnd;
                tb.taskbar_tpids = taskbar_tpids;
                tb.taskbar_hooks = Some(rx);
                tb.start_menu_hwnd = start_menu_hwnd;

                taskbar_hwnd.hide()?;

                info!("{}: rehooked taskbar", module_path!());

                Ok(())
            };

            attempt(rehook_taskbar, 10, Duration::from_secs(1))
                .unwrap_or_else(|err| {
                    panic!("{}: failed to rehook taskbar: {}", module_path!(), err)
                });
        },
        _ => ()
    }

    ts.win_errored.remove(&hwnd.as_usize());

    Ok(())
} }

fn handle_win_event_hook_shell_experience_host_destroy(ts: &mut ThreadState, hook: HWINEVENTHOOK, hwnd: HWND) -> Res1<()> {
    let tb = ts.tb.as_mut().unwrap();

    if matches!(tb.shell_experience_state, Some((shell_experience_hwnd, _)) if shell_experience_hwnd == hwnd) {
        let request = WinEventUnhookRequest { hooks: vec![hook] };
        request_win_event_unhooks(ts.hook_mgr_tid, request)?;

        tb.shell_experience_state = None;
    }

    Ok(())
}

fn handle_win_event_hook_foreground_location_change(tb: &Taskbar, hwnd: HWND) -> Res1<()> {
    if hwnd == tb.foreground_hwnd {
        match hwnd.is_fullscreen(tb.screen_extent)? {
            true => tb.hitbox_hwnd.hide()?,
            false => tb.hitbox_hwnd.show_na(true)?
        }
    }

    Ok(())
}

fn handle_win_event_hook_shell_experience_host_location_change(ts: &mut ThreadState, hook: HWINEVENTHOOK, hwnd: HWND) -> Res2<()> {
    let caption = hwnd.get_caption()?;

    match caption {
        _ if caption == "Windows Shell Experience Host" => return Ok(()), // First entry will match here - will then re-enter as "Jump List"
        _ if caption.starts_with("Jump List") => {
            let tb = ts.tb.as_mut().unwrap();

            let request = WinEventUnhookRequest { hooks: vec![hook] };
            request_win_event_unhooks(ts.hook_mgr_tid, request)?;

            let tpids = hwnd.get_thread_proc_ids()?;
            let request = WinEventHookRequest {
                infos: vec![WinEventHookInfo { eventmin: EVENT_OBJECT_DESTROY, eventmax: EVENT_OBJECT_DESTROY, idprocess: tpids.proc, idthread: tpids.thread, ctx: WinEventHookContext::ShellExperienceHostDestroy }]
            };
            let rx = request_win_event_hooks(ts.hook_mgr_tid, request)?;
            tb.shell_experience_state = Some((hwnd, PendingHooks::Check(rx)));

            move_hitbox_about_jump_list(tb, hwnd)?;
        },
        _ => ()
    }

    Ok(())
}

fn handle_win_event_hook_taskbar_location_change(tb: &mut Taskbar, hwnd: HWND) -> Res1<()> { unsafe {
    match hwnd {
        _ if hwnd == tb.taskbar_hwnd => {
            let taskbar_rect = tb.taskbar_hwnd.get_rect()?;

            if tb.screen_extent_changed ||
                match tb.taskbar_side { // Taskbar dragged to different side
                    Side::Left => {
                        let taskbar_inset = tb.taskbar_rect.width();

                        taskbar_inset < taskbar_rect.right
                    },
                    Side::Top => {
                        let taskbar_inset = tb.taskbar_rect.height();

                        taskbar_inset < taskbar_rect.bottom
                    },
                    Side::Right => {
                        let taskbar_inset = tb.screen_extent.width - tb.taskbar_rect.width();

                        taskbar_rect.left < taskbar_inset
                    },
                    Side::Bottom => {
                        let taskbar_inset = tb.screen_extent.height - tb.taskbar_rect.height();

                        taskbar_rect.top < taskbar_inset
                    }
                }
            {
                let taskbar_side = get_taskbar_side(taskbar_rect, tb.screen_extent);
                let hitbox_pos = get_hitbox_pos(taskbar_rect, taskbar_side, tb.hitbox_entry_side, tb.hitbox_entry_inset_px, tb.hitbox_exit_taskbar_offset_px, tb.screen_extent);

                SetWindowPos(tb.hitbox_hwnd, Some(HWND_TOPMOST), hitbox_pos.entry.x, hitbox_pos.entry.y, tb.screen_extent.width, tb.screen_extent.height, SWP_NOACTIVATE | SWP_NOCOPYBITS)?;
                tb.hitbox_hwnd.show_na(true)?;
                tb.taskbar_hwnd.hide()?;

                tb.taskbar_side = taskbar_side;
                tb.taskbar_rect = taskbar_rect;
                tb.hitbox_pos = hitbox_pos;
                tb.hitbox_state = HitboxState::Entry;
                tb.hitbox_exit_snap_ordinate = tb.hitbox_exit_cursor_snap_offset_pc.map(|pc| {
                    get_hitbox_exit_snap_ordinate(taskbar_side, tb.screen_extent, pc)
                });

                tb.screen_extent_changed = false;
            }
        },
        _ if hwnd == tb.start_menu_hwnd => {
            tb.start_menu_rect = tb.start_menu_hwnd.get_rect()?;
        },
        _ => ()
    }

    Ok(())
} }

fn can_i_have_a_go(binds: &mut Binds, win_info: &WinInfo, active_game: Option<&str>) {
    match active_game {
        Some(exe) if exe == win_info.exe.as_str() => { // Game is foreground
            if !win_info.has_maps.as_bool() || // No maps, no need for input hooks
                win_info.has_maps != HasMaps::InclMouse && binds.qmk.is_some() // No mouse maps/hook, and no need for keyboard hook - QMK can take over
            {
                mki::clear();
                mki::uninstall_hooks();

                binds.input_hooks_disabled = true;
            }
        },
        _ => { // Game is not foreground (active or not)
            match binds.input_hooks_disabled {
                true => { // Game was foreground
                    mki::clear();
                    mki::install_hooks();
                    binds::configure_static_binds(binds.error_sx.clone()).unwrap_or_else(|err| {
                        binds.error_sx.send(format!("{}: failed to reconfigure static binds: {}", module_path!(), err)).unwrap();
                    });

                    binds.input_hooks_disabled = false;
                },
                false => unbind_maps(binds) // Some other app was foreground
            }
        }
    }

    if win_info.has_maps.as_bool() {
        bind_maps(binds, win_info.exe.as_str());
    }
}

fn handle_win_event_hook_all_foreground(ts: &mut ThreadState, hwnd: HWND) -> Res2<()> {
    // Garner info
    let win_info = match ts.win_infos.get(&hwnd.as_usize()) {
        Some(win_info) => win_info,
        None => {
            // Taskbar/hitbox
            if let Some(tb) = ts.tb.as_mut() {
                // Progman
                if hwnd == tb.progman_hwnd {
                    tb.hitbox_hwnd.show_na(true)?;

                    return Ok(())
                }

                // Shell experience host / jump list
                let mut shell_experience_state = tb.shell_experience_state.take_if(|(shell_experience_hwnd, _)| *shell_experience_hwnd == hwnd);
                if let Some((_, PendingHooks::Check(rx))) = shell_experience_state {
                    _ = rx.blocking_recv()??;
                    shell_experience_state = Some((hwnd, PendingHooks::Ok()));
                }
                if let Some((_, PendingHooks::Ok())) = shell_experience_state {
                    tb.shell_experience_state = Some((hwnd, PendingHooks::Ok()));

                    move_hitbox_about_jump_list(tb, hwnd)?;

                    return Ok(())
                }

                let caption = hwnd.get_caption()?;
                match caption {
                    _ if caption == "Windows Shell Experience Host" => { // Shell experience host process doesn't exist
                        let tpids = hwnd.get_thread_proc_ids()?;
                        let request = WinEventHookRequest {
                            infos: vec![WinEventHookInfo { eventmin: EVENT_OBJECT_LOCATIONCHANGE, eventmax: EVENT_OBJECT_LOCATIONCHANGE, idprocess: tpids.proc, idthread: tpids.thread, ctx: WinEventHookContext::ShellExperienceHostLocationChange }]
                        };
                        leak_win_event_hooks(ts.hook_mgr_tid, request)?;

                        return Ok(())
                    },
                    _ if caption.starts_with("Jump List") => { // Shell experience host process exists
                        let tpids = hwnd.get_thread_proc_ids()?;
                        let request = WinEventHookRequest {
                            infos: vec![WinEventHookInfo { eventmin: EVENT_OBJECT_DESTROY, eventmax: EVENT_OBJECT_DESTROY, idprocess: tpids.proc, idthread: tpids.thread, ctx: WinEventHookContext::ShellExperienceHostDestroy }]
                        };
                        tb.shell_experience_state = Some((hwnd, PendingHooks::Check(request_win_event_hooks(ts.hook_mgr_tid, request)?)));

                        move_hitbox_about_jump_list(tb, hwnd)?;

                        return Ok(())
                    },
                    _ => ()
                }
            }

            // Everything else
            let exe = hwnd.get_exe()?;
            let has_maps = ts.binds.as_ref()
                .map(|binds| binds.has_maps(exe.as_str()))
                .unwrap_or_default();

            let win_info = WinInfo {
                exe,
                tpids: hwnd.get_thread_proc_ids()?,
                may_hook_loc_change: hwnd.may_hook_location_change()?,
                has_maps
            };

            let key = Tid(win_info.tpids.thread);
            match ts.thread_hwnd_counts.get_mut(&key) {
                Some(hwnd_count) => *hwnd_count += 1,
                None => {
                    ts.thread_hwnd_counts.insert(key, 1);

                    let request = WinEventHookRequest {
                        infos: vec![WinEventHookInfo { eventmin: EVENT_OBJECT_DESTROY, eventmax: EVENT_OBJECT_DESTROY, idprocess: win_info.tpids.proc, idthread: win_info.tpids.thread, ctx: WinEventHookContext::AllOtherForegroundDestroy { hwnd: hwnd.as_usize() } }]
                    };
                    leak_win_event_hooks(ts.hook_mgr_tid, request)?;
                }
            }

            ts.win_infos.entry(hwnd.as_usize()).or_insert(win_info)
        }
    };

    // Hook location changes and set binds for current foreground
    if win_info.may_hook_loc_change {
        match win_info.tpids != ts.last_foreground_tpids {
            true => {
                ts.last_foreground_tpids = win_info.tpids;

                // Location changes
                if let Some(tb) = ts.tb.as_mut() {
                    tb.foreground_hwnd = hwnd;

                    if let Some(rx) = tb.loc_change_hook.take() {
                        let request = WinEventUnhookRequest { hooks: rx.blocking_recv()?? };
                        request_win_event_unhooks(ts.hook_mgr_tid, request)?;
                    }
                    let request = WinEventHookRequest {
                        infos: vec![WinEventHookInfo { eventmin: EVENT_OBJECT_LOCATIONCHANGE, eventmax: EVENT_OBJECT_LOCATIONCHANGE, idprocess: win_info.tpids.proc, idthread: win_info.tpids.thread, ctx: WinEventHookContext::ForegroundLocationChange }]
                    };
                    let rx = request_win_event_hooks(ts.hook_mgr_tid, request)?;
                    tb.loc_change_hook = Some(rx);
                }

                // Binds
                if let Some(binds) = ts.binds.as_mut() {
                    can_i_have_a_go(binds, win_info, ts.active_game.as_deref());
                }
            },
            false => {
                // Just update current foreground
                if let Some(tb) = ts.tb.as_mut() {
                    tb.foreground_hwnd = hwnd;
                }
            }
        }

        // Handle foreground extent now
        if let Some(tb) = ts.tb.as_ref() {
            match hwnd.is_fullscreen(tb.screen_extent)? {
                true => tb.hitbox_hwnd.hide()?,
                false => tb.hitbox_hwnd.show_na(true)?
            }
        }
    }

    Ok(())
}

fn unbind_maps(binds: &mut Binds) {
    for map in binds.bound.drain(..) {
        match map {
            InputEventMap::PressMirror { from: InputEvent::Keyboard(from), to: InputEvent::Keyboard(_) } => {
                match binds.qmk.as_ref() {
                    Some(qmk) => unmap_qmk(qmk, from),
                    None => unmap_mki(InputEvent::Keyboard(from))
                }
            },
            InputEventMap::PressMirror { from, .. } |
            InputEventMap::WheelClick { from, .. } => unmap_mki(from)
        }
    }
}

fn bind_maps(binds: &mut Binds, exe: &str) {
    let maps = binds.maps.as_ref().unwrap()
        .get(exe)
        .unwrap();

    for map in maps.iter().copied() {
        let press_mirror_action = |to: InputEvent| -> Action {
            Action {
                callback: Box::new(move |_, state| {
                    match state {
                        State::Pressed => to.press(),
                        State::Released => to.release(),
                        _ => ()
                    }
                }),
                inhibit: InhibitEvent::Yes,
                defer: false,
                sequencer: true
            }
        };
        let wheel_click_action = |to: InputEvent, dur: Duration| -> Action {
            Action {
                callback: Box::new(move |_, state| {
                    if matches!(state, State::WheelUp | State::WheelDown) {
                        to.click(dur);
                    }
                }),
                inhibit: InhibitEvent::Yes,
                defer: true,
                sequencer: false
            }
        };

        match map {
            InputEventMap::PressMirror { from: InputEvent::Keyboard(from), to: InputEvent::Keyboard(to) } => {
                match binds.qmk.as_ref() {
                    Some(qmk) => map_qmk(qmk, from, to.as_keycode()),
                    None => from.act_on(press_mirror_action(InputEvent::Keyboard(to)))
                }
            },
            InputEventMap::PressMirror { from, to } => from.act_on(press_mirror_action(to)),
            InputEventMap::WheelClick { from, to, dur } => from.act_on(wheel_click_action(to, dur))
        }

        binds.bound.push(map);
    }
}

fn move_hitbox_about_jump_list(tb: &Taskbar, jump_list_hwnd: HWND) -> Res1<()> { unsafe {
    if tb.taskbar_side == Side::Bottom && tb.hitbox_state == HitboxState::Exit && tb.hitbox_exit_snap_ordinate.is_some() {
        let jump_list_rect = jump_list_hwnd.get_rect()?;
        let hitbox_pos_exit_y = jump_list_rect.top - tb.screen_extent.height - tb.hitbox_exit_jump_list_offset_px;

        SetWindowPos(tb.hitbox_hwnd, Some(HWND_TOPMOST), tb.hitbox_pos.exit.x, hitbox_pos_exit_y, 0, 0, SWP_NOACTIVATE | SWP_NOCOPYBITS | SWP_NOSIZE)?;
    }

    Ok(())
} }

pub(crate) fn get_hitbox_pos(taskbar_rect: RECT, taskbar_side: Side, hitbox_side: Side, hitbox_entry_inset: i32, hitbox_exit_taskbar_offset: i32, screen_extent: Extent2d) -> HitboxPos {
    let screen_rect = screen_extent.into_rect();

    let entry = match hitbox_side {
        Side::Left => POINT { x: -screen_rect.right + hitbox_entry_inset, y: 0 },
        Side::Top => POINT { x: 0, y: -screen_rect.bottom + hitbox_entry_inset },
        Side::Right => POINT { x: screen_rect.right - hitbox_entry_inset, y: 0 },
        Side::Bottom => POINT { x: 0, y: screen_rect.bottom - hitbox_entry_inset }
    };
    let exit = match taskbar_side {
        Side::Left => POINT { x: taskbar_rect.width() + hitbox_exit_taskbar_offset, y: 0 },
        Side::Top => POINT { x: 0, y: taskbar_rect.height() + hitbox_exit_taskbar_offset },
        Side::Right => POINT { x: 0 - taskbar_rect.width() - hitbox_exit_taskbar_offset, y: 0 },
        Side::Bottom => POINT { x: 0, y: 0 - taskbar_rect.height() - hitbox_exit_taskbar_offset }
    };

    HitboxPos {
        entry,
        exit
    }
}

pub(crate) fn get_hitbox_exit_snap_ordinate(taskbar_side: Side, extent_extent: Extent2d, pc: u32) -> i32 {
    let pc = f64::from(pc) / f64::from(100);

    match taskbar_side {
        Side::Left | Side::Right => (f64::from(extent_extent.width) * pc) as i32,
        Side::Top | Side::Bottom => (f64::from(extent_extent.height) * pc) as i32
    }
}

pub(crate) fn get_taskbar_side(taskbar_rect: RECT, screen_extent: Extent2d) -> Side {
    let screen_rect = screen_extent.into_rect();

    match taskbar_rect.width() > taskbar_rect.height() {
        true => { // Horizontal taskbar
            match taskbar_rect.top == screen_rect.top {
                true => Side::Top,
                false => Side::Bottom
            }
        },
        false => { // Vertical taskbar
            match taskbar_rect.left == screen_rect.left {
                true => Side::Left,
                false => Side::Right
            }
        }
    }
}

fn init_binds<'a>(error_sx: &mpsc::Sender<String>) -> Res1<Binds<'a>> {
    (|| -> Res1<_> {
        const QMK_VID: u16 = 0x3434;
        const QMK_PID: u16 = 0x0140;
        const USAGE_PAGE: u16 = 0xff60;

        let config = config::get().read()?;
        let binds_config = config.binds.as_ref().ok_or(ErrVar::MissingConfigKey { name: config::Binds::NAME })?;
        let qmk_config = binds_config.qmk.as_ref();

        let qmk = qmk_config.map(|qmk_config| {
            binds::QmkRuntime::new(QMK_VID, QMK_PID, USAGE_PAGE, qmk_config)
        })
        .transpose()
        .unwrap_or_else(|err| {
            error_sx.send(err.to_string()).unwrap();

            None
        });

        Ok(Binds {
            qmk,
            maps: binds_config.maps.clone(),
            bound: default!(),
            input_hooks_disabled: default!(),
            error_sx: error_sx.clone()
        })
    })()
    .map_err(|err| {
        let err = err.msg("failed to init binds");

        error_sx.send(err.to_string()).unwrap();

        err
    })
}

fn init_taskbar(rx: &mpsc::Receiver<Msg>) -> Res1<Taskbar> {
    match rx.recv()? {
        Msg::Taskbar(tb) => {
            tb.taskbar_hwnd.hide()?;
            tb.hitbox_hwnd.show_na(true)?;

            Ok(*tb)
        },
        _ => Err(ErrVar::MissingTaskbarRelatedInfo)?
    }
}

fn begin(enable: WindowForegroundComponents, rx: mpsc::Receiver<Msg>, hook_mgr_tid: Tid, error_sx: &mpsc::Sender<String>) -> Res<()> { unsafe {
    info!("{}: begin", module_path!());

    let mut ts = ThreadState {
        hook_mgr_tid: hook_mgr_tid.0,
        binds: enable.contains(WindowForegroundComponents::DYNAMIC_BINDS).then_some(init_binds(error_sx)?),
        tb: enable.contains(WindowForegroundComponents::TASKBAR).then_some(init_taskbar(&rx)?),
        ..default!()
    };

    let mut handle_msg = |msg: Msg| -> Res<()> {
        match msg {
            Msg::Broadcast(BroadcastMsg::Close) => Err(ErrVar::Close)?,
            Msg::Broadcast(BroadcastMsg::WmDisplayChange(lparam)) => {
                if let Some(tb) = ts.tb.as_mut() {
                    handle_wm_display_change(tb, lparam);
                }
            },
            Msg::Broadcast(BroadcastMsg::WmReloadConfig) => {
                if ts.binds.is_some() {
                    let mut binds = init_binds(error_sx)?;
                    unbind_maps(&mut binds);

                    for (_, win_info) in ts.win_infos.iter_mut() {
                        win_info.has_maps = binds.has_maps(&win_info.exe);
                    }

                    ts.binds.replace(binds);
                }
            },
            Msg::Pipe((msg, ack_sx)) => if let pipe_server::Msg::ActiveGame(exe) = msg {
                ts.active_game = exe;

                ack_sx.send(()).unwrap();
            },
            Msg::WmMouseMove(lparam, stamp) => {
                if let Err(mut err) = handle_wm_mouse_move(ts.tb.as_mut().unwrap(), lparam, stamp) {
                    let fg_hwnd = GetForegroundWindow();
                    let win_errored = ts.win_errored.entry(fg_hwnd.as_usize())
                        .or_default();

                    if let ErrVar::WinCore(inner) = *err.var &&
                        win_errored.hresults.insert(inner.code())
                    {
                        err.var = Box::new(ErrVar::FailedWmMouseMouse { inner, fg_hwnd, fg_exe: fg_hwnd.get_exe_or_err() });
                        Err(err)?;
                    }
                }
            },
            Msg::WinEventHookAllForeground { hwnd } => {
                if let Err(err) = handle_win_event_hook_all_foreground(&mut ts, hwnd.as_hwnd()) {
                    let win_errored = ts.win_errored.entry(hwnd)
                        .or_default();

                    if match err.var.as_ref() {
                        ErrVar::WinCore(inner) => win_errored.hresults.insert(inner.code()),
                        _ => win_errored.others.insert(mem::discriminant(&err.var))
                    } {
                        Err(err)?;
                    }
                }
            },
            Msg::WinEventHookAllOtherForegroundDestroy { hook, hwnd } => {
                handle_win_event_hook_all_other_foreground_destroy(&mut ts, hook.as_hwineventhook(), hwnd.as_hwnd())?;
            },
            Msg::WinEventHookExplorerDestroy { hwnd } => {
                handle_win_event_hook_explorer_destroy(&mut ts, hwnd.as_hwnd())?;
            },
            Msg::WinEventHookShellExperienceHostDestroy { hook, hwnd } => {
                handle_win_event_hook_shell_experience_host_destroy(&mut ts, hook.as_hwineventhook(), hwnd.as_hwnd())?;
            },
            Msg::WinEventHookForegroundLocationChange { hwnd } => {
                handle_win_event_hook_foreground_location_change(ts.tb.as_ref().unwrap(), hwnd.as_hwnd())?;
            },
            Msg::WinEventHookShellExperienceHostLocationChange { hook, hwnd } => {
                handle_win_event_hook_shell_experience_host_location_change(&mut ts, hook.as_hwineventhook(), hwnd.as_hwnd())?;
            },
            Msg::WinEventHookTaskbarLocationChange { hwnd } => {
                handle_win_event_hook_taskbar_location_change(ts.tb.as_mut().unwrap(), hwnd.as_hwnd())?;
            },
            _ => info!("{}: {}", module_path!(), msg)
        }

        Ok(())
    };

    for msg in rx.iter() {
        let msg_name = msg.var_name();

        if let Err(err) = handle_msg(msg) {
            if let ErrVar::Close = *err.var {
                break
            }

            error!("{}: failure on message loop: {}: {}", module_path!(), msg_name, err);

            if let ErrVar::FailedContactHookMgr { .. } = *err.var {
                break
            }
        }
    }

    if let Some(tb) = ts.tb {
        tb.taskbar_hwnd.show_na(false).unwrap_or_else(|err| {
            error!("{}: failed to show taskbar: {}", module_path!(), err);
        });
    }

    info!("{}: closed", module_path!());

    Ok(())
} }

pub(crate) fn spawn(enable: WindowForegroundComponents, rx: mpsc::Receiver<Msg>, hook_mgr_tid: Tid, error_sx: mpsc::Sender<String>) -> JoinHandle<()> {
    thread::spawn(move || {
        begin(enable, rx, hook_mgr_tid, &error_sx).unwrap_or_else(|err| {
            error_sx.send(format!("{}: terminated: {}", module_path!(), err)).unwrap();
        });
    })
}
