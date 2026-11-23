import { afterAll, expect, test } from "bun:test";
import { createServer } from "../src/app.ts";
import { MAX_BATCH, MAX_BODY_BYTES } from "../src/config.ts";
import { connect } from "../src/db.ts";
import { chains, seed, seedFixture, start } from "./helpers.ts";

// Fixture names are single-use per database. Tests that need a fresh chain own their name (demo-10,
// 12, 13, 14-big); the rest share names and are correct whether the name is fresh or already holds
// its first three entries: they upload those themselves (a replay is a no-op).

const GENESIS = "0".repeat(64);
const sql = connect(process.env.DATABASE_URL ?? "");
const srv = start(sql);
afterAll(async () => {
  srv.stop();
  await sql.close();
});

type P = { projectId: string; token: string };
const post = (p: P, body: unknown, token = p.token, url = srv.url) =>
  fetch(`${url}/v1/ledger/${p.projectId}/entries`, {
    method: "POST",
    headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
    body: typeof body === "string" ? body : JSON.stringify(body),
  });
const rows = async (p: P) =>
  Number(
    (
      await sql`select count(*) as n from ledger_entries le join chains ch on ch.id = le.chain_id
        where ch.project_id = ${p.projectId}`
    )[0].n,
  );
const ok = async (r: Response, head_hash: string, count: number) => {
  expect(r.status).toBe(200);
  expect(await r.json()).toEqual({ head_hash, count });
};
const broken = async (r: Response, index: number, reason: string) => {
  expect(r.status).toBe(422);
  expect(await r.json()).toEqual({ error: "broken_chain", index, reason });
};
const stale = async (r: Response, head_hash: string, count: number) => {
  expect(r.status).toBe(409);
  expect(await r.json()).toEqual({ error: "stale_prev_hash", head_hash, count });
};
const at = (name: string, i: number) => chains[name]?.[i]?.hash ?? "";

test("first batch from genesis is stored", async () => {
  const c = chains["acme/demo-10"] ?? [];
  const p = await seedFixture(sql, "acme/demo-10");
  await ok(await post(p, { prev_hash: GENESIS, entries: c.slice(0, 3) }), at("acme/demo-10", 2), 3);
  const [row] =
    await sql`select entry::text as e from ledger_entries le join chains ch on ch.id = le.chain_id where ch.project_id = ${p.projectId} and seq = 0`;
  expect(JSON.parse(row.e)).toEqual(c[0]); // stored as received
  expect(row.e).toBe(JSON.stringify(c[0])); // byte for byte
});

test("second batch appends against the head", async () => {
  const c = chains["acme/demo-2"] ?? [];
  const p = await seedFixture(sql, "acme/demo-2");
  await ok(await post(p, { prev_hash: GENESIS, entries: c.slice(0, 3) }), at("acme/demo-2", 2), 3);
  await ok(await post(p, { prev_hash: c[2]?.hash, entries: c.slice(3) }), at("acme/demo-2", 5), 6);
  const [ch] = await sql`select verified_s from chains where project_id = ${p.projectId}`;
  expect(Number(ch.verified_s)).toBe(c.reduce((s, e) => s + e.verified_s, 0));
});

test("replayed batch is a no-op 200", async () => {
  const c = chains["acme/demo-7"] ?? [];
  const p = await seedFixture(sql, "acme/demo-7");
  const body = { prev_hash: GENESIS, entries: c.slice(0, 3) };
  await ok(await post(p, body), at("acme/demo-7", 2), 3);
  await ok(await post(p, body), at("acme/demo-7", 2), 3);
  expect(await rows(p)).toBe(3);
});

test("overlapping batch appends only the new part", async () => {
  const c = chains["acme/demo-5"] ?? [];
  const p = await seedFixture(sql, "acme/demo-5");
  await ok(await post(p, { prev_hash: GENESIS, entries: c.slice(0, 3) }), at("acme/demo-5", 2), 3);
  await ok(await post(p, { prev_hash: GENESIS, entries: c.slice(0, 6) }), at("acme/demo-5", 5), 6);
  expect(await rows(p)).toBe(6);
});

test("valid alone but not against the head is 409", async () => {
  const c = chains["acme/demo-7"] ?? [];
  const p = await seedFixture(sql, "acme/demo-7");
  await ok(await post(p, { prev_hash: GENESIS, entries: c.slice(0, 3) }), at("acme/demo-7", 2), 3);
  await stale(
    await post(p, { prev_hash: c[3]?.hash, entries: c.slice(4) }),
    at("acme/demo-7", 2),
    3,
  );
});

