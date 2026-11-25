//! GNOME Wayland idle time: `org.gnome.Mutter.IdleMonitor.GetIdletime` on the session bus (milliseconds, no contents).
use crate::presence::{Platform, PlatformError};
use zbus::blocking::Connection;

const NAME: &str = "org.gnome.Mutter.IdleMonitor";
const PATH: &str = "/org/gnome/Mutter/IdleMonitor/Core";

/// The session bus has the Mutter idle-monitor name.
pub fn available() -> bool {
    let Ok(conn) = Connection::session() else { return false };
    let Ok(bus) = zbus::blocking::fdo::DBusProxy::new(&conn) else { return false };
    let Ok(name) = zbus::names::BusName::try_from(NAME) else { return false };
    bus.name_has_owner(name).unwrap_or(false)
}

#[derive(Default)]
pub struct MutterPlatform {
    conn: Option<Connection>,
}

impl Platform for MutterPlatform {
    fn idle_ms(&mut self) -> Result<u64, PlatformError> {
        let fail = |e: &dyn std::fmt::Display| PlatformError::Failed(format!("Mutter IdleMonitor: {e}"));
        if self.conn.is_none() {
            self.conn = Some(Connection::session().map_err(|e| fail(&e))?);
        }
        let r = self.conn.as_ref().expect("just connected").call_method(Some(NAME), PATH, Some(NAME), "GetIdletime", &());
        match r.map_err(|e| e.to_string()).and_then(|m| m.body().deserialize::<u64>().map_err(|e| e.to_string())) {
            Ok(ms) => Ok(ms),
            Err(e) => {
                self.conn = None;
                Err(fail(&e))
            }
        }
    }
}
