import type { Bugs, Envelope, Hours, Page, PromptsAfterFailure, Session, StatusData } from "./types";

export class ApiError extends Error {
  constructor(public status: number, message: string) {
    super(message);
  }
}

type Fetch = (url: string) => Promise<Response>;
const qs = (o: Record<string, string | number | undefined>) =>
  Object.entries(o).filter(([, v]) => v !== undefined).map(([k, v]) => `${k}=${encodeURIComponent(String(v))}`).join("&");

export class Api {
  constructor(private base = "", private fetchImpl: Fetch = (u) => fetch(u)) {}

  private async get<T>(path: string, q: Record<string, string | number | undefined> = {}): Promise<Envelope<T>> {
    const s = qs(q);
    let r: Response;
    try {
      r = await this.fetchImpl(`${this.base}${path}${s ? "?" + s : ""}`);
    } catch (e) {
      throw new ApiError(0, String(e));
    }
    if (!r.ok) {
      let msg = r.statusText;
      try { msg = (await r.json()).error ?? msg; } catch { /* keep statusText */ }
      throw new ApiError(r.status, msg);
    }
    return r.json();
  }

  status() { return this.get<StatusData>("/api/status"); }
  bugs(from: number, to: number) { return this.get<Bugs>("/api/bugs", { from, to }); }
  prompts(from: number, to: number) { return this.get<PromptsAfterFailure>("/api/prompts", { from, to }); }
  hours(edges: number[]) { return this.get<Hours>("/api/hours", { edges: edges.join(",") }); }
  sessions(from: number, to: number, limit?: number) { return this.get<Session[]>("/api/sessions", { from, to, limit }); }
  replay(from: number, to: number, o: { project?: string; after?: string; limit?: number } = {}) {
    return this.get<Page>("/api/replay", { from, to, ...o });
  }
}

export type ApiLike = Pick<Api, "status" | "bugs" | "prompts" | "hours" | "sessions" | "replay">;

export type PollOpts<T> = {
  intervalMs?: number;
  maxBackoffMs?: number;
  onResult: (v: T) => void;
  onError: (e: unknown) => void;
  /** Injected in tests. Returns a cancel function. */
  schedule?: (fn: () => void, ms: number) => () => void;
};

/** Calls `fn` now and then every interval; pauses while the tab is hidden; backs off on failure. */
export function poll<T>(fn: () => Promise<T>, o: PollOpts<T>): () => void {
  const base = o.intervalMs ?? 15000, max = o.maxBackoffMs ?? 30000;
  const schedule = o.schedule ?? ((f, ms) => { const t = setTimeout(f, ms); return () => clearTimeout(t); });
  let stopped = false, delay = base, cancel = () => {};
  const next = (ms: number) => { cancel = schedule(tick, ms); };
  async function tick() {
    if (stopped) return;
    if (typeof document !== "undefined" && document.hidden) return next(base);
    try {
      const v = await fn();
      if (stopped) return;
      delay = base;
      o.onResult(v);
    } catch (e) {
      if (stopped) return;
      delay = Math.min(delay * 2, max);
      o.onError(e);
    }
    next(delay);
  }
  void tick();
  return () => { stopped = true; cancel(); };
}
