import type { SQL } from "bun";
import { MAX_BODY_BYTES } from "./config";

export function createServer(deps: { sql: SQL; port: number; ruleset?: string }) {
  return Bun.serve({
    port: deps.port,
    maxRequestBodySize: MAX_BODY_BYTES,
    routes: { "/healthz": { GET: () => Response.json({ ok: true }) } },
    fetch: () => Response.json({ error: "not_found" }, { status: 404 }),
  });
}
