import { afterAll, expect, test } from "bun:test";
import { createServer } from "../src/app.ts";
import { testConfig } from "../src/config.ts";
import { connect } from "../src/db.ts";
import { streakOf } from "../src/pages.ts";
import { createProject } from "../src/projects.ts";
import { createSession } from "../src/session.ts";

const sql = connect(process.env.DATABASE_URL ?? "");
const GENESIS = "0".repeat(64);
const srv = createServer({ cfg: testConfig(), sql, port: 0 });
const base = `http://localhost:${srv.port}`;
afterAll(async () => {
  srv.stop(true);
  await sql.close();
});

const h = () => `u${crypto.randomUUID().replaceAll("-", "").slice(0, 12)}`;
const rnd = () => Math.floor(Math.random() * 1e12);

async function owner() {
  const id = crypto.randomUUID();
  const handle = h();
  await sql`insert into users (id, hca_sub, email, handle) values (${id}, ${`sub!dash!${id}`}, ${"d@example.test"}, ${handle})`;
  return { id, handle };
}
async function project(userId: string, handle: string) {
  const name = `${handle}/p${rnd()}`;
  const r = await createProject(sql, userId, name);
  if (!("id" in r)) throw new Error(`setup: ${JSON.stringify(r)}`);
  return { id: r.id, name };
}
async function auth(userId: string) {
  const { setCookie } = await createSession(sql, testConfig(), userId);
  return { cookie: setCookie.split(";")[0] as string };
}
const get = (path: string, cookie?: string) =>
  fetch(`${base}${path}`, { redirect: "manual", headers: cookie ? { cookie } : {} });

let hashNo = 0;
async function ledger(projectId: string, rows: { ts: number; v: number; u?: number }[]) {
  const [c] =
    await sql`insert into chains (project_id, ruleset, head_hash) values (${projectId}, ${`r${rnd()}`}, ${GENESIS}) returning id`;
  for (const [i, r] of rows.entries())
    await sql`insert into ledger_entries (chain_id, seq, hash, prev_hash, action_id, ts, kind, verified_s, unverified_s, entry)
      values (${c.id}, ${i}, ${`${++hashNo}`.padStart(64, "0")}, ${GENESIS}, ${`a${i}`}, ${r.ts}, 'k', ${r.v}, ${r.u ?? 0}, '{}'::json)`;
  await sql`update chains set verified_s = ${rows.reduce((n, r) => n + r.v, 0)},
    unverified_s = ${rows.reduce((n, r) => n + (r.u ?? 0), 0)} where id = ${c.id}`;
}

const dayMs = (n: number) => {
  const d = new Date();
  return Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate()) - n * 86_400_000 + 3600_000;
};

test("signed-out is redirected to login", async () => {
  const r = await get("/dashboard");
  expect(r.status).toBe(303);
  expect(r.headers.get("location")).toBe("/auth/login?return_to=%2Fdashboard");
});

test("dashboard shows totals, per-project rows and recent activity", async () => {
  const u = await owner();
  const p = await project(u.id, u.handle);
  await ledger(p.id, [
    { ts: dayMs(0), v: 3600, u: 120 },
    { ts: dayMs(3), v: 1800 },
    { ts: dayMs(20), v: 900 },
  ]);
  const t = await (await get("/dashboard", (await auth(u.id)).cookie)).text();
  expect(t).toContain("60 min"); // today
  expect(t).toContain("1.5 h"); // 7 days
  expect(t).toContain("1.7 h"); // all time
  expect(t).toContain(`<a href="/p/${p.name}">${p.name}</a>`);
  expect(t).toContain("2 min (not counted)"); // unverified
  expect(t).toContain("streak");
});

test("streak: today plus yesterday counts, a gap breaks it", async () => {
  expect(streakOf([])).toBe(0);
  const t = new Date().toISOString().slice(0, 10);
  const y = new Date(Date.now() - 86_400_000).toISOString().slice(0, 10);
  const g = new Date(Date.now() - 3 * 86_400_000).toISOString().slice(0, 10);
  expect(streakOf([t])).toBe(1);
  expect(streakOf([t, y, g])).toBe(2);
  expect(streakOf([y, g])).toBe(1); // gap after yesterday stops the run
  // started yesterday but nothing today still counts
  expect(streakOf([y])).toBe(1);
});
