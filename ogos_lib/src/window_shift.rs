use crate::{
    common::*,
    config::{self, *},
    display::*,
    err::*,
    win32::*,
    window_foreground
};

use log::*;
use serde::*;
use std::{
    collections::*,
    ffi::*,
    fmt::{self, Display},
    mem::{self, *},
    ops::Sub,
    sync::mpsc::*,
    thread::{self, *},
    time::*
};
use rand::*;
use windows::Win32::{
    Foundation::*,
    Graphics::Dwm::*,
    System::{
        StationsAndDesktops::*,
        Threading::*
    },
    UI::{
        Input::KeyboardAndMouse::*,
        WindowsAndMessaging::*
    }
};

#[derive(Clone, Copy, Default)]
pub(crate) struct AnchorAbsolute {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) right: i32,
    pub(crate) bottom: i32
}
impl AnchorAbsolute {
    pub(crate) fn width(&self) -> i32 {
        self.right - self.left
    }

    pub(crate) fn height(&self) -> i32 {
        self.bottom - self.top
    }
}
impl Display for AnchorAbsolute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{{}, {}, {}, {}}}{{{}, {}}}", self.left, self.top, self.right, self.bottom, self.width(), self.height())
    }
}
impl From<RECT> for AnchorAbsolute {
    fn from(value: RECT) -> Self {
        Self {
            left: value.left,
            top: value.top,
            right: value.right,
            bottom: value.bottom
        }
    }
}
impl Into<RECT> for AnchorAbsolute {
    fn into(self) -> RECT {
        RECT {
            left: self.left,
            top: self.top,
            right: self.right,
            bottom: self.bottom
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
pub(crate) struct AnchorRelative {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) right: i32,
    pub(crate) bottom: i32
}
impl AnchorRelative {
    pub(crate) fn into_abs(self, screen_extent: Extent2d) -> AnchorAbsolute {
        AnchorAbsolute {
            left: self.left,
            top: self.top,
            right: screen_extent.width + self.right,
            bottom: screen_extent.height + self.bottom
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq)]
pub(crate) struct Delta {
    pub(crate) x: i32,
    pub(crate) y: i32
}
impl Delta {
    pub(crate) fn add_checked(self, rhs: Self, leeway: u32) -> Self {
        let x = self.x + rhs.x;
        let y = self.y + rhs.y;

        Self {
            x: match x.unsigned_abs() <= leeway {
                true => x,
                false => self.x - rhs.x
            },
            y: match y.unsigned_abs() <= leeway {
                true => y,
                false => self.y - rhs.y
            }
        }
    }
}
impl Display for Delta {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{{}, {}}}", self.x, self.y)
    }
}
impl Sub for Delta {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y
        }
    }
}

#[derive(Default)]
struct ThreadState {
    hwnds: Vec<HWND>,
    screen_extent: Extent2d
}

struct TopLevelSiblingsInfo {
    tid_of: HWND,
    siblings: Vec<HWND>
}

#[derive(Default)]
struct WinInfo {
    hwnd: HWND,
    exe: String,
    anchor_rel: Option<AnchorRelative>,
    anchor_abs: AnchorAbsolute,
    anchor_is_constrained: bool,
    delta_from_anchor: Delta,
    keep_centered: bool,
    border_disable: bool,
    round_corners_disable: bool,
    papers: Papers
}
impl WinInfo {
    fn reset_anchor(&mut self, rect: RECT) {
        self.anchor_abs = rect.into();
        self.delta_from_anchor = default!();
    }

    unsafe fn shift_win_to_anchor(&mut self) -> windows::core::Result<()> {
        SetWindowPos(
            self.hwnd,
            None,
            self.anchor_abs.left,
            self.anchor_abs.top,
            self.anchor_abs.width(),
            self.anchor_abs.height(),
            SWP_ASYNCWINDOWPOS | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOREDRAW | SWP_NOSENDCHANGING | SWP_NOZORDER
        )
        .inspect(|_| {
            self.delta_from_anchor = default!();
        })
    }

    unsafe fn shift_win_to_screen_center(&self, win_rect: RECT, screen_extent: Extent2d) -> windows::core::Result<()> {
        SetWindowPos(
            self.hwnd,
            None,
            (screen_extent.width - win_rect.width()) / 2,
            (screen_extent.height - win_rect.height()) / 2,
            win_rect.width(),
            win_rect.height(),
            SWP_ASYNCWINDOWPOS | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOREDRAW | SWP_NOSENDCHANGING | SWP_NOZORDER
        )
    }

