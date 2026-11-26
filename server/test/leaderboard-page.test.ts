import { afterAll, expect, test } from "bun:test";
import { connect } from "../src/db.ts";
import { leaderboard } from "../src/leaderboard.ts";
import { formatHours } from "../src/ledger.ts";
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

async function owner(profilePublic = true) {
  const id = crypto.randomUUID();
  const handle = h();
  await sql`insert into users (id, hca_sub, handle, profile_public)
    values (${id}, ${`sub!${id}`}, ${handle}, ${profilePublic})`;
  return { id, handle };
}
async function project(o: { public?: boolean; profilePublic?: boolean } = {}) {
  const u = await owner(o.profilePublic ?? true);
  const name = `${u.handle}/lb${rnd()}`;
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
  await sql`update chains set verified_s = ${rows.reduce((n, r) => n + r.v, 0)} where id = ${c.id}`;
}

/** The suite shares the DB with other suites and its own earlier runs: seed above the current max so the fixture always ranks in. */
async function topSum() {
  const rows = await leaderboard(sql, "week");
  return Math.max(0, ...rows.map((r) => r.verified_s));
}

test("leaderboard is a public page reachable signed out", async () => {
  const r = await fetch(`${base}/leaderboard`);
  expect(r.status).toBe(200);
  const t = await r.text();
  expect(t).toContain("Leaderboard");
  expect(t).toContain("Only verified time counts. Unverified time is never added in.");
  expect(t).not.toContain("Sign in with Hack Club"); // not the landing page
});

test("lists only public projects with an owner; unverified never added", async () => {
  const m = await topSum();
  const pub = await project();
  const priv = await project({ public: false });
  const orphan = crypto.randomUUID();
  await sql`insert into projects (id, public_name) values (${orphan}, ${`${h()}/lb-orphan`})`;
  await ledger(pub.id, [{ ts: NOW - 1000, v: m + 3000, u: 5000 }]);
  await ledger(priv.id, [{ ts: NOW - 1000, v: m + 5000 }]);
  await ledger(orphan, [{ ts: NOW - 1000, v: m + 4000 }]);
  const t = await (await fetch(`${base}/leaderboard`)).text();
  expect(t).toContain(`href="/p/${pub.name}"`);
  expect(t).toContain(formatHours(m + 3000)); // 5000 unverified s never added in
  expect(t).not.toContain(priv.name);
  expect(t).not.toContain("lb-orphan");
});

test("day vs week sums differ correctly", async () => {
  const m = await topSum();
  const p = await project();
  // today: m+1000; plus an entry 25h ago (inside the ISO week, outside the UTC day): m+10000
  await ledger(p.id, [
    { ts: NOW - 25 * 3_600_000, v: m + 10000 },
    { ts: NOW, v: m + 1000 },
  ]);
  const day = await (await fetch(`${base}/leaderboard`)).text();
  const week = await (await fetch(`${base}/leaderboard?range=week`)).text();
  expect(day).toContain(formatHours(m + 1000));
  expect(day).not.toContain(formatHours(m + 10000));
  expect(week).toContain(formatHours(2 * m + 11000));
});

test("ordering is by verified time desc with rank numbers", async () => {
  const m = await topSum();
  const a = await project();
  const b = await project();
  await ledger(a.id, [{ ts: NOW - 1000, v: m + 100 }]);
  await ledger(b.id, [{ ts: NOW - 1000, v: m + 200 }]);
  const t = await (await fetch(`${base}/leaderboard`)).text();
  const ai = t.indexOf(`href="/p/${a.name}"`);
  const bi = t.indexOf(`href="/p/${b.name}"`);
  expect(bi).toBeGreaterThan(-1);
  expect(bi).toBeLessThan(ai); // b has more verified time, ranks higher
  expect(t).toContain("<td>1</td>");
});

test("limit is 25 rows", async () => {
  const m = await topSum();
  for (let i = 0; i < 27; i++) {
    const p = await project();
    await ledger(p.id, [{ ts: NOW - 1000, v: m + 100000 + i }]);
  }
  const t = await (await fetch(`${base}/leaderboard`)).text();
  expect((t.match(/href="\/p\//g) ?? []).length).toBeLessThanOrEqual(25);
});

test("private profile hides owner link, public profile links /u/:handle", async () => {
  const m = await topSum();
  const pub = await project({ profilePublic: true });
  const priv = await project({ profilePublic: false });
  await ledger(pub.id, [{ ts: NOW - 1000, v: m + 600 }]);
  await ledger(priv.id, [{ ts: NOW - 1000, v: m + 700 }]);
  const t = await (await fetch(`${base}/leaderboard`)).text();
  expect(t).toContain(`href="/u/${pub.u.handle}"`);
  expect(t).not.toContain(`href="/u/${priv.u.handle}"`);
  expect(t).toContain(`href="/p/${priv.name}"`); // the project still ranks, without its owner
});

test("bad range falls back to day; module returns rows sorted", async () => {
  const t = await (await fetch(`${base}/leaderboard?range=bogus`)).text();
  expect(t).toContain("Today (UTC)");
  expect(t).toContain('href="/leaderboard?range=week"');
  expect(await (await fetch(`${base}/leaderboard?range=week`)).text()).toContain("This week (ISO)");
  const rows = await leaderboard(sql, "day");
  for (let i = 1; i < rows.length; i++)
    expect(rows[i - 1]?.verified_s).toBeGreaterThanOrEqual(rows[i]?.verified_s ?? 0);
});
