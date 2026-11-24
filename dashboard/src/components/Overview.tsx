import { useState } from "preact/hooks";
import { dayEdges, formatClock, formatDuration } from "../time";
import type { Bugs, Hours, PromptGroup, PromptsAfterFailure } from "../types";
import { Gates, pickUnit, Spark } from "./Gates";
import { Period, projectLabel, useLoad, ViewProps } from "./util";

const pct = (pm: number | null) => (pm === null ? "no prompts" : `${Math.round(pm / 10)} %`);

function Group({ name, g }: { name: string; g: PromptGroup }) {
  return (
    <p class="group">
      {name}: <strong>{pct(g.informed_pm)}</strong>
      <span class="muted"> ({g.prompts} prompts)</span>
      <span class="bar" role="img" aria-label={`${name}: ${pct(g.informed_pm)}`}>
        <span style={{ width: `${Math.round((g.informed_pm ?? 0) / 10)}%` }} />
      </span>
    </p>
  );
}

const REASON: Record<string, string> = {
  idle_subtracted: "while you were away from the keyboard",
  agent_busy_subtracted: "while the agent worked and you were idle",
  "agent_busy_subtracted+idle_subtracted": "both",
  unexplained: "rounding",
};

function BugsCard({ b, projects }: { b: Bugs; projects: ViewProps["projects"] }) {
  const shown = b.fixes.slice(0, 5);
  const longest = Math.max(1, ...shown.map((f) => f.ms_to_fix));
  return (
    <section class="loops" aria-labelledby="bugs-h">
      <div>
        <h3 id="bugs-h">Bugs fixed</h3>
        <p class="big">{b.fixed}</p>
        {b.avg_ms_to_fix === null ? (
          <p>No fixed bugs in this period yet. A fix is a failing test run that later passes after you did something in between.</p>
        ) : (
          <p>
            Average time from failure to fix: {formatDuration(b.avg_ms_to_fix / 1000)}
            {b.agent_fixes > 0 && " (includes fixes the agent completed)"}
          </p>
        )}
        {b.agent_fixes > 0 && <p>{b.agent_fixes} more fixed by the agent (not counted as your time)</p>}
        {b.skipped_fixes > 0 && <p>{b.skipped_fixes} skipped (no presence coverage)</p>}
      </div>
      {shown.length > 0 && (
        <div>
          <ul class="loop-list">
            {shown.map((f) => (
              <li>
                <span>
                  {formatClock(f.fixed_ts)} {projectLabel(f.project, projects)}: {formatDuration(f.ms_to_fix / 1000)}
                  {f.by_agent && " (fixed by the agent)"}
                  {f.skipped && " (skipped, no presence coverage)"}
                </span>
                <span class={`loop-bar${f.by_agent ? " agent" : ""}`} role="img" aria-label={`${formatDuration(f.ms_to_fix / 1000)} from failure to fix`}>
                  <i style={{ width: `${Math.max(3, Math.round((f.ms_to_fix * 100) / longest))}%` }} />
                </span>
              </li>
            ))}
          </ul>
          <p class="note">Each bar runs from the failing run (left dot) to the passing run (right end). Bars share one scale, the longest of these.</p>
        </div>
      )}
    </section>
  );
}

function PromptsCard({ p }: { p: PromptsAfterFailure }) {
  return (
    <section aria-labelledby="prompts-h">
      <h3 id="prompts-h">Prompts that used what you saw</h3>
      <div class="pair">
        <Group name="after a failing run" g={p.after_failure} />
        <Group name="other prompts" g={p.other} />
      </div>
      {!p.enough && <p>Not enough failing runs yet to compare (5 needed, {p.after_failure.prompts} so far).</p>}
      <div class="fine">
        <p>
          A prompt counts as informed when at least {p.informed_pm_threshold / 10} % of its words come from the project, the agent's last output or the last error.
          After a failing run means within {Math.round(p.window_ms / 60000)} minutes of it.
        </p>
        <p>
          Limits: words from an error cannot be told apart from the project's own vocabulary, so a high number is a hint, not proof. A prompt zeroed for low novelty counts as not informed.
          It needs test runs to compare against, and it shows a trend over weeks, not single prompts. Runs the agent started count as runs you could see.
        </p>
      </div>
    </section>
  );
}

