import { isIP } from "node:net";
import type { Server } from "bun";
import { type Config, RATE_BURST, RATE_PER_MIN } from "./config";

export type Rate = { perMin: number; burst: number };

/** Per-key limits (tokens refill at perMin; a fresh key starts with burst). Tokens keep plan 06's RATE_*. */
export const LIMITS = {
  auth: { perMin: 20, burst: 20 }, // auth:<ip>: /auth/login, /auth/callback, /github/connect, /github/callback
  device: { perMin: 10, burst: 10 }, // device:<ip>: /v1/device/code
  form: { perMin: 60, burst: 60 }, // form:<user_id>: every session POST
  page: { perMin: 120, burst: 120 }, // page:<ip>: /u/*, /p/* and fragments
  badge: { perMin: 600, burst: 600 }, // badge:<ip>: camo and shields fetch from few IPs
} as const satisfies Record<string, Rate>;
export type Limits = { [K in keyof typeof LIMITS]: Rate };

const SWEEP_EVERY = 1000;

// ponytail: in-process limiter; move to Postgres or a proxy limit if the server runs more than one instance.
export function createLimiter(perMin = RATE_PER_MIN, burst = RATE_BURST, now = Date.now) {
  const buckets = new Map<string, { tokens: number; at: number; rate: Rate }>();
  const refill = (b: { tokens: number; at: number; rate: Rate }, t: number) =>
    Math.min(b.rate.burst, b.tokens + (t - b.at) * (b.rate.perMin / 60_000));
  let calls = 0;
  return {
    /** Spends one token from key's bucket; rate defaults to the one given at construction. */
    take(key: string, rate: Rate = { perMin, burst }): boolean {
      const t = now();
      // A full bucket is the same as no bucket, so dropping full ones loses nothing.
      if (++calls % SWEEP_EVERY === 0)
        for (const [k, b] of buckets) if (refill(b, t) >= b.rate.burst) buckets.delete(k);
      const b = buckets.get(key) ?? { tokens: rate.burst, at: t, rate };
      b.rate = rate;
      b.tokens = refill(b, t);
      b.at = t;
      const ok = b.tokens >= 1;
      if (ok) b.tokens -= 1;
      buckets.set(key, b);
      return ok;
    },
    size: () => buckets.size,
  };
}

export type Limiter = ReturnType<typeof createLimiter>;

/**
 * The limiting identity of an address: IPv4 as is, IPv4-mapped IPv6 as its IPv4 form, any other
 * IPv6 as its /64 (one subscriber usually holds a whole /64, so per-address keys could be rotated).
 */
export function ipKey(addr: string): string {
  if (isIP(addr) !== 6) return addr;
  let s = (addr.split("%")[0] ?? "").toLowerCase(); // drop a zone id
  const v4 = /(\d+)\.(\d+)\.(\d+)\.(\d+)$/.exec(s);
  if (v4) {
    const [a, b, c, d] = v4.slice(1).map(Number) as [number, number, number, number];
    s = `${s.slice(0, v4.index)}${((a << 8) | b).toString(16)}:${((c << 8) | d).toString(16)}`;
  }
  const [h = "", t] = s.split("::");
  const head = h ? h.split(":") : [];
  const tail = t ? t.split(":") : [];
  const groups =
    t === undefined ? head : [...head, ...Array(8 - head.length - tail.length).fill("0"), ...tail];
  const n = groups.map((g) => Number.parseInt(g, 16));
  const [g6 = 0, g7 = 0] = n.slice(6);
  if (n.slice(0, 5).every((x) => x === 0) && n[5] === 0xffff)
    return `${g6 >> 8}.${g6 & 255}.${g7 >> 8}.${g7 & 255}`;
  return `${n
    .slice(0, 4)
    .map((x) => x.toString(16))
    .join(":")}::/64`;
}

/**
 * The caller's limiting key (see ipKey). Behind a proxy (TRUST_PROXY=1) the address is the last
 * x-forwarded-for hop, the one the proxy appended; earlier hops are client-supplied. Anything else,
 * or a malformed hop, is the socket peer.
 */
export function clientIp(
  req: Request,
  server: Pick<Server<undefined>, "requestIP">,
  cfg: Pick<Config, "trustProxy">,
): string {
  if (cfg.trustProxy) {
    const last = (req.headers.get("x-forwarded-for") ?? "").split(",").at(-1)?.trim() ?? "";
    if (isIP(last)) return ipKey(last);
  }
  return ipKey(server.requestIP(req)?.address ?? "unknown");
}
