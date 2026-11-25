//! Picks the idle platform for this desktop. `choose` is pure; `current_choice` asks the OS.
use crate::config::PresenceConfig;
use crate::presence::{Platform, UnsupportedPlatform};

pub struct DesktopEnv {
    pub os: &'static str,
    pub session_type: Option<String>,
    pub wayland_display: Option<String>,
    pub x_display: Option<String>,
    pub desktop: Option<String>,
}

impl DesktopEnv {
    pub fn from_process() -> Self {
        let v = |k: &str| std::env::var(k).ok().filter(|s| !s.is_empty());
        Self {
            os: std::env::consts::OS,
            session_type: v("XDG_SESSION_TYPE"),
            wayland_display: v("WAYLAND_DISPLAY"),
            x_display: v("DISPLAY"),
            desktop: v("XDG_CURRENT_DESKTOP"),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Choice {
    Windows,
    MacOs,
    X11,
    MutterWayland,
    ExtIdleNotify,
    Unsupported(String),
}

impl Choice {
    pub fn name(&self) -> &'static str {
        match self {
            Choice::Windows => "windows-lastinput",
            Choice::MacOs => "macos-cgevent",
            Choice::X11 => "x11-screensaver",
            Choice::MutterWayland => "mutter-idlemonitor",
            Choice::ExtIdleNotify => "ext-idle-notify",
            Choice::Unsupported(_) => "unsupported",
        }
    }
}

/// Wayland wins over X11 even when `DISPLAY` is set: XWayland's idle counter only sees X clients.
pub fn choose(env: &DesktopEnv, mutter_present: bool, ext_idle_present: bool) -> Choice {
    match env.os {
        "windows" => Choice::Windows,
        "macos" => Choice::MacOs,
        "linux" => {
            if env.session_type.as_deref() == Some("wayland") || env.wayland_display.is_some() {
                if mutter_present {
                    Choice::MutterWayland
                } else if ext_idle_present {
                    Choice::ExtIdleNotify
                } else {
                    let d = env.desktop.as_deref().unwrap_or("unknown desktop");
                    Choice::Unsupported(format!("Wayland compositor {d} offers neither org.gnome.Mutter.IdleMonitor nor ext-idle-notify-v1"))
                }
            } else if env.x_display.is_some() {
                Choice::X11
            } else {
                Choice::Unsupported("no graphical session found (no WAYLAND_DISPLAY or DISPLAY)".into())
            }
        }
        os => Choice::Unsupported(format!("no idle source for {os}")),
    }
}

pub fn current_choice() -> Choice {
    let env = DesktopEnv::from_process();
    choose(&env, mutter_available(), ext_idle_available())
}

#[cfg(target_os = "linux")]
fn mutter_available() -> bool {
    crate::presence_mutter::available()
}
#[cfg(not(target_os = "linux"))]
fn mutter_available() -> bool {
    false
}
#[cfg(target_os = "linux")]
fn ext_idle_available() -> bool {
    crate::presence_wayland::available()
}
#[cfg(not(target_os = "linux"))]
fn ext_idle_available() -> bool {
    false
}

pub fn platform_for(c: &Choice) -> Box<dyn Platform> {
    match c {
        #[cfg(windows)]
        Choice::Windows => Box::new(crate::presence_win::WindowsPlatform),
        #[cfg(target_os = "macos")]
        Choice::MacOs => Box::new(crate::presence_macos::MacPlatform),
        #[cfg(target_os = "linux")]
        Choice::X11 => Box::new(crate::presence_x11::X11Platform::new()),
        #[cfg(target_os = "linux")]
        Choice::MutterWayland => Box::<crate::presence_mutter::MutterPlatform>::default(),
        #[cfg(target_os = "linux")]
        Choice::ExtIdleNotify => Box::new(crate::presence_wayland::ExtIdlePlatform::new()),
        _ => Box::new(UnsupportedPlatform),
    }
}

/// The `presence:` line of `tally status`.
pub fn presence_line(c: &Choice, cfg: &PresenceConfig) -> String {
    if !cfg.enabled {
        return "presence: disabled ([presence] require = false; hours are not checked against idle time)".into();
    }
    match c {
        Choice::Unsupported(r) => format!("presence: unsupported: {r} (claims are not finalized)"),
        c => format!("presence: {} (ok)", c.name()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presence::PlatformError;

    fn env(os: &'static str, session: Option<&str>, wl: bool, x: bool) -> DesktopEnv {
        DesktopEnv {
            os,
            session_type: session.map(String::from),
            wayland_display: wl.then(|| "wayland-0".into()),
            x_display: x.then(|| ":0".into()),
            desktop: Some("sway".into()),
        }
    }

    #[test]
    fn windows_and_macos_choose_their_native_probe() {
        assert_eq!(choose(&env("windows", None, false, false), false, false), Choice::Windows);
        assert_eq!(choose(&env("macos", None, false, false), false, false), Choice::MacOs);
    }
    #[test]
    fn x11_session_chooses_x11() {
        assert_eq!(choose(&env("linux", Some("x11"), false, true), false, false), Choice::X11);
    }
    #[test]
    fn wayland_with_xwayland_never_uses_the_x11_probe() {
        assert_eq!(choose(&env("linux", Some("wayland"), true, true), false, true), Choice::ExtIdleNotify);
        assert_eq!(choose(&env("linux", Some("wayland"), true, true), true, true), Choice::MutterWayland);
    }
    #[test]
    fn wayland_without_idle_support_is_unsupported_with_a_reason() {
        match choose(&env("linux", Some("wayland"), true, true), false, false) {
            Choice::Unsupported(r) => assert!(r.contains("ext-idle-notify") && r.contains("sway")),
            c => panic!("{c:?}"),
        }
    }
    #[test]
    fn headless_linux_is_unsupported() {
        assert!(matches!(choose(&env("linux", None, false, false), false, false), Choice::Unsupported(_)));
    }
    #[test]
    fn unsupported_choice_gives_the_unsupported_platform() {
        let mut p = platform_for(&Choice::Unsupported("x".into()));
        assert!(matches!(p.idle_ms(), Err(PlatformError::Unsupported)));
    }
    #[test]
    fn presence_is_enabled_and_required_by_default_on_the_three_desktop_oses() {
        let c = PresenceConfig::default();
        let want = cfg!(any(windows, target_os = "macos", target_os = "linux"));
        assert_eq!((c.enabled, c.required()), (want, want));
    }
    #[test]
    fn status_line_covers_ok_unsupported_and_disabled() {
        let on = PresenceConfig { enabled: true, require: None };
        let off = PresenceConfig { enabled: false, require: Some(false) };
        assert_eq!(presence_line(&Choice::X11, &on), "presence: x11-screensaver (ok)");
        assert!(presence_line(&Choice::Unsupported("why".into()), &on).contains("unsupported: why (claims are not finalized)"));
        assert!(presence_line(&Choice::X11, &off).starts_with("presence: disabled"));
    }
}
