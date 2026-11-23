import { afterAll, expect, test } from "bun:test";
import { createServer } from "../src/app.ts";
import { renderSvg } from "../src/badge.ts";
import { testConfig } from "../src/config.ts";
import { connect } from "../src/db.ts";
import { createProject } from "../src/projects.ts";
import { createLimiter } from "../src/ratelimit.ts";

const sql = connect(process.env.DATABASE_URL ?? "");
const limiter = createLimiter(100_000, 100_000);
const srv = createServer({ cfg: testConfig(), sql, port: 0, limiter });
const base = `http://localhost:${srv.port}`;
afterAll(async () => {
  srv.stop(true);
  await sql.close();
});

const rnd = () => Math.floor(Math.random() * 1e12);
const GENESIS = "0".repeat(64);
let hashNo = 1_000_000;

async function project(o: { public?: boolean; owned?: boolean } = {}) {
  const id = crypto.randomUUID();
  const handle = `b${crypto.randomUUID().replaceAll("-", "").slice(0, 12)}`;
  await sql`insert into users (id, hca_sub, handle) values (${id}, ${`sub!${id}`}, ${handle})`;
  const name = `${handle}/p${rnd()}`;
  const r = await createProject(sql, id, name);
  if (!("id" in r)) throw new Error("setup");
  if (o.public === false) await sql`update projects set public = false where id = ${r.id}`;
  if (o.owned === false) await sql`update projects set owner_id = null where id = ${r.id}`;
  return { id: r.id, name };
}
/** One chain: verified/unverified seconds, stored on both the entries and the chain row. */
async function ledger(projectId: string, v: number, u: number) {
  const [c] =
    await sql`insert into chains (project_id, ruleset, head_hash, count, verified_s) values (${projectId}, ${`r${rnd()}`}, ${GENESIS}, 1, ${v}) returning id`;
  await sql`insert into ledger_entries (chain_id, seq, hash, prev_hash, action_id, ts, kind, verified_s, unverified_s, entry)
    values (${c.id}, 0, ${`${++hashNo}`.padStart(64, "0")}, ${GENESIS}, 'a0', ${Date.now()}, 'k', ${v}, ${u}, '{}'::json)`;
  await sql`update projects set change_seq = change_seq + 1 where id = ${projectId}`;
}
const get = (path: string, headers: Record<string, string> = {}) =>
  fetch(`${base}${path}`, { headers });

test("svg and json agree with C8", async () => {
  const p = await project();
  await ledger(p.id, 10_800, 7_200);
  const j = await (await get(`/badge/${p.name}.json`)).json();
  expect(j).toEqual({
    schemaVersion: 1,
    label: "tracked with Tally",
    message: "3.0 h",
    color: "blue",
  });
  expect(j).toEqual(await (await get(`/v1/projects/${p.id}/badge.json`)).json());
  const r = await get(`/badge/${p.name}.svg`);
  expect(r.headers.get("content-type")).toBe("image/svg+xml; charset=utf-8");
  const s = await r.text();
  expect(s).toContain("3.0 h");
  expect(s).not.toContain("5.0 h");
});

test("zero hours is 0.0 h, 200, blue", async () => {
  const p = await project();
  const r = await get(`/badge/${p.name}.json`);
  expect(r.status).toBe(200);
  expect(await r.json()).toMatchObject({ message: "0.0 h", color: "blue" });
  expect(await (await get(`/badge/${p.name}.svg`)).text()).toContain("0.0 h");
});

test("unknown is a 404 svg with 'unknown'", async () => {
  const r = await get("/badge/nobody/nothing.svg");
  expect(r.status).toBe(404);
  expect(r.headers.get("content-type")).toBe("image/svg+xml; charset=utf-8");
  expect(await r.text()).toContain("unknown");
  const j = await get("/badge/nobody/nothing.json");
  expect(j.status).toBe(404);
  expect(await j.json()).toMatchObject({ message: "unknown", color: "lightgrey" });
});

test("unowned project behaves like unknown", async () => {
  const p = await project({ owned: false });
  const r = await get(`/badge/${p.name}.svg`);
  expect(r.status).toBe(404);
  expect(await r.text()).toContain("unknown");
});

test("private is 'private' and lightgrey", async () => {
  const p = await project({ public: false });
  await ledger(p.id, 3600, 0);
  const r = await get(`/badge/${p.name}.json`);
  expect(r.status).toBe(200);
  expect(await r.json()).toMatchObject({ message: "private", color: "lightgrey" });
  const s = await (await get(`/badge/${p.name}.svg`)).text();
  expect(s).toContain("private");
  expect(s).not.toContain("1.0 h");
});

