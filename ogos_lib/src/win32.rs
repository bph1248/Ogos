use crate::common::*;
use ogos_err::*;

use log::*;
use std::{
    ops::*,
    path::*
};
use widestring::*;
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        Graphics::Dwm::*,
        System::{
            ProcessStatus::*,
            Threading::*
        },
        UI::{
            Accessibility::*,
            Shell::*,
            WindowsAndMessaging::*
        }
    }
};

pub(crate) const ERR_STR: &str = "<err>";
pub(crate) const MAX_CLASS_NAME_LEN: usize = 256;

pub(crate) trait WinErrorExt {
    fn as_win32_error(&self) -> WIN32_ERROR;
}
impl WinErrorExt for windows::core::Error {
    fn as_win32_error(&self) -> WIN32_ERROR {
        WIN32_ERROR((self.code().0 & 0xFFFF) as u32) // Assume facility is FACILITY_WIN32
    }
}

pub(crate) struct WinStr {
    _wide: U16CString,
    pcwstr: PCWSTR
}
impl WinStr {
    pub(crate) unsafe fn new(s: &str) -> Self {
        let _wide = U16CString::from_str_unchecked(s);
        let pcwstr = PCWSTR(_wide.as_ptr());

        Self {
            _wide,
            pcwstr
        }
    }
}
impl Default for WinStr {
    fn default() -> Self {
        let _wide: U16CString = default!();
        let pcwstr = PCWSTR(_wide.as_ptr());

        Self {
            _wide,
            pcwstr
        }
    }
}
impl Deref for WinStr {
    type Target = PCWSTR;

    fn deref(&self) -> &Self::Target {
        &self.pcwstr
    }
}
impl From<&PathBuf> for WinStr {
    fn from(value: &PathBuf) -> Self {
        unsafe {
            let _wide = U16CString::from_os_str_unchecked(value.as_os_str());
            let pcwstr = PCWSTR(_wide.as_ptr());

            Self {
                _wide,
                pcwstr
            }
        }
    }
}

pub(crate) struct WindowText {
    pub(crate) caption: String,
    pub(crate) class: String
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum WindowPlacement {
    Windowed,
    Maximised,
    Fullscreen
}

pub(crate) trait AsHwineventhook {
    fn as_hwineventhook(&self) -> HWINEVENTHOOK;
}
impl AsHwineventhook for usize {
    fn as_hwineventhook(&self) -> HWINEVENTHOOK {
        HWINEVENTHOOK(*self as *mut _)
    }
}

pub(crate) trait AsHwnd {
    fn as_hwnd(&self) -> HWND;
}
impl AsHwnd for usize {
    fn as_hwnd(&self) -> HWND {
        HWND(*self as *mut _)
    }
}

pub(crate) trait HwndExt {
    fn as_usize(&self) -> usize;
    unsafe fn get_caption(&self) -> Res1<String>;
    unsafe fn get_caption_or_err(&self) -> String;
    unsafe fn get_class(&self) -> Res1<String>;
    unsafe fn _get_class_or_err(&self) -> String;
    unsafe fn get_exe(&self) -> Res1<String>;
    unsafe fn get_exe_or_err(&self) -> String;
    unsafe fn _get_last_visible_active_popup(&self) -> HWND;
    unsafe fn get_parent(&self) -> windows::core::Result<HWND>;
    unsafe fn get_placement(&self, screen_extent: Extent2d) -> Res1<WindowPlacement>;
    unsafe fn get_rect(&self) -> windows::core::Result<RECT>;
    unsafe fn get_text(&self) -> Res2<WindowText>;
    unsafe fn get_thread_proc_ids(&self) -> windows::core::Result<Tpids>;
    unsafe fn has_parent(&self) -> bool;
    unsafe fn hide(&self);
    unsafe fn _is_alt_tab_window(&self) -> bool;
    unsafe fn is_cloaked(&self) -> windows::core::Result<bool>;
    unsafe fn is_eligible_for_shift(&self, screen_extent: Extent2d) -> Res2<bool>;
    unsafe fn is_fullscreen(&self, screen_extent: Extent2d) -> windows::core::Result<bool>;
    unsafe fn is_iconic(&self) -> bool;
    unsafe fn is_visible(&self) -> bool;
    unsafe fn may_hook_location_change(&self) -> Res1<bool>;
    unsafe fn show_na(&self);
    fn to_string(&self) -> String;
}
impl HwndExt for HWND {
    fn as_usize(&self) -> usize {
        self.0 as usize
    }

    unsafe fn get_caption(&self) -> Res1<String> {
        SetLastError(NO_ERROR);

        let text_len = GetWindowTextLengthW(*self);
        if text_len == 0 {
            return Ok("".into())
        }

        let mut text = vec![0_u16; text_len as usize + 1];
        GetWindowTextW(*self, &mut text).win32_core_ok()?;

        Ok(String::from_utf16(&text[..text_len as usize])?)
    }

