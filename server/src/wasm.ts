// The nodejs wasm-pack target instantiates synchronously on import: loaded once per process.
import * as w from "../wasm/tally_scoring.js";

// ponytail: declared here until Task 4 moves it to schema.ts (inferred from the zod schema).
export type LedgerEntry = {
  v: number;
  ruleset: string;
  action_id: string;
  ts: number;
  project: string | null;
  kind: string;
  verified_s: number;
  unverified_s: number;
  reasons: string[];
  features: Record<string, number>;
  prev_hash: string;
  hash: string;
};

export type VerifyResult =
  | { ok: true }
  | { ok: false; index: number; reason: "bad_prev_hash" | "bad_hash" };

export function rulesetVersion(): string {
  return w.rulesetVersion();
}

/** Chain validity comes only from the Rust code; a thrown WASM error is a bug (schema validation runs first). */
export function verifyChain(entries: LedgerEntry[], prevHash: string): VerifyResult {
  const i = w.verifyChainJson(JSON.stringify(entries), prevHash);
  if (i < 0) return { ok: true };
  const expected = i === 0 ? prevHash : entries[i - 1]?.hash;
  return {
    ok: false,
    index: i,
    reason: entries[i]?.prev_hash !== expected ? "bad_prev_hash" : "bad_hash",
  };
}
