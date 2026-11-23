import { RATE_BURST, RATE_PER_MIN } from "./config";

const IDLE_MS = 10 * 60_000;
const SWEEP_EVERY = 1000;

// ponytail: in-process limiter; move to Postgres or a proxy limit if the server runs more than one instance.
export function createLimiter(perMin = RATE_PER_MIN, burst = RATE_BURST, now = Date.now) {
  const buckets = new Map<string, { tokens: number; at: number }>();
  let calls = 0;
  return {
    take(key: string): boolean {
      const t = now();
      if (++calls % SWEEP_EVERY === 0)
        for (const [k, b] of buckets) if (t - b.at > IDLE_MS) buckets.delete(k);
      const b = buckets.get(key) ?? { tokens: burst, at: t };
      b.tokens = Math.min(burst, b.tokens + (t - b.at) * (perMin / 60_000));
      b.at = t;
      const ok = b.tokens >= 1;
      if (ok) b.tokens -= 1;
      buckets.set(key, b);
      return ok;
    },
  };
}

export type Limiter = ReturnType<typeof createLimiter>;
