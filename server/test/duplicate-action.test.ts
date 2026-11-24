import { afterAll, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { connect } from "../src/db.ts";
import { rulesetVersion } from "../src/wasm.ts";
import { seed, start } from "./helpers.ts";

const sql = connect(process.env.DATABASE_URL ?? "");
const srv = start(sql);
afterAll(async () => {
  srv.stop();
  await sql.close();
});
const G = "0".repeat(64);

// Mirrors the spec's canonical form: sorted keys, hash excluded.
function mk(project: string, prev: string, ids: string[]) {
  return ids.map((action_id) => {
    const e: Record<string, unknown> = {
      action_id,
      features: {},
      kind: "prompt",
      prev_hash: prev,
      project,
      reasons: [],
      ruleset: rulesetVersion(),
      ts: 1000,
      unverified_s: 0,
      v: 1,
      verified_s: 60,
    };
    e.hash = createHash("sha256").update(JSON.stringify(e)).digest("hex");
    prev = e.hash as string;
    return e;
  });
}

test("a repeated action_id is rejected, in one batch and across batches", async () => {
  const name = `qa/${crypto.randomUUID().slice(0, 8)}`;
  const p = await seed(sql, name);
  const post = (prev_hash: string, entries: unknown[]) =>
    fetch(`${srv.url}/v1/ledger/${p.projectId}/entries`, {
      method: "POST",
      headers: { authorization: `Bearer ${p.token}`, "content-type": "application/json" },
      body: JSON.stringify({ prev_hash, entries }),
    });
  const inBatch = await post(G, mk(name, G, ["a", "a"]));
  expect(inBatch.status).toBe(422);
  expect(await inBatch.json()).toEqual({
    error: "broken_chain",
    index: 1,
    reason: "duplicate_action",
  });

  const first = mk(name, G, ["a"]);
  expect((await post(G, first)).status).toBe(200);
  const head = first[0]?.hash as string;
  const later = await post(head, mk(name, head, ["a"]));
  expect(later.status).toBe(422);
  expect(((await later.json()) as { reason: string }).reason).toBe("duplicate_action");
});
