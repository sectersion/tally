import type { StatusData } from "../types";
import { formatClock } from "../time";

export function Onboarding({ status }: { status: StatusData | null }) {
  return (
    <section aria-labelledby="ob">
      <h2 id="ob" tabIndex={-1}>Get started</h2>
      <ol>
        <li>Start tracking: run <code>tally run</code></li>
        <li>Add a project: run <code>tally project add &lt;path&gt;</code></li>
        <li>Work with your AI tool as usual; this page fills in within a few minutes.</li>
      </ol>
      {status && status.sources.length === 0 && <p>No source has reported yet.</p>}
      {status && status.sources.length > 0 && (
        <ul>
          {status.sources.map((s) => (
            <li>{s.source}: {s.watermark === null ? "no events yet" : `last heard from ${formatClock(s.watermark)}`}</li>
          ))}
        </ul>
      )}
      <div class="two">
      <div class="card soft">
      <h3>What Tally records</h3>
      <ul>
        <li>The kind and time of each action (prompt, run, edit, commit)</li>
        <li>Numbers: sizes, exit codes, lines changed, how much a prompt drew on what you saw</li>
        <li>Hashes of commands and files, so repeats can be matched</li>
      </ul>
      </div>
      <div class="card cream">
      <h3>What it never records</h3>
      <ul>
        <li>What you typed or what the agent said</li>
        <li>Commands, file names or file contents</li>
        <li>Window titles or web addresses</li>
      </ul>
      </div>
      </div>
    </section>
  );
}
