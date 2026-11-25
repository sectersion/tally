import type { SQL } from "bun";
import { z } from "zod";
import { BUNDLE_MAX_BYTES, BUNDLE_MAX_COMPRESSED } from "./config.ts";
import { HttpError, parse } from "./errors.ts";
import { type Anchored, backfillFlag, type Flag, triage } from "./flags.ts";
import type { Storage } from "./storage.ts";
import { type RederivedClaim, rederive, rulesetVersion } from "./wasm.ts";

const RETENTION_DAYS = 14;
const tooBig = () =>
  new HttpError(413, { error: "bundle_too_large", max_compressed: BUNDLE_MAX_COMPRESSED });

/** Streams a gzip body, counting bytes on both sides so a zip bomb stops at the cap. Never logs content. */
export async function gunzipLimited(req: Request): Promise<Uint8Array> {
  if (Number(req.headers.get("content-length")) > BUNDLE_MAX_COMPRESSED) throw tooBig();
  if (!req.body) throw new HttpError(400, { error: "invalid_gzip" });
  let compressed = 0;
  const count = new TransformStream<Uint8Array, Uint8Array>({
    transform(chunk, c) {
      compressed += chunk.length;
      if (compressed > BUNDLE_MAX_COMPRESSED) throw tooBig();
      c.enqueue(chunk);
    },
  });
  const chunks: Uint8Array[] = [];
  let total = 0;
  try {
    const reader = req.body
      .pipeThrough(count)
      .pipeThrough(new DecompressionStream("gzip"))
      .getReader();
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      total += value.length;
      if (total > BUNDLE_MAX_BYTES) {
        await reader.cancel();
        throw new HttpError(413, { error: "bundle_too_large", max: BUNDLE_MAX_BYTES });
      }
      chunks.push(value);
    }
  } catch (e) {
    if (e instanceof HttpError) throw e;
    // the counting stage's own error surfaces here as a plain error: tell it apart by the counter
    if (compressed > BUNDLE_MAX_COMPRESSED) throw tooBig();
    throw new HttpError(400, { error: "invalid_gzip" });
  }
  return Buffer.concat(chunks);
}

const Turn = z
  .object({
    action_id: z.string().min(1).max(256),
    ts: z.number().int(),
    prompt: z.string().nullable(),
    reply: z.string().nullable(),
    missing: z.string().nullable(),
  })
  .strict();
export const BundleSchema = z
  .object({
    format: z.literal("tally-bundle/1"),
    ruleset: z.string().min(1).max(32),
    project: z.string().min(1).max(200),
    from_ms: z.number().int().nonnegative(),
    to_ms: z.number().int().nonnegative(),
    created_ms: z.number().int(),
    client: z.string().max(200),
    timeline: z.array(z.record(z.string(), z.unknown())),
    excluded_action_ids: z.array(z.string().min(1).max(256)),
    turns: z.array(Turn),
    redactions: z.record(z.string().max(64), z.number().int().nonnegative()),
  })
  .strict()
  .refine((b) => b.from_ms <= b.to_ms);
type Bundle = z.infer<typeof BundleSchema>;

const bad = (error: string, extra: Record<string, unknown> = {}) =>
  new HttpError(422, { error, ...extra });

type Key = { ts: number; action_id: string };
const cmp = (a: Key, b: Key) =>
  a.ts - b.ts || (a.action_id < b.action_id ? -1 : a.action_id > b.action_id ? 1 : 0);
const tuple = (c: RederivedClaim) => [c.action_id, c.ts, c.verified_s, c.unverified_s].join("|");

function derive(b: Bundle, excluded: string[]): RederivedClaim[] {
  try {
    return rederive({
      ruleset: b.ruleset,
      project: b.project,
      from_ms: b.from_ms,
      to_ms: b.to_ms,
      events: b.timeline,
      excluded_action_ids: excluded,
      review: {},
    }).claims.sort(cmp);
  } catch {
    throw bad("invalid_timeline"); // the Rust message can quote input: not forwarded
  }
}

