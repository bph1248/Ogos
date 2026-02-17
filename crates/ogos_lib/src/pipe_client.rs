use crate::pipe_server::{self, *};
use ogos_common::*;
use ogos_err::*;

use windows::Win32::{
    Foundation::*,
    Storage::FileSystem::*
};

pub(crate) fn pipe_msg(msg: pipe_server::Msg) -> Res1<()> { unsafe {
    let pipe_name = PIPE_NAME.to_win_str();
    let pipe = CreateFileW(
        *pipe_name,
        FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
        FILE_SHARE_NONE,
        None,
        OPEN_EXISTING,
        FILE_ATTRIBUTE_NORMAL,
        None,
    )?;

    let msg = bincode::serialize(&msg)?;
    WriteFile(pipe, Some(&msg), None, None)?;

    let mut buf = [0_u8; PIPE_SIZE as usize];
    ReadFile(pipe, Some(&mut buf), None, None)?;

    bincode::deserialize::<pipe_server::Msg>(&buf)?; // Only receiving ack

    CloseHandle(pipe)?;

    Ok(())
} }
