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

    fn user_sets_ssh_command(&self) -> bool {
        Command::new("git").current_dir(&self.dir).args(["config", "--get", "core.sshCommand"]).env("LC_ALL", "C").output()
            .is_ok_and(|o| o.status.success() && !o.stdout.is_empty())
    }

    fn scrub(&self, s: &str) -> String {
        let mut out = s.replace(&self.remote, "<remote>");
        // userinfo is everything before the LAST '@' of the authority (passwords may contain '@')
        if let Some((_, rest)) = self.remote.split_once("://") {
            let authority = rest.split('/').next().unwrap_or("");
            if let Some((ui, _)) = authority.rsplit_once('@') {
                out = out.replace(ui, "<redacted>");
            }
        }
        out.trim().to_string()
    }

    // ponytail: no git timeout, so a hung remote blocks `Scheduler::tick` (and the run loop) until git gives up;
    // upgrade path: spawn + wait with a deadline and kill.
    fn git(&self, args: &[&str]) -> Result<String, HostingError> {
        let mut cmd = Command::new("git");
        // HTTPS prompts are off via GIT_TERMINAL_PROMPT; keep ssh from prompting too unless the user chose an ssh command.
        if std::env::var_os("GIT_SSH_COMMAND").is_none() && !self.user_sets_ssh_command() {
            cmd.args(["-c", "core.sshCommand=ssh -o BatchMode=yes"]);
        }
        let out = cmd
            .current_dir(&self.dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never")
            .env("LC_ALL", "C") // stderr is matched below; keep it English
            .args(["-c", "core.autocrlf=false", "-c", "user.name=tally", "-c", "user.email=tally@localhost", "-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { HostingError::GitMissing } else { e.into() })?;
        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
        }
        Err(HostingError::Git { args: args[0].into(), stderr: self.scrub(&String::from_utf8_lossy(&out.stderr)) })
    }

    /// Blocks until no other publish is using this clone; the lock is released when the file drops.
    /// The lock file sits beside the clone, since `sync_down` runs `git clean`.
    pub fn lock(&self) -> Result<std::fs::File, HostingError> {
        std::fs::create_dir_all(&self.dir)?;
        let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(self.dir.with_extension("lock"))?;
        f.lock()?;
        Ok(f)
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

    /// Relative paths of `<dir>/ledger-*.json` files already in the clone.
    pub fn ledger_files(&self, dir: &str) -> Result<Vec<String>, HostingError> {
        let rd = match std::fs::read_dir(self.dir.join(dir)) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut out = Vec::new();
        for e in rd {
            let n = e?.file_name().to_string_lossy().into_owned();
            if n.starts_with("ledger-") && n.ends_with(".json") {
                out.push(format!("{dir}/{n}"));
            }
        }
        Ok(out)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_hides_the_url_and_the_whole_userinfo() {
        let h = Hosting::new(Path::new("h"), "https://user:p@ss@example.invalid/x.git", "b");
        let out = h.scrub("fatal: unable to access 'https://user:p@ss@example.invalid/x.git/' as user:p@ss");
        assert!(!out.contains("p@ss") && !out.contains("ss@") && !out.contains("user"), "{out}");
    }

    #[test]
    fn lock_excludes_a_second_publish_until_dropped() {
        let home = std::env::temp_dir().join(format!("tally-hosting-lock-{}", std::process::id()));
        let h = Hosting::new(&home, "https://example.invalid/x.git", "b");
        let first = h.lock().unwrap();
        let other = std::fs::OpenOptions::new().write(true).open(h.dir.with_extension("lock")).unwrap();
        assert!(matches!(other.try_lock(), Err(std::fs::TryLockError::WouldBlock)));
        drop(first);
        assert!(other.try_lock().is_ok());
        let _ = std::fs::remove_dir_all(&home);
    }
}
