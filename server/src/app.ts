import type { BunRequest, SQL } from "bun";
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
import { CommitBody, UploadBody } from "./schema.ts";
import { rulesetVersion } from "./wasm.ts";

type Deps = { sql: SQL; port: number; ruleset?: string };

/** The router's one try/catch: HttpError becomes its response, anything else a logged 500. */
function handle<P extends string>(f: (req: BunRequest<P>) => Promise<Response>) {
  return async (req: BunRequest<P>): Promise<Response> => {
    try {
      return await f(req);
    } catch (e) {
      if (e instanceof HttpError) return json(e.status, e.body);
      log("error", e instanceof Error ? e.message : String(e)); // message only: never bodies or tokens
      return json(500, { error: "internal" });
    }
  };
}

export function createServer(deps: Deps) {
  const { sql } = deps;
  const ruleset = deps.ruleset ?? rulesetVersion();
  return Bun.serve({
    port: deps.port,
    // Transport hard cap only: Bun resets the connection past it, so it sits above MAX_BODY_BYTES
    // and readJson answers the JSON 413 for everything in between (Content-Length is checked first).
    maxRequestBodySize: 2 * MAX_BODY_BYTES,
    routes: {
      "/healthz": { GET: () => Response.json({ ok: true }) },
      "/v1/ledger/:project/entries": {
        POST: handle(async (req) => {
          const projectId = req.params.project;
          // Task 9: rate limit call site goes here, before auth.
          await requireToken(sql, req, projectId);
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
        GET: handle(async (req) => {
          const projectId = req.params.project;
          // Task 9: rate limit call site goes here, before auth.
          await requireToken(sql, req, projectId);
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
        POST: handle(async (req) => {
          const projectId = req.params.project;
          // Task 9: rate limit call site goes here, before auth.
          await requireToken(sql, req, projectId);
          await recordCommit(sql, projectId, parse(CommitBody, await readJson(req)));
          return json(200, { ok: true });
        }),
      },
      "/v1/projects/:id/badge.json": {
        GET: handle(async (req) => {
          // Task 9: rate limit call site goes here (keyed by ip; public, no auth).
          const s = await verifiedSeconds(sql, req.params.id);
          if (s === null) throw new HttpError(404, { error: "not_found" });
          return json(200, badge(s), { "cache-control": "public, max-age=300" });
        }),
      },
    },
    fetch: () => Response.json({ error: "not_found" }, { status: 404 }),
  });
}
