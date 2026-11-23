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
 * The caller's address. Behind a proxy (TRUST_PROXY=1) it is the last x-forwarded-for hop, the one
 * the proxy appended; earlier hops are client-supplied. Anything else, or a malformed hop, is the socket peer.
 */
export function clientIp(
  req: Request,
  server: Pick<Server<undefined>, "requestIP">,
  cfg: Pick<Config, "trustProxy">,
): string {
  if (cfg.trustProxy) {
    const last = (req.headers.get("x-forwarded-for") ?? "").split(",").at(-1)?.trim() ?? "";
    if (isIP(last)) return last;
  }
  return server.requestIP(req)?.address ?? "unknown";
}
