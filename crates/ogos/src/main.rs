#![cfg_attr(not(feature = "dbg_console"), windows_subsystem = "windows")]

use ogos_err::*;

#[hotpath::main(percentiles = [95, 99.99])]
fn main() -> Res<()> {
    ogos_lib::entry()
}
