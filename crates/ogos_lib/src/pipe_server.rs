use crate::{window_foreground};
use ogos_common::*;
use ogos_core::*;
use ogos_err::*;

use log::*;
use serde::*;
use std::{
    ffi::*,
    sync::mpsc::*,
    thread::{self, *}
};
use strum::*;
use tokio::sync::*;
use windows::{
    core::*,
    Win32::{
        Foundation::*,
        Security::{
            Authorization::*,
            *
        },
        Storage::FileSystem::*,
        System::{
            Memory::*,
            Pipes::*,
            SystemServices::*
        }
    }
};

const TAG_SIZE: usize = size_of::<u8>();
const STRING_LENGTH_SIZE: usize = size_of::<u64>();
const MAX_PATH_LENGTH_UTF8_WORST_CASE_SIZE: usize = 256 * 4;
const PIPE_SIZE_CHECK: usize =
    TAG_SIZE +
    STRING_LENGTH_SIZE +
    MAX_PATH_LENGTH_UTF8_WORST_CASE_SIZE;
const _: () = assert!(PIPE_SIZE_CHECK <= u32::MAX as usize);

pub(crate) const PIPE_NAME: &str = r"\\.\pipe\ogos";
pub(crate) const PIPE_SIZE: u32 = PIPE_SIZE_CHECK as u32;

#[derive(Deserialize, Display, Serialize)]
pub(crate) enum Msg {
    Ack,
    ActiveGame(Option<String>),
    Close
}

// See https://learn.microsoft.com/en-us/windows/win32/secauthz/creating-a-security-descriptor-for-a-new-object-in-c--
fn begin(send_ready: Sender<ReadyMsg>, window_foreground_sx: Option<Sender<window_foreground::Msg>>) -> Res<()> { unsafe {
    info!("{}: begin", module_path!());

    // Init a SID for the well-known Everyone group
    let mut everyone_sid = PSID::default();
    AllocateAndInitializeSid(&SECURITY_WORLD_SID_AUTHORITY, 1, SECURITY_WORLD_RID as u32, 0, 0, 0, 0, 0, 0, 0, &mut everyone_sid)?;

    // Init a SID for the BUILTIN\Administrators group
    let mut builtin_admins_sid = PSID::default();
    AllocateAndInitializeSid(&SECURITY_NT_AUTHORITY, 2, SECURITY_BUILTIN_DOMAIN_RID as u32, DOMAIN_ALIAS_RID_ADMINS as u32, 0, 0, 0, 0, 0, 0, &mut builtin_admins_sid)?;

    // Set entries in the ACL (access control list)
    let explicit_accesses = [
        // Everyone
        EXPLICIT_ACCESS_W {
            grfAccessPermissions: GENERIC_READ.0 | GENERIC_WRITE.0 | SYNCHRONIZE.0,
            grfAccessMode: SET_ACCESS,
            grfInheritance: NO_INHERITANCE,
            Trustee: TRUSTEE_W {
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
                ptstrName: PWSTR(everyone_sid.0.cast::<u16>()),
                ..default!()
            }
        },
        // Admins
        EXPLICIT_ACCESS_W {
            grfAccessPermissions: GENERIC_READ.0 | GENERIC_WRITE.0 | SYNCHRONIZE.0,
            grfAccessMode: SET_ACCESS,
            grfInheritance: NO_INHERITANCE,
            Trustee: TRUSTEE_W {
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_GROUP,
                ptstrName: PWSTR(builtin_admins_sid.0.cast::<u16>()),
                ..default!()
            }
        }
    ];
    let mut access_control_list = ACL::default();
    let mut access_control_list_ptr = &mut access_control_list as *mut _;
    SetEntriesInAclW(Some(&explicit_accesses), None, &mut access_control_list_ptr).ok()?;

    // Init a security descriptor and set its DACL (discretionary access control list)
    let security_descriptor_alloc = LocalAlloc(LPTR, size_of::<SECURITY_DESCRIPTOR>())?;
    let security_descriptor = PSECURITY_DESCRIPTOR(security_descriptor_alloc.0.cast::<c_void>());
    InitializeSecurityDescriptor(security_descriptor, SECURITY_DESCRIPTOR_REVISION)?;
    SetSecurityDescriptorDacl(security_descriptor, true, Some(access_control_list_ptr), false)?;

    let security_attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: security_descriptor.0,
        bInheritHandle: FALSE
    };
    let pipe_name = PIPE_NAME.to_win_str();
    let pipe_hnd = CreateNamedPipeW(
        *pipe_name,
        PIPE_ACCESS_DUPLEX,
        PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
        1,
        0,
        PIPE_SIZE,
        0,
        Some(&security_attributes)
    ).win32_core_ok()?;

    send_ready.send(ReadyMsg::PipeServer)?;
    drop(send_ready);

    let ack = || -> Res1<_> {
        let pipe_ack = bincode::serialize(&Msg::Ack)?;
        WriteFile(pipe_hnd, Some(&pipe_ack), None, None)?;

        Ok(())
    };

    loop {
        ConnectNamedPipe(pipe_hnd, None)?;
        info!("{}: connected: {}", module_path!(), PIPE_NAME);

        let mut buf = [0_u8; PIPE_SIZE as usize];
        ReadFile(pipe_hnd, Some(&mut buf), None, None)?;

        let msg = bincode::deserialize::<Msg>(&buf)?;
        info!("{}: recvd: {}", module_path!(), msg);

        match msg {
            Msg::ActiveGame(_) => if let Some(sx) = window_foreground_sx.as_ref() {
                let (ack_sx, ack_rx) = oneshot::channel::<()>();

                sx.send(window_foreground::Msg::Pipe((msg, ack_sx))).unwrap();
                ack_rx.blocking_recv().unwrap();

                ack()?;
            },
            Msg::Close => {
                ack()?;

                DisconnectNamedPipe(pipe_hnd)?;

                break
            },
            Msg::Ack => ()
        }

        DisconnectNamedPipe(pipe_hnd)?;
    }

    CloseHandle(pipe_hnd)?;
    info!("{}: closed", module_path!());

    Ok(())
} }

pub(crate) fn spawn(send_ready: Sender<ReadyMsg>, window_foreground_sx: Option<Sender<window_foreground::Msg>>) -> JoinHandle<()> {
    thread::spawn(|| {
        begin(send_ready, window_foreground_sx).unwrap_or_else(|err| {
            error!("{}: terminated: {}", module_path!(), err);
        });
    })
}
