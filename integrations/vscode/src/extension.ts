// Thin wiring; the only file that imports vscode. No runtime dependencies, no downloads.
import * as vscode from "vscode";
import * as http from "http";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import { execFile } from "child_process";
import { heartbeat, isLoopbackUrl, isTrackable } from "./payload";
import { Throttle } from "./throttle";

const WINDOW_MS = 120000;
const EXE = process.platform === "win32" ? "tally.exe" : "tally";

function tallyHome(): string {
  if (process.env.TALLY_HOME) return process.env.TALLY_HOME;
  if (process.platform === "win32") return path.join(process.env.LOCALAPPDATA || path.join(os.homedir(), "AppData", "Local"), "tally");
  if (process.platform === "darwin") return path.join(os.homedir(), "Library", "Application Support", "tally");
  return path.join(process.env.XDG_DATA_HOME || path.join(os.homedir(), ".local", "share"), "tally");
}

const run = (bin: string, args: string[], env?: NodeJS.ProcessEnv, timeout = 5000): Promise<string> =>
  new Promise((res, rej) =>
    execFile(bin, args, { timeout, windowsHide: true, env: { ...process.env, ...env } }, (e, out) => (e ? rej(e) : res(out))),
  );

// Same lookup as the Claude Code hook: <home>/bin first, then PATH, version-gated with `tally --version`.
async function findTally(version: string): Promise<string | undefined> {
  const candidates = [path.join(tallyHome(), "bin", EXE), "tally"];
  for (const c of candidates) {
    try {
      const v = /(\d+(?:\.\d+)*)/.exec(await run(c, ["--version"]))?.[1];
      if (v && v.localeCompare(version, undefined, { numeric: true }) >= 0) return c;
    } catch {}
  }
  return undefined;
}

async function ensure(bin: string): Promise<void> {
  try {
    const out = (await run(bin, ["ensure", "--client", "vscode", "--hook-json"])).trim();
    const msg = out ? JSON.parse(out).systemMessage : undefined;
    if (typeof msg === "string") void vscode.window.showInformationMessage(msg);
  } catch {}
}

async function offerInstall(ctx: vscode.ExtensionContext, version: string): Promise<void> {
  const pick = await vscode.window.showInformationMessage("Tally is not installed, so VS Code time is not being tracked.", "Install Tally");
  if (!pick) return;
  const win = process.platform === "win32";
  const inst = path.join(ctx.extensionPath, "install", win ? "install.ps1" : "install.sh");
  if (!fs.existsSync(inst)) {
    void vscode.window.showInformationMessage("This build has no bundled installer. See docs/extensions.md in the Tally repository for how to install.");
    return;
  }
  const env = { TALLY_VERSION: version, TALLY_INSTALL_DIR: path.join(tallyHome(), "bin") };
  try {
    await run(win ? "powershell" : "bash", win ? ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", inst, "--binary-only"] : [inst, "--binary-only"], env, 120000);
    const bin = await findTally(version);
    if (bin) await ensure(bin);
  } catch {
    void vscode.window.showWarningMessage("Tally install failed. See docs/extensions.md in the Tally repository.");
  }
}

function readConfig(out: string): { url: string; key: string } | undefined {
  const g = (k: string) => new RegExp(`^${k} = (.+)$`, "m").exec(out)?.[1].trim();
  const url = g("api_url"), key = g("api_key");
  if (url && key && !isLoopbackUrl(url)) console.log("tally: heartbeat api_url is not a loopback http url; not sending");
  return url && key && isLoopbackUrl(url) ? { url, key } : undefined;
}

// Dropped silently on any failure: the daemon discards stale heartbeats, so a queue would not help.
function post(cfg: { url: string; key: string }, body: object): void {
  try {
  const req = http.request(
    `${cfg.url}/users/current/heartbeats`,
    {
      method: "POST",
      timeout: 5000,
      headers: { "Content-Type": "application/json", Authorization: "Basic " + Buffer.from(cfg.key).toString("base64") },
    },
    (r) => r.resume(),
  );
  req.on("error", () => {});
  req.on("timeout", () => req.destroy());
  req.end(JSON.stringify(body));
  } catch {}
}

export async function activate(ctx: vscode.ExtensionContext): Promise<void> {
  const version: string = ctx.extension.packageJSON.version;
  const bin = await findTally(version);
  if (!bin) return void offerInstall(ctx, version);
  await ensure(bin);
  let cfg: { url: string; key: string } | undefined;
  try { cfg = readConfig(await run(bin, ["heartbeat-config"])); } catch {}
  if (!cfg) return;
  const c = cfg;
  const throttle = new Throttle(WINDOW_MS);

  const send = (d: vscode.TextDocument, isWrite: boolean) => {
    const doc = { scheme: d.uri.scheme, fsPath: d.uri.fsPath, isUntitled: d.isUntitled };
    if (!isTrackable(doc)) return;
    const now = Date.now();
    if (!throttle.shouldSend(d.uri.fsPath, isWrite, now)) return;
    const ed = vscode.window.activeTextEditor;
    const pos = ed && ed.document === d ? ed.selection.active : undefined;
    post(c, heartbeat(
      doc,
      { isWrite, lineno: (pos?.line ?? 0) + 1, cursorpos: (pos?.character ?? 0) + 1, lines: d.lineCount, language: d.languageId, nowMs: now, version },
    ));
  };

  ctx.subscriptions.push(
    vscode.workspace.onDidSaveTextDocument((d) => send(d, true)),
    vscode.window.onDidChangeActiveTextEditor((e) => e && send(e.document, false)),
    // Empty contentChanges are dirty-flag changes (OBSERVED in docs/platform-notes.md), not typing.
    vscode.workspace.onDidChangeTextDocument((e) => e.contentChanges.length > 0 && send(e.document, false)),
  );
  if (vscode.window.activeTextEditor) send(vscode.window.activeTextEditor.document, false);
}

export function deactivate(): void {}
