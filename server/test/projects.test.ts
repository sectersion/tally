import { afterAll, expect, test } from "bun:test";
import type { SQL } from "bun";
import { testConfig } from "../src/config.ts";
import { connect } from "../src/db.ts";
import {
  createProject,
  nameAllowed,
  projectDaily,
  projectHours,
  setHandle,
  validName,
} from "../src/projects.ts";
import { createSession } from "../src/session.ts";
import { chains, seed, start } from "./helpers.ts";

const sql = connect(process.env.DATABASE_URL ?? "");
const srv = start(sql);
const ORIGIN = "http://localhost:8080";
const GENESIS = "0".repeat(64);
afterAll(async () => {
  srv.stop();
  await sql.close();
});

const h = () => `u${crypto.randomUUID().replaceAll("-", "").slice(0, 12)}`;
async function user(db: SQL, handle: string | null) {
  const id = crypto.randomUUID();
  await db`insert into users (id, hca_sub, handle) values (${id}, ${`sub!${id}`}, ${handle})`;
  return { id, handle: handle as string };
}
let chainNo = 0;
/** Inserts a chain with the given (verified, unverified) rows straight into the tables. */
async function ledgerRows(db: SQL, projectId: string, ruleset: string, rows: [number, number][]) {
  const [c] =
    await db`insert into chains (project_id, ruleset, head_hash) values (${projectId}, ${ruleset}, ${GENESIS}) returning id`;
  for (const [i, [v, u]] of rows.entries()) {
    const hash = `${++chainNo}`.padStart(64, "0");
    await db`insert into ledger_entries (chain_id, seq, hash, prev_hash, action_id, ts, kind, verified_s, unverified_s, entry)
      values (${c.id}, ${i}, ${hash}, ${GENESIS}, ${`a${i}`}, ${1_700_000_000_000 + i * 86_400_000}, 'k', ${v}, ${u}, '{}'::json)`;
  }
}
async function newProject(handle?: string) {
  const u = await user(sql, handle ?? h());
  const r = await createProject(sql, u.id, `${u.handle}/p${chainNo++}`);
  if (!("id" in r)) throw new Error("setup");
  return { u, id: r.id };
}
async function auth(userId: string) {
  const { setCookie, csrf } = await createSession(sql, testConfig(), userId);
  return { cookie: setCookie.split(";")[0] as string, csrf };
}
const form = (path: string, a: { cookie: string }, body: Record<string, string>, origin = ORIGIN) =>
  fetch(`${srv.url}${path}`, {
    method: "POST",
    redirect: "manual",
    headers: { cookie: a.cookie, origin },
    body: new URLSearchParams(body),
  });

test("names are unique ignoring case", async () => {
  const u = await user(sql, h());
  expect(await createProject(sql, u.id, `${u.handle}/Tally`)).toHaveProperty("id");
  expect(await createProject(sql, u.id, `${u.handle}/tally`)).toEqual({ error: "taken" });
});
test("concurrent creates of one name: exactly one wins", async () => {
  const u = await user(sql, h());
  const r = await Promise.all([
    createProject(sql, u.id, `${u.handle}/x`),
    createProject(sql, u.id, `${u.handle}/x`),
  ]);
  expect(r.filter((x) => "id" in x).length).toBe(1);
});
test("a name under someone else's handle is not allowed", async () => {
  const a = await user(sql, h());
  const b = await user(sql, h());
  expect(await createProject(sql, b.id, `${a.handle}/x`)).toEqual({ error: "not_allowed" });
  expect(await createProject(sql, b.id, "torvalds/linux")).toEqual({ error: "not_allowed" });
  expect(await nameAllowed(sql, b.id, `${b.handle.toUpperCase()}/x`)).toBe(true);
  expect(await createProject(sql, b.id, "no-slash")).toEqual({ error: "bad_name" });
});
test("validName", () => {
  expect(validName("a/b.c_d-e")).toBe(true);
  for (const bad of ["a", "/b", "a/", "-a/b", "a/b/c", "a b/c", `${"a".repeat(40)}/b`])
    expect(validName(bad)).toBe(false);
});
test("handles: reserved, malformed, set once", async () => {
  const u = await user(sql, null);
  expect(await setHandle(sql, u.id, "badge")).toBe("reserved");
  expect(await setHandle(sql, u.id, "-x")).toBe("bad");
  const want = `Ok-${h()}`;
  expect(await setHandle(sql, u.id, want)).toBe("ok");
  const [row] = await sql`select handle from users where id = ${u.id}`;
  expect(row.handle).toBe(want.toLowerCase());
  expect(await setHandle(sql, u.id, h())).toBe("bad"); // already set
});
test("a taken handle is reported, ignoring case", async () => {
  const a = await user(sql, h());
  const b = await user(sql, null);
  expect(await setHandle(sql, b.id, a.handle.toUpperCase())).toBe("taken");
});

test("projectHours keeps unverified apart and sums across chains", async () => {
  const { id } = await newProject();
  await ledgerRows(sql, id, "2026.11-0", [
    [60, 20],
    [40, 30],
  ]);
  await ledgerRows(sql, id, "old-ruleset", [[0, 30]]);
  expect(await projectHours(sql, id)).toEqual({ verified_s: 100, unverified_s: 80 });
});
test("empty project is zeros, not null", async () => {
  const { id } = await newProject();
  expect(await projectHours(sql, id)).toEqual({ verified_s: 0, unverified_s: 0 });
});
test("projectDaily groups by UTC day", async () => {
  const { id } = await newProject();
  await ledgerRows(sql, id, "r", [[5, 1]]);
  expect(await projectDaily(sql, id, 100_000)).toEqual([
    { day: "2023-11-14", verified_s: 5, unverified_s: 1 },
  ]);
  expect(await projectDaily(sql, id, 90)).toEqual([]); // too old for the window
});

