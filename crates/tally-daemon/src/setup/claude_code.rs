//! Claude Code: installs the `tally@tally` plugin with the `claude plugin ...` commands recorded in docs/platform-notes.md.
use super::{Ctx, Detection, Env, Integration, Outcome, SetupError};
use std::path::Path;

const PLUGIN: &str = "tally@tally";
// UNVERIFIED until the sectersion/tally marketplace is published: only a local-directory source was observed.
const MARKETPLACE_SOURCE: &str = "sectersion/tally";

pub struct ClaudeCode;

fn claude(e: &dyn Env, cli: &Path, args: &[&str]) -> Result<String, SetupError> {
    let a: Vec<String> = ["plugin"].iter().chain(args).map(|s| s.to_string()).collect();
    let o = e.run(cli, &a).map_err(|x| SetupError(format!("claude plugin {}: {x}", args.join(" "))))?;
    if o.status.success() {
        Ok(String::from_utf8_lossy(&o.stdout).into_owned())
    } else {
        Err(SetupError(format!("claude plugin {} failed: {}", args.join(" "), String::from_utf8_lossy(&o.stderr).trim())))
    }
}

fn plugin_listed(e: &dyn Env, cli: &Path) -> Result<bool, SetupError> {
    let v: serde_json::Value = serde_json::from_str(&claude(e, cli, &["list", "--json"])?).map_err(|x| SetupError(format!("claude plugin list --json: {x}")))?;
    Ok(v.as_array().is_some_and(|a| a.iter().any(|p| p["id"] == PLUGIN)))
}

impl Integration for ClaudeCode {
    fn name(&self) -> String {
        "Claude Code".into()
    }
    fn detect(&self, e: &dyn Env) -> Detection {
        if let Some(cli) = e.which("claude") {
            Detection::Found { cli }
        } else if e.exists(&e.home().join(".claude")) {
            Detection::ConfigOnly { hint: "the claude command is not on PATH".into() }
        } else {
            Detection::NotFound
        }
    }
    fn install(&self, e: &dyn Env, _ctx: &Ctx) -> Result<Outcome, SetupError> {
        let Detection::Found { cli } = self.detect(e) else { return Err(SetupError("claude not found".into())) };
        if plugin_listed(e, &cli)? {
            return Ok(Outcome::AlreadyInstalled);
        }
        claude(e, &cli, &["marketplace", "add", MARKETPLACE_SOURCE])?;
        claude(e, &cli, &["install", PLUGIN])?;
        Ok(Outcome::Installed)
    }
    fn remove(&self, e: &dyn Env) -> Result<Outcome, SetupError> {
        let Detection::Found { cli } = self.detect(e) else { return Err(SetupError("claude not found".into())) };
        // Both removals exit 1 when there is nothing to remove, so look first.
        let had_plugin = plugin_listed(e, &cli)?;
        if had_plugin {
            claude(e, &cli, &["uninstall", PLUGIN])?;
        }
        let had_market = claude(e, &cli, &["marketplace", "list"])?.split(|c: char| !c.is_alphanumeric() && c != '-').any(|w| w == "tally");
        if had_market {
            claude(e, &cli, &["marketplace", "remove", "tally"])?;
        }
        Ok(if had_plugin || had_market { Outcome::Skipped("removed".into()) } else { Outcome::Skipped("not installed".into()) })
    }
}
