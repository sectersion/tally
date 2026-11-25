//! Wayland idle time through `ext-idle-notify-v1`.
//!
//! The protocol does not report elapsed idle time: it reports `idled` after a timeout and `resumed` on input.
//! So below `IDLE_TIMEOUT_MS` the true idle time is unknown and reported as 0; resolution below 30 s is not
//! available on this path. `Tracker` only opens spans at `IDLE_SAMPLE_MIN_MS` and dates them from `now - idle_ms`,
//! so the span start stays exact.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))] // pure core is tested on every OS, used on Linux

/// Pure core, tested on every OS.
pub struct IdleTracker {
    timeout_ms: u64,
    idled_at: Option<i64>,
}

impl IdleTracker {
    pub fn new(timeout_ms: u64) -> Self {
        Self { timeout_ms, idled_at: None }
    }
    /// The compositor says: idle for `timeout_ms` as of `now_ms`.
    pub fn on_idled(&mut self, now_ms: i64) {
        self.idled_at = Some(now_ms);
    }
    pub fn on_resumed(&mut self) {
        self.idled_at = None;
    }
    pub fn idle_ms(&self, now_ms: i64) -> u64 {
        self.idled_at.map_or(0, |t| self.timeout_ms + (now_ms - t).max(0) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tracker_reports_zero_until_idled_then_timeout_plus_elapsed() {
        let mut t = IdleTracker::new(30_000);
        assert_eq!(t.idle_ms(1_000), 0);
        t.on_idled(100_000);
        assert_eq!(t.idle_ms(100_000), 30_000);
        assert_eq!(t.idle_ms(160_000), 90_000);
        t.on_resumed();
        assert_eq!(t.idle_ms(170_000), 0);
    }
}

#[cfg(target_os = "linux")]
pub use os::{available, ExtIdlePlatform};

#[cfg(target_os = "linux")]
mod os {
    use super::IdleTracker;
    use crate::presence::{Platform, PlatformError};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;
    use wayland_client::globals::{registry_queue_init, GlobalListContents};
    use wayland_client::protocol::{wl_registry, wl_seat};
    use wayland_client::{Connection, Dispatch, QueueHandle};
    use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notification_v1::{self, ExtIdleNotificationV1};
    use wayland_protocols::ext::idle_notify::v1::client::ext_idle_notifier_v1::{self, ExtIdleNotifierV1};

    const IDLE_TIMEOUT_MS: u32 = 30_000;

    struct Shared {
        tracker: IdleTracker,
        dead: Option<String>,
    }

    struct State {
        shared: Arc<Mutex<Shared>>,
        epoch: Instant,
    }

    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
        fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
    }
    impl Dispatch<wl_seat::WlSeat, ()> for State {
        fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
    }
    impl Dispatch<ExtIdleNotifierV1, ()> for State {
        fn event(_: &mut Self, _: &ExtIdleNotifierV1, _: ext_idle_notifier_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
    }
    impl Dispatch<ExtIdleNotificationV1, ()> for State {
        fn event(st: &mut Self, _: &ExtIdleNotificationV1, ev: ext_idle_notification_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
            let now = st.epoch.elapsed().as_millis() as i64;
            let mut s = st.shared.lock().expect("idle state");
            match ev {
                ext_idle_notification_v1::Event::Idled => s.tracker.on_idled(now),
                ext_idle_notification_v1::Event::Resumed => s.tracker.on_resumed(),
                _ => {}
            }
        }
    }

    /// `WAYLAND_DISPLAY` connects and the compositor advertises `ext_idle_notifier_v1`.
    pub fn available() -> bool {
        let Ok(conn) = Connection::connect_to_env() else { return false };
        let Ok((globals, _q)) = registry_queue_init::<State>(&conn) else { return false };
        globals.contents().with_list(|l| l.iter().any(|g| g.interface == "ext_idle_notifier_v1"))
    }

    pub struct ExtIdlePlatform {
        shared: Arc<Mutex<Shared>>,
        epoch: Instant,
    }

    impl ExtIdlePlatform {
        pub fn new() -> Self {
            let epoch = Instant::now();
            let shared = Arc::new(Mutex::new(Shared { tracker: IdleTracker::new(u64::from(IDLE_TIMEOUT_MS)), dead: None }));
            let s2 = shared.clone();
            std::thread::spawn(move || {
                if let Err(e) = run(s2.clone(), epoch) {
                    s2.lock().expect("idle state").dead = Some(e);
                }
            });
            Self { shared, epoch }
        }
    }

    impl Default for ExtIdlePlatform {
        fn default() -> Self {
            Self::new()
        }
    }

    fn run(shared: Arc<Mutex<Shared>>, epoch: Instant) -> Result<(), String> {
        let conn = Connection::connect_to_env().map_err(|e| format!("wayland connect: {e}"))?;
        let (globals, mut queue) = registry_queue_init::<State>(&conn).map_err(|e| format!("wayland registry: {e}"))?;
        let qh = queue.handle();
        let seat: wl_seat::WlSeat = globals.bind(&qh, 1..=1, ()).map_err(|e| format!("wl_seat: {e}"))?;
        let notifier: ExtIdleNotifierV1 = globals.bind(&qh, 1..=1, ()).map_err(|e| format!("ext_idle_notifier_v1: {e}"))?;
        let _notification = notifier.get_idle_notification(IDLE_TIMEOUT_MS, &seat, &qh, ());
        let mut state = State { shared, epoch };
        loop {
            queue.blocking_dispatch(&mut state).map_err(|e| format!("wayland dispatch: {e}"))?;
        }
    }

    impl Platform for ExtIdlePlatform {
        fn idle_ms(&mut self) -> Result<u64, PlatformError> {
            let s = self.shared.lock().expect("idle state");
            if let Some(e) = &s.dead {
                return Err(PlatformError::Failed(e.clone()));
            }
            Ok(s.tracker.idle_ms(self.epoch.elapsed().as_millis() as i64))
        }
    }
}
