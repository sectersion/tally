import { useState } from "preact/hooks";
import { dayEdges, formatClock, formatDuration } from "../time";
import type { Bugs, Hours, PromptGroup, PromptsAfterFailure } from "../types";
import { Period, projectLabel, useLoad, ViewProps } from "./util";

const pct = (pm: number | null) => (pm === null ? "no prompts" : `${Math.round(pm / 10)} %`);

function Group({ name, g }: { name: string; g: PromptGroup }) {
  return (
    <p>
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
  return (
    <section class="card" aria-labelledby="bugs-h">
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
      {b.fixes.length > 0 && (
        <ul>
          {b.fixes.slice(0, 5).map((f) => (
            <li>
              {formatClock(f.fixed_ts)} {projectLabel(f.project, projects)}: {formatDuration(f.ms_to_fix / 1000)}
              {f.by_agent && " (fixed by the agent)"}
              {f.skipped && " (skipped, no presence coverage)"}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function PromptsCard({ p }: { p: PromptsAfterFailure }) {
  return (
    <section class="card" aria-labelledby="prompts-h">
      <h3 id="prompts-h">Prompts that used what you saw</h3>
      <Group name="after a failing run" g={p.after_failure} />
      <Group name="other prompts" g={p.other} />
      {!p.enough && <p>Not enough failing runs yet to compare (5 needed, {p.after_failure.prompts} so far).</p>}
      <p class="muted">
        A prompt counts as informed when at least {p.informed_pm_threshold / 10} % of its words come from the project, the agent's last output or the last error.
        After a failing run means within {Math.round(p.window_ms / 60000)} minutes of it.
      </p>
      <p class="muted">
        Limits: words from an error cannot be told apart from the project's own vocabulary, so a high number is a hint, not proof. A prompt zeroed for low novelty counts as not informed.
        It needs test runs to compare against, and it shows a trend over weeks, not single prompts. Runs the agent started count as runs you could see.
      </p>
    </section>
  );
}

function RemovedCard({ h }: { h: Hours }) {
  return (
    <section class="card" aria-labelledby="removed-h">
      <h3 id="removed-h">Time Tally removed</h3>
      <p>Of the time between your actions, {formatDuration(h.totals.removed_s)} was not counted.</p>
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
  return (
    <div>
      <h2 id="overview-h" tabIndex={-1}>Overview</h2>
      <Period days={days} onChange={setDays} />
      <div class="grid">
        {bugs.data ? <BugsCard b={bugs.data.data} projects={p.projects} /> : <p>Loading bugs</p>}
        {prompts.data ? <PromptsCard p={prompts.data.data} /> : <p>Loading prompts</p>}
        {hours.data ? <RemovedCard h={hours.data.data} /> : <p>Loading time</p>}
      </div>
    </div>
  );
}
