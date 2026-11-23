//! A throwaway local clone of the public data branch, driven by the `git` binary.
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(thiserror::Error, Debug)]
pub enum HostingError {
    #[error("git is not installed or not on PATH")]
    GitMissing,
    /// `args` is the subcommand only; `stderr` has the remote URL (and its credentials) scrubbed.
    #[error("git {args} failed: {stderr}")]
    Git { args: String, stderr: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub struct Hosting {
    pub dir: PathBuf,
    pub remote: String,
    pub branch: String,
}

impl Hosting {
    pub fn new(home: &Path, remote: &str, branch: &str) -> Hosting {
        let h = hex::encode(Sha256::digest(remote.as_bytes()));
        Hosting { dir: home.join("publish").join(&h[..12]), remote: remote.into(), branch: branch.into() }
    }

    fn scrub(&self, s: &str) -> String {
        let mut out = s.replace(&self.remote, "<remote>");
        if let Some(ui) = self.remote.split_once("://").and_then(|(_, r)| r.split_once('@')).map(|(u, _)| u) {
            out = out.replace(ui, "<redacted>");
        }
        out.trim().to_string()
    }

    fn git(&self, args: &[&str]) -> Result<String, HostingError> {
        let out = Command::new("git")
            .current_dir(&self.dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never")
            .args(["-c", "user.name=tally", "-c", "user.email=tally@localhost", "-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { HostingError::GitMissing } else { e.into() })?;
        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
        }
        Err(HostingError::Git { args: args[0].into(), stderr: self.scrub(&String::from_utf8_lossy(&out.stderr)) })
    }

    /// Makes the clone match the remote branch, or an empty unborn branch when the remote has none.
    pub fn sync_down(&self) -> Result<(), HostingError> {
        std::fs::create_dir_all(&self.dir)?;
        if !self.dir.join(".git").exists() {
            self.git(&["init", "-b", &self.branch])?;
        }
        if self.git(&["remote", "set-url", "origin", &self.remote]).is_err() {
            self.git(&["remote", "add", "origin", &self.remote])?;
        }
        let b = &self.branch;
        match self.git(&["fetch", "origin", b]) {
            Ok(_) => {
                let r = format!("origin/{b}");
                self.git(&["checkout", "-f", "-B", b, &r])?;
                self.git(&["reset", "--hard", &r])?;
            }
            Err(HostingError::Git { stderr, .. }) if stderr.contains("couldn't find remote ref") => {
                // Drop any local commits from an earlier failed first push.
                let full = format!("refs/heads/{b}");
                self.git(&["symbolic-ref", "HEAD", &full])?;
                let _ = self.git(&["update-ref", "-d", &full]);
                self.git(&["rm", "-rf", "--ignore-unmatch", "."])?;
            }
            Err(e) => return Err(e),
        }
        self.git(&["clean", "-fd"])?;
        Ok(())
    }

    pub fn read(&self, rel: &str) -> Result<Option<String>, HostingError> {
        match std::fs::read_to_string(self.dir.join(rel)) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// `Ok(false)` when the files changed nothing.
    pub fn commit_and_push(&self, files: &[(String, String)], message: &str) -> Result<bool, HostingError> {
        for (rel, text) in files {
            let p = self.dir.join(rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(p, text)?;
        }
        self.git(&["add", "-A"])?;
        if self.git(&["status", "--porcelain", "-uall"])?.trim().is_empty() {
            return Ok(false);
        }
        self.git(&["commit", "-m", message])?;
        self.git(&["push", "origin", &self.branch])?;
        Ok(true)
    }
}
