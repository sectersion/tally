import type { SQL } from "bun";
import { HttpError } from "./errors.ts";

const MAX_ENTRIES = 100_000;

export async function exportLedger(
  sql: SQL,
  projectId: string,
  publicName: string,
): Promise<Response> {
  const [count] =
    await sql`select count(*)::int as n from ledger_entries e join chains c on c.id = e.chain_id where c.project_id = ${projectId}`;
  if (count.n > MAX_ENTRIES) throw new HttpError(413, { error: "too_large" });

  // The chain's internal id is the only ordering it has; it is exposed as `seq` and never as id.
  const chains = await sql`select id, verified_s, unverified_s, ruleset
    from chains where project_id = ${projectId} order by id`;
  const entries = await sql`select c.id as chain_id, e.seq, e.hash, e.prev_hash, e.ts, e.kind,
      e.verified_s, e.unverified_s, e.entry
    from ledger_entries e join chains c on c.id = e.chain_id
    where c.project_id = ${projectId} order by c.id, e.seq`;
  const byChain = new Map<string, Array<Record<string, unknown>>>();
  for (const e of entries) {
    let list = byChain.get(e.chain_id);
    if (!list) {
      list = [];
      byChain.set(e.chain_id, list);
    }
    list.push({
      seq: Number(e.seq),
      hash: e.hash,
      prev_hash: e.prev_hash,
      ts: Number(e.ts),
      kind: e.kind,
      verified_s: e.verified_s,
      unverified_s: e.unverified_s,
      entry: e.entry,
    });
  }

  const body = {
    project: publicName,
    chains: chains.map(
      (
        c: {
          id: string;
          head_hash: string;
          verified_s: number;
          unverified_s: number;
          ruleset: string;
        },
        i: number,
      ) => ({
        seq: i,
        hash: c.head_hash,
        prev_hash: null,
        verified_s: c.verified_s,
        unverified_s: c.unverified_s,
        ruleset: c.ruleset,
        entries: byChain.get(c.id) ?? [],
      }),
    ),
  };
  return new Response(JSON.stringify(body), {
    headers: {
      "content-type": "application/json",
      "content-disposition": `attachment; filename="${publicName.replaceAll("/", "-->")}.json"`,
      "cache-control": "no-store",
    },
  });
}
