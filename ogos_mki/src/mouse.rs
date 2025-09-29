use serde::*;
use std::{
    fmt,
    str::*
};

#[cfg(target_os = "windows")] // Not sure how to detect double on linux
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Button {
    Left,
    DoubleLeft,
    Right,
    DoubleRight,
    Middle,
    DoubleMiddle,
    Back,
    DoubleSide,
    Forward,
    DoubleExtra
}
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Mouse {
    Left,
    Right,
    Middle,
    Side,
    Extra,
    Forward,
    Back,
    Task,
}
impl FromStr for Button {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use Button::*;

        Ok(match s {
            "left_button" | "lft_but" | "lb" => Left,
            "right_button" | "rht_but" | "rb" => Right,
            "middle_button" | "mdl_but" | "mb" => Middle,
            "back_button" | "bck_but" | "xb1" | "bb" => Back,
            "forward_button" | "fwd_but" | "xb2" | "fb" => Forward,
            _ => Err(())?
        })
    }
}
impl fmt::Display for Button {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("{:?}", self))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Wheel {
    Up,
    Down
}
impl FromStr for Wheel {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use Wheel::*;

        Ok(match s {
            "wheel_up" | "wup" | "wu" => Up,
            "wheel_down" | "wdown" | "wd" => Down,
            _ => Err(())?
        })
    }
}
impl fmt::Display for Wheel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("{:?}", self))
    }
}
