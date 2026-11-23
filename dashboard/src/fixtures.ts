import type { ApiLike } from "./api";
import type { Bugs, Envelope, Hours, Page, PromptsAfterFailure, Seconds, Session, StatusData } from "./types";

export const NOW = Date.parse("2026-11-23T12:00:00Z");
export const env = <T,>(data: T, through: number | null = NOW - 120_000): Envelope<T> => ({
  as_of: { horizon: through, finalized_through: through },
  data,
});
const sec = (v: number, u = 0, r = 0, claims = 1): Seconds => ({ verified_s: v, unverified_s: u, removed_s: r, claims });

export const status: StatusData = {
  events: 40, final_claims: 12, provisional_claims: 1, verified_s: 3600, unverified_s: 300,
  sources: [{ source: "claude-code", watermark: NOW }],
  projects: [{ key: "a".repeat(64), label: "sectersion/tally" }],
};
export const bugs: Bugs = {
  fixed: 2, skipped_fixes: 0, agent_fixes: 0, avg_ms_to_fix: 12 * 60_000, median_ms_to_fix: 12 * 60_000,
  fixes: [
    { fixed_ts: NOW - 3_600_000, failed_ts: NOW - 4_320_000, project: "sectersion/tally", ms_to_fix: 12 * 60_000, skipped: false, by_agent: false },
  ],
};
export const noBugs: Bugs = { fixed: 0, skipped_fixes: 0, agent_fixes: 0, avg_ms_to_fix: null, median_ms_to_fix: null, fixes: [] };
export const prompts: PromptsAfterFailure = {
  after_failure: { prompts: 8, informed: 5, informed_pm: 625, median_relevance_pm: 400 },
  other: { prompts: 20, informed: 8, informed_pm: 400, median_relevance_pm: 250 },
  enough: true, informed_pm_threshold: 300, window_ms: 300_000,
};
export const hours: Hours = {
  days: [
    { day: 26, project: "sectersion/tally", seconds: sec(3000, 200, 400) },
    { day: 27, project: "sectersion/tally", seconds: sec(600, 100, 60) },
  ],
  totals: sec(3600, 300, 460, 2),
  removed_by: [
    { reason: "idle_subtracted", claims: 3, seconds: 300 },
    { reason: "agent_busy_subtracted+idle_subtracted", claims: 1, seconds: 160 },
  ],
  capped: { reason: "capped", claims: 2, seconds: 300 },
  no_presence: { reason: "no_presence_coverage", claims: 0, seconds: 0 },
  no_presence_days: [],
  protected_claims: 0,
  away_runs: 2,
};
export const sessions: Session[] = [
  { project: "sectersion/tally", start_ts: NOW - 3_600_000, end_ts: NOW - 1_800_000, actions: 9, verified_s: 1500, unverified_s: 100 },
];
export const page: Page = {
  items: [
    { ts: NOW - 3_600_000, end_ts: null, kind: "prompt", n: 640, short: null, agent: false, project: null },
    { ts: NOW - 3_500_000, end_ts: null, kind: "error", n: 1, short: "abcd1234", agent: false, project: null },
    { ts: NOW - 3_400_000, end_ts: null, kind: "fix", n: 0, short: "abcd1234", agent: false, project: null },
  ],
  next: null, truncated: false,
};

export const fakeApi = (o: Partial<Record<keyof ApiLike, unknown>> = {}): ApiLike =>
  ({
    status: async () => env(status),
    bugs: async () => env(bugs),
    prompts: async () => env(prompts),
    hours: async (edges: number[]) => env({ ...hours, edges }),
    sessions: async () => env(sessions),
    replay: async () => env(page),
    ...o,
  }) as ApiLike;

export const emptyStatus: StatusData = { ...status, events: 0, final_claims: 0, sources: [] };
