import { afterAll, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { connect } from "../src/db.ts";
import { rulesetVersion } from "../src/wasm.ts";
import { seed, start } from "./helpers.ts";

const GENESIS = "0".repeat(64);
const sql = connect(process.env.DATABASE_URL ?? "");
const srv = start(sql);
afterAll(async () => {
  srv.stop();
  await sql.close();
});

test("entries stamped beyond server now plus slack are rejected as future_ts", async () => {
  const p = await seed(sql);
  const e: Record<string, unknown> = {
    v: 1,
    ruleset: rulesetVersion(),
    action_id: "a0",
    ts: Date.now() + 86400000 * 365,
    project: p.publicName,
    kind: "prompt",
    verified_s: 3600,
    unverified_s: 0,
    reasons: [],
    features: {},
    prev_hash: GENESIS,
  };
  const { hash: _h, ...rest } = { hash: "", ...e };
  e.hash = createHash("sha256")
    .update(JSON.stringify(Object.fromEntries(Object.entries(rest).sort())))
    .digest("hex");
  const r = await fetch(`${srv.url}/v1/ledger/${p.projectId}/entries`, {
    method: "POST",
    headers: { authorization: `Bearer ${p.token}`, "content-type": "application/json" },
    body: JSON.stringify({ prev_hash: GENESIS, entries: [e] }),
  });
  expect(r.status).toBe(422);
  expect(await r.json()).toEqual({ error: "broken_chain", index: 0, reason: "future_ts" });
});