/** Three separate shares of the time between actions. Each is labelled alone; nothing here is a combined total. */
function Shares({ h }: { h: Hours }) {
  const v = h.totals.verified_s, u = h.totals.unverified_s, r = h.totals.removed_s;
  const sum = v + u + r;
  if (sum === 0) return null;
  const w = (x: number) => `${(x * 100) / sum}%`;
  return (
    <div class="shares">
      <div class="strip" role="img" aria-label={`Shares of the time between your actions: verified ${formatDuration(v)}, unverified ${formatDuration(u)}, not counted ${formatDuration(r)}`}>
        <i class="s-v" style={{ width: w(v) }} />
        <i class="s-u" style={{ width: w(u) }} />
        <i class="s-r" style={{ width: w(r) }} />
      </div>
      <dl>
        <div><dt><span class="swatch solid" /> Verified</dt><dd>{formatDuration(v)}</dd></div>
        <div><dt><span class="swatch hatch" /> Unverified</dt><dd>{formatDuration(u)}</dd></div>
        <div><dt><span class="swatch hole" /> Not counted</dt><dd>{formatDuration(r)}</dd></div>
      </dl>
    </div>
  );
}

function RemovedCard({ h }: { h: Hours }) {
  return (
    <section aria-labelledby="removed-h">
      <h3 id="removed-h">Time Tally removed</h3>
      <p>Of the time between your actions, {formatDuration(h.totals.removed_s)} was not counted.</p>
      <Shares h={h} />
      {h.removed_by.length > 0 && (
        <table>
          <caption>Removed time by reason</caption>
          <thead><tr><th scope="col">Reason</th><th scope="col">Actions</th><th scope="col">Time</th></tr></thead>
          <tbody>
            {h.removed_by.map((r) => (
              <tr><th scope="row">{REASON[r.reason] ?? r.reason}</th><td>{r.claims}</td><td>{formatDuration(r.seconds)}</td></tr>
            ))}
          </tbody>
        </table>
      )}
      {h.capped.seconds > 0 && (
        <p>{formatDuration(h.capped.seconds)} exceeded the cap for that kind of action and is unverified.</p>
      )}
      {h.away_runs > 0 && <p>A gap of over an hour is never counted at all ({h.away_runs} times).</p>}
      {h.no_presence.claims > 0 && (
        <p>
          No presence coverage, not counted: {formatDuration(h.no_presence.seconds)} of actions could not be checked against your keyboard presence.
        </p>
      )}
    </section>
  );
}

export function Overview(p: ViewProps) {
  const [days, setDays] = useState(28);
  const range = () => { const e = dayEdges(p.now(), days); return [e[0], e[e.length - 1]] as const; };
  const bugs = useLoad(() => p.api.bugs(...range()), [p.api, days, p.tick]);
  const prompts = useLoad(() => p.api.prompts(...range()), [p.api, days, p.tick]);
  const hours = useLoad(() => p.api.hours(dayEdges(p.now(), days)), [p.api, days, p.tick]);
  const h = hours.data?.data;
  const perDay = Array<number>(days).fill(0);
  for (const d of h?.days ?? []) if (d.day < days) perDay[d.day] += d.seconds.verified_s;
  const unit = h ? pickUnit(Math.max(h.totals.verified_s, h.totals.unverified_s)) : 60; // one stroke size for both rows
  return (
    <div>
      <div class="wrap head">
        <h2 id="overview-h" tabIndex={-1}>Overview</h2>
        <Period days={days} onChange={setDays} />
      </div>
      <div class="band forest">
        <div class="wrap">
          {h ? (
            <section aria-label="Verified time">
              <p class="eyebrow">Verified time, last {days} days</p>
              <p class="stat giant">{formatDuration(h.totals.verified_s)}</p>
              <div class="hero-grid">
                <Gates seconds={h.totals.verified_s} unit={unit} tone="#ffffff" label="Verified time" />
                <Spark values={perDay} tone="#ffffff" label={`Verified per day, last ${days} days`} />
              </div>
              <p class="muted">Counted toward your hours.</p>
            </section>
          ) : <p>Loading time</p>}
        </div>
      </div>
      <div class="band cream">
        <div class="wrap split">
          {h ? (
            <section aria-label="Unverified time">
              <p class="eyebrow"><span class="swatch hatch" aria-hidden="true" /> Unverified time, last {days} days</p>
              <p class="stat">{formatDuration(h.totals.unverified_s)}</p>
              <Gates seconds={h.totals.unverified_s} unit={unit} tone="#aa2d00" dashed label="Unverified time" />
              <p class="muted">Kept apart and never added to verified time.</p>
            </section>
          ) : null}
          {h ? <RemovedCard h={h} /> : <p>Loading time</p>}
        </div>
      </div>
      <div class="band peach">
        <div class="wrap">{bugs.data ? <BugsCard b={bugs.data.data} projects={p.projects} /> : <p>Loading bugs</p>}</div>
      </div>
      <div class="band dark">
        <div class="wrap">{prompts.data ? <PromptsCard p={prompts.data.data} /> : <p>Loading prompts</p>}</div>
      </div>
    </div>
  );
}
