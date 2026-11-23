import type { SQL } from "bun";

export function createServer(deps: { sql: SQL; port: number; ruleset?: string }) {
  return Bun.serve({
    port: deps.port,
    routes: { "/healthz": { GET: () => Response.json({ ok: true }) } },
    fetch: () => Response.json({ error: "not_found" }, { status: 404 }),
  });
}
