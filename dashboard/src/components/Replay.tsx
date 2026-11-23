import { useEffect, useRef, useState } from "preact/hooks";
import { dayEdges, dayLabel, formatClock } from "../time";
import type { Item, Session } from "../types";
import { Timeline } from "./Timeline";
import { projectLabel, useLoad, ViewProps } from "./util";

const GAP_MS = 30 * 60 * 1000;

export function Replay(p: ViewProps) {
  const sessions = useLoad(() => {
    const e = dayEdges(p.now(), 28);
    return p.api.sessions(e[0], e[e.length - 1]);
  }, [p.api, p.tick]);
  const list: Session[] = sessions.data?.data ?? [];
  const [sel, setSel] = useState(0);
  const s = list[sel];
  return (
    <div>
      <h2 id="replay-h" tabIndex={-1}>Session replay</h2>
      <p class="muted" id="replay-note">
        What you see here and what you do not: you see when each moment happened, its kind, and numbers (a prompt's share drawn from what you saw, exit codes, lines changed, the first 8 characters of a command or commit hash).
        You do not see what was typed, what the agent said, which command or file, or any diff. Tally never stored them. Sessions are split by gaps of 30 minutes; moments with no presence coverage appear as their own rows.
      </p>
      <div class="replay">
        <div>
          <h3>Sessions</h3>
          {list.length === 0 && <p>No sessions in the last 28 days.</p>}
          <ul class="sessions">
            {list.map((x, i) => (
              <li>
                <button type="button" aria-pressed={i === sel} onClick={() => setSel(i)}>
                  {projectLabel(x.project, p.projects)}, {dayLabel(x.start_ts)} {formatClock(x.start_ts)} to {formatClock(x.end_ts)}, {Math.round(x.verified_s / 60)} min verified
                </button>
              </li>
            ))}
          </ul>
        </div>
        <div>{s && <Moments key={`${s.project}|${s.start_ts}`} p={p} s={s} />}</div>
      </div>
    </div>
  );
}

function Moments({ p, s }: { p: ViewProps; s: Session }) {
  const [items, setItems] = useState<Item[]>([]);
  const [next, setNext] = useState<string | null>(null);
  const [announce, setAnnounce] = useState("");
  const firstPage = useRef(0);
  const range = [s.start_ts, s.end_ts + GAP_MS + 1] as const;
  const opts = (after?: string) => ({ project: s.project ?? undefined, after });
  // First page on mount; later polls only append (shown items are never reordered).
  useEffect(() => {
    let live = true;
    p.api.replay(...range, opts()).then((r) => {
      if (!live) return;
      const d = r.data;
      setItems((cur) => {
        if (cur.length === 0) { firstPage.current = d.items.length; setNext(d.next); return d.items; }
        if (cur.length === firstPage.current && d.items.length > cur.length) {
          setAnnounce(`${d.items.length - cur.length} new moments`);
          firstPage.current = d.items.length;
          setNext(d.next);
          return [...cur, ...d.items.slice(cur.length)];
        }
        return cur; // ponytail: after "Load more", live appends pause; reselect the session to refresh
      });
    }).catch(() => {});
    return () => { live = false; };
  }, [p.api, p.tick]);
  const more = async () => {
    if (!next) return;
    const r = await p.api.replay(...range, opts(next));
    setItems((cur) => [...cur, ...r.data.items]);
    setNext(r.data.next);
  };
  return (
    <section aria-labelledby="moments-h">
      <h3 id="moments-h">Moments</h3>
      <div role="status" aria-live="polite">{announce}</div>
      <Timeline items={items} />
      {next && <button type="button" onClick={more}>Load more</button>}
    </section>
  );
}
