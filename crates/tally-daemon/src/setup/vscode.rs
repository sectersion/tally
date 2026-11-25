//! The VS Code family: one table, one struct. Installs our own `.vsix` through the editor's CLI.
use super::{detect_cli, run_cli, Ctx, Detection, Env, Integration, Outcome, SetupError};
use std::path::Path;

/// Must equal `publisher.name` in integrations/vscode/package.json (scripts/check-integrations.sh checks it).
pub const EXTENSION_ID: &str = "sectersion.tally";
const VSIX: &str = "tally-vscode.vsix";

pub struct VsCodeFamily {
    pub name: &'static str,
    pub cli: &'static str,
    pub config_dir: &'static str,
    pub mac_app: &'static str,
    pub win_folder: &'static str,
}

const TABLE: [VsCodeFamily; 6] = [
    VsCodeFamily { name: "VS Code", cli: "code", config_dir: ".vscode", mac_app: "Visual Studio Code", win_folder: "Microsoft VS Code" },
    VsCodeFamily { name: "Cursor", cli: "cursor", config_dir: ".cursor", mac_app: "Cursor", win_folder: "cursor" },
    VsCodeFamily { name: "Windsurf", cli: "windsurf", config_dir: ".windsurf", mac_app: "Windsurf", win_folder: "Windsurf" },
    VsCodeFamily { name: "Antigravity", cli: "antigravity", config_dir: ".antigravity", mac_app: "Antigravity", win_folder: "Antigravity" },
    VsCodeFamily { name: "VSCodium", cli: "codium", config_dir: ".vscode-oss", mac_app: "VSCodium", win_folder: "VSCodium" },
    VsCodeFamily { name: "Trae", cli: "trae", config_dir: ".trae", mac_app: "Trae", win_folder: "Trae" },
];

pub fn family() -> Vec<Box<dyn Integration>> {
    TABLE.into_iter().map(|t| Box::new(t) as Box<dyn Integration>).collect()
}

impl VsCodeFamily {
    fn cli_path(&self, e: &dyn Env) -> Result<std::path::PathBuf, SetupError> {
        match self.detect(e) {
            Detection::Found { cli } => Ok(cli),
            _ => Err(SetupError(format!("{} not found", self.name))),
        }
    }
    fn installed(&self, e: &dyn Env, cli: &Path) -> Result<bool, SetupError> {
        let o = run_cli(e, cli, &["--list-extensions"]).map_err(|x| SetupError(format!("{} --list-extensions: {x}", self.cli)))?;
        Ok(String::from_utf8_lossy(&o.stdout).lines().any(|l| l.trim().eq_ignore_ascii_case(EXTENSION_ID)))
    }
}

impl Integration for VsCodeFamily {
    fn name(&self) -> String {
        self.name.to_string()
    }
    fn detect(&self, e: &dyn Env) -> Detection {
        detect_cli(e, self.cli, self.config_dir, self.mac_app, self.win_folder)
    }
    fn install(&self, e: &dyn Env, ctx: &Ctx) -> Result<Outcome, SetupError> {
        let cli = self.cli_path(e)?;
        let vsix = ctx.extras_dir.join(VSIX);
        if !e.exists(&vsix) {
            return Err(SetupError(format!("{} is missing; pass --extras-dir", vsix.display())));
        }
        let v = vsix.to_string_lossy();
        super::check_cmd_args(e.os(), [cli.to_string_lossy().as_ref(), v.as_ref()]).map_err(|x| SetupError(x.to_string()))?;
        if self.installed(e, &cli)? {
            return Ok(Outcome::AlreadyInstalled);
        }
        let o = run_cli(e, &cli, &["--install-extension", &v, "--force"]).map_err(|x| SetupError(format!("{}: {x}", self.cli)))?;
        if o.status.success() {
            Ok(Outcome::Installed)
        } else {
            Err(SetupError(format!("{} --install-extension failed: {}", self.cli, String::from_utf8_lossy(&o.stderr).trim())))
        }
    }
    fn remove(&self, e: &dyn Env) -> Result<Outcome, SetupError> {
        let cli = self.cli_path(e)?;
        if !self.installed(e, &cli)? {
            return Ok(Outcome::Skipped("not installed".into()));
        }
        let o = run_cli(e, &cli, &["--uninstall-extension", EXTENSION_ID]).map_err(|x| SetupError(format!("{}: {x}", self.cli)))?;
        if o.status.success() {
            Ok(Outcome::Skipped("removed".into()))
        } else {
            Err(SetupError(format!("{} --uninstall-extension failed", self.cli)))
        }
    }
}