    unsafe fn get_caption_or_err(&self) -> String {
        self.get_caption().unwrap_or_else(|_| ERR_STR.into())
    }

    unsafe fn get_class(&self) -> Res1<String> {
        let mut class_name = [0_u16; MAX_CLASS_NAME_LEN + 1];
        let class_name_len = GetClassNameW(*self, &mut class_name).win32_core_ok()?;

        Ok(String::from_utf16(&class_name[..class_name_len as usize])?)
    }

    unsafe fn _get_class_or_err(&self) -> String {
        self.get_class().unwrap_or_else(|_| ERR_STR.into())
    }

    unsafe fn get_exe(&self) -> Res1<String> {
        let tpids = self.get_thread_proc_ids()?;
        let proc_hnd = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, tpids.proc)?;

        let mut proc_image_file_name = [0_u16; MAX_PATH as usize + 1];
        GetProcessImageFileNameW(proc_hnd, &mut proc_image_file_name).win32_core_ok()?;

        CloseHandle(proc_hnd)?;

        let proc_image_exe = PCWSTR::from_raw(proc_image_file_name.as_ptr());
        let proc_image_exe = PathFindFileNameW(proc_image_exe);

        Ok(String::from_utf16(proc_image_exe.as_wide())?)
    }

    unsafe fn get_exe_or_err(&self) -> String {
        self.get_exe().unwrap_or_else(|_| ERR_STR.into())
    }

    unsafe fn _get_last_visible_active_popup(&self) -> Self { // "Popup" is a bit of a misnomer and doesn't strictly refer to windows with the WS_POPUP style
        let last_active_popup = GetLastActivePopup(*self);

        if IsWindowVisible(last_active_popup).as_bool() {
            return last_active_popup
        }

        if last_active_popup == *self {
            return default!()
        }

        last_active_popup._get_last_visible_active_popup()
    }

    unsafe fn get_parent(&self) -> windows::core::Result<HWND> {
        GetParent(*self)
    }

    unsafe fn get_placement(&self, screen_extent: Extent2d) -> Res1<WindowPlacement> {
        use WindowPlacement::*;

        match self.is_fullscreen(screen_extent)? {
            true => Ok(Fullscreen),
            false => {
                let mut window_placement = WINDOWPLACEMENT::default();
                GetWindowPlacement(*self, &mut window_placement)?;

                Ok(
                    match window_placement.showCmd == SW_MAXIMIZE.0 as u32 ||
                        window_placement.showCmd == SW_SHOWMAXIMIZED.0 as u32
                    {
                        true => Maximised,
                        false => Windowed
                    }
                )
            }
        }
    }

    unsafe fn get_rect(&self) -> windows::core::Result<RECT> {
        let mut rect = RECT::default();
        GetWindowRect(*self, &mut rect)?;

        Ok(rect)
    }

    unsafe fn get_text(&self) -> Res2<WindowText> {
        Ok(
            WindowText {
                caption: self.get_caption()?,
                class: self.get_class()?
            }
        )
    }

    unsafe fn get_thread_proc_ids(&self) -> windows::core::Result<Tpids> {
        let mut proc_id = 0;
        let thread_id = GetWindowThreadProcessId(*self, Some(&mut proc_id)).win32_core_ok()?;

        Ok(Tpids { thread: thread_id, proc: proc_id })
    }

    unsafe fn has_parent(&self) -> bool {
        self.get_parent().is_ok()
    }

    unsafe fn hide(&self) {
        _ = ShowWindow(*self, SW_HIDE);
    }

    unsafe fn _is_alt_tab_window(&self) -> bool {
        let root_owner = GetAncestor(*self, GA_ROOTOWNER);

        root_owner._get_last_visible_active_popup() == *self
    }

    unsafe fn is_cloaked(&self) -> windows::core::Result<bool> {
        let mut cloak_kind = 0_u32;
        DwmGetWindowAttribute(*self, DWMWA_CLOAKED, &mut cloak_kind as *mut _ as *mut _, size_of_val(&cloak_kind) as u32)?;

        Ok(cloak_kind > 0)
    }

    unsafe fn is_eligible_for_shift(&self, screen_extent: Extent2d) -> Res2<bool> {
        Ok(
            self.is_visible() &&
            !self.has_parent() &&
            !self.is_cloaked()? &&
            !self.is_iconic() &&
            self.get_placement(screen_extent)? == WindowPlacement::Windowed
        )
    }

    unsafe fn is_fullscreen(&self, screen_extent: Extent2d) -> windows::core::Result<bool> {
        let win_rect = self.get_rect()?;

        Ok(win_rect == screen_extent.into_rect())
    }

    unsafe fn is_iconic(&self) -> bool {
        IsIconic(*self).as_bool()
    }

    unsafe fn is_visible(&self) -> bool {
        IsWindowVisible(*self).as_bool()
    }

