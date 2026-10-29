//! Dropping every privilege but the one the service needs. Opening a volume
//! for reading is granted by the volume's security descriptor (SYSTEM and
//! Administrators), not by a privilege, so only `SeChangeNotifyPrivilege`
//! (bypass traverse checking, which every process has) is kept. The
//! installer also limits the service's privileges with `sc.exe privs`, so
//! this is the second line.

use windows_sys::Win32::Foundation::{GetLastError, LUID};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, LookupPrivilegeNameW, TokenPrivileges, LUID_AND_ATTRIBUTES,
    SE_PRIVILEGE_REMOVED, TOKEN_ADJUST_PRIVILEGES, TOKEN_QUERY,
};

use crate::account::{process_token, token_info};

/// The privileges the service keeps.
const KEEP: &[&str] = &["SeChangeNotifyPrivilege"];

fn keep(name: &str) -> bool {
    KEEP.contains(&name)
}

fn privilege_name(luid: &LUID) -> Option<String> {
    let mut buf = [0u16; 128];
    let mut len = buf.len() as u32;
    // SAFETY: the buffer and its length match.
    let ok = unsafe { LookupPrivilegeNameW(std::ptr::null(), luid, buf.as_mut_ptr(), &mut len) };
    (ok != 0).then(|| String::from_utf16_lossy(&buf[..(len as usize).min(buf.len())]))
}

/// Remove all privileges except [`KEEP`] from this process's token (for
/// good: removed privileges can't be enabled again). True on success.
pub fn drop_privileges() -> bool {
    let Some(token) = process_token(TOKEN_QUERY | TOKEN_ADJUST_PRIVILEGES) else {
        return false;
    };
    let Some(mut buf) = token_info(token.0, TokenPrivileges) else {
        return false;
    };
    let bytes = buf.len() * 8;
    let base = buf.as_mut_ptr().cast::<u8>();
    // SAFETY: TOKEN_PRIVILEGES starts with its count (u32), followed by the
    // LUID_AND_ATTRIBUTES array at offset 4; both stay inside the buffer.
    let count = unsafe { std::ptr::read_unaligned(base.cast::<u32>()) } as usize;
    let size = size_of::<LUID_AND_ATTRIBUTES>();
    if 4 + count * size > bytes {
        return false;
    }
    let mut removed = 0;
    for i in 0..count {
        // SAFETY: entry `i` is inside the buffer (checked above).
        let entry = unsafe { base.add(4 + i * size).cast::<LUID_AND_ATTRIBUTES>() };
        // SAFETY: as above; read and written unaligned.
        let mut e = unsafe { std::ptr::read_unaligned(entry) };
        if privilege_name(&e.Luid).is_some_and(|n| keep(&n)) {
            continue;
        }
        e.Attributes = SE_PRIVILEGE_REMOVED;
        // SAFETY: as above.
        unsafe { std::ptr::write_unaligned(entry, e) };
        removed += 1;
    }
    if removed == 0 {
        return true;
    }
    // SAFETY: `buf` holds a TOKEN_PRIVILEGES with `count` entries; no
    // previous state is asked for.
    let ok = unsafe {
        AdjustTokenPrivileges(
            token.0,
            0,
            buf.as_ptr().cast(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    // SAFETY: no arguments. ERROR_NOT_ALL_ASSIGNED (1300) means some weren't removed.
    ok != 0 && unsafe { GetLastError() } == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_bypass_traverse_checking() {
        assert!(keep("SeChangeNotifyPrivilege"));
        for name in [
            "SeDebugPrivilege",
            "SeBackupPrivilege",
            "SeRestorePrivilege",
            "SeTakeOwnershipPrivilege",
            "SeImpersonatePrivilege",
            "SeTcbPrivilege",
            "SeLoadDriverPrivilege",
            "SeManageVolumePrivilege",
            "SeAssignPrimaryTokenPrivilege",
        ] {
            assert!(!keep(name), "{name}");
        }
    }
}
