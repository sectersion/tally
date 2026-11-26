import { afterAll, expect, test } from "bun:test";
import { connect } from "../src/db.ts";
import { leaderboard } from "../src/leaderboard.ts";
import { createProject } from "../src/projects.ts";
import { start } from "./helpers.ts";

const sql = connect(
  process.env.DATABASE_URL ?? "postgres://postgres:postgres@localhost:5432/postgres",
);
const srv = start(sql);
const base = srv.url;
afterAll(() => {
  srv.stop();
  void sql.close();
});

const h = () => `u${crypto.randomUUID().replaceAll("-", "").slice(0, 12)}`;
const rnd = () => Math.floor(Math.random() * 1e12);
const GENESIS = "0".repeat(64);
const NOW = Date.now();
// Deterministic UTC-day timestamps: midway through today, and 12h before today's start (yesterday).
const n = new Date();
const dayStart = Date.UTC(n.getUTCFullYear(), n.getUTCMonth(), n.getUTCDate());
const TODAY = dayStart + Math.floor((NOW - dayStart) / 2);
const YESTERDAY = dayStart - 12 * 3_600_000;

async function owner(profilePublic = true) {
  const id = crypto.randomUUID();
  const handle = h();
  await sql`insert into users (id, hca_sub, handle, profile_public, display_name)
    values (${id}, ${`sub!${id}`}, ${handle}, ${profilePublic}, ${profilePublic ? `DN ${handle}` : null})`;
  return { id, handle };
}
async function project(o: { public?: boolean; profilePublic?: boolean } = {}) {
  const u = await owner(o.profilePublic ?? true);
  const name = `${u.handle}/api${rnd()}`;
  const r = await createProject(sql, u.id, name);
  if (!("id" in r)) throw new Error("setup");
  if (o.public === false) await sql`update projects set public = false where id = ${r.id}`;
  return { u, id: r.id, name };
}
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

test("private profile is 404 on stats and projects", async () => {
  const p = await project({ profilePublic: false });
  for (const path of ["stats", "projects"]) {
    const r = await fetch(`${base}/api/v1/users/${p.u.handle}/${path}`);
    expect(r.status).toBe(404);
  }
});

test("unknown handle is 404", async () => {
  expect((await fetch(`${base}/api/v1/users/nobody-${rnd()}/stats`)).status).toBe(404);
});

test("no auth works, cache header set", async () => {
  const p = await project();
  await ledger(p.id, [{ ts: NOW - 1000, v: 60, u: 30 }]);
  const r = await fetch(`${base}/api/v1/users/${p.u.handle}/stats`);
  expect(r.status).toBe(200);
  expect(r.headers.get("cache-control")).toBe("public, max-age=60");
});

test("stats numbers: sums over public projects only", async () => {
  const pub = await project();
  const priv = await project({ profilePublic: true });
  await sql`update projects set public = false where id = ${priv.id}`;
  await ledger(pub.id, [
    { ts: TODAY, v: 60, u: 30 },
    { ts: YESTERDAY, v: 120 },
  ]);
  await ledger(priv.id, [{ ts: NOW - 1000, v: 9999 }]);
  const b = await (await fetch(`${base}/api/v1/users/${pub.u.handle}/stats`)).json();
  expect(b).toEqual({
    handle: pub.u.handle,
    display_name: `DN ${pub.u.handle}`,
    verified_s: 180,
    unverified_s: 30,
    projects_count: 1,
    streak_days: 2,
    active_days_last_30: 2,
  });
});

test("stats: no verified days means streak 0 and display_name null when unset", async () => {
  const p = await project({ profilePublic: true });
  await sql`update users set display_name = null where id = ${p.u.id}`;
  await ledger(p.id, [{ ts: NOW - 1000, v: 0, u: 5000 }]);
  const b = await (await fetch(`${base}/api/v1/users/${p.u.handle}/stats`)).json();
  expect(b.streak_days).toBe(0);
  expect(b.display_name).toBeNull();
  expect(b.verified_s).toBe(0);
  expect(b.unverified_s).toBe(5000);
});

test("projects list: public projects with sums and last activity", async () => {
  const p = await project();
  const priv = await project({ profilePublic: true });
  await sql`update projects set public = false where id = ${priv.id}`;
  await ledger(p.id, [{ ts: NOW - 1000, v: 60, u: 15 }]);
  await ledger(priv.id, [{ ts: NOW - 1000, v: 9999 }]);
  const d = `d${rnd()}`;
  await sql`insert into webhook_deliveries (delivery_id, event) values (${d}, 'push')`;
  await sql`insert into push_receipts (delivery_id, sha, github_repo_id, project_id, ref, is_head, forced)
    values (${d}, ${`s${rnd()}`}, 0, ${p.id}, 'refs/heads/main', true, false)`;
  const rows = (await (await fetch(`${base}/api/v1/users/${p.u.handle}/projects`)).json()) as {
    public_name: string;
    verified_s: number;
    unverified_s: number;
    last_activity_at: string | null;
  }[];
  const r0 = rows[0] as {
    public_name: string;
    verified_s: number;
    unverified_s: number;
    last_activity_at: string | null;
  };
  expect(rows).toHaveLength(1);
  expect(r0.public_name).toBe(p.name);
  expect(r0.verified_s).toBe(60);
  expect(r0.unverified_s).toBe(15);
  expect(r0.last_activity_at).not.toBeNull();
});

test("leaderboard api: day vs week, private profile handle null, bad range 400", async () => {
  const a = await project();
  // Shared DB: seed above the current max so these always rank into the top 25.
  const big = Math.max(0, ...(await leaderboard(sql, "week")).map((r) => r.verified_s)) + 1_000_000;
  await ledger(a.id, [
    { ts: TODAY, v: big + 60 },
    { ts: YESTERDAY, v: 120 },
  ]);
  const b = await project({ profilePublic: false });
  await ledger(b.id, [{ ts: TODAY, v: big + 600 }]);
  const day = (await (await fetch(`${base}/api/v1/leaderboard?range=day`)).json()) as {
    rank: number;
    public_name: string;
    handle: string | null;
    verified_s: number;
  }[];
  const week = (await (await fetch(`${base}/api/v1/leaderboard?range=week`)).json()) as typeof day;
  const da = day.find((r) => r.public_name === a.name);
  expect(da).toMatchObject({ handle: a.u.handle, verified_s: big + 60 });
  const db = day.find((r) => r.public_name === b.name);
  expect(db).toMatchObject({ handle: null, verified_s: big + 600 });
  expect(day.map((r) => r.rank)).toEqual(day.map((_, i) => i + 1));
  expect(week.find((r) => r.public_name === a.name)?.verified_s).toBe(big + 180);
  expect(week.find((r) => r.public_name === b.name)?.verified_s).toBe(big + 600);
  const bad = await fetch(`${base}/api/v1/leaderboard?range=bogus`);
  expect(bad.status).toBe(400);
  expect((await bad.json()).error).toBe("invalid_body");
  expect((await fetch(`${base}/api/v1/leaderboard`)).status).toBe(400);
});
