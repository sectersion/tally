//! Windows idle sampling.
//!
//! `GetLastInputInfo` sees only the current session, so the daemon must run in the user's
//! session (logon task or startup entry), not as a service.
use crate::presence::{Platform, PlatformError};
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};

pub struct WindowsPlatform;

impl Platform for WindowsPlatform {
    fn idle_ms(&mut self) -> Result<u64, PlatformError> {
        let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        // SAFETY: `info` is a valid, correctly sized LASTINPUTINFO.
        if unsafe { GetLastInputInfo(&mut info) } == 0 {
            return Err(PlatformError::Failed("GetLastInputInfo failed".into()));
        }
        // dwTime is a 32-bit tick count that wraps every 49 days.
        let now = unsafe { GetTickCount64() } as u32;
        Ok(u64::from(now.wrapping_sub(info.dwTime)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_reports_a_small_idle_value() {
        let idle = WindowsPlatform.idle_ms().unwrap();
        assert!(idle < 49 * 86_400_000, "{idle}");
    }
}
