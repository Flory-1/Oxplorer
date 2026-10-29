//! Windows only: connecting to the index service's pipe, starting the
//! service when it isn't running, and asking how it is.

use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    GetLastError, LocalFree, ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY,
    ERROR_SERVICE_ALREADY_RUNNING, ERROR_SERVICE_DOES_NOT_EXIST,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, GetSecurityInfo, SE_KERNEL_OBJECT,
};
use windows_sys::Win32::Security::{OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID};
use windows_sys::Win32::Storage::FileSystem::SECURITY_IDENTIFICATION;
use windows_sys::Win32::System::Pipes::WaitNamedPipeW;
use windows_sys::Win32::System::Services::{
    CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatus, StartServiceW, SC_HANDLE,
    SC_MANAGER_CONNECT, SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_START, SERVICE_STATUS,
};

use super::client::Client;
use super::security::{trusted_owner, CLIENT_ACCESS};
use super::{Status, PIPE_NAME, SERVICE_NAME};
use crate::index::mft::wide;

/// How long to wait for a service that was just started.
const START_WAIT: Duration = Duration::from_secs(5);

struct Service(SC_HANDLE);

impl Drop for Service {
    fn drop(&mut self) {
        // SAFETY: the handle came from OpenSCManagerW/OpenServiceW and is closed once.
        unsafe { CloseServiceHandle(self.0) };
    }
}

/// The service with `access`, or the error code.
fn open_service(access: u32) -> Result<Service, u32> {
    // SAFETY: null machine and database names mean the local SCM.
    let scm = unsafe { OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT) };
    if scm.is_null() {
        // SAFETY: no arguments.
        return Err(unsafe { GetLastError() });
    }
    let scm = Service(scm);
    let name = wide(SERVICE_NAME);
    // SAFETY: `scm` is open and `name` is NUL-terminated.
    let svc = unsafe { OpenServiceW(scm.0, name.as_ptr(), access) };
    if svc.is_null() {
        // SAFETY: no arguments.
        return Err(unsafe { GetLastError() });
    }
    Ok(Service(svc))
}

/// Start the service (the installer lets the allowed user do that).
fn start_service() -> bool {
    let Ok(svc) = open_service(SERVICE_START) else {
        return false;
    };
    // SAFETY: `svc` is open; no arguments are passed to the service.
    let ok = unsafe { StartServiceW(svc.0, 0, std::ptr::null()) };
    // SAFETY: no arguments.
    ok != 0 || unsafe { GetLastError() } == ERROR_SERVICE_ALREADY_RUNNING
}

/// The SID that owns the pipe (`S-1-5-18` for the service).
fn owner_sid(file: &File) -> Option<String> {
    let mut owner: PSID = std::ptr::null_mut();
    let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: the handle is open; `owner` points into `sd`, which is freed below.
    let err = unsafe {
        GetSecurityInfo(
            file.as_raw_handle(),
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut sd,
        )
    };
    if err != 0 || owner.is_null() {
        return None;
    }
    let mut text = std::ptr::null_mut();
    // SAFETY: `owner` is a valid SID inside `sd`.
    let ok = unsafe { ConvertSidToStringSidW(owner, &mut text) };
    let sid = (ok != 0).then(|| {
        // SAFETY: `text` is a NUL-terminated string allocated by Windows.
        let len = (0..).take_while(|&i| unsafe { *text.add(i) } != 0).count();
        // SAFETY: `len` characters were just checked.
        String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, len) })
    });
    // SAFETY: both were allocated with LocalAlloc by the calls above.
    unsafe {
        if !text.is_null() {
            LocalFree(text.cast());
        }
        LocalFree(sd);
    }
    sid
}

/// Open the pipe, asking for exactly the rights its descriptor grants the
/// user (`GENERIC_WRITE` would include creating pipe instances, which it
/// doesn't). The service may only identify the app, not act as it.
fn open_pipe() -> std::io::Result<File> {
    OpenOptions::new()
        .access_mode(CLIENT_ACCESS)
        .security_qos_flags(SECURITY_IDENTIFICATION)
        .open(PIPE_NAME)
}

/// Connect to the service, starting it first if `start` is set and it isn't
/// running. `Err` carries the Windows error code (access denied = the
/// service was installed for another user).
pub(super) fn connect_raw(start: bool) -> Result<Client<File>, u32> {
    let mut started: Option<Instant> = None;
    loop {
        match open_pipe() {
            Ok(f) => {
                return match owner_sid(&f) {
                    Some(sid) if trusted_owner(&sid) => Ok(Client::new(f)),
                    other => {
                        log::warn!("index service: the pipe is owned by {other:?}; not using it");
                        Err(ERROR_ACCESS_DENIED)
                    }
                };
            }
            Err(e) => {
                let code = e.raw_os_error().unwrap_or(0) as u32;
                match code {
                    ERROR_PIPE_BUSY => {
                        let name = wide(PIPE_NAME);
                        // SAFETY: `name` is NUL-terminated.
                        if unsafe { WaitNamedPipeW(name.as_ptr(), 2000) } == 0 {
                            return Err(code);
                        }
                    }
                    ERROR_FILE_NOT_FOUND if start => match started {
                        None => {
                            if !start_service() {
                                return Err(code);
                            }
                            started = Some(Instant::now());
                        }
                        Some(t) if t.elapsed() < START_WAIT => {
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        Some(_) => return Err(code),
                    },
                    _ => return Err(code),
                }
            }
        }
    }
}

/// How the service is, for the settings page (talks to the SCM and the pipe;
/// call off the UI thread).
pub(super) fn query_status() -> Status {
    let svc = match open_service(SERVICE_QUERY_STATUS) {
        Ok(s) => s,
        Err(ERROR_SERVICE_DOES_NOT_EXIST) => return Status::NotInstalled,
        Err(_) => return Status::Unavailable,
    };
    // SAFETY: SERVICE_STATUS is plain data.
    let mut st: SERVICE_STATUS = unsafe { std::mem::zeroed() };
    // SAFETY: `svc` is open and `st` is writable.
    if unsafe { QueryServiceStatus(svc.0, &mut st) } == 0 {
        return Status::Unavailable;
    }
    if st.dwCurrentState != SERVICE_RUNNING {
        return Status::Stopped;
    }
    match connect_raw(false) {
        Ok(mut c) => match c.volumes() {
            Ok(v) => Status::Running(v),
            Err(_) => Status::Unavailable,
        },
        Err(ERROR_ACCESS_DENIED) => Status::NoAccess,
        Err(_) => Status::Unavailable,
    }
}