/** Review Focus 7: the uploaded timeline must reproduce the stored ledger for the range. */
export async function checkAgainstLedger(sql: SQL, projectId: string, b: Bundle): Promise<void> {
  const rows = await sql`select e.action_id, e.ts, e.verified_s, e.unverified_s
    from ledger_entries e join chains c on c.id = e.chain_id
    where c.project_id = ${projectId} and e.ts between ${b.from_ms} and ${b.to_ms}`;
  const S: RederivedClaim[] = rows
    .map((r: Record<string, unknown>) => ({
      action_id: String(r.action_id),
      ts: Number(r.ts),
      verified_s: Number(r.verified_s),
      unverified_s: Number(r.unverified_s),
    }))
    .sort(cmp);
  const stored = new Set(S.map((s) => s.action_id));
  // (a) claims the ledger lacks must be exactly the listed exclusions
  const missing = new Set(
    derive(b, [])
      .map((c) => c.action_id)
      .filter((id) => !stored.has(id)),
  );
  const excluded = new Set(b.excluded_action_ids);
  for (const id of [...missing, ...excluded])
    if (missing.has(id) !== excluded.has(id)) throw bad("rederive_mismatch", { action_id: id });
  // (b) everything else must match the ledger exactly, in order
  const C = derive(b, b.excluded_action_ids);
  for (let i = 0; i < Math.max(C.length, S.length); i++) {
    const c = C[i];
    const s = S[i];
    if (!c || !s || tuple(c) !== tuple(s))
      throw bad("rederive_mismatch", { action_id: (c ?? s)?.action_id });
  }
}

/** Server-side evidence per ledger entry: its receipt, or the earliest commit of a head at or after it. */
async function anchors(sql: SQL, projectId: string, b: Bundle): Promise<Anchored[]> {
  // ponytail: correlated subquery per entry; fine for one submission range, index commits by hash if slow
  const rows = await sql`select e.ts, e.verified_s,
      extract(epoch from least(e.received_at, coalesce((
        select min(c.received_at) from commits c
        join ledger_entries h on h.hash = c.hash and h.chain_id = e.chain_id
        where c.project_id = ${projectId} and h.seq >= e.seq), e.received_at))) * 1000 as anchor_ms
    from ledger_entries e join chains ch on ch.id = e.chain_id
    where ch.project_id = ${projectId} and e.ts between ${b.from_ms} and ${b.to_ms}`;
  return rows.map((r: Record<string, unknown>) => ({
    ts: Number(r.ts),
    verified_s: Number(r.verified_s),
    anchor_ms: Number(r.anchor_ms),
  }));
}

export const audit = (
  sql: SQL,
  actor: string,
  action: string,
  submissionId: string | null,
  detail: Record<string, unknown> = {},
) =>
  sql`insert into audit_log (actor, action, submission_id, detail)
    values (${actor}, ${action}, ${submissionId}, ${JSON.stringify(detail)}::text::jsonb)`;

