//! Saved passwords of remote places, in the Windows Credential Manager
//! (never in the settings). Each is a generic credential named after the
//! server, e.g. `Oxplorer:sftp://me@example.com`
//! ([`RemoteAddr::credential_target`]).

use super::url::RemoteAddr;

/// The saved password for the server of `addr`.
pub fn load(addr: &RemoteAddr) -> Option<String> {
    imp::read(&addr.credential_target())
}

/// Save the password for the server of `addr`.
pub fn save(addr: &RemoteAddr, password: &str) -> Result<(), String> {
    imp::write(&addr.credential_target(), &addr.user, password)
}

/// Forget the saved password (nothing happens when there is none).
pub fn delete(addr: &RemoteAddr) {
    imp::delete(&addr.credential_target());
}

/// Passwords can be saved on this system.
pub fn supported() -> bool {
    cfg!(windows)
}

#[cfg(windows)]
mod imp {
    use std::ptr;

    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::Security::Credentials::{
        CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
        CRED_TYPE_GENERIC,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn read(target: &str) -> Option<String> {
        let target = wide(target);
        let mut cred: *mut CREDENTIALW = ptr::null_mut();
        // SAFETY: `target` is NUL-terminated; on success `cred` points to a
        // credential that is freed with `CredFree` below.
        unsafe {
            if CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut cred) == 0 || cred.is_null() {
                return None;
            }
            let c = &*cred;
            let len = c.CredentialBlobSize as usize / 2;
            let password = if c.CredentialBlob.is_null() || len == 0 {
                String::new()
            } else {
                let units = std::slice::from_raw_parts(c.CredentialBlob as *const u16, len);
                String::from_utf16_lossy(units)
            };
            CredFree(cred as *const _);
            Some(password)
        }
    }

    pub fn write(target: &str, user: &str, password: &str) -> Result<(), String> {
        let mut target = wide(target);
        let mut user = wide(user);
        let blob: Vec<u16> = password.encode_utf16().collect();
        let cred = CREDENTIALW {
            Flags: 0,
            Type: CRED_TYPE_GENERIC,
            TargetName: target.as_mut_ptr(),
            Comment: ptr::null_mut(),
            LastWritten: FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            },
            CredentialBlobSize: (blob.len() * 2) as u32,
            CredentialBlob: blob.as_ptr() as *mut u8,
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            AttributeCount: 0,
            Attributes: ptr::null_mut(),
            TargetAlias: ptr::null_mut(),
            UserName: user.as_mut_ptr(),
        };
        // SAFETY: all pointers in `cred` point to live buffers for the call.
        if unsafe { CredWriteW(&cred, 0) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }

    pub fn delete(target: &str) {
        let target = wide(target);
        // SAFETY: `target` is NUL-terminated.
        unsafe {
            CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn read(_target: &str) -> Option<String> {
        None
    }

    pub fn write(_target: &str, _user: &str, _password: &str) -> Result<(), String> {
        Err(crate::i18n::t("Passwords can only be saved on Windows.").to_owned())
    }

    pub fn delete(_target: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_name_the_server_not_the_folder() {
        let a = RemoteAddr::parse("sftp://me@example.com/home/me").unwrap();
        let b = RemoteAddr::parse("sftp://me@example.com:22/srv").unwrap();
        assert_eq!(a.credential_target(), "Oxplorer:sftp://me@example.com");
        assert_eq!(a.credential_target(), b.credential_target());
        let dav = RemoteAddr::parse("webdavs://me@cloud.example.com:8443/dav").unwrap();
        assert_eq!(
            dav.credential_target(),
            "Oxplorer:webdavs://me@cloud.example.com:8443"
        );
    }

    /// Writes, reads and deletes a real credential (Windows only).
    #[test]
    #[ignore]
    fn round_trip_in_the_credential_manager() {
        let a = RemoteAddr::parse("sftp://oxplorer-test@test.invalid/").unwrap();
        save(&a, "pässwörd").unwrap();
        assert_eq!(load(&a).as_deref(), Some("pässwörd"));
        delete(&a);
        assert_eq!(load(&a), None);
    }
}
