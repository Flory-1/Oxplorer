//! Putting the computer to sleep.

/// Send the computer to sleep (not hibernate). Returns once it wakes up
/// again, so call it from a helper thread. Does nothing outside Windows.
pub fn sleep() -> Result<(), String> {
    imp::sleep()
}

#[cfg(windows)]
mod imp {
    pub(super) fn sleep() -> Result<(), String> {
        use windows::Win32::System::Power::SetSuspendState;
        // SAFETY: plain call without pointers.
        if unsafe { SetSuspendState(false, false, false) } {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error().to_string())
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub(super) fn sleep() -> Result<(), String> {
        Ok(())
    }
}