    // cx and cy are set here because some apps don't seem to honor SWP_NOSIZE, ie. Photoshop
    unsafe fn shift_win(&mut self, leeway: u32, shift_by: Delta) -> windows::core::Result<()> {
        let delta_from_anchor = self.delta_from_anchor.add_checked(shift_by, leeway);

        SetWindowPos(
            self.hwnd,
            None,
            self.anchor_abs.left + delta_from_anchor.x,
            self.anchor_abs.top + delta_from_anchor.y,
            self.anchor_abs.width(),
            self.anchor_abs.height(),
            SWP_ASYNCWINDOWPOS | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_NOREDRAW | SWP_NOSENDCHANGING | SWP_NOZORDER
        )
        .inspect(|_| {
            self.delta_from_anchor = delta_from_anchor;
        })
    }
}

#[derive(Default)]
enum Papers {
    #[default]
    Waive,
    Deny,
    CheckConstraint
}

macro_rules! dbg_window_shift_delta {
    ($msg:expr, $win_info:ident, $win_rect:ident) => {
        #[cfg(feature = "dbg_window_shift_delta")]
        info!(
            "{}: {}: hwnd: {:p}, exe: {}, caption: {}, diffs: {}, constrained: {}",
            module_path!(),
            $msg,
            $win_info.hwnd.0,
            $win_info.exe,
            $win_info.hwnd.get_caption_or_err(),
            $win_rect.sub($win_info.anchor_abs.into()).to_string(),
            $win_info.anchor_is_constrained
        );
    };
}

unsafe extern "system" fn enum_desktop_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ts = &mut *(lparam.0 as *mut ThreadState);

    match hwnd.is_eligible_for_shift(ts.screen_extent) {
        Ok(eligible) => if eligible {
            ts.hwnds.push(hwnd);
        },
        Err(err) => error!("{}: failed to determine if window is eligible for shift: hwnd: {:p}: {}", module_path!(), hwnd.0, err)
    }

    TRUE
}

unsafe extern "system" fn enum_thread_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let TopLevelSiblingsInfo { tid_of, siblings } = &mut *(lparam.0 as *mut _);

    if hwnd != *tid_of && hwnd.is_visible() {
        siblings.push(hwnd);
    }

    TRUE
}

unsafe fn get_current_thread_desktop() -> windows::core::Result<HDESK> {
    let current_thread_id = GetCurrentThreadId();

    GetThreadDesktop(current_thread_id)
}

fn criteria_text_matches(criteria: &Criteria, win_text: &WindowText) -> bool {
    let match_text = |win_text: &str| -> bool {
        criteria.text.iter()
            .any(|criteria_text| {
                match criteria.op {
                    Op::Equals => win_text == criteria_text,
                    Op::Contains => win_text.contains(criteria_text)
                }
            })
    };

    match criteria.against {
        Against::Caption => match_text(&win_text.caption),
        Against::Class => match_text(&win_text.class)
    }
}

unsafe fn left_button_is_down() -> bool {
    const MSB: u16 = 0x8000;

    GetAsyncKeyState(i32::from(VK_LBUTTON.0)) as u16 & MSB == MSB
}

unsafe fn top_level_window_relation_found(tid_of: HWND, owned_by: Option<HWND>, criteria: &Criteria) -> Res2<bool> {
    let criteria_matches = |hwnds: &[HWND]| -> Res1<bool> {
        let components_match = |hwnd: &HWND, criteria_text: &str| -> Res1<bool> {
            let win_text = match criteria.against {
                Against::Caption => hwnd.get_caption()?,
                Against::Class => hwnd.get_class()?,
            };

            let text_matches = match criteria.op {
                Op::Equals => win_text == criteria_text,
                Op::Contains => win_text.contains(criteria_text),
            };

            if let Some(owned_by) = owned_by {
                let is_owned = owned_by == GetWindow(*hwnd, GW_OWNER).unwrap_or_default();

                return Ok(is_owned);
            }

            Ok(text_matches)
        };

        let mut criteria_matches = false;
        for criteria_text in criteria.text.iter() {
            for hwnd in hwnds.iter() {
                if components_match(hwnd, criteria_text)? {
                    criteria_matches = true;

                    break
                }
            }

            if criteria_matches {
                break
            }
        };

        Ok(criteria_matches)
    };

    let tid = tid_of.get_thread_proc_ids()?.thread;
    let mut tl_siblings_info = TopLevelSiblingsInfo {
        tid_of,
        siblings: Vec::new()
    };
    EnumThreadWindows(tid, Some(enum_thread_windows_proc), LPARAM(&mut tl_siblings_info as *mut _ as _)).ok()?;

    let relation_found = criteria_matches(&tl_siblings_info.siblings)?;

    Ok(relation_found)
}

