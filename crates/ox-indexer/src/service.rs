//! Running as a Windows service: registering with the service control
//! manager, reporting status, and stopping on request or when idle.

use std::sync::atomic::{AtomicPtr, Ordering};
use std::time::Duration;

use ox_core::index::helper::security::pipe_sddl;
use ox_core::index::helper::SERVICE_NAME;
use windows_sys::core::PWSTR;
use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_CALL_NOT_IMPLEMENTED, ERROR_FAILED_SERVICE_CONTROLLER_CONNECT, NO_ERROR,
};
use windows_sys::Win32::System::Services::{
    RegisterServiceCtrlHandlerExW, SetServiceStatus, StartServiceCtrlDispatcherW,
    SERVICE_ACCEPT_SHUTDOWN, SERVICE_ACCEPT_STOP, SERVICE_CONTROL_INTERROGATE,
    SERVICE_CONTROL_SHUTDOWN, SERVICE_CONTROL_STOP, SERVICE_RUNNING, SERVICE_START_PENDING,
    SERVICE_STATUS, SERVICE_STATUS_CURRENT_STATE, SERVICE_STOPPED, SERVICE_TABLE_ENTRYW,
    SERVICE_WIN32_OWN_PROCESS,
};

use crate::account::{allowed_user, wide};
use crate::{privileges, server};

/// Started on demand: stop after this long without a connection.
const IDLE: Duration = Duration::from_secs(10 * 60);
/// Exit code for "not set up correctly" (no or a bad allowed user).
const BAD_CONFIG: u32 = 1;

static STATUS: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

fn report(state: SERVICE_STATUS_CURRENT_STATE, exit_code: u32) {
    let handle = STATUS.load(Ordering::SeqCst);
    if handle.is_null() {
        return;
    }
    let status = SERVICE_STATUS {
        dwServiceType: SERVICE_WIN32_OWN_PROCESS,
        dwCurrentState: state,
        dwControlsAccepted: if state == SERVICE_RUNNING {
            SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN
        } else {
            0
        },
        dwWin32ExitCode: exit_code,
        dwServiceSpecificExitCode: 0,
        dwCheckPoint: 0,
        dwWaitHint: if state == SERVICE_START_PENDING {
            5000
        } else {
            0
        },
    };
    // SAFETY: `handle` came from RegisterServiceCtrlHandlerExW.
    unsafe { SetServiceStatus(handle, &status) };
}

/// Report stopped and end the process (nothing needs cleaning up: the
/// handles close with it).
fn stop(exit_code: u32) -> ! {
    report(SERVICE_STOPPED, exit_code);
    std::process::exit(exit_code as i32)
}

fn stop_idle() {
    stop(0)
}

unsafe extern "system" fn handler(
    control: u32,
    _event: u32,
    _data: *mut core::ffi::c_void,
    _context: *mut core::ffi::c_void,
) -> u32 {
    match control {
        SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN => stop(0),
        SERVICE_CONTROL_INTERROGATE => NO_ERROR,
        _ => ERROR_CALL_NOT_IMPLEMENTED,
    }
}

unsafe extern "system" fn service_main(_argc: u32, _argv: *mut PWSTR) {
    let name = wide(SERVICE_NAME);
    // SAFETY: `name` is NUL-terminated; `handler` lives as long as the process.
    let handle =
        unsafe { RegisterServiceCtrlHandlerExW(name.as_ptr(), Some(handler), std::ptr::null()) };
    if handle.is_null() {
        return;
    }
    STATUS.store(handle, Ordering::SeqCst);
    report(SERVICE_START_PENDING, 0);
    // Refuse to run without a valid allowed user: never fall back to a
    // broader descriptor.
    let Ok(user) = allowed_user() else {
        stop(BAD_CONFIG)
    };
    let Some(sddl) = pipe_sddl(&user, "SY") else {
        stop(BAD_CONFIG)
    };
    if !privileges::drop_privileges() {
        stop(BAD_CONFIG);
    }
    report(SERVICE_RUNNING, 0);
    // Only returns on failure (e.g. the pipe name is taken).
    let _ = server::run(&sddl, Some((IDLE, stop_idle)), false);
    stop(BAD_CONFIG)
}

/// Hand this process to the service control manager (returns when the
/// service has stopped).
pub fn run_dispatcher() -> i32 {
    let mut name = wide(SERVICE_NAME);
    let table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: name.as_mut_ptr(),
            lpServiceProc: Some(service_main),
        },
        SERVICE_TABLE_ENTRYW {
            lpServiceName: std::ptr::null_mut(),
            lpServiceProc: None,
        },
    ];
    // SAFETY: the table ends with an empty entry and outlives the call.
    if unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) } == 0 {
        // SAFETY: no arguments.
        if unsafe { GetLastError() } == ERROR_FAILED_SERVICE_CONTROLLER_CONNECT {
            eprintln!(
                "This is the Oxplorer index service; Windows starts it. \
                 Use --console to run it here for debugging."
            );
        }
        return 1;
    }
    0
}
