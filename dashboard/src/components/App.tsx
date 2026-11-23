import { useEffect, useRef, useState } from "preact/hooks";
import { ApiError, ApiLike, poll } from "../api";
import type { Envelope, StatusData } from "../types";
import { Onboarding } from "./Onboarding";
import { Overview } from "./Overview";
import { Replay } from "./Replay";
import { StatusBanner } from "./StatusBanner";
import { TimeView } from "./TimeView";

const NAV = [["#/", "Overview"], ["#/time", "Time"], ["#/replay", "Replay"]] as const;

export function App({ api, pollMs = 15000, now = Date.now, hintAfterMs = 300_000 }: { api: ApiLike; pollMs?: number; now?: () => number; hintAfterMs?: number }) {
  const [st, setSt] = useState<Envelope<StatusData> | null>(null);
  const [err, setErr] = useState<unknown>(null);
  const [failSince, setFailSince] = useState<number | null>(null);
  const [route, setRoute] = useState(location.hash || "#/");
  const main = useRef<HTMLElement>(null);
  const first = useRef(true);

  useEffect(
    () =>
      poll(() => api.status(), {
        intervalMs: pollMs,
        maxBackoffMs: Math.max(pollMs, Math.min(30000, pollMs * 8)),
        onResult: (v) => { setSt(v); setErr(null); setFailSince(null); },
        onError: (e) => { setErr(e); setFailSince((s) => s ?? now()); },
      }),
    [api, pollMs],
  );
  useEffect(() => {
    const f = () => setRoute(location.hash || "#/");
    addEventListener("hashchange", f);
    return () => removeEventListener("hashchange", f);
  }, []);
  useEffect(() => {
    if (first.current) { first.current = false; return; }
    (main.current?.querySelector("h2") as HTMLElement | null)?.focus();
  }, [route]);

  const noDb = err instanceof ApiError && err.status === 503;
  const failing = err !== null && !noDb;
  const empty = noDb || (st !== null && st.data.events === 0 && st.data.final_claims === 0);
  const view = { api, now, tick: st?.as_of.finalized_through ?? 0, projects: st?.data.projects ?? [] };
  const body = empty ? <Onboarding status={noDb ? null : st?.data ?? null} /> : !st ? null : route === "#/time" ? <TimeView {...view} /> : route === "#/replay" ? <Replay {...view} /> : <Overview {...view} />;
  return (
    <>
      <header>
        <h1>Tally</h1>
        <nav aria-label="Views">
          {NAV.map(([h, l]) => <a href={h} aria-current={route === h ? "page" : undefined}>{l}</a>)}
        </nav>
        <StatusBanner failing={failing} failingForMs={failSince === null ? 0 : now() - failSince} hintAfterMs={hintAfterMs} lastLoaded={st?.as_of.finalized_through ?? null} finalizedThrough={st?.as_of.finalized_through ?? null} />
      </header>
      <main ref={main}>{body}</main>
      <footer>
        <p class="muted">This page is built only from event kinds, times and numbers. Tally never stores what you typed, what the agent said, commands or file names.</p>
      </footer>
    </>
  );
}
