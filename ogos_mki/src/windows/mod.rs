pub mod keyboard;
pub mod mouse;

use crate::*;
use log::info;
use std::{
    convert::*,
    mem::*,
    ptr::*,
    sync::mpsc::*
};
use winapi::{
    ctypes::*,
    shared::{
        minwindef::*,
        windef::*
    },
    um::{
        processthreadsapi::*,
        winuser::*
    }
};

const MKI_INSTALL_HOOKS: u32 = WM_USER + 1;
const MKI_UNINSTALL_HOOKS: u32 = MKI_INSTALL_HOOKS + 1;

struct Hooks {
    keyboard: Option<HHOOK>,
    mouse: Option<HHOOK>
}
unsafe impl Send for Hooks {}
unsafe impl Sync for Hooks {}

pub unsafe fn install_hooks() {
    PostThreadMessageW(registry().hooks_tid, MKI_INSTALL_HOOKS, 0, 0);
}

pub unsafe fn uninstall_hooks() {
    PostThreadMessageW(registry().hooks_tid, MKI_UNINSTALL_HOOKS, 0, 0);
}

pub(crate) unsafe fn init_hooks(sx: Sender<u32>) {
    std::thread::spawn(move || {
        let tid = GetCurrentThreadId();
        let mut hooks = Hooks {
            keyboard: Some(install_hook(WH_KEYBOARD_LL, keyboard_proc)),
            mouse: Some(install_hook(WH_MOUSE_LL, mouse_proc))
        };

        sx.send(tid).unwrap();

        let mut msg: MSG = MaybeUninit::zeroed().assume_init();
        while GetMessageW(&mut msg, null_mut(), 0, 0) != 0 {
            match msg.message {
                MKI_INSTALL_HOOKS => {
                    if hooks.keyboard.is_none() {
                        hooks.keyboard = Some(install_hook(WH_KEYBOARD_LL, keyboard_proc));

                        info!("{}: installed keyboard hook", module_path!());
                    }
                    if hooks.mouse.is_none() {
                        hooks.mouse = Some(install_hook(WH_MOUSE_LL, mouse_proc));

                        info!("{}: installed mouse hook", module_path!());
                    }
                },
                MKI_UNINSTALL_HOOKS => {
                    hooks.keyboard.take_if(|hook| {
                        match UnhookWindowsHookEx(*hook) {
                            0 => {
                                info!("{}: failed to uninstall keyboard hook", module_path!());
                                false
                            },
                            _ => {
                                info!("{}: uninstalled keyboard hook", module_path!());
                                true
                            }
                        }
                    });
                    hooks.mouse.take_if(|hook| {
                        match UnhookWindowsHookEx(*hook) {
                            0 => {
                                info!("{}: failed to uninstall mouse hook", module_path!());
                                false
                            },
                            _ => {
                                info!("{}: uninstalled mouse hook", module_path!());
                                true
                            }
                        }
                    });
                },
                _ => ()
            }
        }
    });
}

fn install_hook(
    hook_id: c_int,
    hook_proc: unsafe extern "system" fn(c_int, WPARAM, LPARAM) -> LRESULT,
) -> HHOOK {
    unsafe { SetWindowsHookExW(hook_id, Some(hook_proc), 0 as HINSTANCE, 0) }
}

unsafe extern "system" fn keyboard_proc(
    code: c_int,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    let hook_struct = &*(l_param as *const KBDLLHOOKSTRUCT);

    if hook_struct.flags & LLKHF_INJECTED == LLKHF_INJECTED {
        return CallNextHookEx(null_mut(), code, w_param, l_param)
    }

    let vk: i32 = hook_struct
        .vkCode
        .try_into()
        .expect("vkCode does not fit in i32");
    // https://docs.microsoft.com/en-us/windows/win32/inputdev/wm-keydown
    // Says that we can find the repeat bit here, however that does not apply to lowlvlkb hook which this is.
    // Because IDE is not capable of following to the definition here it is:
    // STRUCT!{struct KBDLLHOOKSTRUCT {
    //     vkCode: DWORD,
    //     scanCode: DWORD,
    //     flags: DWORD,
    //     time: DWORD,
    //     dwExtraInfo: ULONG_PTR,
    // }}

    let mut inhibit = InhibitEvent::No;
    // Note this seemingly is only activated when ALT is not pressed, need to handle WM_SYSKEYDOWN then
    // Test that case.
    if let Ok(key) = Key::try_from(vk) {
        let code = w_param as u32;

        match code {
            _ if code == WM_KEYDOWN || code == WM_SYSKEYDOWN => {
                inhibit = registry().event_down(InputEvent::Keyboard(key));
            }
            _ if code == WM_KEYUP || code == WM_SYSKEYUP => {
                inhibit = registry().event_up(InputEvent::Keyboard(key));
            }
            _ => ()
        }

        if inhibit.should_inhibit() {
            return 1
        }
    }

    CallNextHookEx(null_mut(), code, w_param, l_param)
}

