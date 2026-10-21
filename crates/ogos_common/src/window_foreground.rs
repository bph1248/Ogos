use super::*;

pub const PROGMAN_CLASS_NAME: &str = "Progman";
pub const PROGMAN_CLASS_NAME_W: &U16CStr = u16cstr!(PROGMAN_CLASS_NAME);
pub const START_MENU_CLASS_NAME_W: &U16CStr = u16cstr!("Start");
pub const TASKBAR_CLASS_NAME: &str = "Shell_TrayWnd";
pub const TASKBAR_CLASS_NAME_W: &U16CStr = u16cstr!(TASKBAR_CLASS_NAME);
pub const WORKERW_CLASS_NAME: &str = "WorkerW";
pub const WINDOW_WATCH_CLASS_NAME_W: &U16CStr = u16cstr!("OgosWindowWatch");

#[derive(Clone, Copy, Default, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    Top,
    Right,
    #[default]
    Bottom
}
