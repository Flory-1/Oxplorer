//! Whose SID: the allowed user from the registry, the user running this
//! process, and the user logged on to this session (for the installer).

use ox_core::index::helper::security::is_user_sid;
use ox_core::index::helper::{ALLOWED_USER_VALUE, PARAMETERS_KEY};
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE};
use windows_sys::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSidToSidW};
use windows_sys::Win32::Security::{
    GetTokenInformation, LookupAccountNameW, LookupAccountSidW, SidTypeUser, TokenUser, PSID,
    SID_NAME_USE, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
use windows_sys::Win32::System::RemoteDesktop::{
    ProcessIdToSessionId, WTSDomainName, WTSFreeMemory, WTSQuerySessionInformationW, WTSUserName,
    WTS_CURRENT_SERVER_HANDLE,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcessToken,
};

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// A NUL-terminated wide string as a `String`.
///
/// # Safety
/// `p` must be null or point to a NUL-terminated UTF-16 string.
unsafe fn from_wide(p: *const u16) -> Option<String> {
    if p.is_null() {
        return None;
    }
    // SAFETY: the caller promises a NUL-terminated string.
    let len = (0..).take_while(|&i| unsafe { *p.add(i) } != 0).count();
    // SAFETY: `len` characters were just read.
    Some(String::from_utf16_lossy(unsafe {
        std::slice::from_raw_parts(p, len)
    }))
}

fn sid_to_string(sid: PSID) -> Option<String> {
    let mut text = std::ptr::null_mut();
    // SAFETY: `sid` is a valid SID; `text` receives a LocalAlloc'd string.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return None;
    }
    // SAFETY: `text` is NUL-terminated; it is freed right after.
    let s = unsafe { from_wide(text) };
    // SAFETY: allocated by ConvertSidToStringSidW.
    unsafe { LocalFree(text.cast()) };
    s
}

/// Does `sid` belong to a user account (not a group, alias or well-known SID)?
pub fn is_user_account(sid: &str) -> bool {
    let w = wide(sid);
    let mut psid: PSID = std::ptr::null_mut();
    // SAFETY: `w` is NUL-terminated; `psid` receives a LocalAlloc'd SID.
    if unsafe { ConvertStringSidToSidW(w.as_ptr(), &mut psid) } == 0 {
        return false;
    }
    let mut name = [0u16; 256];
    let mut domain = [0u16; 256];
    let (mut nl, mut dl) = (name.len() as u32, domain.len() as u32);
    let mut kind: SID_NAME_USE = 0;
    // SAFETY: the buffers and their lengths match; `psid` is valid.
    let ok = unsafe {
        LookupAccountSidW(
            std::ptr::null(),
            psid,
            name.as_mut_ptr(),
            &mut nl,
            domain.as_mut_ptr(),
            &mut dl,
            &mut kind,
        )
    };
    // SAFETY: allocated by ConvertStringSidToSidW.
    unsafe { LocalFree(psid) };
    ok != 0 && kind == SidTypeUser
}

/// The allowed user the installer recorded, checked.
pub fn allowed_user() -> Result<String, String> {
    let key = wide(PARAMETERS_KEY);
    let value = wide(ALLOWED_USER_VALUE);
    let mut buf = [0u16; 256];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: the buffer and its size in bytes match.
    let err = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if err != 0 {
        return Err(format!("no allowed user in the registry (error {err})"));
    }
    let chars = (len as usize / 2).min(buf.len());
    let sid: String = String::from_utf16_lossy(&buf[..chars])
        .trim_end_matches('\0')
        .to_owned();
    if !is_user_sid(&sid) || !is_user_account(&sid) {
        return Err(format!("{sid:?} isn't a user account"));
    }
    Ok(sid)
}

pub struct Token(pub HANDLE);

impl Drop for Token {
    fn drop(&mut self) {
        // SAFETY: opened by OpenProcessToken, closed once.
        unsafe { CloseHandle(self.0) };
    }
}

/// This process's token, for querying (and adjusting when `access` says so).
pub fn process_token(access: u32) -> Option<Token> {
    let mut h: HANDLE = std::ptr::null_mut();
    // SAFETY: the pseudo handle of this process; `h` receives the token.
    if unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut h) } == 0 {
        return None;
    }
    Some(Token(h))
}

/// Information of `class` about `token`, in a buffer aligned for its structs.
pub fn token_info(token: HANDLE, class: i32) -> Option<Vec<u64>> {
    let mut len = 0u32;
    // SAFETY: asks for the size only.
    unsafe { GetTokenInformation(token, class, std::ptr::null_mut(), 0, &mut len) };
    if len == 0 {
        return None;
    }
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: the buffer holds at least `len` bytes.
    let ok = unsafe { GetTokenInformation(token, class, buf.as_mut_ptr().cast(), len, &mut len) };
    (ok != 0).then_some(buf)
}

/// The SID of the user running this process.
pub fn current_user_sid() -> Option<String> {
    let token = process_token(TOKEN_QUERY)?;
    let buf = token_info(token.0, TokenUser)?;
    // SAFETY: GetTokenInformation(TokenUser) filled the aligned buffer with a TOKEN_USER.
    let user: TOKEN_USER = unsafe { std::ptr::read(buf.as_ptr().cast()) };
    sid_to_string(user.User.Sid)
}

/// A string from WTSQuerySessionInformationW.
fn session_info(session: u32, class: i32) -> Option<String> {
    let mut p: *mut u16 = std::ptr::null_mut();
    let mut bytes = 0u32;
    // SAFETY: `p` receives a buffer freed with WTSFreeMemory below.
    let ok = unsafe {
        WTSQuerySessionInformationW(
            WTS_CURRENT_SERVER_HANDLE,
            session,
            class,
            &mut p,
            &mut bytes,
        )
    };
    if ok == 0 {
        return None;
    }
    // SAFETY: a NUL-terminated string from Windows.
    let s = unsafe { from_wide(p) };
    // SAFETY: allocated by WTSQuerySessionInformationW.
    unsafe { WTSFreeMemory(p.cast()) };
    s.filter(|s| !s.is_empty())
}

/// The SID of the user logged on to this process's session: the person at
/// the screen, also when the installer was elevated with another account.
pub fn session_user_sid() -> Option<String> {
    let mut session = 0u32;
    // SAFETY: `session` receives the id.
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) } == 0 {
        return None;
    }
    let user = session_info(session, WTSUserName)?;
    let domain = session_info(session, WTSDomainName).unwrap_or_default();
    let account = wide(&if domain.is_empty() {
        user
    } else {
        format!("{domain}\\{user}")
    });
    let mut sid = vec![0u64; 16]; // 128 bytes: more than the largest SID (68)
    let mut cb = (sid.len() * 8) as u32;
    let mut dom = [0u16; 256];
    let mut dl = dom.len() as u32;
    let mut kind: SID_NAME_USE = 0;
    // SAFETY: the buffers and their lengths match.
    let ok = unsafe {
        LookupAccountNameW(
            std::ptr::null(),
            account.as_ptr(),
            sid.as_mut_ptr().cast(),
            &mut cb,
            dom.as_mut_ptr(),
            &mut dl,
            &mut kind,
        )
    };
    if ok == 0 || kind != SidTypeUser {
        return None;
    }
    sid_to_string(sid.as_mut_ptr().cast())
}
