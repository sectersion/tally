import type { BunRequest, Server, SQL } from "bun";
import { requireToken } from "./auth.ts";
import { MAX_BATCH, MAX_BODY_BYTES } from "./config";
import { HttpError, json, parse, readJson } from "./errors.ts";
import {
  appendEntries,
  badge,
  checkBatch,
  getHead,
  recordCommit,
  verifiedSeconds,
} from "./ledger.ts";
import { log } from "./log.ts";
import { createLimiter, type Limiter } from "./ratelimit.ts";
import { CommitBody, UploadBody } from "./schema.ts";
import { rulesetVersion } from "./wasm.ts";

type Deps = { sql: SQL; port: number; ruleset?: string; limiter?: Limiter };

type Srv = Server<undefined>;
const ip = (server: Srv, req: Request) => `ip:${server.requestIP(req)?.address ?? "unknown"}`;

/**
 * The router's one try/catch: HttpError becomes its response, anything else a logged 500.
 * Also writes the one request log line (route pattern only; never headers, body, query or token).
 */
function handle<P extends string>(
  route: P,
  f: (req: BunRequest<P>, server: Srv) => Promise<Response>,
) {
  return async (req: BunRequest<P>, server: Srv): Promise<Response> => {
    const t0 = performance.now();
    let res: Response;
    try {
      res = await f(req, server);
    } catch (e) {
      if (e instanceof HttpError)
        res = json(e.status, e.body, e.status === 429 ? { "retry-after": "60" } : undefined);
      else {
        log("error", e instanceof Error ? e.message : String(e)); // message only: never bodies or tokens
        res = json(500, { error: "internal" });
      }
    }
    const params = req.params as Record<string, string>;
    log("info", "request", {
      method: req.method,
      route,
      status: res.status,
      ms: Math.round(performance.now() - t0),
      project: params.project ?? params.id,
    });
    return res;
  };
}

export function createServer(deps: Deps) {
  const { sql } = deps;
  const ruleset = deps.ruleset ?? rulesetVersion();
  const limiter = deps.limiter ?? createLimiter();
  const limit = (key: string) => {
    if (!limiter.take(key)) throw new HttpError(429, { error: "rate_limited" });
  };
  /** Token auth, then the per-token bucket; failed auth spends the caller's ip bucket instead. */
  const authed = async (req: Request, server: Srv, projectId: string) => {
    let tokenId: number;
    try {
      ({ tokenId } = await requireToken(sql, req, projectId));
    } catch (e) {
      if (e instanceof HttpError && e.status === 401) limit(ip(server, req));
      throw e;
    }
    limit(`tok:${tokenId}`);
  };
  return Bun.serve({
    port: deps.port,
    development: false,
    // Transport hard cap only: Bun resets the connection past it, so it sits above MAX_BODY_BYTES
    // and readJson answers the JSON 413 for everything in between (Content-Length is checked first).
    maxRequestBodySize: 2 * MAX_BODY_BYTES,
    routes: {
      "/healthz": { GET: () => Response.json({ ok: true }) },
      "/v1/ledger/:project/entries": {
        POST: handle("/v1/ledger/:project/entries", async (req, server) => {
          const projectId = req.params.project;
          await authed(req, server, projectId);
          const raw = (await readJson(req)) as { entries?: unknown } | null;
          if (Array.isArray(raw?.entries) && raw.entries.length > MAX_BATCH)
            throw new HttpError(413, { error: "batch_too_large", max: MAX_BATCH });
          const body = parse(UploadBody, raw);
          const [p] = await sql`select public_name from projects where id = ${projectId}`;
          const bad = checkBatch(
            body.entries,
            body.prev_hash,
            p.public_name,
            (body.entries[0] as (typeof body.entries)[0]).ruleset,
          );
          if (bad) throw new HttpError(422, { error: "broken_chain", ...bad });
          return json(200, await appendEntries(sql, projectId, body, ruleset));
        }),
      },
      "/v1/ledger/:project/head": {
        GET: handle("/v1/ledger/:project/head", async (req, server) => {
          const projectId = req.params.project;
          await authed(req, server, projectId);
          const r = new URL(req.url).searchParams.get("ruleset");
          if (r !== null && (r.length < 1 || r.length > 32))
            throw new HttpError(400, {
              error: "invalid_body",
              issues: [{ path: "ruleset", message: "must be 1-32 characters" }],
            });
          return json(200, await getHead(sql, projectId, r ?? ruleset));
        }),
      },
      "/v1/ledger/:project/commit": {
        POST: handle("/v1/ledger/:project/commit", async (req, server) => {
          const projectId = req.params.project;
          await authed(req, server, projectId);
          await recordCommit(sql, projectId, parse(CommitBody, await readJson(req)));
          return json(200, { ok: true });
        }),
      },
      "/v1/projects/:id/badge.json": {
        GET: handle("/v1/projects/:id/badge.json", async (req, server) => {
          limit(ip(server, req));
          const s = await verifiedSeconds(sql, req.params.id);
          if (s === null) throw new HttpError(404, { error: "not_found" });
          return json(200, badge(s), { "cache-control": "public, max-age=300" });
        }),
      },
    },
    fetch: () => Response.json({ error: "not_found" }, { status: 404 }),
  });
}
