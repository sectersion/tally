import { expect, test } from "bun:test";
import { triage } from "../src/flags";

const MIN = 60_000;
const prompt = (ts: number, chars = 80) => ({ kind: "prompt", ts, chars });
const codes = (t: object[]) => triage(t as Record<string, unknown>[]).flags.map((f) => f.code);

// Irregular, varied prompts like a person: no flags.
const person = Array.from({ length: 20 }, (_, i) =>
  prompt(i * 7 * MIN + ((i * 37) % 11) * 40_000, 30 + ((i * 53) % 120)),
);

test("a normal session has no flags", () => {
  expect(triage(person).flags).toEqual([]);
  expect(triage(person).risk).toBe(0);
});

test("robotic_cadence: prompts exactly 4 min apart", () => {
  const bot = Array.from({ length: 12 }, (_, i) => prompt(i * 4 * MIN));
  expect(codes(bot)).toContain("robotic_cadence");
  expect(codes(bot.slice(0, 5))).not.toContain("robotic_cadence"); // too few to judge
});

test("low_effort_prompts: mostly 'continue'", () => {
  const farm = Array.from({ length: 12 }, (_, i) =>
    prompt(i * (3 + (i % 5)) * MIN, i % 4 ? 8 : 90),
  );
  expect(codes(farm)).toContain("low_effort_prompts");
});

test("superhuman_typing: 200 chars two seconds after the agent stops, three times", () => {
  const t: object[] = [];
  for (let i = 0; i < 3; i++) {
    const end = i * 10 * MIN + 5 * MIN;
    t.push({ kind: "agent_busy", ts: end - 4 * MIN, end_ts: end }, prompt(end + 2000, 200));
  }
  expect(codes(t)).toContain("superhuman_typing");
  expect(codes(t.slice(0, 2))).not.toContain("superhuman_typing");
});

test("long_day: a human action in 16 of 24 hours", () => {
  const t = Array.from({ length: 16 }, (_, i) => ({ kind: "user_edit", ts: i * 3_600_000 + 1000 }));
  expect(codes(t)).toContain("long_day");
  expect(codes(t.slice(0, 10))).not.toContain("long_day");
});

test("risk is the sum of weights", () => {
  const bot = Array.from({ length: 12 }, (_, i) => prompt(i * 4 * MIN, 5));
  const r = triage(bot);
  expect(r.flags.map((f) => f.code).sort()).toEqual(["low_effort_prompts", "robotic_cadence"]);
  expect(r.risk).toBe(50);
});
