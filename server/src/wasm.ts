// The nodejs wasm-pack target instantiates synchronously on import: loaded once per process.
import * as w from "../wasm/tally_scoring.js";
import type { LedgerEntry } from "./schema";

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

export type RederivedClaim = {
  action_id: string;
  ts: number;
  verified_s: number;
  unverified_s: number;
};
export type RederiveInput = {
  ruleset: string;
  project: string;
  from_ms: number;
  to_ms: number;
  events: unknown[];
  excluded_action_ids: string[];
  review: Record<string, never>;
};

/** Re-scores a timeline (Rust `rederive`). Throws on a malformed event or a foreign ruleset. */
export function rederive(input: RederiveInput): { claims: RederivedClaim[] } {
  return JSON.parse(w.rederiveJson(JSON.stringify(input)));
}