test("etag round trip gives 304 and changes after an upload", async () => {
  const p = await project();
  const r1 = await get(`/badge/${p.name}.svg`);
  const etag = r1.headers.get("etag") ?? "";
  expect(etag).toMatch(new RegExp(`^"${p.id}-\\d+"$`));
  const r2 = await get(`/badge/${p.name}.svg`, { "if-none-match": etag });
  expect(r2.status).toBe(304);
  expect(await r2.text()).toBe("");
  await ledger(p.id, 3600, 0);
  const r3 = await get(`/badge/${p.name}.svg`, { "if-none-match": etag });
  expect(r3.status).toBe(200);
  expect(r3.headers.get("etag")).not.toBe(etag);
  expect(await r3.text()).toContain("1.0 h");
});

test("etag changes when the project is made private", async () => {
  const p = await project();
  const etag = (await get(`/badge/${p.name}.svg`)).headers.get("etag") ?? "";
  const [u] = await sql`select owner_id from projects where id = ${p.id}`;
  const { createSession } = await import("../src/session.ts");
  const { setCookie, csrf } = await createSession(sql, testConfig(), u.owner_id);
  const res = await fetch(`${base}/p/${p.name}/settings`, {
    method: "POST",
    redirect: "manual",
    headers: { cookie: setCookie.split(";")[0] as string, origin: "http://localhost:8080" },
    body: new URLSearchParams({ csrf }),
  });
  expect(res.status).toBe(303);
  const r = await get(`/badge/${p.name}.svg`, { "if-none-match": etag });
  expect(r.status).toBe(200);
  expect(await r.text()).toContain("private");
});

test("case-insensitive name", async () => {
  const p = await project();
  const r = await get(`/badge/${p.name.toUpperCase()}.json`);
  expect(r.status).toBe(200);
  expect(await r.json()).toMatchObject({ message: "0.0 h" });
});

test("cache-control is public max-age=300", async () => {
  const p = await project();
  const r = await get(`/badge/${p.name}.svg`);
  expect(r.headers.get("cache-control")).toBe("public, max-age=300, stale-while-revalidate=3600");
  expect(r.headers.get("x-content-type-options")).toBe("nosniff");
  expect(r.headers.get("content-security-policy")).toBe("default-src 'none'");
});

test("old github name after rename still serves the Tally name's badge", async () => {
  const p = await project();
  const gid = rnd();
  await sql`insert into github_repos (id, full_name) values (${gid}, 'octo/new-name')`;
  await sql`insert into project_repos (project_id, github_repo_id) values (${p.id}, ${gid})`;
  await sql`insert into repo_events (github_repo_id, kind, detail) values (${gid}, 'renamed', 'octo/old-name -> octo/new-name')`;
  expect((await get(`/badge/${p.name}.svg`)).status).toBe(200);
  expect((await get("/badge/octo/old-name.svg")).status).toBe(404);
});

test("database error is 503 unavailable, no-store", async () => {
  const bad = createServer({
    cfg: testConfig(),
    sql: new Proxy(sql, {
      apply: () => Promise.reject(new Error("db down")),
    }) as typeof sql,
    port: 0,
  });
  try {
    const r = await fetch(`http://localhost:${bad.port}/badge/a/b.svg`);
    expect(r.status).toBe(503);
    expect(r.headers.get("cache-control")).toBe("no-store");
    expect(await r.text()).toContain("unavailable");
  } finally {
    bad.stop(true);
  }
});

test("rate limited per ip", async () => {
  const lim = createLimiter(1, 1);
  const s = createServer({ cfg: testConfig(), sql, port: 0, badgeLimiter: lim });
  try {
    const u = `http://localhost:${s.port}/badge/x/y.svg`;
    expect((await fetch(u)).status).toBe(404);
    const r = await fetch(u);
    expect(r.status).toBe(429);
    expect(r.headers.get("content-type")).toBe("image/svg+xml; charset=utf-8");
    expect(r.headers.get("cache-control")).toBe("no-store");
    expect(r.headers.get("retry-after")).toBe("60");
    expect(await r.text()).toContain("rate limited");
  } finally {
    s.stop(true);
  }
});

test("renderSvg escapes both strings and sizes by character count", () => {
  const s = renderSvg(`<a "x">`, "&<script>", "blue");
  expect(s).not.toContain("<script>");
  expect(s).toContain("&lt;script&gt;");
  expect(s).toContain("&quot;x&quot;");
  expect((s.match(/<rect/g) ?? []).length).toBe(2);
  expect((s.match(/<text/g) ?? []).length).toBe(2);
  expect(s).toContain(`width="${Math.ceil(7 * 6.5 + 20) + Math.ceil(9 * 6.5 + 20)}"`);
});

test("if-none-match: weak, listed and * all match; others get 200", async () => {
  const p = await project();
  const etag = (await get(`/badge/${p.name}.svg`)).headers.get("etag") ?? "";
  for (const inm of [`W/${etag}`, `"x", ${etag}`, "*", `"x",W/${etag} , "y"`])
    expect((await get(`/badge/${p.name}.svg`, { "if-none-match": inm })).status).toBe(304);
  for (const inm of [`"x"`, `"x", "y"`, `${etag.slice(0, -1)}9"`])
    expect((await get(`/badge/${p.name}.svg`, { "if-none-match": inm })).status).toBe(200);
});
