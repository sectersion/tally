import { afterAll, expect, test } from "bun:test";
import { createServer } from "../src/app.ts";
import { testConfig } from "../src/config.ts";
import { connect } from "../src/db.ts";
import { createProject } from "../src/projects.ts";

const sql = connect(process.env.DATABASE_URL ?? "");
const NOW = Date.now();

const srv = createServer({ cfg: testConfig(), sql, port: 0, fetchFn: fetch });
const base = `http://localhost:${srv.port}`;
afterAll(async () => {
  srv.stop(true);
  await sql.close();
});

const h = () => `u${crypto.randomUUID().replaceAll("-", "").slice(0, 12)}`;
const rnd = () => Math.floor(Math.random() * 1e12);

async function owner(o: { profilePublic?: boolean } = {}) {
  const id = crypto.randomUUID();
  const handle = h();
  await sql`insert into users (id, hca_sub, handle, email, name, verification_status, profile_public)
    values (${id}, ${`sub!${id}`}, ${handle}, ${`${handle}@example.test`}, ${"Test Name"}, ${"leak-verified-status"}, ${o.profilePublic ?? true})`;
  return { id, handle };
}

let seq = 0;
async function ledger(projectId: string, rows: { ts: number; v: number; kind?: string }[]) {
  const [c] =
    await sql`insert into chains (project_id, ruleset, head_hash) values (${projectId}, ${`r${rnd()}`}, ${"0".repeat(64)}) returning id`;
  for (const [i, r] of rows.entries())
    await sql`insert into ledger_entries (chain_id, seq, hash, prev_hash, action_id, ts, kind, verified_s, unverified_s, entry)
      values (${c.id}, ${i}, ${`${++seq}`.padStart(64, "0")}, ${"0".repeat(64)}, ${`a${i}`}, ${r.ts}, ${r.kind ?? "k"}, ${r.v}, 0, '{}'::json)`;
  await sql`update chains set verified_s = ${rows.reduce((n, r) => n + r.v, 0)} where id = ${c.id}`;
}

async function project(ownerId: string, handle: string, pre: string) {
  const a = await createProject(sql, ownerId, `${handle}/${pre}${rnd()}`);
  if (!("id" in a)) throw new Error(`setup: ${JSON.stringify(a)}`);
  return a.id;
}

const get = (path: string) => fetch(`${base}${path}`, { redirect: "manual" });
const day = 86_400_000;

test("profile shows total, action mix, active, streak", async () => {
  const u = await owner();
  const a = await project(u.id, u.handle, "a");
  const b = await project(u.id, u.handle, "b");
  await ledger(a, [
    { ts: NOW - 2 * day, v: 3600, kind: "prompt" },
    { ts: NOW - day, v: 7200, kind: "user_edit" },
  ]);
  await ledger(b, [{ ts: NOW, v: 1800, kind: "test_run" }]);
  const t = await (await get(`/u/${u.handle}`)).text();
  expect(t).toContain("3.5 h"); // total across both projects
  expect(t).toContain("Active in the last 7 days: <strong>yes</strong>");
  expect(t).toContain("Current streak: <strong>3 days</strong>"); // today+yesterday+day before
  expect(t).toContain("<td>prompt</td><td>1.0 h</td>");
  expect(t).toContain("<td>user_edit</td><td>2.0 h</td>");
  expect(t).toContain("<td>test_run</td><td>0.5 h</td>");
  expect(t).toContain("<td>commit</td><td>0.0 h</td>");
});

test("streak is 1 for an active today only, 0 when last entry is older", async () => {
  const u = await owner();
  const a = await project(u.id, u.handle, "c");
  await ledger(a, [{ ts: NOW, v: 60 }]);
  expect(await (await get(`/u/${u.handle}`)).text()).toContain(
    "Current streak: <strong>1 day</strong>",
  );
  const u2 = await owner();
  const b = await project(u2.id, u2.handle, "d");
  await ledger(b, [{ ts: NOW - 3 * day, v: 60 }]);
  const t = await (await get(`/u/${u2.handle}`)).text();
  expect(t).toContain("Current streak: <strong>0 days</strong>");
  expect(t).toContain("Active in the last 7 days: <strong>yes</strong>"); // 3 days ago is within 7
});

test("private profile is 404; private projects excluded from totals and table", async () => {
  const u = await owner({ profilePublic: false });
  expect((await get(`/u/${u.handle}`)).status).toBe(404);
  const pub = await owner();
  const a = await project(pub.id, pub.handle, "pub");
  await ledger(a, [{ ts: NOW, v: 3600 }]);
  const hid = await project(pub.id, pub.handle, "hid");
  await sql`update projects set public = false where id = ${hid}`;
  await ledger(hid, [{ ts: NOW, v: 99999, kind: "commit" }]);
  const t = await (await get(`/u/${pub.handle}`)).text();
  expect(t).toContain("1.0 h");
  expect(t).not.toContain(`${pub.handle}/hid`);
  expect(t).toContain("<td>commit</td><td>0.0 h</td>");
});