test("different genesis batch on a non-empty chain is 409", async () => {
  const c = chains["acme/demo-7"] ?? [];
  const p = await seedFixture(sql, "acme/demo-7");
  await ok(await post(p, { prev_hash: GENESIS, entries: c.slice(0, 3) }), at("acme/demo-7", 2), 3);
  const alt = chains["acme/demo-7-alt"]?.slice(0, 3);
  await stale(await post(p, { prev_hash: GENESIS, entries: alt }), at("acme/demo-7", 2), 3);
  expect(await rows(p)).toBe(3);
});

test("racing uploaders: exactly one wins", async () => {
  const a = chains["acme/demo-4"] ?? [];
  const b = chains["acme/demo-4-alt"] ?? [];
  expect(b[2]?.hash).toBe(a[2]?.hash as string);
  expect(b[3]?.hash).not.toBe(a[3]?.hash as string);
  const p = await seedFixture(sql, "acme/demo-4");
  await ok(await post(p, { prev_hash: GENESIS, entries: a.slice(0, 3) }), at("acme/demo-4", 2), 3);
  const prev_hash = a[2]?.hash;
  const rs = await Promise.all([
    post(p, { prev_hash, entries: a.slice(3) }),
    post(p, { prev_hash, entries: b.slice(3) }),
  ]);
  expect(rs.map((r) => r.status).sort()).toEqual([200, 409]);
  const bodies = await Promise.all(rs.map((r) => r.json()));
  const winner = rs[0]?.status === 200 ? a : b;
  const won = { head_hash: winner[5]?.hash, count: 6 };
  expect(bodies).toContainEqual(won);
  expect(bodies).toContainEqual({ error: "stale_prev_hash", ...won });
  expect(await rows(p)).toBe(6);
});

test("broken chain is 422 with index and reason", async () => {
  const c = structuredClone(chains["acme/demo-7"] ?? []);
  const p = await seedFixture(sql, "acme/demo-7");
  const before = await rows(p);
  if (c[1]) c[1].verified_s += 1;
  await broken(await post(p, { prev_hash: GENESIS, entries: c }), 1, "bad_hash");
  expect(await rows(p)).toBe(before);
});

test("new chain at an unknown ruleset is 422", async () => {
  // Rust-hashed at ruleset 2099.01-0 (TS cannot rehash); demo-3 has no chain at that ruleset.
  const c = chains["acme/demo-3-2099"] ?? [];
  expect(c.every((e) => e.ruleset === "2099.01-0" && e.project === "acme/demo-3")).toBe(true);
  const p = await seedFixture(sql, "acme/demo-3");
  await broken(await post(p, { prev_hash: GENESIS, entries: c }), 0, "unknown_ruleset");
  const n =
    await sql`select 1 from chains where project_id = ${p.projectId} and ruleset = '2099.01-0'`;
  expect(n.length).toBe(0);
});

test("append to an existing older-ruleset chain succeeds", async () => {
  const c = chains["acme/demo-6"] ?? [];
  const p = await seedFixture(sql, "acme/demo-6");
  await ok(await post(p, { prev_hash: GENESIS, entries: c.slice(0, 3) }), at("acme/demo-6", 2), 3);
  const next = createServer({ sql, port: 0, ruleset: "2099.01-0" });
  const url = `http://localhost:${next.port}`;
  try {
    const r = await post(p, { prev_hash: c[2]?.hash, entries: c.slice(3) }, p.token, url);
    await ok(r, at("acme/demo-6", 5), 6);
    const [ch] =
      await sql`select verified_s from chains where project_id = ${p.projectId} and ruleset = '2026.11-0'`;
    expect(Number(ch.verified_s)).toBe(c.reduce((s, e) => s + e.verified_s, 0));
    // demo-3 has no 2026.11-0 chain: on this server that ruleset is old, so it cannot start one
    const fresh = await seedFixture(sql, "acme/demo-3");
    const d = chains["acme/demo-3"]?.slice(0, 3);
    const r2 = await post(fresh, { prev_hash: GENESIS, entries: d }, fresh.token, url);
    await broken(r2, 0, "unknown_ruleset");
    expect(await rows(fresh)).toBe(0);
  } finally {
    next.stop(true);
  }
});

