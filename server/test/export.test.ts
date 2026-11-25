import { afterAll, expect, test } from "bun:test";
import { createServer } from "../src/app.ts";
import { testConfig } from "../src/config.ts";
import { connect } from "../src/db.ts";
import { createProject } from "../src/projects.ts";
import { createSession } from "../src/session.ts";

const sql = connect(process.env.DATABASE_URL ?? "");
const srv = createServer({ cfg: testConfig(), sql, port: 0 });
const base = `http://localhost:${srv.port}`;
afterAll(async () => {
  srv.stop(true);
  await sql.close();
});

const rnd = () => Math.floor(Math.random() * 1e12);
const h = () => `u${crypto.randomUUID().replaceAll("-", "").slice(0, 12)}`;
const GENESIS = "0".repeat(64);

async function owner() {
  const id = crypto.randomUUID();
  const handle = h();
  await sql`insert into users (id, hca_sub, handle, email, name, verification_status, profile_public)
    values (${id}, ${`sub!export!${id}`}, ${handle}, 'e@example.test', 'n', 'ok', true)`;
  return { id, handle };
}
async function project(o: { public?: boolean } = {}) {
  const u = await owner();
  const name = `${u.handle}/p${rnd()}`;
  const r = await createProject(sql, u.id, name);
  if (!("id" in r)) throw new Error("setup");
  if (o.public === false) await sql`update projects set public = false where id = ${r.id}`;
  return { u, id: r.id, name };
}
async function session(userId: string) {
  const { setCookie } = await createSession(sql, testConfig(), userId);
  return setCookie.split(";")[0] as string;
}
/** Two chains with entries, inserted at the raw column level. */
async function ledger(projectId: string) {
  let n = 0;
  const next = () => `${++n}`.padStart(64, "0");
  for (let c = 0; c < 2; c++) {
    const [chain] =
      await sql`insert into chains (project_id, ruleset, head_hash) values (${projectId}, ${`r${c}`}, ${next()}) returning id`;
    for (let i = 0; i < 2; i++)
      await sql`insert into ledger_entries (chain_id, seq, hash, prev_hash, action_id, ts, kind, verified_s, unverified_s, entry)
        values (${chain.id}, ${i}, ${next()}, ${GENESIS}, ${`a${c}-${i}`}, ${1000 + i}, 'k', ${i + 1}, ${i}, ${sql`json_build_object('f', ${i})`})`;
    await sql`update chains set verified_s = 3, unverified_s = 1 where id = ${chain.id}`;
  }
}
const get = (path: string, cookie?: string) =>
  fetch(`${base}${path}`, { redirect: "manual", headers: cookie ? { cookie } : {} });

test("signed-out public project exports as a JSON attachment", async () => {
  const { id, name } = await project();
  await ledger(id);
  const r = await get(`/p/${name}/export`);
  expect(r.status).toBe(200);
  expect(r.headers.get("content-type")).toContain("application/json");
  const file = r.headers.get("content-disposition");
  expect(file).toContain("attachment");
  expect(file).toContain(`filename="${name.replaceAll("/", "-->")}.json"`);
  const body = await r.json();
  expect(body.project).toBe(name);
  expect(body.chains).toHaveLength(2);
  expect(body.chains.map((c: { seq: number }) => c.seq)).toEqual([0, 1]);
  for (const c of body.chains as { entries: { seq: number; entry: { f: number } }[] }[]) {
    expect(c.entries.map((e) => e.seq)).toEqual([0, 1]);
    for (const e of c.entries) {
      expect(Object.keys(e).toSorted()).toEqual(
        [
          "entry",
          "hash",
          "kind",
          "prev_hash",
          "seq",
          "ts",
          "unverified_s",
          "verified_s",
        ].toSorted(),
      );
      expect(e.entry.f).toBeLessThan(2);
    }
  }
  const raw = JSON.stringify(body);
  expect(raw).not.toContain("owner");
});

test("private project 404 signed-out; owner sees it", async () => {
  const { id, name, u } = await project({ public: false });
  await ledger(id);
  expect((await get(`/p/${name}/export`)).status).toBe(404);
  const r = await get(`/p/${name}/export`, await session(u.id));
  expect(r.status).toBe(200);
  expect((await r.json()).chains).toHaveLength(2);
});

test("unknown project 404", async () => {
  expect((await get(`/p/${h()}/nope/export`)).status).toBe(404);
});