unsafe fn evaluate_for_shift<'a>(win_info: &'a mut WinInfo, window_shift_config: &'a WindowShift, screen_extent: Extent2d, win_rect: RECT, shift_by: Delta) -> Res<()> {
    if win_info.keep_centered && !win_rect.is_centered(screen_extent) {
        match win_info.anchor_is_constrained {
            true => win_info.shift_win_to_screen_center(win_info.anchor_abs.into(), screen_extent)?, // Window must adhere to anchor layout
            false => {
                win_info.shift_win_to_screen_center(win_rect, screen_extent)?; // Window can ignore anchor layout
                win_info.anchor_abs = win_rect.into();
            }
        }
    }

    match win_info.papers {
        Papers::Deny => return Ok(()),
        Papers::CheckConstraint => {
            if let Some(shift_constraint) = window_shift_config.get_shift_constraint(&win_info.exe) {
                match shift_constraint.criteria.relation {
                    WindowRelation::TopLevelFree => if top_level_window_relation_found(win_info.hwnd, None, &shift_constraint.criteria)? {
                        return Ok(())
                    },
                    WindowRelation::TopLevelOwned => if top_level_window_relation_found(win_info.hwnd, Some(win_info.hwnd), &shift_constraint.criteria)? {
                        return Ok(())
                    },
                    _ => ()
                }
            }
        },
        _ => ()
    }

    match win_rect.get_congruent_delta_from_anchor(win_info.anchor_abs, window_shift_config.leeway) {
        Some(delta_from_anchor) => {
            let touched = delta_from_anchor != win_info.delta_from_anchor; // If delta doesn't match what's been cached then someone or something has moved (touched) the window

            if touched {
                dbg_window_shift_delta!("touched", win_info, win_rect);

                match win_info.anchor_is_constrained {
                    true => win_info.delta_from_anchor = delta_from_anchor,
                    false => win_info.reset_anchor(win_rect)
                }
            }

            win_info.shift_win(window_shift_config.leeway, shift_by)?;
        },
        None => {
            dbg_window_shift_delta!("incongruence", win_info, win_rect);

            match win_info.anchor_is_constrained {
                true => win_info.shift_win_to_anchor()?,
                false => win_info.reset_anchor(win_rect)
            }
        }
    }

    Ok(())
}

unsafe fn set_win_attributes(win_info: &WinInfo, window_shift_config: &WindowShift) {
    (|| -> Res<()> {
        if win_info.border_disable {
            DwmSetWindowAttribute(win_info.hwnd, DWMWA_BORDER_COLOR, &DWMWA_COLOR_NONE as *const _ as *const c_void, size_of_val(&DWMWA_COLOR_NONE) as u32)?;
        }

        if win_info.round_corners_disable {
            DwmSetWindowAttribute(win_info.hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &DWMWCP_DONOTROUND as *const _ as *const c_void, size_of_val(&DWMWCP_DONOTROUND) as u32)?;
        }

        if window_shift_config.immersive_dark_mode_enable {
            DwmSetWindowAttribute(win_info.hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &window_shift_config.immersive_dark_mode_enable.as_win32_bool() as *const _ as *const c_void, size_of::<BOOL>() as u32)?;
        }

        Ok(())
    })()
    .unwrap_or_else(|err| {
        error!("{}: failed to set dwm window attribute: hwnd: {:p}, {}", module_path!(), win_info.hwnd.0, err);
    });
}

fn class_is_denied(class: &str) -> bool {
    matches!(class, window_foreground::TASKBAR_CLASS_NAME | window_foreground::WINDOW_WATCH_CLASS_NAME)
}

