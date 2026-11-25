//! X11 idle time from the X server's own XScreenSaver counter (milliseconds since the last input, no contents).
//! Never used in a Wayland session: under XWayland the counter only sees X clients (see `presence_select::choose`).
use crate::presence::{Platform, PlatformError};
use x11rb::connection::Connection;
use x11rb::protocol::screensaver;
use x11rb::rust_connection::RustConnection;

pub struct X11Platform {
    conn: Option<(RustConnection, u32)>,
}

fn connect() -> Result<(RustConnection, u32), PlatformError> {
    let fail = |e: &dyn std::fmt::Display| PlatformError::Failed(format!("X11: {e}"));
    let (conn, screen) = RustConnection::connect(None).map_err(|e| fail(&e))?;
    let root = conn.setup().roots[screen].root;
    screensaver::query_version(&conn, 1, 1).map_err(|e| fail(&e))?.reply().map_err(|e| fail(&e))?;
    Ok((conn, root))
}

impl X11Platform {
    /// Lazy: a failed connection is retried on the next sample.
    pub fn new() -> Self {
        Self { conn: connect().ok() }
    }
}

impl Default for X11Platform {
    fn default() -> Self {
        Self::new()
    }
}

impl Platform for X11Platform {
    fn idle_ms(&mut self) -> Result<u64, PlatformError> {
        if self.conn.is_none() {
            self.conn = Some(connect()?);
        }
        let (conn, root) = self.conn.as_ref().expect("just connected");
        let r = screensaver::query_info(conn, *root).map_err(|e| e.to_string()).and_then(|c| c.reply().map_err(|e| e.to_string()));
        match r {
            Ok(info) => Ok(u64::from(info.ms_since_user_input)),
            Err(e) => {
                self.conn = None; // reconnect on the next call
                Err(PlatformError::Failed(format!("X11: {e}")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn x11_idle_grows_while_untouched() {
        if std::env::var("DISPLAY").is_err() {
            return;
        }
        let mut p = X11Platform::new();
        assert!(p.idle_ms().is_ok());
        std::thread::sleep(std::time::Duration::from_millis(1200));
        assert!(p.idle_ms().unwrap() >= 1000);
    }
}
