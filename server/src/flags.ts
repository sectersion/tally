// Behaviour flags for a submission timeline (issue #113). Pure: events in, flags out.
// Flags only order the reviewer queue and point at evidence; they never change hours.
// Thresholds are vibetime's (internal/verify) starting points, untuned on Tally data.

export type Flag = { code: string; weight: number; detail: string };
export type Triage = { flags: Flag[]; risk: number };

type Ev = {
  ts: number;
  kind: string;
  chars?: number;
  end_ts?: number;
  initiator?: string;
  by?: string;
};

const MIN_PROMPTS = 10;
const HOUR = 3_600_000;

const human = (e: Ev) =>
  ["prompt", "interrupt", "tool_decision", "user_edit"].includes(e.kind) ||
  (e.kind === "test_run" && e.initiator === "user") ||
  (e.kind === "commit" && e.by === "user");

export function triage(timeline: Record<string, unknown>[]): Triage {
  const evs = (timeline as Ev[]).filter((e) => Number.isFinite(e.ts)).sort((a, b) => a.ts - b.ts);
  const flags: Flag[] = [];
  const add = (code: string, weight: number, detail: string) =>
    flags.push({ code, weight, detail });

  const prompts = evs.filter((e) => e.kind === "prompt");

  // Scripted prompts fire on a clock: the spread of the gaps is tiny next to their mean.
  if (prompts.length >= MIN_PROMPTS) {
    const gaps = prompts.slice(1).map((p, i) => p.ts - (prompts[i]?.ts ?? p.ts));
    const mean = gaps.reduce((a, g) => a + g, 0) / gaps.length;
    const sd = Math.sqrt(gaps.reduce((a, g) => a + (g - mean) ** 2, 0) / gaps.length);
    if (mean > 0 && sd / mean < 0.2) {
      add(
        "robotic_cadence",
        30,
        `${prompts.length} prompts, gap variation ${((sd / mean) * 100).toFixed(0)}%`,
      );
    }
  }

  // "continue" farming: most prompts are too short to steer anything.
  const short = prompts.filter((p) => (p.chars ?? 0) < 20).length;
  if (prompts.length >= MIN_PROMPTS && short * 2 >= prompts.length) {
    add("low_effort_prompts", 20, `${short} of ${prompts.length} prompts under 20 chars`);
  }

  // Another model writing the prompts: long text arriving right after the agent stopped.
  let fast = 0;
  let stoppedAt = -Infinity;
  const busy = evs.filter((e) => e.kind === "agent_busy" && Number.isFinite(e.end_ts));
  for (const p of prompts) {
    for (const b of busy)
      if ((b.end_ts as number) <= p.ts) stoppedAt = Math.max(stoppedAt, b.end_ts as number);
    const secs = Math.max(p.ts - stoppedAt, 1000) / 1000;
    if (Number.isFinite(stoppedAt) && (p.chars ?? 0) >= 30 && (p.chars ?? 0) / secs > 15) fast++;
  }
  if (fast >= 3)
    add(
      "superhuman_typing",
      30,
      `${fast} prompts typed faster than 15 chars/s after the agent stopped`,
    );

  // More than 14 hours with a human action inside one 24 h window.
  const hours = [...new Set(evs.filter(human).map((e) => Math.floor(e.ts / HOUR)))];
  let worst = 0;
  for (let i = 0, j = 0; i < hours.length; i++) {
    while ((hours[i] ?? 0) - (hours[j] ?? 0) >= 24) j++;
    worst = Math.max(worst, i - j + 1);
  }
  if (worst > 14) add("long_day", 30, `${worst} active hours inside 24 h`);

  return { flags, risk: flags.reduce((a, f) => a + f.weight, 0) };
}

/** One ledger entry in the submitted range, with when the server first saw it (or a head covering it). */
export type Anchored = { ts: number; verified_s: number; anchor_ms: number };

// Honest lag is the finalize wait, the 5 min commit tick and offline backoff (capped at 1 h).
const LATE_MS = 2 * HOUR;

/**
 * Claims the server learned about long after they say they happened: what a backfilled or
 * fabricated history looks like. Advisory: an honest laptop that was offline for a day trips it too.
 */
export function backfillFlag(rows: Anchored[]): Flag | null {
  const total = rows.reduce((a, r) => a + r.verified_s, 0);
  const late = rows.filter((r) => r.anchor_ms - r.ts > LATE_MS);
  const lateS = late.reduce((a, r) => a + r.verified_s, 0);
  if (total === 0 || lateS * 4 < total) return null; // under 25% of the hours
  return {
    code: "backfilled",
    weight: 50,
    detail: `${late.length} claims (${((lateS / total) * 100).toFixed(0)}% of verified time) reached the server over 2 h after they happened`,
  };
}
