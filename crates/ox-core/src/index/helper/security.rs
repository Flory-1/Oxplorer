//! Who may talk to the index service: the security descriptor of its pipe
//! and the checks on the user SID the installer records.

/// The pipe's access rights for the allowed user: `FILE_GENERIC_READ |
/// FILE_WRITE_DATA`. Deliberately not `GENERIC_WRITE`, which includes
/// `FILE_CREATE_PIPE_INSTANCE` and would let the user create server ends of
/// the pipe (and pose as the service).
pub const CLIENT_ACCESS: u32 = 0x0012_0089 | 0x2;

/// Is `sid` the SID of a person's account: a local, domain or Microsoft
/// account (`S-1-5-21-…`) or an Entra ID account (`S-1-12-1-…`)? Groups
/// like Everyone (`S-1-1-0`), Authenticated Users (`S-1-5-11`), Users
/// (`S-1-5-32-545`) and the service accounts are refused; the service also
/// asks Windows that the SID really is a user.
pub fn is_user_sid(sid: &str) -> bool {
    let (rest, parts) = if let Some(r) = sid.strip_prefix("S-1-5-21-") {
        (r, 4)
    } else if let Some(r) = sid.strip_prefix("S-1-12-1-") {
        (r, 4)
    } else {
        return false;
    };
    let nums: Vec<&str> = rest.split('-').collect();
    nums.len() == parts
        && nums.iter().all(|n| {
            !n.is_empty()
                && n.len() <= 10
                && n.bytes().all(|b| b.is_ascii_digit())
                && n.parse::<u32>().is_ok()
        })
}

/// The pipe's security descriptor in SDDL: owned by `owner` (`SY` for the
/// service, `BA` when run in a console as administrator), full access for
/// SYSTEM and Administrators, read and write (but not creating pipe
/// instances) for the one allowed user, and nothing for anyone else.
/// Network logons are denied outright (the pipe also rejects remote
/// clients). The DACL is protected, so nothing is inherited. `None` if
/// `user` isn't a user SID.
pub fn pipe_sddl(user: &str, owner: &str) -> Option<String> {
    if !is_user_sid(user) || !matches!(owner, "SY" | "BA") {
        return None;
    }
    Some(format!(
        "O:{owner}G:SYD:P(D;;GA;;;NU)(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x{CLIENT_ACCESS:08x};;;{user})"
    ))
}

/// May the pipe's owner be trusted as the service? The service sets its
/// pipe's owner to SYSTEM (or Administrators in console mode); only those
/// can create objects owned by them, so a pipe squatted by another user is
/// refused before anything is sent.
pub fn trusted_owner(sid: &str) -> bool {
    sid == "S-1-5-18" || sid == "S-1-5-32-544"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_user_sids() {
        for ok in [
            "S-1-5-21-3623811015-3361044348-30300820-1013",
            "S-1-5-21-1-2-3-500",
            "S-1-12-1-1234567890-1234567890-1234567890-1234567890",
        ] {
            assert!(is_user_sid(ok), "{ok}");
        }
        for bad in [
            "",
            "S-1-1-0",
            "S-1-5-11",
            "S-1-5-18",
            "S-1-5-32-545",
            "S-1-5-32-544",
            "S-1-5-21-1-2-3",
            "S-1-5-21-1-2-3-4-5",
            "S-1-5-21-1-2-3-",
            "S-1-5-21-1-2-3-x",
            "S-1-5-21-1-2-3-99999999999",
            "S-1-5-21-1-2-3-+5",
            "S-1-5-21-1-2-3-4)(A;;GA;;;WD",
            "s-1-5-21-1-2-3-4",
            " S-1-5-21-1-2-3-4",
            "S-1-5-21-1-2-3-4\n",
        ] {
            assert!(!is_user_sid(bad), "{bad:?}");
        }
    }

    #[test]
    fn pipe_sddl_allows_only_the_user() {
        let sid = "S-1-5-21-1-2-3-1001";
        let sddl = pipe_sddl(sid, "SY").unwrap();
        assert_eq!(
            sddl,
            "O:SYG:SYD:P(D;;GA;;;NU)(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x0012008b;;;S-1-5-21-1-2-3-1001)"
        );
        for broad in ["WD", "AU", "BU", "IU", "AN", "S-1-1-0"] {
            assert!(!sddl.contains(&format!(";;;{broad})")), "{broad}");
        }
        assert_eq!(CLIENT_ACCESS & 0x4, 0, "no FILE_CREATE_PIPE_INSTANCE");
        assert!(pipe_sddl(sid, "BA").unwrap().starts_with("O:BA"));
        assert!(pipe_sddl(sid, "WD").is_none());
        assert!(pipe_sddl("S-1-1-0", "SY").is_none());
        assert!(pipe_sddl("S-1-5-21-1-2-3-4)(A;;GA;;;WD)", "SY").is_none());
    }

    #[test]
    fn trusts_only_system_and_administrators() {
        assert!(trusted_owner("S-1-5-18"));
        assert!(trusted_owner("S-1-5-32-544"));
        assert!(!trusted_owner("S-1-5-21-1-2-3-1001"));
        assert!(!trusted_owner("S-1-5-19"));
    }
}
