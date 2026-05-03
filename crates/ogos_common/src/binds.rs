use super::*;
use ogos_mki::*;

use const_format::*;

#[derive(Deserialize)]
#[serde(untagged)]
enum BindRepr<'a> {
    Enum(BindVarRaw),
    Num(u32),
    Str(&'a str)
}

macro_rules! impl_BindVar {
    ($($variant:ident),+) => {
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum BindVarRaw {
            $($variant,)+
        }

        #[derive(Debug, Deserialize)]
        #[serde(try_from = "BindRepr")]
        pub enum BindVar {
            $($variant,)+
        }
        impl BindVar {
            pub(crate) fn as_str(&self) -> &'static str {
                match self {
                    $(Self::$variant => map_ascii_case!(Case::Snake, stringify!($variant)),)+
                }
            }
        }
        impl From<BindVarRaw> for BindVar {
            fn from(value: BindVarRaw) -> Self {
                match value {
                    $(BindVarRaw::$variant => Self::$variant,)+
                }
            }
        }
    };
}
impl_BindVar! {
    N0, N1, N2, N3, N4, N5, N6, N7, N8, N9, N10, N12, N16,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    Minus, Mns,
    Equal, Eql,
    Backspace, Bspc,
    LeftBracket, Lbrc,
    RightBracket, Rbrc,
    Backslash, Bsls,
    Semicolon, Scln,
    Quote, Quot,
    Comma, Comm,
    Dot,
    Slash, Sls,
    Escape, Esc,
    Grave, Grv,
    Tab,
    CapsLock, Caps,
    LeftShift, Lsft,
    LeftCtrl, Lctrl,
    LeftWin, Lwin,
    LeftAlt, Lalt,
    RightShift, Rsft,
    RightCtrl, Rctrl,
    RightWin, Rwin,
    RightAlt, Ralt,
    Space, Spc,
    Enter, Ent,
    PrintScreen, Pscr,
    ScrollLock, Scrl,
    Pause, Paus,
    Insert, Ins,
    Delete, Del,
    Home,
    End,
    PageUp, Pgup,
    PageDown, Pgdn,
    Left,
    Up,
    Right,
    Down,
    NumLock, Num,
    Keypad0, Kp0,
    Keypad1, Kp1,
    Keypad2, Kp2,
    Keypad3, Kp3,
    Keypad4, Kp4,
    Keypad5, Kp5,
    Keypad6, Kp6,
    Keypad7, Kp7,
    Keypad8, Kp8,
    Keypad9, Kp9,
    KeypadSlash, KpSls,
    KeypadAsterisk, KpAst,
    KeypadMinus, KpMns,
    KeypadPlus, KpPls,
    KeypadDot, KpDot,

    WheelUp,
    WheelDown,

    LeftButton, Lb,
    RightButton, Rb,
    MiddleButton, Mb,
    BackButton, Xb1, Bb,
    ForwardButton, Xb2, Fb,

    Click,
    Default,
    DurMs
}
impl BindVar {
    pub fn try_as_input_event(&self) -> ResVar<InputEvent> {
        use InputEvent::*;

        Ok(match self {
            Self::N0 => Keyboard(Key::N0),
            Self::N1 => Keyboard(Key::N1),
            Self::N2 => Keyboard(Key::N2),
            Self::N3 => Keyboard(Key::N3),
            Self::N4 => Keyboard(Key::N4),
            Self::N5 => Keyboard(Key::N5),
            Self::N6 => Keyboard(Key::N6),
            Self::N7 => Keyboard(Key::N7),
            Self::N8 => Keyboard(Key::N8),
            Self::N9 => Keyboard(Key::N9),
            Self::F1 => Keyboard(Key::F1),
            Self::F2 => Keyboard(Key::F2),
            Self::F3 => Keyboard(Key::F3),
            Self::F4 => Keyboard(Key::F4),
            Self::F5 => Keyboard(Key::F5),
            Self::F6 => Keyboard(Key::F6),
            Self::F7 => Keyboard(Key::F7),
            Self::F8 => Keyboard(Key::F8),
            Self::F9 => Keyboard(Key::F9),
            Self::F10 => Keyboard(Key::F10),
            Self::F11 => Keyboard(Key::F11),
            Self::F12 => Keyboard(Key::F12),
            Self::A => Keyboard(Key::A),
            Self::B => Keyboard(Key::B),
            Self::C => Keyboard(Key::C),
            Self::D => Keyboard(Key::D),
            Self::E => Keyboard(Key::E),
            Self::F => Keyboard(Key::F),
            Self::G => Keyboard(Key::G),
            Self::H => Keyboard(Key::H),
            Self::I => Keyboard(Key::I),
            Self::J => Keyboard(Key::J),
            Self::K => Keyboard(Key::K),
            Self::L => Keyboard(Key::L),
            Self::M => Keyboard(Key::M),
            Self::N => Keyboard(Key::N),
            Self::O => Keyboard(Key::O),
            Self::P => Keyboard(Key::P),
            Self::Q => Keyboard(Key::Q),
            Self::R => Keyboard(Key::R),
            Self::S => Keyboard(Key::S),
            Self::T => Keyboard(Key::T),
            Self::U => Keyboard(Key::U),
            Self::V => Keyboard(Key::V),
            Self::W => Keyboard(Key::W),
            Self::X => Keyboard(Key::X),
            Self::Y => Keyboard(Key::Y),
            Self::Z => Keyboard(Key::Z),
            Self::Minus |
            Self::Mns => Keyboard(Key::Minus),
            Self::Equal |
            Self::Eql => Keyboard(Key::Equal),
            Self::Backspace |
            Self::Bspc => Keyboard(Key::Backspace),
            Self::LeftBracket |
            Self::Lbrc => Keyboard(Key::LeftBracket),
            Self::RightBracket |
            Self::Rbrc => Keyboard(Key::RightBracket),
            Self::Backslash |
            Self::Bsls => Keyboard(Key::Backslash),
            Self::Semicolon |
            Self::Scln => Keyboard(Key::Semicolon),
            Self::Quote |
            Self::Quot => Keyboard(Key::Quote),
            Self::Comma |
            Self::Comm => Keyboard(Key::Comma),
            Self::Dot => Keyboard(Key::Dot),
            Self::Slash |
            Self::Sls => Keyboard(Key::Slash),
            Self::Escape |
            Self::Esc => Keyboard(Key::Escape),
            Self::Grave |
            Self::Grv => Keyboard(Key::Grave),
            Self::Tab => Keyboard(Key::Tab),
            Self::CapsLock |
            Self::Caps => Keyboard(Key::CapsLock),
            Self::LeftShift |
            Self::Lsft => Keyboard(Key::LeftShift),
            Self::LeftCtrl |
            Self::Lctrl => Keyboard(Key::LeftCtrl),
            Self::LeftWin |
            Self::Lwin => Keyboard(Key::LeftWin),
            Self::LeftAlt |
            Self::Lalt => Keyboard(Key::LeftAlt),
            Self::RightShift |
            Self::Rsft => Keyboard(Key::RightShift),
            Self::RightCtrl |
            Self::Rctrl => Keyboard(Key::RightCtrl),
            Self::RightWin |
            Self::Rwin => Keyboard(Key::RightWin),
            Self::RightAlt |
            Self::Ralt => Keyboard(Key::RightAlt),
            Self::Space |
            Self::Spc => Keyboard(Key::Space),
            Self::Enter |
            Self::Ent => Keyboard(Key::Enter),
            Self::PrintScreen |
            Self::Pscr => Keyboard(Key::PrintScreen),
            Self::ScrollLock |
            Self::Scrl => Keyboard(Key::ScrollLock),
            Self::Pause |
            Self::Paus => Keyboard(Key::Pause),
            Self::Insert |
            Self::Ins => Keyboard(Key::Insert),
            Self::Delete |
            Self::Del => Keyboard(Key::Delete),
            Self::Home => Keyboard(Key::Home),
            Self::End => Keyboard(Key::End),
            Self::PageUp |
            Self::Pgup => Keyboard(Key::PageUp),
            Self::PageDown |
            Self::Pgdn => Keyboard(Key::PageDown),
            Self::Left => Keyboard(Key::Left),
            Self::Up => Keyboard(Key::Up),
            Self::Right => Keyboard(Key::Right),
            Self::Down => Keyboard(Key::Down),
            Self::NumLock |
            Self::Num => Keyboard(Key::NumLock),
            Self::Keypad0 |
            Self::Kp0 => Keyboard(Key::Keypad0),
            Self::Keypad1 |
            Self::Kp1 => Keyboard(Key::Keypad1),
            Self::Keypad2 |
            Self::Kp2 => Keyboard(Key::Keypad2),
            Self::Keypad3 |
            Self::Kp3 => Keyboard(Key::Keypad3),
            Self::Keypad4 |
            Self::Kp4 => Keyboard(Key::Keypad4),
            Self::Keypad5 |
            Self::Kp5 => Keyboard(Key::Keypad5),
            Self::Keypad6 |
            Self::Kp6 => Keyboard(Key::Keypad6),
            Self::Keypad7 |
            Self::Kp7 => Keyboard(Key::Keypad7),
            Self::Keypad8 |
            Self::Kp8 => Keyboard(Key::Keypad8),
            Self::Keypad9 |
            Self::Kp9 => Keyboard(Key::Keypad9),
            Self::KeypadSlash |
            Self::KpSls => Keyboard(Key::KeypadSlash),
            Self::KeypadAsterisk |
            Self::KpAst => Keyboard(Key::KeypadAsterisk),
            Self::KeypadMinus |
            Self::KpMns => Keyboard(Key::KeypadMinus),
            Self::KeypadPlus |
            Self::KpPls => Keyboard(Key::KeypadPlus),
            Self::KeypadDot |
            Self::KpDot => Keyboard(Key::KeypadDot),

            Self::WheelUp => MouseWheel(Wheel::Up),
            Self::WheelDown => MouseWheel(Wheel::Down),

            Self::LeftButton |
            Self::Lb => MouseButton(Button::Left),
            Self::RightButton |
            Self::Rb => MouseButton(Button::Right),
            Self::MiddleButton |
            Self::Mb => MouseButton(Button::Middle),
            Self::BackButton |
            Self::Xb1 |
            Self::Bb => MouseButton(Button::Back),
            Self::ForwardButton |
            Self::Xb2 |
            Self::Fb => MouseButton(Button::Forward),

            _ => Err(ErrVar::FailedInputEventFrom { from: self.as_str().into() })?
        })
    }

