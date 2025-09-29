use crate::Keyboard::{self, *};

use std::{
    convert::*,
    mem::*
};
use winapi::{
    shared::minwindef::*,
    um::winuser::*
};

pub(crate) mod kimpl {
    use super::*;

    pub(crate) fn press(key: Keyboard) {
        send_key_stroke(true, key)
    }

    pub(crate) fn release(key: Keyboard) {
        send_key_stroke(false, key)
    }

    pub(crate) fn click(key: Keyboard) {
        // Do we need sleep in between?
        press(key);
        release(key);
    }

    pub(crate) fn is_toggled(key: Keyboard) -> bool {
        // GetAsync is universal, but does not provide whether button is toggled.
        // as the GetKeyState seems to guarantee the correctness.
        let state = unsafe { GetKeyState(vk_code(key).into()) };
        i32::from(state) & 0x8001 != 0
    }
}

pub fn send_key_stroke(press: bool, key: Keyboard) {
    let action = if press {
        0 // 0 means to press.
    } else {
        KEYEVENTF_KEYUP
    };
    unsafe {
        let mut input_u: INPUT_u = std::mem::zeroed();
        *input_u.ki_mut() = KEYBDINPUT {
            wVk: 0,
            wScan: MapVirtualKeyW(vk_code(key).into(), 0)
                .try_into()
                .expect("Failed to map vk to scan code"), // This ignores the keyboard layout so better than vk?
            dwFlags: KEYEVENTF_SCANCODE | action,
            time: 0,
            dwExtraInfo: 0,
        };

        let mut x = INPUT {
            type_: INPUT_KEYBOARD,
            u: input_u,
        };

        SendInput(1, &mut x as LPINPUT, size_of::<INPUT>() as libc::c_int);
    }
}

// Missing defines
const VK_0: i32 = 0x30;
const VK_1: i32 = 0x31;
const VK_2: i32 = 0x32;
const VK_3: i32 = 0x33;
const VK_4: i32 = 0x34;
const VK_5: i32 = 0x35;
const VK_6: i32 = 0x36;
const VK_7: i32 = 0x37;
const VK_8: i32 = 0x38;
const VK_9: i32 = 0x39;
const VK_A: i32 = 0x41;
const VK_B: i32 = 0x42;
const VK_C: i32 = 0x43;
const VK_D: i32 = 0x44;
const VK_E: i32 = 0x45;
const VK_F: i32 = 0x46;
const VK_G: i32 = 0x47;
const VK_H: i32 = 0x48;
const VK_I: i32 = 0x49;
const VK_J: i32 = 0x4A;
const VK_K: i32 = 0x4B;
const VK_L: i32 = 0x4C;
const VK_M: i32 = 0x4D;
const VK_N: i32 = 0x4E;
const VK_O: i32 = 0x4F;
const VK_P: i32 = 0x50;
const VK_Q: i32 = 0x51;
const VK_R: i32 = 0x52;
const VK_S: i32 = 0x53;
const VK_T: i32 = 0x54;
const VK_U: i32 = 0x55;
const VK_V: i32 = 0x56;
const VK_W: i32 = 0x57;
const VK_X: i32 = 0x58;
const VK_Y: i32 = 0x59;
const VK_Z: i32 = 0x5A;

fn vk_code(key: Keyboard) -> WORD {
    i32::from(key)
        .try_into()
        .expect("vk does not fit into WORD")
}

impl From<Keyboard> for i32 {
    fn from(key: Keyboard) -> i32 {
        match key {
            Escape => VK_ESCAPE,
            F1 => VK_F1,
            F2 => VK_F2,
            F3 => VK_F3,
            F4 => VK_F4,
            F5 => VK_F5,
            F6 => VK_F6,
            F7 => VK_F7,
            F8 => VK_F8,
            F9 => VK_F9,
            F10 => VK_F10,
            F11 => VK_F11,
            F12 => VK_F12,
            PrintScreen => VK_SNAPSHOT,
            ScrollLock => VK_SCROLL,
            Pause => VK_PAUSE,
            Grave => VK_OEM_3,
            N0 => VK_0,
            N1 => VK_1,
            N2 => VK_2,
            N3 => VK_3,
            N4 => VK_4,
            N5 => VK_5,
            N6 => VK_6,
            N7 => VK_7,
            N8 => VK_8,
            N9 => VK_9,
            Minus => VK_OEM_MINUS,
            Equal => VK_OEM_PLUS,
            A => VK_A,
            B => VK_B,
            C => VK_C,
            D => VK_D,
            E => VK_E,
            F => VK_F,
            G => VK_G,
            H => VK_H,
            I => VK_I,
            J => VK_J,
            K => VK_K,
            L => VK_L,
            M => VK_M,
            N => VK_N,
            O => VK_O,
            P => VK_P,
            Q => VK_Q,
            R => VK_R,
            S => VK_S,
            T => VK_T,
            U => VK_U,
            V => VK_V,
            W => VK_W,
            X => VK_X,
            Y => VK_Y,
            Z => VK_Z,
            LeftBracket => VK_OEM_4,
            RightBracket => VK_OEM_6,
            Backslash => VK_OEM_5,
            Semicolon => VK_OEM_1,
            Quote => VK_OEM_7,
            Comma => VK_OEM_COMMA,
            Dot => VK_OEM_PERIOD,
            Slash => VK_OEM_2,
            Tab => VK_TAB,
            CapsLock => VK_CAPITAL,
            LeftShift => VK_LSHIFT,
            LeftCtrl => VK_LCONTROL,
            LeftWin => VK_LWIN,
            LeftAlt => VK_LMENU,
            Space => VK_SPACE,
            Backspace => VK_BACK,
            Enter => VK_RETURN,
            RightShift => VK_RSHIFT,
            RightCtrl => VK_RCONTROL,
            RightWin => VK_RWIN,
            RightAlt => VK_RMENU,
            Insert => VK_INSERT,
            Delete => VK_DELETE,
            Home => VK_HOME,
            End => VK_END,
            PageUp => VK_PRIOR,
            PageDown => VK_NEXT,
            Left => VK_LEFT,
            Up => VK_UP,
            Right => VK_RIGHT,
            Down => VK_DOWN,
            NumLock => VK_NUMLOCK,
            Keypad0 => VK_NUMPAD0,
            Keypad1 => VK_NUMPAD1,
            Keypad2 => VK_NUMPAD2,
            Keypad3 => VK_NUMPAD3,
            Keypad4 => VK_NUMPAD4,
            Keypad5 => VK_NUMPAD5,
            Keypad6 => VK_NUMPAD6,
            Keypad7 => VK_NUMPAD7,
            Keypad8 => VK_NUMPAD8,
            Keypad9 => VK_NUMPAD9,
            KeypadSlash => VK_DIVIDE,
            KeypadAsterisk => VK_MULTIPLY,
            KeypadMinus => VK_SUBTRACT,
            KeypadPlus => VK_ADD,
            KeypadDot => VK_DECIMAL
        }
    }
}

