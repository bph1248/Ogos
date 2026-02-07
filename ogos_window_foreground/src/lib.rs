use serde::*;

#[derive(Clone, Copy, Default, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    Top,
    Right,
    #[default]
    Bottom
}
