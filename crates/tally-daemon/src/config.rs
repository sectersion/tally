//! `<home>/config.toml` loader. Missing file means defaults; unknown keys are errors.

use std::path::{Path, PathBuf};

#[derive(serde::Deserialize, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct Config {
    pub run: RunConfig,
    pub claude_code: ClaudeCodeConfig,
    pub presence: PresenceConfig,
    pub publish: crate::publish::PublishConfig,
    pub sync: SyncConfig,
}

/// No token key by design: tokens live in `device_tokens`, never in the config.
#[derive(serde::Deserialize, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct SyncConfig {
    pub enabled: bool,
    pub server: Option<String>,
}

/// `require: None` means "same as `enabled`".
#[derive(serde::Deserialize, Debug, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct PresenceConfig {
    pub enabled: bool,
    pub require: Option<bool>,
}

#[allow(clippy::derivable_impls)] // default is true on windows, so not derivable there
impl Default for PresenceConfig {
    fn default() -> Self {
        Self { enabled: cfg!(windows), require: None }
    }
}

impl Config {
    /// Rejects settings that contradict each other.
    pub fn validate(&self) -> Result<(), String> {
        if !self.presence.enabled && self.presence.require == Some(true) {
            return Err("presence.require is true but presence.enabled is false; enable presence or set require = false".into());
        }
        Ok(())
    }
}

impl PresenceConfig {
    pub fn required(&self) -> bool {
        self.require.unwrap_or(self.enabled)
    }
}

#[derive(serde::Deserialize, Debug, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct RunConfig {
    pub interval_secs: u64,
    pub dashboard: bool,
    pub dashboard_port: u16,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            interval_secs: 15,
            dashboard: false,
            dashboard_port: 7878,
        }
    }
}

/// `projects_dir: None` means `dirs::home_dir()/.claude/projects`, resolved by the source.
#[derive(serde::Deserialize, Debug, PartialEq)]
#[serde(deny_unknown_fields, default)]
pub struct ClaudeCodeConfig {
    pub enabled: bool,
    pub projects_dir: Option<PathBuf>,
}

impl Default for ClaudeCodeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            projects_dir: None,
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ConfigError {
    #[error("reading {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Parse {
        path: String,
        source: toml::de::Error,
    },
    #[error("{path}: {msg}")]
    Invalid { path: String, msg: String },
}

pub fn load(home: &Path) -> Result<Config, ConfigError> {
    let p = home.join("config.toml");
    let path = p.display().to_string();
    let text = match std::fs::read_to_string(&p) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(source) => return Err(ConfigError::Io { path, source }),
    };
    let mut cfg: Config = toml::from_str(&text).map_err(|source| ConfigError::Parse { path: path.clone(), source })?;
    if let Some(s) = cfg.sync.server.as_mut() {
        // the host must end after the loopback name, so http://localhost.evil.example is refused
        let local = ["http://127.0.0.1", "http://localhost"]
            .iter()
            .any(|p| s.strip_prefix(p).is_some_and(|r| r.is_empty() || r.starts_with([':', '/'])));
        let ok = s.starts_with("https://") || local;
        if !ok {
            return Err(ConfigError::Invalid { path, msg: "sync.server must start with https:// (http:// only for 127.0.0.1 or localhost)".into() });
        }
        *s = s.trim_end_matches('/').to_string();
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(d: &tempfile::TempDir, s: &str) {
        std::fs::write(d.path().join("config.toml"), s).unwrap();
    }

    #[test]
    fn missing_file_is_defaults() {
        let d = tempfile::tempdir().unwrap();
        let c = load(d.path()).unwrap();
        assert_eq!(
            (
                c.run.interval_secs,
                c.claude_code.enabled,
                c.claude_code.projects_dir
            ),
            (15, true, None)
        );
        assert_eq!((c.run.dashboard, c.run.dashboard_port), (false, 7878));
    }

    #[test]
    fn require_without_enabled_is_rejected() {
        let d = tempfile::tempdir().unwrap();
        write(&d, "[presence]\nenabled = false\nrequire = true\n");
        let e = load(d.path()).unwrap().validate().unwrap_err();
        assert!(e.contains("presence.require") && e.contains("presence.enabled"), "{e}");
        write(&d, "[presence]\nenabled = false\n");
        assert!(load(d.path()).unwrap().validate().is_ok());
    }
    #[test]
    fn presence_require_defaults_to_enabled() {
        let d = tempfile::tempdir().unwrap();
        write(&d, "[presence]\nenabled = true\n");
        assert!(load(d.path()).unwrap().presence.required());
        write(&d, "[presence]\nenabled = true\nrequire = false\n");
        assert!(!load(d.path()).unwrap().presence.required());
        write(&d, "[presence]\nenabled = false\n");
        assert!(!load(d.path()).unwrap().presence.required());
    }

    #[test]
    fn dashboard_settings_parse() {
        let d = tempfile::tempdir().unwrap();
        write(&d, "[run]\ndashboard = true\ndashboard_port = 8123\n");
        let c = load(d.path()).unwrap();
        assert_eq!(
            (c.run.dashboard, c.run.dashboard_port, c.run.interval_secs),
            (true, 8123, 15)
        );
    }

    #[test]
    fn sync_section_parses_and_rejects_plain_http_and_unknown_keys() {
        let d = tempfile::tempdir().unwrap();
        write(&d, "[sync]\nenabled = true\nserver = \"https://t.example/\"\n");
        let c = load(d.path()).unwrap();
        assert_eq!((c.sync.enabled, c.sync.server.as_deref()), (true, Some("https://t.example")));
        write(&d, "[sync]\nserver = \"http://t.example\"\n");
        assert!(load(d.path()).unwrap_err().to_string().contains("sync.server"));
        write(&d, "[sync]\nserver = \"http://localhost.evil.example\"\n");
        assert!(load(d.path()).is_err());
        write(&d, "[sync]\nserver = \"http://127.0.0.1:8787\"\n");
        assert!(load(d.path()).is_ok());
        write(&d, "[sync]\ntoken = \"x\"\n");
        assert!(load(d.path()).unwrap_err().to_string().contains("unknown"));
        assert_eq!(Config::default().sync, SyncConfig { enabled: false, server: None });
    }

    #[test]
    fn partial_tables_keep_other_defaults() {
        let d = tempfile::tempdir().unwrap();
        write(&d, "[claude_code]\nenabled = false\n");
        let c = load(d.path()).unwrap();
        assert_eq!((c.run.interval_secs, c.claude_code.enabled), (15, false));
    }

    #[test]
    fn unknown_keys_and_tables_are_errors() {
        let d = tempfile::tempdir().unwrap();
        for bad in ["[run]\ninterval = 5\n", "[nope]\n", "top = 1\n"] {
            write(&d, bad);
            let e = load(d.path()).unwrap_err().to_string();
            assert!(
                e.contains("config.toml") && e.contains("unknown"),
                "{bad}: {e}"
            );
        }
    }
}
