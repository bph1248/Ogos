use crate::*;

use std::mem::{self, *};
use winapi::{
    ctypes::*,
    um::winuser::*
};

pub(crate) mod mimpl {
    use crate::windows::mouse::{
        button_to_event_down, button_to_mouse_data, mouse_click, mouse_interact_with, mouse_press,
        mouse_release, Pos,
    };
    use crate::Button;

    pub(crate) fn press(button: Button) {
        mouse_press(button)
    }

    pub(crate) fn click(button: Button) {
        mouse_click(button);
    }

    pub(crate) fn release(button: Button) {
        mouse_release(button);
    }

    // normalized absolute coordinates between 0 and 65,535
    // See remarks: https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-mouseinput#remarks
    // For now the library does not support a solution of it - caller is expected to solve it for himself
    // One of possible solutions would be to try to accept multiple parameters such as
    // "Screen Index + position on that screen index, but hey that sounds a bit too complex. sorry.
    pub(crate) fn move_to(x: i32, y: i32) {
        mouse_interact_with(0, 0, Some(Pos::absolute(x, y)));
    }

    // Unlike move_to this uses a human friendly coordinates, 10 is 10 pixels.
    pub(crate) fn move_by(x: i32, y: i32) {
        mouse_interact_with(0, 0, Some(Pos::relative(x, y)));
    }

    pub(crate) fn click_at(x: i32, y: i32, button: Button) {
        mouse_interact_with(
            button_to_event_down(button),
            button_to_mouse_data(button),
            Some(Pos::absolute(x, y)),
        )
    }
}

struct Pos {
    x: i32,
    y: i32,
    absolute: bool,
}

impl Pos {
    fn absolute(x: i32, y: i32) -> Self {
        Pos {
            x,
            y,
            absolute: true,
        }
    }

    fn relative(x: i32, y: i32) -> Self {
        Pos {
            x,
            y,
            absolute: false,
        }
    }
}

fn mouse_interact_with(mut interaction: u32, mouse_data: u16, pos: Option<Pos>) {
    let mut x = 0;
    let mut y = 0;
    if let Some(pos) = pos {
        if pos.absolute {
            interaction |= MOUSEEVENTF_ABSOLUTE;
        }
        x = pos.x;
        y = pos.y;
        interaction |= MOUSEEVENTF_MOVE;
    }
    unsafe {
        let mut input: INPUT_u = mem::zeroed();
        *input.mi_mut() = MOUSEINPUT {
            dx: x,
            dy: y,
            mouseData: mouse_data.into(),
            time: 0,
            dwFlags: interaction,
            dwExtraInfo: 0,
        };
        let mut x = INPUT {
            type_: INPUT_MOUSE,
            u: input,
        };

        SendInput(1, &mut x as LPINPUT, size_of::<INPUT>() as c_int);
    }
}

pub fn mouse_press(button: Button) {
    mouse_interact_with(
        button_to_event_down(button),
        button_to_mouse_data(button),
        mouse_to_pos(button),
    )
}

pub fn mouse_release(button: Button) {
    mouse_interact_with(
        button_to_event_up(button),
        button_to_mouse_data(button),
        mouse_to_pos(button),
    )
}

pub fn mouse_click(button: Button) {
    let click = button_to_event_down(button) | button_to_event_up(button);
    mouse_interact_with(click, button_to_mouse_data(button), mouse_to_pos(button))
}

fn button_to_mouse_data(button: Button) -> u16 {
    match button {
        Button::Back | Button::DoubleSide => XBUTTON1,
        Button::Forward | Button::DoubleExtra => XBUTTON2,
        _ => 0,
    }
}

fn button_to_event_up(button: Button) -> u32 {
    use Button::*;
    match button {
        Left | DoubleLeft => MOUSEEVENTF_LEFTUP,
        Right | DoubleRight => MOUSEEVENTF_RIGHTUP,
        Middle | DoubleMiddle => MOUSEEVENTF_MIDDLEUP,
        Back | DoubleSide | Forward | DoubleExtra => MOUSEEVENTF_XUP
    }
}

fn button_to_event_down(button: Button) -> u32 {
    use Button::*;
    match button {
        Left | DoubleLeft => MOUSEEVENTF_LEFTDOWN,
        Right | DoubleRight => MOUSEEVENTF_RIGHTDOWN,
        Middle | DoubleMiddle => MOUSEEVENTF_MIDDLEDOWN,
        Back | DoubleSide | Forward | DoubleExtra => MOUSEEVENTF_XDOWN
    }
}

fn mouse_to_pos(button: Button) -> Option<Pos> {
    use Button::*;
    match button {
        Left | DoubleLeft => None,
        Right | DoubleRight => None,
        Middle | DoubleMiddle => None,
        Back | DoubleSide | Forward | DoubleExtra => None
    }
}
