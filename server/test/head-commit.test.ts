import { afterAll, expect, test } from "bun:test";
import { connect } from "../src/db.ts";
import { chains, seed, seedFixture, start } from "./helpers.ts";

const GENESIS = "0".repeat(64);
const sql = connect(process.env.DATABASE_URL ?? "");
const srv = start(sql);
afterAll(async () => {
  srv.stop();
  await sql.close();
});

type P = { projectId: string; token: string };
const auth = (t: string) => ({ authorization: `Bearer ${t}`, "content-type": "application/json" });
const head = (p: P, q = "", token = p.token) =>
  fetch(`${srv.url}/v1/ledger/${p.projectId}/head${q}`, { headers: auth(token) });
const commit = (p: P, body: unknown, token = p.token) =>
  fetch(`${srv.url}/v1/ledger/${p.projectId}/commit`, {
    method: "POST",
    headers: auth(token),
    body: JSON.stringify(body),
  });

test("head of an empty project is genesis", async () => {
  const p = await seed(sql);
  const r = await head(p);
  expect(r.status).toBe(200);
  expect(await r.json()).toEqual({ head_hash: GENESIS, count: 0 });
});

test("head follows uploads", async () => {
  const c = chains["acme/demo-8"] ?? [];
  const p = await seedFixture(sql, "acme/demo-8");
  const up = await fetch(`${srv.url}/v1/ledger/${p.projectId}/entries`, {
    method: "POST",
    headers: auth(p.token),
    body: JSON.stringify({ prev_hash: GENESIS, entries: c.slice(0, 3) }),
  });
  expect(up.status).toBe(200);
  expect(await (await head(p)).json()).toEqual({ head_hash: c[2]?.hash, count: 3 });
});

test("head for an unknown ruleset is genesis/0", async () => {
  const p = await seed(sql);
  const r = await head(p, "?ruleset=2099.01-0");
  expect(await r.json()).toEqual({ head_hash: GENESIS, count: 0 });
});

test("head with an over-long ruleset is 400", async () => {
  const p = await seed(sql);
  const r = await head(p, `?ruleset=${"x".repeat(33)}`);
  expect(r.status).toBe(400);
  expect((await r.json()).error).toBe("invalid_body");
});

test("commit is recorded with receipt time", async () => {
  const p = await seed(sql);
  const r = await commit(p, { hash: "a".repeat(64), ts: 123 });
  expect(r.status).toBe(200);
  expect(await r.json()).toEqual({ ok: true });
  const rows =
    await sql`select hash, ts, received_at from commits where project_id = ${p.projectId}`;
  expect(rows.length).toBe(1);
  expect(rows[0].hash).toBe("a".repeat(64));
  expect(Number(rows[0].ts)).toBe(123);
  expect(Math.abs(Date.now() - new Date(rows[0].received_at).getTime())).toBeLessThan(60_000);
});

test("commit with a bad hash is 400", async () => {
  const p = await seed(sql);
  const r = await commit(p, { hash: "xyz", ts: 1 });
  expect(r.status).toBe(400);
  expect((await r.json()).error).toBe("invalid_body");
});

test("head and commit need the project's token", async () => {
  const p = await seed(sql);
  const other = await seed(sql);
  const body = { hash: "b".repeat(64), ts: 1 };
  expect((await fetch(`${srv.url}/v1/ledger/${p.projectId}/head`)).status).toBe(401);
  expect(
    (await fetch(`${srv.url}/v1/ledger/${p.projectId}/commit`, { method: "POST", body: "{}" }))
      .status,
  ).toBe(401);
  expect((await head(p, "", other.token)).status).toBe(403);
  expect((await commit(p, body, other.token)).status).toBe(403);
});