test("mixed rulesets report the first foreign index", async () => {
  const c = structuredClone(chains["acme/demo-12"] ?? []);
  const p = await seedFixture(sql, "acme/demo-12");
  if (c[2]) c[2].ruleset = "2099.01-0";
  await broken(await post(p, { prev_hash: GENESIS, entries: c }), 2, "unknown_ruleset");
});

test("entry for another project is 422 project_mismatch", async () => {
  const p = await seedFixture(sql, "acme/demo-6");
  const r = await post(p, { prev_hash: GENESIS, entries: chains["acme/demo-5"] });
  await broken(r, 0, "project_mismatch");
  const nul = (chains["acme/demo-6"] ?? []).map((e) => ({ ...e, project: null }));
  await broken(await post(p, { prev_hash: GENESIS, entries: nul }), 0, "project_mismatch");
});

test("unknown field is 400 and nothing is stored", async () => {
  const c = chains["acme/demo-3"] ?? [];
  const p = await seedFixture(sql, "acme/demo-3");
  const r = await post(p, { prev_hash: GENESIS, entries: [{ ...c[0], extra: 1 }] });
  expect(r.status).toBe(400);
  expect((await r.json()).error).toBe("invalid_body");
  expect(await rows(p)).toBe(0);
});

test("1001 entries is 413 batch_too_large", async () => {
  const p = await seedFixture(sql, "acme/demo-3");
  const entries = Array(MAX_BATCH + 1).fill(chains["acme/demo-3"]?.[0]);
  const r = await post(p, { prev_hash: GENESIS, entries });
  expect(r.status).toBe(413);
  expect(await r.json()).toEqual({ error: "batch_too_large", max: MAX_BATCH });
});

test("body over 1 MiB is 413 body_too_large", async () => {
  const p = await seedFixture(sql, "acme/demo-3");
  const body = JSON.stringify({ prev_hash: GENESIS, entries: [], pad: "x".repeat(MAX_BODY_BYTES) });
  const r = await post(p, body);
  expect(r.status).toBe(413);
  expect(await r.json()).toEqual({ error: "body_too_large", max: MAX_BODY_BYTES });
});

test("no token 401, other project's token 403", async () => {
  const p = await seedFixture(sql, "acme/demo-3");
  const other = await seed(sql);
  const body = { prev_hash: GENESIS, entries: chains["acme/demo-3"]?.slice(0, 1) };
  const none = await fetch(`${srv.url}/v1/ledger/${p.projectId}/entries`, {
    method: "POST",
    body: JSON.stringify(body),
  });
  expect(none.status).toBe(401);
  expect(await none.json()).toEqual({ error: "unauthorized" });
  const r = await post(p, body, other.token);
  expect(r.status).toBe(403);
  expect(await r.json()).toEqual({ error: "forbidden" });
  expect(await rows(p)).toBe(0);
});

test("resume from mid-chain: overlap is verified, only the tail is appended", async () => {
  const c = chains["acme/demo-13"] ?? [];
  const p = await seedFixture(sql, "acme/demo-13");
  await ok(await post(p, { prev_hash: GENESIS, entries: c.slice(0, 4) }), at("acme/demo-13", 3), 4);
  await ok(await post(p, { prev_hash: c[1]?.hash, entries: c.slice(2) }), at("acme/demo-13", 5), 6);
  expect(await rows(p)).toBe(6);
});

test("verified_s beyond SQL integer is stored and summed", async () => {
  const c = chains["acme/demo-14-big"] ?? [];
  expect(c.some((e) => e.verified_s === 3_000_000_000)).toBe(true);
  const p = await seedFixture(sql, "acme/demo-14-big");
  await ok(await post(p, { prev_hash: GENESIS, entries: c }), at("acme/demo-14-big", 5), 6);
  const [ch] = await sql`select verified_s from chains where project_id = ${p.projectId}`;
  expect(Number(ch.verified_s)).toBe(c.reduce((s, e) => s + e.verified_s, 0));
});

test("invalid_body issue paths are capped", async () => {
  const p = await seedFixture(sql, "acme/demo-3");
  const r = await post(p, { prev_hash: GENESIS, entries: [], ["k".repeat(5000)]: 1 });
  expect(r.status).toBe(400);
  const text = await r.text();
  expect(text.length).toBeLessThan(500);
  expect(text).toContain("k".repeat(60));
});
