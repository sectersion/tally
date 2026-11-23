/**
 * Live project pages over server-sent events.
 *
 * Delivery: Postgres LISTEN/NOTIFY through Bun.sql's `sql.listen` (the pinned Bun 1.4.5 has it;
 * checked by a spike on 2026-11-23), not the 2 s poll fallback. bumpChange's pg_notify carries
 * only a project id, so an upload handled by another process still reaches the streams held here.
 * A server's hub listens only while it has streams. The first LISTEN is retried with 1, 2, 5, then
 * 10 s backoff; after that Bun reconnects a dropped connection itself and runs `onlisten` again,
 * and every `onlisten` (first one included) resends each subscribed project its current seq,
 * because notifies may have been missed.
 *
 * Streams send only change_seq, never seconds; the page re-fetches /fragment. No state lives only
 * in this process: a reconnecting browser sends Last-Event-ID and gets a snapshot if it is stale.
 */
import type { Server, SQL } from "bun";
import { log } from "./log.ts";

const CHANNEL = "tally_project";
const BACKOFF_MS = [1000, 2000, 5000, 10000];
const PING_MS = 25_000;
const PER_IP = 4;
const PER_PROCESS = 2000;
const enc = new TextEncoder();
const reason = (e: unknown) => (e instanceof Error ? e.message : String(e));

export type Live = ReturnType<typeof createLive>;

export function createLive(sql: SQL) {
  const subs = new Map<string, Set<(seq: number) => void>>();
  const perIp = new Map<string, number>();
  const streams = new Set<() => void>();
  let listening: Promise<SQL.ListenSubscription | null> | null = null;
  let closed = false;

  const fanOut = (id: string, seq: number) => {
    for (const send of subs.get(id) ?? []) {
      try {
        send(seq);
      } catch (e) {
        log("warn", "live send failed", { reason: reason(e) });
      }
    }
  };
  /** Reads these projects' change_seq and fans it out. Never throws. */
  const refresh = async (ids: string[]) => {
    if (!ids.length) return;
    try {
      const rows = await sql`select id, change_seq from projects where id = any(${ids}::text[])`;
      for (const r of rows) fanOut(r.id, Number(r.change_seq));
    } catch (e) {
      log("warn", "live refresh failed", { reason: reason(e) });
    }
  };
  const listen = async () => {
    for (let i = 0; subs.size; i++) {
      try {
        return await sql.listen(
          CHANNEL,
          (id) => {
            if (subs.has(id)) void refresh([id]);
          },
          () => void refresh([...subs.keys()]),
        );
      } catch (e) {
        log("warn", "live listen failed", { reason: reason(e) });
        await Bun.sleep(BACKOFF_MS[Math.min(i, BACKOFF_MS.length - 1)] as number);
      }
    }
    return null;
  };

  /** Registers send for projectId; the returned function removes it (idempotent). */
  const subscribe = (projectId: string, send: (seq: number) => void) => {
    const set = subs.get(projectId) ?? new Set();
    subs.set(projectId, set.add(send));
    listening ??= listen();
    return () => {
      const s = subs.get(projectId);
      if (!s?.delete(send)) return;
      if (!s.size) subs.delete(projectId);
      if (!subs.size && listening) {
        const l = listening;
        listening = null;
        l.then((x) => x?.unlisten()).catch((e) =>
          log("warn", "live unlisten failed", { reason: reason(e) }),
        );
      }
    };
  };

  /** The SSE response for a project the caller already checked is visible. */
  const stream = (req: Request, server: Server<undefined>, projectId: string, ip: string) => {
    // Shutting down: a stream opened now would hold server.stop() open.
    if (closed)
      return Response.json(
        { error: "shutting_down" },
        { status: 503, headers: { "retry-after": "3", "cache-control": "no-store" } },
      );
    if (streams.size >= PER_PROCESS || (perIp.get(ip) ?? 0) >= PER_IP)
      return Response.json(
        { error: "too_many_streams" },
        { status: 429, headers: { "retry-after": "30", "cache-control": "no-store" } },
      );
    perIp.set(ip, (perIp.get(ip) ?? 0) + 1);
    server.timeout(req, 0); // Bun's idle timeout would otherwise cut the stream
    // Last-Event-ID (set by the browser on reconnect) wins over the page-load ?seq=.
    const claimed = req.headers.get("last-event-id") ?? new URL(req.url).searchParams.get("seq");
    const initial = claimed !== null && /^\d{1,15}$/.test(claimed) ? Number(claimed) : -1;
    let last = initial;
    let sent = false;
    let done = false;
    let ctl: ReadableStreamDefaultController<Uint8Array> | undefined;
    let timer: ReturnType<typeof setInterval> | undefined;
    let unsub = () => {};
    const end = () => {
      if (done) return;
      done = true;
      clearInterval(timer);
      unsub();
      streams.delete(end);
      const n = (perIp.get(ip) ?? 1) - 1;
      if (n > 0) perIp.set(ip, n);
      else perIp.delete(ip);
      try {
        ctl?.close();
      } catch {} // already closed or cancelled
    };
    const write = (s: string) => {
      if (done) return;
      try {
        ctl?.enqueue(enc.encode(s));
      } catch {
        end();
      }
    };
    const emit = (seq: number) => {
      last = seq;
      sent = true;
      write(`id: ${seq}\nevent: changed\ndata: ${seq}\n\n`);
    };
    streams.add(end);
    const body = new ReadableStream<Uint8Array>({
      start(c) {
        ctl = c;
        write("retry: 3000\n\n");
        unsub = subscribe(projectId, (seq) => {
          if (seq > last) emit(seq);
        });
        timer = setInterval(() => write(": ping\n\n"), PING_MS);
        sql`select change_seq from projects where id = ${projectId}`
          .then(([r]) => {
            const cur = Number(r?.change_seq);
            if (r && !sent && cur !== initial) emit(cur);
          })
          .catch((e) => log("warn", "live snapshot failed", { reason: reason(e) }));
        if (req.signal.aborted) end();
        else req.signal.addEventListener("abort", end, { once: true });
      },
      cancel: end,
    });
    return new Response(body, {
      headers: {
        "content-type": "text/event-stream",
        "cache-control": "no-store",
        "x-accel-buffering": "no",
      },
    });
  };

  /** Ends every stream (SIGTERM): browsers reconnect to the next process after `retry`. */
  const closeAll = () => {
    closed = true;
    for (const end of [...streams]) end();
  };
  const size = () => ({ streams: streams.size, projects: subs.size, ips: perIp.size });

  return { subscribe, stream, closeAll, size };
}
