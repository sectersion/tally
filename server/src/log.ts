const SECRET = new Set([
  "authorization",
  "token",
  "token_hash",
  "database_url",
  "cookie",
  "csrf",
  "code",
  "client_secret",
]);

const RANK: Record<string, number> = { debug: 0, info: 1, warn: 2, error: 3 };
let min = 1;
/** Lines below this level are dropped. main.ts sets it from LOG_LEVEL. */
export function setLogLevel(level: string): void {
  min = RANK[level] ?? 1;
}

export function log(level: string, msg: string, fields: Record<string, unknown> = {}): void {
  if ((RANK[level] ?? 1) < min) return;
  const safe: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(fields))
    safe[k] = SECRET.has(k.toLowerCase()) ? "[redacted]" : v;
  process.stdout.write(`${JSON.stringify({ ts: Date.now(), level, msg, ...safe })}\n`);
}
