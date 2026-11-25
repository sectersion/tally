//! Background service definitions: rendering and command planning are pure; `execute` (below) runs them.
//!
//! A per-user session task, not a Windows Service: idle input is per interactive session, and a service in
//! session 0 cannot read it. Windows: Task Scheduler logon task running `tally start` (hidden, detached, idempotent;
//! a console-subsystem exe started by the scheduler would otherwise open a window). macOS: launchd agent.
//! Linux: systemd user unit. Uninstall never touches `home` (the store and ledger).
use std::path::{Path, PathBuf};

pub const LABEL: &str = "dev.tally.agent";
/// Not plain "Tally": a hand-made task of that name may exist, and `/Create /F` would replace it.
pub const TASK_NAME: &str = "dev.tally.agent";

#[derive(Clone, Debug, PartialEq)]
pub struct ServiceSpec {
    pub exe: PathBuf,
    pub home: PathBuf,
    pub label: &'static str,
    /// Where the unit/plist/task XML is written (`~/.config/systemd/user`, `~/Library/LaunchAgents`, a temp dir).
    pub unit_dir: PathBuf,
    /// For `launchctl gui/<uid>`.
    pub uid: u32,
    /// `DOMAIN\name` for the Task Scheduler trigger and principal: without a `UserId` the logon trigger is for
    /// any user and `/Create` needs admin (observed, Windows 11).
    pub user: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Install,
    Start,
    Stop,
    /// Is it registered? (running is read from the daemon's lock by the caller).
    Status,
    Uninstall,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cmd {
    pub program: String,
    pub args: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    WriteFile { path: PathBuf, contents: String },
    Run(Cmd),
    /// Failure is fine (not installed, not loaded): keeps install and uninstall idempotent.
    TryRun(Cmd),
    RemoveFile(PathBuf),
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// systemd quoting: a word with a space, quote or backslash goes in double quotes.
fn sd(s: &str) -> String {
    if s.chars().any(|c| c.is_whitespace() || c == '"' || c == '\\' || c == '\'') {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn p(x: &Path) -> String {
    x.display().to_string()
}

pub fn render_systemd_unit(s: &ServiceSpec) -> String {
    format!(
        "[Unit]\nDescription=Tally time tracker\nAfter=graphical-session.target\nPartOf=graphical-session.target\n\n\
         [Service]\nExecStart={} run --home {}\nRestart=on-failure\nRestartSec=10\nRestartPreventExitStatus=75\nEnvironment=TALLY_HOME={}\n\n\
         [Install]\nWantedBy=graphical-session.target\n",
        sd(&p(&s.exe)),
        sd(&p(&s.home)),
        sd(&p(&s.home)),
    )
}

pub fn render_launchd_plist(s: &ServiceSpec) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\
         \t<key>Label</key>\n\t<string>{}</string>\n\
         \t<key>ProgramArguments</key>\n\t<array>\n\t\t<string>{}</string>\n\t\t<string>run</string>\n\t\t<string>--home</string>\n\t\t<string>{}</string>\n\t</array>\n\
         \t<key>RunAtLoad</key>\n\t<true/>\n\
         \t<key>KeepAlive</key>\n\t<dict>\n\t\t<key>SuccessfulExit</key>\n\t\t<false/>\n\t</dict>\n\
         \t<key>ThrottleInterval</key>\n\t<integer>60</integer>\n\
         \t<key>ProcessType</key>\n\t<string>Background</string>\n\
         \t<key>StandardErrorPath</key>\n\t<string>{}</string>\n\
         </dict>\n</plist>\n",
        xml(s.label),
        xml(&p(&s.exe)),
        xml(&p(&s.home)),
        xml(&format!("{}/tally.log", p(&s.home))), // not join(): on a Windows host that puts a backslash into the golden
    )
}

/// Task Scheduler 1.2 XML (the executor writes it as UTF-16 with a BOM, which `schtasks /XML` requires).
pub fn render_task_xml(s: &ServiceSpec) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n\
         <Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\n\
         \x20 <RegistrationInfo>\n    <Description>Tally time tracker</Description>\n  </RegistrationInfo>\n\
         \x20 <Triggers>\n    <LogonTrigger>\n      <Enabled>true</Enabled>\n      <UserId>{u}</UserId>\n    </LogonTrigger>\n  </Triggers>\n\
         \x20 <Principals>\n    <Principal id=\"Author\">\n      <UserId>{u}</UserId>\n      <LogonType>InteractiveToken</LogonType>\n      <RunLevel>LeastPrivilege</RunLevel>\n    </Principal>\n  </Principals>\n\
         \x20 <Settings>\n\
         \x20   <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>\n\
         \x20   <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>\n\
         \x20   <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>\n\
         \x20   <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>\n\
         \x20   <Hidden>true</Hidden>\n\
         \x20   <RestartOnFailure>\n      <Interval>PT1M</Interval>\n      <Count>3</Count>\n    </RestartOnFailure>\n\
         \x20   <Enabled>true</Enabled>\n\
         \x20 </Settings>\n\
         \x20 <Actions Context=\"Author\">\n    <Exec>\n      <Command>{}</Command>\n      <Arguments>ensure --home \"{}\"</Arguments>\n    </Exec>\n  </Actions>\n\
         </Task>\n",
        xml(&p(&s.exe)),
        xml(&p(&s.home)),
        u = xml(&s.user),
    )
}

