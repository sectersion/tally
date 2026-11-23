// Boots the real `tally dashboard` against a seeded temp home and renders the served page in happy-dom
// (registered globally by src/test-setup.ts) against the live API. No layout or contrast checks here.
import { afterAll, beforeAll, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { connect } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";

const BIN = process.env.TALLY_BIN ?? join(import.meta.dir, "../../target/debug", process.platform === "win32" ? "tally.exe" : "tally");
const SECRETS = ["SECRETCMD", "SECRETFILE", "aaaaaaaa11111111SECRETCMD"];
let home = "";
let child: ReturnType<typeof Bun.spawn> | null = null;
let url = "";
let port = 0;
const realFetch = globalThis.fetch;

function tally(...args: string[]) {
  const r = Bun.spawnSync([BIN, "--home", home, ...args]);
  if (r.exitCode !== 0) throw new Error(`tally ${args.join(" ")}: ${r.stderr.toString()}`);
}

async function startServer(p: number): Promise<void> {
  child = Bun.spawn([BIN, "--home", home, "dashboard", "--port", String(p), "--no-open"], { stdout: "pipe", stderr: "pipe" });
  const reader = (child.stdout as ReadableStream<Uint8Array>).getReader();
  let text = "";
  while (!/http:\/\/127\.0\.0\.1:\d+\//.test(text)) {
    const { value, done } = await reader.read();
    if (done) throw new Error("dashboard exited: " + text);
    text += new TextDecoder().decode(value);
  }
  url = text.match(/http:\/\/127\.0\.0\.1:\d+\//)![0];
  port = Number(new URL(url).port);
}

async function waitFor(pred: () => boolean, ms = 3000): Promise<void> {
  const end = Date.now() + ms;
  while (!pred()) {
    if (Date.now() > end) throw new Error("timeout; page text: " + document.body.textContent?.slice(0, 600));
    await new Promise((r) => setTimeout(r, 25));
  }
}

const text = () => document.body.textContent ?? "";

/** Loads the served index and main.js into the global happy-dom window. */
async function boot(pollMs = 15000): Promise<void> {
  (globalThis as any).fetch = (u: any, init?: any) => realFetch(new URL(String(u), url), init);
  (window as any).happyDOM.setURL(url);
  const html = await (await realFetch(url)).text();
  expect(html).toContain('id="app"');
  document.body.innerHTML = '<div id="app"></div>';
  document.head.querySelectorAll('meta[name="tally-poll-ms"]').forEach((m) => m.remove());
  const meta = document.createElement("meta");
  meta.setAttribute("name", "tally-poll-ms");
  meta.setAttribute("content", String(pollMs));
  document.head.appendChild(meta);
  const js = await (await realFetch(url + "main.js")).text();
  (0, eval)(js);
}

beforeAll(async () => {
  home = mkdtempSync(join(tmpdir(), "tally-e2e-"));
  // fixtures/seed.ndjson has ts in seconds from a base; rebase it to three days ago so it is inside the page's window
  const base = Date.now() - 3 * 86_400_000;
  const lines = readFileSync(join(import.meta.dir, "../fixtures/seed.ndjson"), "utf8").trim().split("\n").map((l) => {
    const e = JSON.parse(l);
    e.ts = base + e.ts * 1000;
    if (e.end_ts !== undefined) e.end_ts = base + e.end_ts * 1000;
    return JSON.stringify(e);
  });
  const file = join(home, "seed.ndjson");
  writeFileSync(file, lines.join("\n") + "\n");
  tally("ingest", file, "--source", "seed", "--watermark", String(base + 2 * 86_400_000));
  tally("finalize");
  await startServer(0);
});

afterAll(() => {
  child?.kill();
  (globalThis as any).fetch = realFetch;
  rmSync(home, { recursive: true, force: true });
});

test("served page boots against the live API and shows the fixture's numbers", async () => {
  await boot();
  // two loops: 5 min (310 - 10 s) and 9 min (1540 - 1000 s) -> average 7 min
  await waitFor(() => text().includes("Bugs fixed") && text().includes("Average time from failure to fix: 7 min"));
  await waitFor(() => text().includes("Time Tally removed") && text().includes("while you were away from the keyboard"));
  expect(document.querySelectorAll("h1").length).toBe(1);
});

test("replay lists the fixture sessions, the first moment is a prompt and the fixes are marked", async () => {
  await boot();
  await waitFor(() => text().includes("Bugs fixed"));
  window.location.hash = "#/replay";
  window.dispatchEvent(new window.HashChangeEvent("hashchange"));
  await waitFor(() => [...document.querySelectorAll("button")].some((b) => (b.textContent ?? "").startsWith("p1")));
  [...document.querySelectorAll("button")].find((b) => (b.textContent ?? "").startsWith("p1"))!.dispatchEvent(new window.MouseEvent("click", { bubbles: true }));
  await waitFor(() => document.querySelectorAll("ol li").length > 3);
  const items = [...document.querySelectorAll("ol li")].map((li) => li.textContent ?? "");
  expect(items.some((t) => /fix/i.test(t))).toBe(true);
  expect(items.some((t) => /prompt/i.test(t))).toBe(true);
});

test("the page and every API response contain none of the fixture's secret strings", async () => {
  const to = Date.now() + 86_400_000, from = to - 28 * 86_400_000;
  const edges = Array.from({ length: 29 }, (_, i) => from + i * 86_400_000).join(",");
  const paths = ["/", "/main.js", "/api/status", `/api/bugs?from=${from}&to=${to}`, `/api/prompts?from=${from}&to=${to}`,
    `/api/hours?edges=${edges}`, `/api/sessions?from=${from}&to=${to}`, `/api/replay?from=${from}&to=${to}`];
  for (const p of paths) {
    const r = await realFetch(url.slice(0, -1) + p);
    expect(r.status).toBe(200);
    const body = await r.text();
    for (const s of SECRETS) expect(body).not.toContain(s);
  }
});

test("the server refuses a foreign Host header and sends no CORS header", async () => {
  const raw = (host: string) =>
    new Promise<string>((resolve, reject) => {
      const s = connect(port, "127.0.0.1", () => s.write(`GET /api/status HTTP/1.1\r\nHost: ${host}\r\nConnection: close\r\n\r\n`));
      let out = "";
      s.on("data", (d) => (out += d));
      s.on("end", () => resolve(out));
      s.on("error", reject);
    });
  expect((await raw(`evil.example:${port}`)).startsWith("HTTP/1.1 403")).toBe(true);
  const ok = await raw(`127.0.0.1:${port}`);
  expect(ok.startsWith("HTTP/1.1 200")).toBe(true);
  expect(ok.toLowerCase()).not.toContain("access-control-");
});

test("restarting the daemon on the same port brings the open page back without reload", async () => {
  await boot(200);
  await waitFor(() => text().includes("Bugs fixed"));
  child!.kill();
  await child!.exited;
  await waitFor(() => !!document.querySelector('[role="status"]') && /not running/i.test(text()), 20000);
  expect(text()).toContain("Bugs fixed"); // last data stays visible
  await startServer(port);
  await waitFor(() => !/not running/i.test(text()), 20000);
});
