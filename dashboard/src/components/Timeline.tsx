import { formatClock, formatDuration } from "../time";
import type { Item, ItemKind } from "../types";

const LABEL: Record<ItemKind, string> = {
  prompt: "prompt", run: "run passed", error: "run failed", fix: "bug fixed", agent_fix: "fixed by the agent",
  commit: "commit", edit: "edit", interrupt: "interrupt", approval: "approval", denial: "denial",
  skipped: "no presence coverage, not counted", agent_working: "agent working", away: "away",
};
const ICON: Record<ItemKind, string> = {
  prompt: "?", run: "+", error: "x", fix: "*", agent_fix: "*", commit: "#", edit: "~", interrupt: "!",
  approval: "y", denial: "n", skipped: "-", agent_working: "=", away: "_",
};

function detail(i: Item): string {
  const bits: string[] = [];
  if (i.end_ts !== null) bits.push(formatDuration((i.end_ts - i.ts) / 1000));
  else if (i.n !== null) {
    if (i.kind === "prompt") bits.push(`${Math.round(i.n / 10)} % drew on what you saw`);
    else if (i.kind === "run" || i.kind === "error" || i.kind === "fix" || i.kind === "agent_fix") bits.push(`exit code ${i.n}`);
    else if (i.kind === "edit" || i.kind === "commit") bits.push(`${i.n} lines`);
  }
  if (i.short) bits.push(`id ${i.short}`);
  if (i.agent && i.kind !== "agent_fix") bits.push("agent");
  return bits.join(", ");
}

export function Timeline({ items }: { items: Item[] }) {
  const failedAt = new Map<string, number>(); // command id (first 8 characters) to the last failing run
  let prev: number | null = null;
  return (
    <ol class="timeline">
      {items.map((i) => {
        // vertical space grows with the time since the previous moment (square-root scale, capped)
        const gap = prev === null ? 0 : Math.max(0, (i.ts - prev) / 1000);
        prev = i.ts;
        const gapPx = Math.min(72, 6 + Math.round(Math.sqrt(gap) * 1.4));
        let loop = "";
        if (i.kind === "error" && i.short) failedAt.set(i.short, i.ts);
        if ((i.kind === "fix" || i.kind === "agent_fix") && i.short && failedAt.has(i.short)) {
          loop = `fixed ${formatDuration((i.ts - failedAt.get(i.short)!) / 1000)} after it failed`;
        }
        return (
          <li class={`k-${i.kind}`} style={{ marginTop: `${gapPx}px` }}>
            <time>{formatClock(i.ts)}</time> <span class={`chip k-${i.kind}`} aria-hidden="true">{ICON[i.kind]}</span> <strong>{LABEL[i.kind]}</strong>
            {detail(i) && <span class="muted">{` (${detail(i)})`}</span>}
            {loop && <span class="loop-note">{loop}</span>}
          </li>
        );
      })}
    </ol>
  );
}
