import { expect, test } from "bun:test";
import type { SQL } from "bun";
import { connect } from "../src/db.ts";
import { memoryStorage } from "../src/storage.ts";
import { uploadSubmission } from "../src/submissions.ts";
import { type RederiveInput, rederive, rulesetVersion } from "../src/wasm.ts";
import { seed } from "./helpers.ts";

const sql: SQL = connect(process.env.DATABASE_URL ?? "");
const T0 = 1_700_000_000_000;
const HOUR = 3_600_000;

const ev = (project: string, id: string, s: number, kind: Record<string, unknown>) => ({
  v: 1,
  id,
  ts: T0 + s * 1000,
  source: "test",
  project,
  session: "s",
  ...kind,
});
const prompt = { kind: "prompt", chars: 200, relevance_pm: 600, novelty_bytes: 100 };
const timeline = (project: string) => [
  ev(project, "a", 0, prompt),
  ev(project, "t", 60, { kind: "tool_decision", decision: "approve" }),
  ev(project, "b", 300, prompt),
];

/** A project whose stored ledger is exactly what the timeline re-scores to, each entry received `lagMs` late. */
async function setup(lagMs: number) {
  const { projectId, publicName } = await seed(sql);
  const uid = crypto.randomUUID();
  await sql`insert into users (id, hca_sub) values (${uid}, ${uid})`;
  await sql`update projects set owner_id = ${uid} where id = ${projectId}`;
  const events = timeline(publicName);
  const input: RederiveInput = {
    ruleset: rulesetVersion(),
    project: publicName,
    from_ms: T0 - 1000,
    to_ms: T0 + HOUR,
    events,
    excluded_action_ids: [],
    review: {},
  };
  const { claims } = rederive(input);
  const [ch] =
    await sql`insert into chains (project_id, ruleset, head_hash) values (${projectId}, ${rulesetVersion()}, ${"0".repeat(64)}) returning id`;
  for (const [seq, c] of claims.entries()) {
    await sql`insert into ledger_entries (chain_id, seq, hash, prev_hash, action_id, ts, kind, verified_s, unverified_s, entry, received_at)
      values (${ch.id}, ${seq}, ${`h${seq}-${projectId}`}, ${`h${seq - 1}`}, ${c.action_id}, ${c.ts}, 'claim', ${c.verified_s}, ${c.unverified_s},
        '{}'::text::json, ${new Date(c.ts + lagMs)})`;
  }
  return {
    projectId,
    publicName,
    events,
    claims,
    chainId: ch.id as number,
    from_ms: input.from_ms,
    to_ms: input.to_ms,
  };
}

async function upload(s: Awaited<ReturnType<typeof setup>>, timelineOverride?: unknown[]) {
  const bundle = {
    format: "tally-bundle/1",
    ruleset: rulesetVersion(),
    project: s.publicName,
    from_ms: s.from_ms,
    to_ms: s.to_ms,
    created_ms: T0 + 2 * HOUR,
    client: "test",
    timeline: timelineOverride ?? s.events,
    excluded_action_ids: [],
    turns: [],
    redactions: {},
  };
  const body = Bun.gzipSync(JSON.stringify(bundle));
  const req = new Request("http://x/", { method: "POST", body });
  return uploadSubmission(sql, memoryStorage(), req, s.projectId, "test");
}

test("honest upload: no flags, stored on the row and in the audit log", async () => {
  const s = await setup(10 * 60_000);
  const r = await upload(s);
  expect(r).toMatchObject({ status: "uploaded", risk: 0, flags: [] });
  const [row] = await sql`select risk, flags from submissions where id = ${r.id}`;
  expect(row).toMatchObject({ risk: 0, flags: [] });
  const [a] = await sql`select detail from audit_log where submission_id = ${r.id}`;
  expect(a.detail).toMatchObject({ risk: 0, flags: [] });
});

test("backfilled: entries that reached the server days after they happened are flagged", async () => {
  const s = await setup(3 * 24 * HOUR);
  const r = await upload(s);
  expect(r.flags.map((f) => f.code)).toEqual(["backfilled"]);
  expect(r.risk).toBe(50);
  const [row] = await sql`select risk, flags from submissions where id = ${r.id}`;
  expect(row.risk).toBe(50);
  expect(row.flags[0].code).toBe("backfilled");
});

test("a head committed on time anchors earlier entries even when they were uploaded late", async () => {
  const s = await setup(3 * 24 * HOUR);
  const last = s.claims.length - 1;
  // the daemon committed its head 10 minutes after the last claim; entries were uploaded much later
  await sql`insert into commits (project_id, hash, ts, received_at)
    values (${s.projectId}, ${`h${last}-${s.projectId}`}, ${T0}, ${new Date((s.claims[last]?.ts ?? T0) + 10 * 60_000)})`;
  const r = await upload(s);
  expect(r.flags).toEqual([]);
});

test("a timeline kinder than the ledger is refused before any flag is stored", async () => {
  const s = await setup(60_000);
  const kinder = s.events.filter((e) => e.id !== "t");
  await expect(upload(s, kinder)).rejects.toMatchObject({ body: { error: "rederive_mismatch" } });
  const [n] =
    await sql`select count(*)::int as n from submissions where project_id = ${s.projectId}`;
  expect(n.n).toBe(0);
});
