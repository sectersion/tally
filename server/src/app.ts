import type { BunRequest, Server, SQL } from "bun";
import { hashToken, newToken, requireToken } from "./auth.ts";
import { type Config, MAX_BATCH, MAX_BODY_BYTES } from "./config";
import { HttpError, json, parse, readJson } from "./errors.ts";
import { authorizeUrl, exchange } from "./hca.ts";
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
import {
  clearCookie,
  clearOauthCookie,
  cookieValue,
  createSession,
  OAUTH_COOKIE,
  oauthCookie,
  requireCsrf,
  requireUser,
  same,
  sessionToken,
} from "./session.ts";
import { rulesetVersion } from "./wasm.ts";

type Deps = { sql: SQL; port: number; ruleset?: string; limiter?: Limiter; cfg: Config };

const b64url = (b: ArrayBuffer | Uint8Array) => Buffer.from(b as Uint8Array).toString("base64url");
const redirect = (to: string, setCookies: string[] = []) => {
  const headers = new Headers({ location: to });
  for (const c of setCookies) headers.append("set-cookie", c);
  return new Response(null, { status: 303, headers });
};

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
      if (e instanceof HttpError && e.location) res = redirect(e.location);
      else if (e instanceof HttpError)
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
  const { cfg } = deps;
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
      "/auth/login": {
        GET: handle("/auth/login", async (req, server) => {
          limit(ip(server, req));
          const rt = new URL(req.url).searchParams.get("return_to") ?? "/";
          // Printable ASCII only: browsers strip tabs/newlines from URLs, so "/\t/evil" would become "//evil".
          const returnTo = /^\/(?![/\\])[\x21-\x7e]*$/.test(rt) ? rt : "/";
          const state = newToken();
          const nonce = newToken();
          const verifier = b64url(crypto.getRandomValues(new Uint8Array(32)));
          const challenge = b64url(
            await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier)),
          );
          await sql`delete from oauth_states where expires_at < now()`;
          await sql`insert into oauth_states (state_hash, verifier, nonce, return_to, expires_at)
            values (${hashToken(state)}, ${verifier}, ${nonce}, ${returnTo}, now() + interval '10 minutes')`;
          return redirect(authorizeUrl(cfg, state, nonce, challenge), [oauthCookie(cfg, state)]);
        }),
      },
      "/auth/callback": {
        GET: handle("/auth/callback", async (req, server) => {
          limit(ip(server, req));
          const q = new URL(req.url).searchParams;
          const code = q.get("code");
          const state = q.get("state");
          if (!code || !state) throw new HttpError(400, { error: "bad_callback" });
          // The state must come back with the browser that started the login (login CSRF).
          const bound = cookieValue(req, OAUTH_COOKIE);
          if (!bound || !same(bound, state)) throw new HttpError(400, { error: "bad_state" });
          const [st] =
            await sql`delete from oauth_states where state_hash = ${hashToken(state)} and expires_at > now() returning verifier, nonce, return_to`;
          if (!st) throw new HttpError(400, { error: "bad_state" });
          let u: Awaited<ReturnType<typeof exchange>>;
          try {
            u = await exchange(cfg, code, st.verifier, st.nonce);
          } catch (e) {
            log("warn", "hca exchange failed", { reason: e instanceof Error ? e.message : "?" });
            throw new HttpError(502, { error: "hca_failed" });
          }
          const [user] = await sql`insert into users (id, hca_sub, email, name, verification_status)
            values (${crypto.randomUUID()}, ${u.sub}, ${u.email}, ${u.name}, ${u.verification_status})
            on conflict (hca_sub) do update set email = excluded.email, name = excluded.name,
              verification_status = excluded.verification_status returning id`;
          const { setCookie } = await createSession(sql, cfg, user.id);
          return redirect(st.return_to, [setCookie, clearOauthCookie(cfg)]);
        }),
      },
      "/auth/logout": {
        POST: handle("/auth/logout", async (req) => {
          const s = await requireUser(sql, req);
          requireCsrf(cfg, req, s, await req.formData().catch(() => new FormData()));
          await sql`delete from sessions where token_hash = ${hashToken(sessionToken(req) as string)}`;
          return redirect("/", [clearCookie(cfg)]);
        }),
      },
      // Placeholder until the settings page lands: proves requireUser's login redirect.
      "/settings": {
        GET: handle("/settings", async (req) => {
          const s = await requireUser(sql, req);
          return json(200, { user_id: s.userId });
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
