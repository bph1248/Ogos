#![allow(unsafe_op_in_unsafe_fn)]

mod taskbar;

use log::error;
use ogos_lib::{
    common::*
};
use ogos_err::*;

use simplelog::*;
use std::{
    fs::File,
    path::Path
};
use windows::Win32::{
    Foundation::*,
    System::{
        LibraryLoader::*,
        SystemServices::*
    }
};

pub const LOG_FILE_DLL_NAME: &str = "ogos.dll.log";

#[allow(non_snake_case)]
#[unsafe(no_mangle)]
unsafe extern "system" fn DllMain(dll_module: HMODULE, call_reason: u32, _: *mut ()) -> BOOL {
    match call_reason {
        DLL_PROCESS_ATTACH => {
            let inner = || -> Res<()> {
                let mut dll_path = MAX_PATH_WITH_NULL_BUF;
                GetModuleFileNameW(dll_module, &mut dll_path);

                let dll_path = String::from_utf16(&dll_path)?;
                let dll_path = Path::new(&dll_path);
                let dll_parent_path = dll_path.parent().ok_or(ErrVar::InvalidPathParent)?;

                let log_path = dll_parent_path.join(LOG_FILE_DLL_NAME);
                let log_file = File::options()
                    .create(true)
                    .append(true)
                    .open(log_path)?;

                let logger_config = ConfigBuilder::new()
                    .set_thread_mode(ThreadLogMode::IDs)
                    .set_thread_level(LevelFilter::Error)
                    .set_thread_padding(ThreadPadding::Left(2))
                    .set_time_offset_to_local()?
                    .build();
                CombinedLogger::init(
                    vec![
                        WriteLogger::new(LevelFilter::Info, logger_config, log_file)
                    ]
                )?;

                Ok(())
            };

            if let Err(err) = inner() {
                error!("{}: dll process attach: {}", module_path!(), err);

                return FALSE
            }
        },
        DLL_PROCESS_DETACH => (),
        DLL_THREAD_ATTACH => (),
        DLL_THREAD_DETACH => (),
        _ => ()
    }

    TRUE
}