const upload = (p: { projectId: string; token: string }, entries: unknown[], prev = GENESIS) =>
  fetch(`${srv.url}/v1/ledger/${p.projectId}/entries`, {
    method: "POST",
    headers: { authorization: `Bearer ${p.token}`, "content-type": "application/json" },
    body: JSON.stringify({ prev_hash: prev, entries }),
  });
const seq = async (id: string) =>
  Number((await sql`select change_seq from projects where id = ${id}`)[0].change_seq);

test("projectHours agrees with chains.verified_s after real uploads", async () => {
  const p = await seed(sql, "acme/demo-16");
  const c = chains["acme/demo-16"] ?? [];
  expect((await upload(p, c.slice(0, 3))).status).toBe(200);
  const [ch] =
    await sql`select sum(verified_s)::bigint as s from chains where project_id = ${p.projectId}`;
  const hrs = await projectHours(sql, p.projectId);
  expect(hrs.verified_s).toBe(Number(ch.s));
  expect(hrs.verified_s).toBeGreaterThan(0);
});
test("an upload that inserts bumps change_seq once; a replayed upload does not", async () => {
  const p = await seed(sql, "acme/demo-15");
  const c = chains["acme/demo-15"] ?? [];
  expect(await seq(p.projectId)).toBe(0);
  expect((await upload(p, c.slice(0, 3))).status).toBe(200);
  expect(await seq(p.projectId)).toBe(1);
  expect((await upload(p, c.slice(0, 3))).status).toBe(200); // replay
  expect(await seq(p.projectId)).toBe(1);
});

test("summary: public project gives seconds and handle; private and unowned are 404", async () => {
  const { u, id } = await newProject();
  await ledgerRows(sql, id, "r", [[100, 50]]);
  const r = await fetch(`${srv.url}/v1/projects/${id}/summary`);
  expect(r.status).toBe(200);
  expect(r.headers.get("cache-control")).toBe("public, max-age=60");
  const body = await r.json();
  expect(body).toEqual({
    id,
    public_name: expect.stringContaining(u.handle),
    owner_handle: u.handle,
    verified_s: 100,
    unverified_s: 50,
    last_commit_at: null, // no receipts yet; webhook.test.ts covers the value
  });
  await sql`update users set profile_public = false where id = ${u.id}`;
  expect(
    (
      (await (await fetch(`${srv.url}/v1/projects/${id}/summary`)).json()) as {
        owner_handle: unknown;
      }
    ).owner_handle,
  ).toBeNull();
  await sql`update projects set public = false where id = ${id}`;
  expect((await fetch(`${srv.url}/v1/projects/${id}/summary`)).status).toBe(404);
  const orphan = await seed(sql);
  expect((await fetch(`${srv.url}/v1/projects/${orphan.projectId}/summary`)).status).toBe(404);
  expect((await fetch(`${srv.url}/v1/projects/nope/summary`)).status).toBe(404);
});

test("POST /settings/handle sets the handle behind csrf", async () => {
  const u = await user(sql, null);
  const a = await auth(u.id);
  const hh = h();
  expect((await form("/settings/handle", a, { handle: hh })).status).toBe(403);
  expect((await form("/settings/handle", a, { handle: hh, csrf: a.csrf })).status).toBe(303);
  expect((await sql`select handle from users where id = ${u.id}`)[0].handle).toBe(hh);
  expect(
    (
      await fetch(`${srv.url}/settings/handle`, {
        method: "POST",
        body: new URLSearchParams({ handle: "x" }),
      })
    ).status,
  ).toBe(401);
});
test("POST /projects creates and redirects to the project page", async () => {
  const u = await user(sql, h());
  const a = await auth(u.id);
  const name = `${u.handle}/web`;
  expect((await form("/projects", a, { name })).status).toBe(403);
  const r = await form("/projects", a, { name, csrf: a.csrf });
  expect(r.status).toBe(303);
  expect(r.headers.get("location")).toBe(`/p/${name}`);
  expect((await sql`select 1 from projects where public_name = ${name}`).length).toBe(1);
  const bad = await form("/projects", a, { name: "torvalds/linux", csrf: a.csrf });
  expect(bad.status).toBe(303);
  expect(bad.headers.get("location")).toContain("error=not_allowed");
});
test("project settings: owner toggles public, others get 404", async () => {
  const { u, id } = await newProject();
  const [{ public_name }] = await sql`select public_name from projects where id = ${id}`;
  const o = await auth(u.id);
  const other = await auth((await user(sql, h())).id);
  const path = `/p/${public_name}/settings`;
  expect((await form(path, other, { public: "off", csrf: other.csrf })).status).toBe(404);
  expect((await form(path, o, { public: "off" })).status).toBe(403);
  expect((await form(path, o, { public: "off", csrf: o.csrf })).status).toBe(303);
  expect((await sql`select public from projects where id = ${id}`)[0].public).toBe(false);
  expect((await form(path, o, { public: "on", csrf: o.csrf })).status).toBe(303);
  expect((await sql`select public from projects where id = ${id}`)[0].public).toBe(true);
  const get = (a: { cookie: string }) =>
    fetch(`${srv.url}${path}`, { headers: { cookie: a.cookie } });
  const page = await get(o);
  expect(page.status).toBe(200);
  const text = await page.text();
  expect(text).toContain(id);
  expect(text).toContain(`tally project link &lt;path&gt; --server-id ${id}`);
  expect((await get(other)).status).toBe(404);
});
