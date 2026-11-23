import { afterAll, expect, test } from "bun:test";
import { connect } from "../src/db.ts";
import { badge } from "../src/ledger.ts";
import { chains, seed, seedFixture, start } from "./helpers.ts";

const GENESIS = "0".repeat(64);
const OLD = "2025.01-0";
const sql = connect(process.env.DATABASE_URL ?? "");
const srv = start(sql);
afterAll(async () => {
  srv.stop();
  await sql.close();
});

const auth = (t: string) => ({ authorization: `Bearer ${t}`, "content-type": "application/json" });
const get = (id: string) => fetch(`${srv.url}/v1/projects/${id}/badge.json`);

test("badge formatting floors to a tenth", () => {
  expect(badge(0).message).toBe("0.0 h");
  expect(badge(359).message).toBe("0.0 h");
  expect(badge(360).message).toBe("0.1 h");
  expect(badge(5_399).message).toBe("1.4 h");
});

test("badge is the sum of verified seconds across chains", async () => {
  const c = chains["acme/demo-9"] ?? [];
  const p = await seedFixture(sql, "acme/demo-9");
  const up = await fetch(`${srv.url}/v1/ledger/${p.projectId}/entries`, {
    method: "POST",
    headers: auth(p.token),
    body: JSON.stringify({ prev_hash: GENESIS, entries: c }),
  });
  expect(up.status).toBe(200);
  const s = c.reduce((n, e) => n + e.verified_s, 0);
  await sql`insert into chains (project_id, ruleset, head_hash, count, verified_s)
    values (${p.projectId}, ${OLD}, ${GENESIS}, 0, 3600)`;
  const r = await get(p.projectId);
  expect(r.status).toBe(200);
  expect(await r.json()).toEqual({
    schemaVersion: 1,
    label: "tracked with Tally",
    message: badge(s + 3600).message,
    color: "blue",
  });
  // the old-ruleset chain does not disturb the current head
  const h = await fetch(`${srv.url}/v1/ledger/${p.projectId}/head`, { headers: auth(p.token) });
  expect(await h.json()).toEqual({ head_hash: c[5]?.hash, count: 6 });
});

test("chains.verified_s matches the stored entries", async () => {
  const c = chains["acme/demo-11"] ?? [];
  const p = await seedFixture(sql, "acme/demo-11");
  const up = await fetch(`${srv.url}/v1/ledger/${p.projectId}/entries`, {
    method: "POST",
    headers: auth(p.token),
    body: JSON.stringify({ prev_hash: GENESIS, entries: c }),
  });
  expect(up.status).toBe(200);
  const [r] = await sql`select c.verified_s as cv, (select sum(e.verified_s) from ledger_entries e
    where e.chain_id = c.id) as ev from chains c where c.project_id = ${p.projectId}`;
  expect(Number(r.cv)).toBe(Number(r.ev));
  expect(Number(r.cv)).toBeGreaterThan(0);
});

test("unknown project badge is 404", async () => {
  const r = await get("nope");
  expect(r.status).toBe(404);
  expect(await r.json()).toEqual({ error: "not_found" });
});

test("badge needs no token and is cacheable", async () => {
  const p = await seed(sql);
  const r = await get(p.projectId);
  expect(r.status).toBe(200);
  expect(r.headers.get("cache-control")).toBe("public, max-age=300");
  expect((await r.json()).message).toBe("0.0 h");
});
