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

export function log(level: string, msg: string, fields: Record<string, unknown> = {}): void {
  const safe: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(fields))
    safe[k] = SECRET.has(k.toLowerCase()) ? "[redacted]" : v;
  process.stdout.write(`${JSON.stringify({ ts: Date.now(), level, msg, ...safe })}\n`);
}