    pub fn try_as_key(&self) -> ResVar<Key> {
        if let Ok(InputEvent::Keyboard(key)) = self.try_as_input_event() {
            return Ok(key)
        }

        Err(ErrVar::FailedKeyFrom { from: self.as_str().into() })
    }

    pub fn try_as_hotkey_prefix(&self) -> ResVar<Key> {
        if let Ok(InputEvent::Keyboard(key)) = self.try_as_input_event() && key.is_modifier() {
            return Ok(key)
        }

        Err(ErrVar::InvalidHotkeyPrefix { key: self.as_str() })
    }
}
impl TryFrom<&str> for BindVar {
    type Error = ErrVar;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Ok(match value {
            "-" => Self::Minus,
            "=" => Self::Equal,
            "[" => Self::LeftBracket,
            "]" => Self::RightBracket,
            "\\" => Self::Backslash,
            ";" => Self::Semicolon,
            "'" => Self::Quote,
            "," => Self::Comma,
            "." => Self::Dot,
            "/" => Self::Slash,
            "kp/" => Self::KeypadSlash,
            "kp*" => Self::KeypadAsterisk,
            "kp-" => Self::KeypadMinus,
            "kp+" => Self::KeypadPlus,
            "kp." => Self::KeypadDot,
            _ => Err(ErrVar::FailedBindVarFrom { from: value.into() })?
        })
    }
}
impl<'a> TryFrom<BindRepr<'a>> for BindVar {
    type Error = ErrVar;

    fn try_from(value: BindRepr) -> Result<Self, Self::Error> {
        match value {
            BindRepr::Enum(raw) => Ok(raw.into()),
            BindRepr::Num(n) => n.try_into(),
            BindRepr::Str(s) => s.try_into()
        }
    }
}
impl TryFrom<u32> for BindVar {
    type Error = ErrVar;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Ok(match value {
            0 => Self::N0,
            1 => Self::N1,
            2 => Self::N2,
            3 => Self::N3,
            4 => Self::N4,
            5 => Self::N5,
            6 => Self::N6,
            7 => Self::N7,
            8 => Self::N8,
            9 => Self::N9,
            10 => Self::N10,
            12 => Self::N12,
            16 => Self::N16,
            _ => Err(ErrVar::FailedBindVarFrom { from: value.to_string() })?
        })
    }
}