unsafe fn garner_win_info<'a>(win_infos: &'a mut HashMap<usize, WinInfo>, window_shift_config: &'a WindowShift, screen_extent: Extent2d, win_rect: RECT, hwnd: HWND) -> Res<&'a mut WinInfo> {
    let win_exe = hwnd.get_exe()?;
    let win_text = hwnd.get_text()?;
    let WindowShift {
        leeway,
        constraints,
        ..
    } = window_shift_config;

    let win_info = match constraints.get(&win_exe) {
        Some(constraints) => {
            let (anchor_rel,
                anchor_abs,
                anchor_is_constrained
            ) = constraints.anchor.as_ref()
                .filter(|anchor_constraint| {
                    criteria_text_matches(&anchor_constraint.criteria, &win_text)
                })
                .map(|anchor_constraint| { // Map to screen coords
                    (
                        Some(anchor_constraint.relative),
                        anchor_constraint.relative.into_abs(screen_extent),
                        true
                    )
                })
                .unwrap_or_else(|| {
                    (
                        None,
                        win_rect.into(),
                        false
                    )
                });

            let delta_from_anchor = anchor_is_constrained.and_then(|| {
                win_rect.get_congruent_delta_from_anchor(anchor_abs, *leeway)
            })
            .unwrap_or_default();

            let keep_centered = constraints.center.as_ref()
                .map(|center_constraint| {
                    criteria_text_matches(&center_constraint.criteria, &win_text)
                })
                .unwrap_or_default();

            let (border_disable,
                round_corners_disable
            ) = constraints.attributes.as_ref()
                .filter(|&attributes_constraint| {
                    criteria_text_matches(&attributes_constraint.criteria, &win_text)
                })
                .map(|attributes_constraint| {
                    (
                        attributes_constraint.border_disable,
                        attributes_constraint.round_corners_disable
                    )
                })
                .unwrap_or_default();

            let papers = match class_is_denied(win_text.class.as_ref()) {
                true => Papers::Deny,
                false => {
                    constraints.shift.as_ref()
                        .map(|shift_constraint| {
                            match shift_constraint.criteria.relation {
                                WindowRelation::This => match criteria_text_matches(&shift_constraint.criteria, &win_text) {
                                    true => Papers::Deny,
                                    false => Papers::Waive
                                },
                                _ => Papers::CheckConstraint
                            }
                        })
                        .unwrap_or_default()
                }
            };

            WinInfo {
                hwnd,
                exe: win_exe,
                anchor_rel,
                anchor_abs,
                anchor_is_constrained,
                delta_from_anchor,
                keep_centered,
                border_disable,
                round_corners_disable,
                papers
            }
        },
        None => WinInfo {
            hwnd,
            exe: win_exe,
            papers: match class_is_denied(win_text.class.as_ref()) {
                true => Papers::Deny,
                _ => Papers::Waive
            },
            anchor_abs: hwnd.get_rect()?.into(),
            ..default!()
        }
    };

    // Insert and get &mut
    let win_info = win_infos.entry(hwnd.as_usize())
        .or_insert(win_info);

    Ok(win_info)
}

unsafe fn smaug(ts: &mut ThreadState, win_infos: &mut HashMap<usize, WinInfo>, win_errored: &mut HashMap<usize, Errored>, window_shift_config: &WindowShift, rx: &Receiver<WindowShiftMsg>) -> Res<()> {
    let interval_begin = now!();
    let interval_end = interval_begin + Duration::from_secs(u64::from(window_shift_config.interval_dur));
    let time_remaining = || interval_end - now!();
    let mut pause_shift = false;

    let mut inner = || -> Res<()> {
        let msg = match pause_shift {
            true => rx.recv()?,
            false => rx.recv_timeout(time_remaining())?
        };

        match msg {
            WindowShiftMsg::BroadcastMsg(BroadcastMsg::WmDisplayChange(lparam)) => {
                let width = (lparam.0 & 0xFFFF) as i32;
                let height = ((lparam.0 >> 16) & 0xFFFF) as i32;
                ts.screen_extent = Extent2d { width, height };

                for win_info in win_infos.values_mut() {
                    if let Some(anchor_rel) = win_info.anchor_rel.as_ref() {
                        win_info.anchor_abs = anchor_rel.into_abs(ts.screen_extent);
                    }

                    // Any anchors not updated here will become incongruent on the next iteration
                }
            },
            WindowShiftMsg::BroadcastMsg(BroadcastMsg::WmReloadConfig) => Err(ErrVar::ReloadConfig)?,
            WindowShiftMsg::Destroy(hwnd) => {
                #[cfg(feature = "bug_hunting_window_shift_destroy")]
                info!("{}: removing hwnd: {:#x}", module_path!(), hwnd);

                win_infos.remove(&hwnd);
                win_errored.remove(&hwnd);
            },
            WindowShiftMsg::MenuStart => pause_shift = true,
            WindowShiftMsg::MenuEnd => pause_shift = false
        }

        Ok(())
    };

    loop {
        if let Err(err) = inner() {
            match *err.var {
                ErrVar::ReloadConfig => break Err(err),
                ErrVar::RecvTimeout(recv_timeout_err) => {
                    match recv_timeout_err {
                        RecvTimeoutError::Timeout => break Ok(()), // Typical, time is up this interval
                        RecvTimeoutError::Disconnected => break Err(err)
                    }
                },
                _ => error!("{}: failed to process messages: {}", module_path!(), err)
            }
        }
    }
}

