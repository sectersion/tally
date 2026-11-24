import type { SQL } from "bun";
import { HttpError } from "./errors.ts";
import { bumpChange } from "./projects.ts";
import type { LedgerEntry } from "./schema.ts";
import { rulesetVersion, verifyChain } from "./wasm.ts";

export const GENESIS = "0".repeat(64);

export type Head = { head_hash: string; count: number };

/** The chain's head for a ruleset; a project with no chain row yet is at genesis. */
export async function getHead(
  sql: SQL,
  projectId: string,
  ruleset: string = rulesetVersion(),
): Promise<Head> {
  const [ch] =
    await sql`select head_hash, count from chains where project_id = ${projectId} and ruleset = ${ruleset}`;
  return ch
    ? { head_hash: ch.head_hash, count: Number(ch.count) }
    : { head_hash: GENESIS, count: 0 };
}

/** Verified seconds over all of a project's chains (every ruleset); null if the project does not exist. */
export async function verifiedSeconds(sql: SQL, projectId: string): Promise<number | null> {
  const [r] =
    await sql`select (select coalesce(sum(verified_s), 0) from chains where project_id = p.id) as s
    from projects p where p.id = ${projectId}`;
  return r ? Number(r.s) : null;
}

/** "12.3 h": floors to a tenth of an hour so it never exceeds verified time. */
export const formatHours = (seconds: number) => `${(Math.floor(seconds / 360) / 10).toFixed(1)} h`;

/** shields.io endpoint JSON. */
export function badge(seconds: number) {
  return {
    schemaVersion: 1,
    label: "tracked with Tally",
    message: formatHours(seconds),
    color: "blue",
  };
}

/** Records a daemon's committed head. The hash need not be stored yet: received_at is the evidence. */
export async function recordCommit(
  sql: SQL,
  projectId: string,
  c: { hash: string; ts: number },
): Promise<void> {
  await sql`insert into commits (project_id, hash, ts) values (${projectId}, ${c.hash}, ${c.ts})`;
}

/** Pure batch checks, first failure wins: foreign ruleset, foreign project, repeated action_id, then the Rust chain check. */
export function checkBatch(
  entries: LedgerEntry[],
  prevHash: string,
  publicName: string,
  ruleset: string,
): { index: number; reason: string } | null {
  const r = entries.findIndex((e) => e.ruleset !== ruleset);
  if (r >= 0) return { index: r, reason: "unknown_ruleset" };
  const p = entries.findIndex((e) => e.project !== publicName);
  if (p >= 0) return { index: p, reason: "project_mismatch" };
  const seen = new Set<string>();
  const d = entries.findIndex((e) => seen.size === seen.add(e.action_id).size);
  if (d >= 0) return { index: d, reason: "duplicate_action" };
  const v = verifyChain(entries, prevHash);
  return v.ok ? null : { index: v.index, reason: v.reason };
}

/**
 * Appends a batch that already passed `checkBatch`. The chain row lock serializes uploaders; a
 * batch overlapping stored entries must match them exactly (replays are no-ops).
 */
export async function appendEntries(
  sql: SQL,
  projectId: string,
  body: { prev_hash: string; entries: LedgerEntry[] },
  ruleset: string = rulesetVersion(),
): Promise<Head> {
  const { prev_hash, entries } = body;
  const R = (entries[0] as LedgerEntry).ruleset;
  return sql
    .begin(async (tx) => {
      if (R === ruleset)
        await tx`insert into chains (project_id, ruleset, head_hash) values (${projectId}, ${R}, ${GENESIS})
        on conflict do nothing`;
      const [ch] =
        await tx`select id, head_hash, count from chains where project_id = ${projectId} and ruleset = ${R} for update`;
      if (!ch)
        throw new HttpError(422, { error: "broken_chain", index: 0, reason: "unknown_ruleset" });
      const count = Number(ch.count);
      const head: Head = { head_hash: ch.head_hash, count };
      const stale = new HttpError(409, { error: "stale_prev_hash", ...head });

      let start = count;
      if (prev_hash !== head.head_hash) {
        if (prev_hash === GENESIS) start = 0;
        else {
          const [row] =
            await tx`select seq from ledger_entries where chain_id = ${ch.id} and hash = ${prev_hash}`;
          if (!row) throw stale;
          start = Number(row.seq) + 1;
        }
      }

      const overlap = Math.min(count, start + entries.length) - start;
      if (overlap > 0) {
        const stored = await tx`select hash from ledger_entries where chain_id = ${ch.id}
        and seq >= ${start} and seq < ${start + overlap} order by seq`;
        if (stored.some((s: { hash: string }, k: number) => s.hash !== entries[k]?.hash))
          throw stale;
      }

      const fresh = entries.slice(overlap);
      if (fresh.length === 0) return head;
      const col = <T>(f: (e: LedgerEntry, i: number) => T) => fresh.map(f);
      // unnest + text[]::json[] keeps the entry text verbatim (Bun JSON-encodes strings bound to json)
      await tx`insert into ledger_entries
        (chain_id, seq, hash, prev_hash, action_id, ts, kind, verified_s, unverified_s, entry)
      select ${ch.id}, * from unnest(
        ${col((_, i) => count + i)}::bigint[], ${col((e) => e.hash)}::text[],
        ${col((e) => e.prev_hash)}::text[], ${col((e) => e.action_id)}::text[],
        ${col((e) => e.ts)}::bigint[], ${col((e) => e.kind)}::text[],
        ${col((e) => e.verified_s)}::bigint[], ${col((e) => e.unverified_s)}::bigint[],
        ${col((e) => JSON.stringify(e))}::text[]::json[])`;
      const last = fresh[fresh.length - 1] as LedgerEntry;
      const added = fresh.reduce((s, e) => s + e.verified_s, 0);
      const [u] =
        await tx`update chains set head_hash = ${last.hash}, count = ${count + fresh.length},
        verified_s = verified_s + ${added} where id = ${ch.id} returning head_hash, count`;
      await bumpChange(tx, projectId);
      return { head_hash: u.head_hash, count: Number(u.count) };
    })
    .catch((e) => {
      // unique (chain_id, action_id): a claim already on the chain, re-chained in a later batch
      if (e?.errno === "23505" && e?.constraint === "ledger_entries_chain_action")
        throw new HttpError(422, { error: "broken_chain", index: 0, reason: "duplicate_action" });
      throw e;
    });
}
