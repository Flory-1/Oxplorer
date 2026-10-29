//! The named pipe server: one thread per connection (at most
//! [`MAX_CLIENTS`]), each answering requests with
//! `ox_core::index::helper::server::serve`.
//!
//! The first instance is created with `FILE_FLAG_FIRST_PIPE_INSTANCE`, so
//! the service refuses to run if someone else already holds the pipe name,
//! and a new instance is always created before a connected one is handed
//! off, so the name is never free for another process to take over. All
//! instances reject remote clients and carry the security descriptor from
//! `security::pipe_sddl`.

use std::fs::File;
use std::io;
use std::os::windows::io::FromRawHandle;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ox_core::index::helper::server::serve;
use ox_core::index::helper::{LocalVolumes, PIPE_NAME};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows_sys::Win32::Storage::FileSystem::{
    FlushFileBuffers, FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::LibraryLoader::{
    SetDefaultDllDirectories, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};

use crate::account::wide;

/// Connections served at the same time.
const MAX_CLIENTS: usize = 4;
const BUFFER: u32 = 64 * 1024;

/// Load system DLLs only from System32 (no planting next to the exe or in
/// the current folder).
pub fn harden() {
    // SAFETY: only changes this process's DLL search order.
    unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) };
}

/// A security descriptor made from SDDL, freed when dropped.
struct Descriptor(PSECURITY_DESCRIPTOR);

impl Descriptor {
    fn from_sddl(sddl: &str) -> io::Result<Self> {
        let w = wide(sddl);
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: `w` is NUL-terminated; `sd` receives a LocalAlloc'd descriptor.
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                w.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Descriptor(sd))
    }
}

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
        unsafe { LocalFree(self.0) };
    }
}

/// Serve the pipe until the process ends. With `idle`, `on_idle` is called
/// once no client was connected for that long.
pub fn run(sddl: &str, idle: Option<(Duration, fn())>, log: bool) -> io::Result<()> {
    let sd = Descriptor::from_sddl(sddl)?;
    let sa = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: 0,
    };
    let name = wide(PIPE_NAME);
    let vols = Arc::new(LocalVolumes::new());
    let active = Arc::new(AtomicUsize::new(0));
    let last = Arc::new(Mutex::new(Instant::now()));
    if let Some((after, on_idle)) = idle {
        let (active, last) = (active.clone(), last.clone());
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(15));
            let quiet = last.lock().unwrap_or_else(|e| e.into_inner()).elapsed();
            if active.load(Ordering::SeqCst) == 0 && quiet >= after {
                on_idle();
            }
        });
    }
    let mut first = true;
    loop {
        while active.load(Ordering::SeqCst) >= MAX_CLIENTS {
            std::thread::sleep(Duration::from_millis(50));
        }
        let flags = PIPE_ACCESS_DUPLEX
            | if first {
                FILE_FLAG_FIRST_PIPE_INSTANCE
            } else {
                0
            };
        // SAFETY: `name` is NUL-terminated and `sa` (with `sd`) outlives the call.
        let h = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                flags,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                MAX_CLIENTS as u32 + 1,
                BUFFER,
                BUFFER,
                0,
                &sa,
            )
        };
        if h == INVALID_HANDLE_VALUE {
            let e = io::Error::last_os_error();
            if first {
                // Someone else holds the name: don't serve next to them.
                return Err(e);
            }
            std::thread::sleep(Duration::from_millis(200));
            continue;
        }
        first = false;
        // SAFETY: `h` is a pipe instance we just created.
        let connected = unsafe { ConnectNamedPipe(h, std::ptr::null_mut()) } != 0
            // SAFETY: no arguments.
            || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        if !connected {
            // SAFETY: created above, closed once.
            unsafe { CloseHandle(h) };
            continue;
        }
        active.fetch_add(1, Ordering::SeqCst);
        *last.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
        // SAFETY: the File takes over the handle and closes it when dropped.
        let mut conn = unsafe { File::from_raw_handle(h) };
        let (vols, done, last) = (vols.clone(), active.clone(), last.clone());
        let spawned = std::thread::Builder::new()
            .name("client".into())
            .spawn(move || {
                if let Err(e) = serve(&mut conn, vols.as_ref()) {
                    if log {
                        eprintln!("connection ended: {e}");
                    }
                }
                // SAFETY: the handle is open (owned by `conn`, closed when it drops).
                unsafe {
                    use std::os::windows::io::AsRawHandle;
                    FlushFileBuffers(conn.as_raw_handle());
                    DisconnectNamedPipe(conn.as_raw_handle());
                }
                drop(conn);
                *last.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
                done.fetch_sub(1, Ordering::SeqCst);
            });
        if spawned.is_err() {
            active.fetch_sub(1, Ordering::SeqCst);
        }
    }
}