export async function uploadSubmission(
  sql: SQL,
  storage: Storage,
  req: Request,
  projectId: string,
  actor: string,
): Promise<{ id: string; status: string; risk: number; flags: Flag[] }> {
  const raw = await gunzipLimited(req);
  let parsed: unknown;
  try {
    parsed = JSON.parse(new TextDecoder().decode(raw));
  } catch {
    throw new HttpError(400, { error: "invalid_json" });
  }
  let b: Bundle;
  try {
    b = parse(BundleSchema, parsed);
  } catch (e) {
    if (e instanceof HttpError) throw new HttpError(422, e.body);
    throw e;
  }
  if (b.ruleset !== rulesetVersion()) throw bad("invalid_bundle", { reason: "ruleset" });
  const [p] = await sql`select public_name, owner_id from projects where id = ${projectId}`;
  if (!p) throw new HttpError(404, { error: "not_found" });
  if (b.project !== p.public_name) throw bad("invalid_bundle", { reason: "project" });
  if (!p.owner_id) throw new HttpError(403, { error: "no_owner" });
  const [open] = await sql`select 1 as x from submissions where project_id = ${projectId}
    and status in ('uploaded','judging','in_review')`;
  if (open) throw new HttpError(409, { error: "open_submission" });
  const [done] = await sql`select max(to_ms) as t from submissions
    where project_id = ${projectId} and status = 'completed'`;
  if (done?.t != null && b.from_ms < Number(done.t))
    throw new HttpError(409, { error: "overlap", overlap_until: Number(done.t) });
  await checkAgainstLedger(sql, projectId, b);

  const t = triage(b.timeline);
  const late = backfillFlag(await anchors(sql, projectId, b));
  const flags = late ? [...t.flags, late] : t.flags;
  const risk = t.risk + (late?.weight ?? 0);
  const id = crypto.randomUUID();
  const objectKey = crypto.randomUUID();
  await storage.put(objectKey, new TextEncoder().encode(JSON.stringify(b.turns)));
  try {
    await sql.begin(async (tx) => {
      await tx`insert into submissions (id, project_id, user_id, from_ms, to_ms, ruleset, status,
          timeline, excluded_action_ids, object_key, flags, risk)
        values (${id}, ${projectId}, ${p.owner_id}, ${b.from_ms}, ${b.to_ms}, ${b.ruleset}, 'uploaded',
          ${JSON.stringify(b.timeline)}::text::jsonb, ${JSON.stringify(b.excluded_action_ids)}::text::jsonb,
          ${objectKey}, ${JSON.stringify(flags)}::text::jsonb, ${risk})`;
      await audit(tx, actor, "submission.uploaded", id, {
        bytes: raw.length,
        turns: b.turns.length,
        risk,
        flags: flags.map((f) => f.code),
      });
    });
  } catch (e) {
    await storage.del(objectKey).catch(() => {});
    const x = e as { errno?: unknown; code?: unknown };
    if (x.errno === "23505" || x.code === "23505")
      throw new HttpError(409, { error: "open_submission" });
    throw e;
  }
  return { id, status: "uploaded", risk, flags };
}

export async function listSubmissions(sql: SQL, projectId: string) {
  const rows =
    await sql`select id, status, from_ms, to_ms, reviewed_verified_s, risk, flags from submissions
    where project_id = ${projectId} order by created_at desc`;
  return rows.map((r: Record<string, unknown>) => ({
    id: r.id,
    status: r.status,
    from_ms: Number(r.from_ms),
    to_ms: Number(r.to_ms),
    reviewed_verified_s: r.reviewed_verified_s == null ? null : Number(r.reviewed_verified_s),
    risk: Number(r.risk),
    flags: r.flags,
  }));
}

/** The project a submission belongs to, or null. */
export async function submissionProject(sql: SQL, sid: string): Promise<string | null> {
  if (!/^[0-9a-f-]{36}$/i.test(sid)) return null;
  const [r] = await sql`select project_id from submissions where id = ${sid}`;
  return r ? (r.project_id as string) : null;
}

/** Deletes the object now; the row, labels and timeline stay. A completed submission stays completed. */
export async function deleteSubmission(sql: SQL, storage: Storage, sid: string, actor: string) {
  await sql.begin(async (tx) => {
    const [r] = await tx`select object_key from submissions where id = ${sid} for update`;
    if (r?.object_key) await storage.del(r.object_key);
    await tx`update submissions set object_key = null,
      object_deleted_at = coalesce(object_deleted_at, now()),
      status = case when status = 'completed' then status else 'withdrawn' end where id = ${sid}`;
    await audit(tx, actor, "submission.deleted_by_owner", sid);
  });
}

/** Retention: objects of finished submissions, and of anything older than 14 days. Returns the count. */
export async function sweepBundles(sql: SQL, storage: Storage, now = new Date()): Promise<number> {
  const cutoff = new Date(now.getTime() - RETENTION_DAYS * 86_400_000);
  const rows = await sql`select id, object_key from submissions where object_key is not null
    and (status in ('completed','withdrawn') or created_at < ${cutoff})`;
  for (const r of rows) {
    await storage.del(r.object_key);
    await sql.begin(async (tx) => {
      await tx`update submissions set object_key = null, object_deleted_at = ${now} where id = ${r.id}`;
      await audit(tx, "system", "submission.object_expired", r.id);
    });
  }
  return rows.length;
}