/// `<exe> <sub> --home <home>`.
fn tally(s: &ServiceSpec, sub: &str) -> Cmd {
    Cmd { program: p(&s.exe), args: vec![sub.into(), "--home".into(), p(&s.home)] }
}

fn cmd(program: &str, args: &[&str]) -> Cmd {
    Cmd { program: program.into(), args: args.iter().map(|a| a.to_string()).collect() }
}

/// Pure: what to do for `action` on `os` (`"windows"`, `"macos"`, `"linux"`). Same input, same steps (idempotent).
pub fn plan(os: &str, action: Action, s: &ServiceSpec) -> Vec<Step> {
    use Action::*;
    match os {
        "linux" => {
            let unit = s.unit_dir.join("tally.service");
            let sc = |a: &[&str]| cmd("systemctl", &[&["--user"][..], a].concat());
            match action {
                Install => vec![
                    Step::WriteFile { path: unit, contents: render_systemd_unit(s) },
                    Step::Run(sc(&["daemon-reload"])),
                    Step::Run(sc(&["enable", "--now", "tally.service"])),
                ],
                // `tally start`/`tally stop` clear/write the `paused` marker; without it the next Claude Code prompt
                // restarts the daemon (ensure), so a stopped service would not stay stopped.
                Start => vec![Step::Run(tally(s, "start"))],
                Stop => vec![Step::TryRun(tally(s, "stop")), Step::Run(sc(&["stop", "tally.service"]))],
                Status => vec![Step::Run(sc(&["is-enabled", "tally.service"]))],
                Uninstall => vec![Step::TryRun(sc(&["disable", "--now", "tally.service"])), Step::RemoveFile(unit), Step::TryRun(sc(&["daemon-reload"]))],
            }
        }
        "macos" => {
            let plist = s.unit_dir.join(format!("{}.plist", s.label));
            let domain = format!("gui/{}", s.uid);
            let target = format!("{domain}/{}", s.label);
            let lc = |a: &[&str]| cmd("launchctl", a);
            let pl = p(&plist);
            match action {
                Install => vec![
                    Step::WriteFile { path: plist, contents: render_launchd_plist(s) },
                    Step::TryRun(lc(&["bootout", &target])),
                    Step::Run(lc(&["bootstrap", &domain, &pl])),
                    Step::TryRun(lc(&["kickstart", &target])),
                ],
                Start => vec![Step::Run(tally(s, "start"))],
                Stop => vec![Step::TryRun(tally(s, "stop")), Step::Run(lc(&["bootout", &target]))],
                Status => vec![Step::Run(lc(&["print", &target]))],
                Uninstall => vec![Step::TryRun(lc(&["bootout", &target])), Step::RemoveFile(plist)],
            }
        }
        _ => {
            let xmlp = s.unit_dir.join("tally-task.xml");
            let xp = p(&xmlp);
            let st = |a: &[&str]| cmd("schtasks", a);
            match action {
                Install => vec![
                    Step::WriteFile { path: xmlp, contents: render_task_xml(s) },
                    Step::Run(st(&["/Create", "/TN", TASK_NAME, "/XML", &xp, "/F"])),
                    Step::Run(st(&["/Run", "/TN", TASK_NAME])),
                ],
                Start => vec![Step::Run(tally(s, "start"))],
                // the task only launches the detached daemon, so stopping means `tally stop`
                Stop => vec![Step::TryRun(st(&["/End", "/TN", TASK_NAME])), Step::Run(tally(s, "stop"))],
                Status => vec![Step::Run(st(&["/Query", "/TN", TASK_NAME]))],
                Uninstall => vec![Step::TryRun(st(&["/End", "/TN", TASK_NAME])), Step::TryRun(st(&["/Delete", "/TN", TASK_NAME, "/F"])), Step::RemoveFile(xmlp)],
            }
        }
    }
}