    unsafe fn may_hook_location_change(&self) -> Res1<bool> {
        let class = self.get_class()?;

        Ok(!matches!(class.as_str(), "Blank Screen Saver" | "ForegroundStaging" | "Shell_TrayWnd" | "WorkerW" | "XamlExplorerHostIslandWindow"))
    }

    unsafe fn show_na(&self) {
        _ = ShowWindow(*self, SW_SHOWNA);
    }

    fn to_string(&self) -> String {
        format!("{:p}", self.0)
    }
}

macro_rules! impl_ConstString {
    ($which_fn:ident, $($const:ident),+) => {
        fn $which_fn(&self) -> String {
            match *self {
                $(
                    $const => stringify!($const).to_string(),
                )+
                _ => format!("<{:#06x}>", *self)
            }
        }
    };
}

pub(crate) trait ConstString {
    fn _to_dbt_string(&self) -> String;
    fn _to_event_string(&self) -> String;
    fn to_wm_string(&self) -> String;
}
impl ConstString for u32 {
    impl_ConstString!(
        _to_dbt_string,
        DBT_CONFIGCHANGECANCELED,
        DBT_CONFIGCHANGED,
        DBT_CUSTOMEVENT,
        DBT_DEVICEARRIVAL,
        DBT_DEVICEQUERYREMOVE,
        DBT_DEVICEQUERYREMOVEFAILED,
        DBT_DEVICEREMOVECOMPLETE,
        DBT_DEVICEREMOVEPENDING,
        DBT_DEVICETYPESPECIFIC,
        DBT_DEVNODES_CHANGED,
        DBT_QUERYCHANGECONFIG,
        DBT_USERDEFINED
    );

    impl_ConstString!(
        _to_event_string,
        EVENT_OBJECT_CLOAKED,
        EVENT_OBJECT_CREATE,
        EVENT_OBJECT_DESTROY,
        EVENT_OBJECT_FOCUS,
        EVENT_OBJECT_HIDE,
        EVENT_OBJECT_INVOKED,
        EVENT_OBJECT_LOCATIONCHANGE,
        EVENT_OBJECT_NAMECHANGE,
        EVENT_OBJECT_REORDER,
        EVENT_OBJECT_SELECTION,
        EVENT_OBJECT_SHOW,
        EVENT_OBJECT_STATECHANGE,
        EVENT_OBJECT_UNCLOAKED,
        EVENT_SYSTEM_ALERT,
        EVENT_SYSTEM_CAPTUREEND,
        EVENT_SYSTEM_CAPTURESTART,
        EVENT_SYSTEM_DESKTOPSWITCH,
        EVENT_SYSTEM_DIALOGEND,
        EVENT_SYSTEM_DIALOGSTART,
        EVENT_SYSTEM_FOREGROUND,
        EVENT_SYSTEM_MENUEND,
        EVENT_SYSTEM_MENUPOPUPEND,
        EVENT_SYSTEM_MENUPOPUPSTART,
        EVENT_SYSTEM_MENUSTART,
        EVENT_SYSTEM_MINIMIZEEND,
        EVENT_SYSTEM_MINIMIZESTART,
        EVENT_SYSTEM_MOVESIZEEND,
        EVENT_SYSTEM_MOVESIZESTART
    );

    impl_ConstString!(
        to_wm_string,
        WM_CLOSE,
        WM_CREATE,
        WM_DESTROY,
        WM_DEVICECHANGE,
        WM_DISPLAYCHANGE,
        WM_ERASEBKGND,
        WM_GETMINMAXINFO,
        WM_GETTEXT,
        WM_GETTEXTLENGTH,
        WM_HOTKEY,
        WM_MOUSEMOVE,
        WM_MOVE,
        WM_NCCALCSIZE,
        WM_NCCREATE,
        WM_NCHITTEST,
        WM_NCPAINT,
        WM_PAINT,
        WM_SHOWWINDOW,
        WM_SIZE,
        WM_SYSCOMMAND,
        WM_USER,
        WM_WINDOWPOSCHANGED,
        WM_WINDOWPOSCHANGING
    );
}

pub(crate) unsafe fn display_message_box(msg: &str) -> ResVar<()> {
    let caption = w!("Ogos");
    let msg = msg.to_win_str();

    MessageBoxW(None, *msg, caption, MB_OK).0.win32_core_ok()?;

    Ok(())
}

pub(crate) unsafe fn set_cursor_size(size: usize) -> windows::core::Result<()> {
    const SPIF_NONE: SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS = SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0); // Don't update user profile or broadcast WM_SETTINGCHANGE
    SystemParametersInfoW(SYSTEM_PARAMETERS_INFO_ACTION(0x2029), 0, Some(size as *mut _), SPIF_NONE)?;

    info!("{}: cursor size: {}", module_path!(), size);

    Ok(())
}
