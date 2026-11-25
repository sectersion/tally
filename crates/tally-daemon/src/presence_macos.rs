//! macOS idle time: `CGEventSourceSecondsSinceLastEventType` reports elapsed seconds only, never the events.
//! The OS call is compiled on macOS only; `seconds_to_ms` is pure and tested everywhere.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))] // pure helper is tested on every OS, used on macOS
use crate::presence::PlatformError;

const TEN_YEARS_MS: u64 = 315_360_000_000;

pub fn seconds_to_ms(secs: f64) -> Result<u64, PlatformError> {
    if !secs.is_finite() || secs < 0.0 {
        return Err(PlatformError::Failed(format!("CGEventSource returned {secs}")));
    }
    Ok(((secs * 1000.0).round().min(TEN_YEARS_MS as f64)) as u64)
}

#[cfg(target_os = "macos")]
mod os {
    use super::*;
    use crate::presence::Platform;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state_id: i32, event_type: u32) -> f64;
    }
    const COMBINED_SESSION_STATE: i32 = 0;
    const ANY_INPUT_EVENT_TYPE: u32 = !0;

    pub struct MacPlatform;

    impl Platform for MacPlatform {
        fn idle_ms(&mut self) -> Result<u64, PlatformError> {
            // SAFETY: plain C call with two integers and no pointers.
            let secs = unsafe { CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION_STATE, ANY_INPUT_EVENT_TYPE) };
            seconds_to_ms(secs)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn platform_returns_a_number() {
            assert!(MacPlatform.idle_ms().is_ok());
        }
    }
}
#[cfg(target_os = "macos")]
pub use os::MacPlatform;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn seconds_convert_and_reject_nonsense() {
        assert_eq!(seconds_to_ms(1.5).unwrap(), 1500);
        assert_eq!(seconds_to_ms(0.0004).unwrap(), 0);
        assert!(seconds_to_ms(-1.0).is_err());
        assert!(seconds_to_ms(f64::NAN).is_err());
        assert_eq!(seconds_to_ms(1e18).unwrap(), 315_360_000_000);
    }
}