impl TryFrom<i32> for Keyboard {
    type Error = ();

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        Ok(match value {
            VK_ESCAPE => Escape,
            VK_F1 => F1,
            VK_F2 => F2,
            VK_F3 => F3,
            VK_F4 => F4,
            VK_F5 => F5,
            VK_F6 => F6,
            VK_F7 => F7,
            VK_F8 => F8,
            VK_F9 => F9,
            VK_F10 => F10,
            VK_F11 => F11,
            VK_F12 => F12,
            VK_SNAPSHOT => PrintScreen,
            VK_SCROLL => ScrollLock,
            VK_PAUSE => Pause,
            VK_OEM_3 => Grave,
            VK_0 => N0,
            VK_1 => N1,
            VK_2 => N2,
            VK_3 => N3,
            VK_4 => N4,
            VK_5 => N5,
            VK_6 => N6,
            VK_7 => N7,
            VK_8 => N8,
            VK_9 => N9,
            VK_OEM_MINUS => Minus,
            VK_OEM_PLUS => Equal,
            VK_A => A,
            VK_B => B,
            VK_C => C,
            VK_D => D,
            VK_E => E,
            VK_F => F,
            VK_G => G,
            VK_H => H,
            VK_I => I,
            VK_J => J,
            VK_K => K,
            VK_L => L,
            VK_M => M,
            VK_N => N,
            VK_O => O,
            VK_P => P,
            VK_Q => Q,
            VK_R => R,
            VK_S => S,
            VK_T => T,
            VK_U => U,
            VK_V => V,
            VK_W => W,
            VK_X => X,
            VK_Y => Y,
            VK_Z => Z,
            VK_OEM_4 => LeftBracket,
            VK_OEM_6 => RightBracket,
            VK_OEM_5 => Backslash,
            VK_OEM_1 => Semicolon,
            VK_OEM_7 => Quote,
            VK_OEM_COMMA => Comma,
            VK_OEM_PERIOD => Dot,
            VK_OEM_2 => Slash,
            VK_TAB => Tab,
            VK_CAPITAL => CapsLock,
            VK_LSHIFT => LeftShift,
            VK_LCONTROL => LeftCtrl,
            VK_LWIN => LeftWin,
            VK_LMENU => LeftAlt,
            VK_SPACE => Space,
            VK_BACK => Backspace,
            VK_RETURN => Enter,
            VK_RSHIFT => RightShift,
            VK_RCONTROL => RightCtrl,
            VK_RWIN => RightWin,
            VK_RMENU => RightAlt,
            VK_INSERT => Insert,
            VK_DELETE => Delete,
            VK_HOME => Home,
            VK_END => End,
            VK_PRIOR => PageUp,
            VK_NEXT => PageDown,
            VK_LEFT => Left,
            VK_UP => Up,
            VK_RIGHT => Right,
            VK_DOWN => Down,
            VK_NUMLOCK => NumLock,
            VK_NUMPAD0 => Keypad0,
            VK_NUMPAD1 => Keypad1,
            VK_NUMPAD2 => Keypad2,
            VK_NUMPAD3 => Keypad3,
            VK_NUMPAD4 => Keypad4,
            VK_NUMPAD5 => Keypad5,
            VK_NUMPAD6 => Keypad6,
            VK_NUMPAD7 => Keypad7,
            VK_NUMPAD8 => Keypad8,
            VK_NUMPAD9 => Keypad9,
            VK_DIVIDE => KeypadSlash,
            VK_MULTIPLY => KeypadAsterisk,
            VK_SUBTRACT => KeypadMinus,
            VK_ADD => KeypadPlus,
            VK_DECIMAL => KeypadDot,
            _ => Err(())?
        })
    }
}
