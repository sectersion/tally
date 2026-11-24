import { expect, test } from "bun:test";
import { scoreJson, verifyChainJson, rulesetVersion } from "../crates/tally-scoring/pkg/tally_scoring.js";

test("ruleset version", () => expect(rulesetVersion()).toBe("2026.11-1"));
test("lunch scenario matches native", () => {
  const ev = (id: string, ts: number, kind: object) => ({ v: 1, id, ts: ts * 1000, source: "t", project: "p", session: null, ...kind });
  const p = { kind: "prompt", chars: 200, relevance_pm: 600, novelty_bytes: 90 };
  const claims = JSON.parse(scoreJson(JSON.stringify([ev("1", 0, p), ev("2", 60, { kind: "idle", end_ts: 1_860_000 }), ev("3", 1900, p)])));
  expect(claims.map((c: any) => c.verified_s)).toEqual([120, 540]);
});
test("bad input throws", () => expect(() => scoreJson("not json")).toThrow());
test("empty chain is valid", () => expect(verifyChainJson("[]", "0".repeat(64))).toBe(-1));
