#![cfg_attr(not(feature = "dbg_console"), windows_subsystem = "windows")]

use ogos_err::*;

fn main() -> Res<()> {
    ogos_lib::entry()
}
