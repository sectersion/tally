//! The optional second tier (`--include-wakatime`): stock WakaTime plugins pointed at the local endpoint.
use super::{run_cli, Ctx, Detection, Env, Integration, Outcome, SetupError};
use std::path::{Path, PathBuf};

pub const JETBRAINS: &str = "JetBrains";
const CLIS: [&str; 10] = ["idea", "pycharm", "webstorm", "goland", "clion", "rubymine", "phpstorm", "rider", "rustrover", "datagrip"];

#[derive(Debug, PartialEq)]
pub enum CfgResult {
    Written,
    AlreadyOurs,
    Conflict { existing_url: String },
    BackedUpAndWritten,
}

fn setting(line: &str) -> Option<(&str, &str)> {
    let (k, v) = line.split_once('=')?;
    Some((k.trim(), v.trim()))
}

/// Line-based edit of `[settings]`: every other line and section is kept. An existing file that is not already ours
/// is never touched without `force`, and then the old file is kept as `<path>.tally-bak`. A symlink is never replaced. An unreadable or unwritable
/// file is reported as a `Conflict` whose `existing_url` carries the error; nothing is changed.
pub fn write_cfg(path: &Path, api_url: &str, api_key: &str, force: bool) -> CfgResult {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return CfgResult::Conflict { existing_url: "is a symlink; not replaced, edit the link target by hand".into() };
    }
    let existed = path.exists();
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return CfgResult::Conflict { existing_url: format!("unreadable: {e}") },
    };
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let (mut in_settings, mut url, mut key) = (false, None, None);
    let mut kept: Vec<&str> = Vec::new();
    let mut head = None;
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            in_settings = t == "[settings]";
            if in_settings && head.is_none() {
                head = Some(kept.len());
            }
        } else if in_settings {
            match setting(l) {
                Some(("api_url", v)) => {
                    url.get_or_insert(v);
                    continue;
                }
                Some(("api_key", v)) => {
                    key.get_or_insert(v);
                    continue;
                }
                _ => {}
            }
        }
        kept.push(l);
    }
    if url == Some(api_url) && key == Some(api_key) {
        return CfgResult::AlreadyOurs;
    }
    // A file Tally did not create is left alone unless `force`, whether or not it has api_url/api_key.
    if existed && !force {
        return CfgResult::Conflict { existing_url: url.map_or_else(|| "an existing file without api_url; use --force to replace it (a backup is kept)".to_string(), str::to_string) };
    }
    let ours = [format!("api_url = {api_url}"), format!("api_key = {api_key}")];
    match head {
        Some(h) => kept.splice(h + 1..h + 1, ours.iter().map(String::as_str)).for_each(drop),
        None => {
            kept.push("[settings]");
            kept.extend(ours.iter().map(String::as_str));
        }
    }
    let mut bak = path.as_os_str().to_owned();
    bak.push(".tally-bak");
    if existed && std::fs::copy(path, &bak).is_err() {
        return CfgResult::Conflict { existing_url: "could not write the backup".into() };
    }
    // Write beside the target and rename over it, so a failed write cannot lose the original.
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tally-tmp");
    if let Err(e) = std::fs::write(&tmp, kept.join(nl) + nl).and_then(|()| std::fs::rename(&tmp, path)) {
        let _ = std::fs::remove_file(&tmp);
        return CfgResult::Conflict { existing_url: format!("unwritable: {e}") };
    }
    if existed { CfgResult::BackedUpAndWritten } else { CfgResult::Written }
}

pub struct JetBrains;

impl JetBrains {
    fn clis(e: &dyn Env) -> Vec<PathBuf> {
        CLIS.iter().filter_map(|c| e.which(c)).collect()
    }
}

impl Integration for JetBrains {
    fn name(&self) -> String {
        JETBRAINS.into()
    }
    fn detect(&self, e: &dyn Env) -> Detection {
        Self::clis(e).into_iter().next().map_or(Detection::NotFound, |cli| Detection::Found { cli })
    }
    fn install(&self, e: &dyn Env, _ctx: &Ctx) -> Result<Outcome, SetupError> {
        // ponytail: every IDE launcher on PATH gets the plugin; "already installed" is not detectable, installPlugins is idempotent.
        for cli in Self::clis(e) {
            let o = run_cli(e, &cli, &["installPlugins", "com.wakatime.intellij.plugin"]).map_err(|x| SetupError(format!("{}: {x}", cli.display())))?;
            if !o.status.success() {
                return Err(SetupError(format!("{} installPlugins failed", cli.display())));
            }
        }
        Ok(Outcome::Installed)
    }
    fn remove(&self, _e: &dyn Env) -> Result<Outcome, SetupError> {
        Ok(Outcome::Skipped("remove the WakaTime plugin in the IDE; ~/.wakatime.cfg is left alone".into()))
    }
}