unsafe fn foreground_disallows_shift(fg_hwnd: HWND, screen_extent: Extent2d) -> Res<bool> {
    let fg_rect = fg_hwnd.get_rect()?;
    let fg_is_fullscreen = fg_rect == screen_extent.into();

    let fg_class = fg_hwnd.get_class()?;
    let fg_class_disallows_shift = !matches!(fg_class.as_str(), window_foreground::PROGMAN_CLASS_NAME | window_foreground::WORKERW_CLASS_NAME);

    if fg_is_fullscreen && fg_class_disallows_shift {
        return Ok(true)
    }

    Ok(false)
}

unsafe fn begin(rx: Receiver<WindowShiftMsg>) -> Res<()> {
    info!("{}: begin", module_path!());

    let config = config::get().read()?;
    let mut window_shift_config = config.window_shift.clone().ok_or(ErrVar::MissingConfigKey { name: config::WindowShift::NAME })?;

    drop(config);

    let current_desktop_hnd = get_current_thread_desktop()?;
    let mut win_infos: HashMap<usize, WinInfo> = HashMap::new();
    let mut win_errored: HashMap<usize, Errored> = HashMap::new();

    let mut ts = ThreadState {
        screen_extent: get_screen_extent(GetDesktopWindow())?,
        ..default!()
    };
    loop {
        // Listen for messages until timeout, then look to shift windows. Rinse and repeat
        match smaug(&mut ts, &mut win_infos, &mut win_errored, &window_shift_config, &rx) {
            Ok(_) => {
                let fg_hwnd = GetForegroundWindow();

                if fg_hwnd.is_invalid() || foreground_disallows_shift(fg_hwnd, ts.screen_extent)? || left_button_is_down() {
                    continue
                }

                EnumDesktopWindows(current_desktop_hnd, Some(enum_desktop_windows_proc), LPARAM(&mut ts as *mut _ as _))?;

                let config::WindowShift { stride, .. } = &window_shift_config;
                let shift_by = {
                    let x_axis_choices = [stride.x as i32, -(stride.x as i32)];
                    let y_axis_choices = [stride.y as i32, -(stride.y as i32)];

                    Delta {
                        x: x_axis_choices[rand::thread_rng().gen_range(0..=1)],
                        y: y_axis_choices[rand::thread_rng().gen_range(0..=1)]
                    }
                };

                for hwnd in ts.hwnds.drain(..) {
                    (|| -> Res<()> {
                        let win_rect = hwnd.get_rect()?;

                        match win_infos.get_mut(&hwnd.as_usize()) {
                            Some(win_info) => evaluate_for_shift(win_info, &window_shift_config, ts.screen_extent, win_rect, shift_by)?,
                            None => {
                                let win_info = garner_win_info(&mut win_infos, &window_shift_config, ts.screen_extent, win_rect, hwnd)?;

                                set_win_attributes(win_info, &window_shift_config);
                                evaluate_for_shift(win_info, &window_shift_config, ts.screen_extent, win_rect, shift_by)?;
                            }
                        }

                        Ok(())
                    })()
                    .unwrap_or_else(|err| {
                        let errored = win_errored.entry(hwnd.as_usize())
                            .or_default();

                        if match err.var.as_ref() {
                            ErrVar::WinCore(error) => errored.hresults.insert(error.code()),
                            _ => errored.others.insert(mem::discriminant(&err.var))

                        } {
                            error!("{}: failed on window enumeration: hwnd: {:p}: exe: {}, {}", module_path!(), hwnd.0, hwnd.get_exe_or_err(), err);
                        }
                    });
                }
            },
            Err(err) => {
                match *err.var {
                    ErrVar::Recv(_) => break,
                    ErrVar::RecvTimeout(recv_timeout_err) => {
                        if let RecvTimeoutError::Disconnected = recv_timeout_err {
                            break
                        }
                    },
                    ErrVar::ReloadConfig => {
                        (|| -> ResVar<()> {
                            let config = config::get().read()?;
                            window_shift_config = config.window_shift.clone().ok_or(ErrVar::MissingConfigKey { name: config::WindowShift::NAME })?;

                            Ok(())
                        })()
                        .unwrap_or_else(|err| {
                            error!("{}: failed to reload config: {}", module_path!(), err);
                        });
                    },
                    _ => error!("{}: failed on iteration: {}", module_path!(), err)
                }
            }
        }
    }

    info!("{}: closed", module_path!());

    Ok(())
}

pub(crate) unsafe fn spawn(rx: Receiver<WindowShiftMsg>) -> JoinHandle<()> {
    thread::spawn(|| {
        begin(rx).unwrap_or_else(|err| {
            error!("{}: terminated: {}", module_path!(), err);
        });
    })
}
