import { expect, test } from "bun:test";
import { rulesetVersion, verifyChain } from "../src/wasm.ts";

const chain = (await Bun.file(new URL("./fixtures/chain.json", import.meta.url)).json())[
  "acme/demo-1"
];
const GENESIS = "0".repeat(64);

test("ruleset is the crate's", () => expect(rulesetVersion()).toBe("2026.11-1"));
test("fixture verifies whole and in two parts", () => {
  expect(verifyChain(chain, GENESIS)).toEqual({ ok: true });
  expect(verifyChain(chain.slice(3), chain[2].hash)).toEqual({ ok: true });
});
test("tampered entry reports bad_hash at its index", () => {
  const t = structuredClone(chain);
  t[2].verified_s += 1;
  expect(verifyChain(t, GENESIS)).toEqual({ ok: false, index: 2, reason: "bad_hash" });
});
test("wrong prev_hash reports bad_prev_hash at 0", () =>
  expect(verifyChain(chain.slice(3), GENESIS)).toEqual({
    ok: false,
    index: 0,
    reason: "bad_prev_hash",
  }));
test("dropped middle entry reports bad_prev_hash", () =>
  expect(verifyChain([chain[0], chain[2]], GENESIS)).toEqual({
    ok: false,
    index: 1,
    reason: "bad_prev_hash",
  }));
