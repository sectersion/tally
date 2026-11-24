// Mirrors the daemon's serde structs (plan 11 tasks 2 to 6). No text fields exist.
export type AsOf = { horizon: number | null; finalized_through: number | null };
export type Envelope<T> = { as_of: AsOf; data: T };

export type BugFix = {
  fixed_ts: number;
  failed_ts: number;
  project: string | null;
  ms_to_fix: number;
  skipped: boolean;
  by_agent: boolean;
};
export type Bugs = {
  fixed: number;
  skipped_fixes: number;
  agent_fixes: number;
  avg_ms_to_fix: number | null;
  median_ms_to_fix: number | null;
  fixes: BugFix[];
};

export type PromptGroup = {
  prompts: number;
  informed: number;
  informed_pm: number | null;
  median_relevance_pm: number | null;
};
export type PromptsAfterFailure = {
  after_failure: PromptGroup;
  other: PromptGroup;
  enough: boolean;
  informed_pm_threshold: number;
  window_ms: number;
};

export type Seconds = { verified_s: number; unverified_s: number; removed_s: number; claims: number };
export type DayProject = { day: number; project: string | null; seconds: Seconds };
export type RemovedReason = { reason: string; claims: number; seconds: number };
export type SkippedDay = { day: number; claims: number; seconds: number };
export type Hours = {
  days: DayProject[];
  totals: Seconds;
  removed_by: RemovedReason[];
  capped: RemovedReason;
  no_presence: RemovedReason;
  no_presence_days: SkippedDay[];
  protected_claims: number;
  away_runs: number;
  edges?: number[];
};

export type Session = {
  project: string | null;
  start_ts: number;
  end_ts: number;
  actions: number;
  verified_s: number;
  unverified_s: number;
};
export type ItemKind =
  | "prompt" | "run" | "error" | "fix" | "agent_fix" | "commit" | "edit" | "interrupt"
  | "approval" | "denial" | "skipped" | "agent_working" | "away";
export type Item = {
  ts: number;
  end_ts: number | null;
  kind: ItemKind;
  n: number | null;
  short: string | null;
  agent: boolean;
  project: string | null;
};
export type Page = { items: Item[]; next: string | null; truncated: boolean };

export type StatusData = {
  events: number;
  final_claims: number;
  provisional_claims: number;
  verified_s: number;
  unverified_s: number;
  sources: { source: string; watermark: number | null }[];
  projects: { key: string; label: string }[];
};