unsafe extern "system" fn mouse_proc(
    code: c_int,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    // because macros > idea
    // typedef struct tagMSLLHOOKSTRUCT {
    //   POINT     pt;
    //   DWORD     mouseData;
    //   DWORD     flags;
    //   DWORD     time;
    //   ULONG_PTR dwExtraInfo;
    // } MSLLHOOKSTRUCT, *LPMSLLHOOKSTRUCT, *PMSLLHOOKSTRUCT;

    let hook_struct = &*(l_param as *const MSLLHOOKSTRUCT);

    if hook_struct.flags & LLMHF_INJECTED == LLMHF_INJECTED {
        return CallNextHookEx(null_mut(), code, w_param, l_param)
    }

    let x_button_param: u16 =
        GET_XBUTTON_WPARAM(hook_struct.mouseData.try_into().expect("u32 fits usize"));
    let maybe_x_button = if x_button_param == XBUTTON1 {
        Some(Button::Back)
    } else if x_button_param == XBUTTON2 {
        Some(Button::Forward)
    } else {
        None
    };
    let w_param_u32: u32 = w_param.try_into().expect("w_param > u32");
    registry().update_mouse_position(hook_struct.pt.x, hook_struct.pt.y);
    let inhibit = match w_param_u32 {
        code if code == WM_LBUTTONDOWN => registry().event_down(InputEvent::MouseButton(Button::Left)),
        code if code == WM_LBUTTONDBLCLK => registry().event_click(InputEvent::MouseButton(Button::DoubleLeft)),
        code if code == WM_RBUTTONDOWN => registry().event_down(InputEvent::MouseButton(Button::Right)),
        code if code == WM_RBUTTONDBLCLK => {
            registry().event_click(InputEvent::MouseButton(Button::DoubleRight))
        }
        code if code == WM_MBUTTONDOWN => registry().event_down(InputEvent::MouseButton(Button::Middle)),
        code if code == WM_MOUSEWHEEL => {
            let wheel_delta = (hook_struct.mouseData >> 16) as i16;

            match wheel_delta > 0 {
                true => registry().event_wheel(InputEvent::MouseWheel(Wheel::Up)),
                false => registry().event_wheel(InputEvent::MouseWheel(Wheel::Down))
            }
        },
        code if code == WM_LBUTTONDOWN => registry().event_down(InputEvent::MouseButton(Button::Left)),
        code if code == WM_RBUTTONDOWN => registry().event_down(InputEvent::MouseButton(Button::Right)),
        code if code == WM_MBUTTONDOWN => registry().event_down(InputEvent::MouseButton(Button::Middle)),
        code if code == WM_XBUTTONDOWN => {
            if let Some(x_button) = maybe_x_button {
                registry().event_down(InputEvent::MouseButton(x_button))
            } else {
                InhibitEvent::No
            }
        }
        code if code == WM_LBUTTONDBLCLK => registry().event_click(InputEvent::MouseButton(Button::DoubleLeft)),
        code if code == WM_RBUTTONDBLCLK => registry().event_click(InputEvent::MouseButton(Button::DoubleRight)),
        code if code == WM_MBUTTONDBLCLK => registry().event_click(InputEvent::MouseButton(Button::DoubleMiddle)),
        code if code == WM_XBUTTONDBLCLK => {
            if let Some(x_button) = maybe_x_button {
                // TODO: figure out the other XButtons.
                if Button::Back == x_button {
                    registry().event_click(InputEvent::MouseButton(Button::DoubleSide))
                } else {
                    registry().event_click(InputEvent::MouseButton(Button::DoubleExtra))
                }
            } else {
                InhibitEvent::No
            }
        }
        code if code == WM_LBUTTONUP => registry().event_up(InputEvent::MouseButton(Button::Left)),
        code if code == WM_RBUTTONUP => registry().event_up(InputEvent::MouseButton(Button::Right)),
        code if code == WM_MBUTTONUP => registry().event_up(InputEvent::MouseButton(Button::Middle)),
        code if code == WM_XBUTTONUP => {
            if let Some(x_button) = maybe_x_button {
                registry().event_up(InputEvent::MouseButton(x_button))
            } else {
                InhibitEvent::No
            }
        }
        _ => InhibitEvent::No,
    };
    if inhibit.should_inhibit() {
        1
    } else {
        CallNextHookEx(null_mut(), code, w_param, l_param)
    }
}
