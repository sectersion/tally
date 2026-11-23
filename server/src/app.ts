import type { BunRequest, Server, SQL } from "bun";
import { hashToken, newToken, requireToken } from "./auth.ts";
import { badgeResponse, rateLimited } from "./badge.ts";
import { type Config, MAX_BATCH, MAX_BODY_BYTES } from "./config";
import {
  createDeviceCode,
  decide,
  normUserCode,
  pendingDevice,
  pollDeviceToken,
} from "./device.ts";
import { HttpError, json, parse, readJson } from "./errors.ts";
import {
  accessibleRepos,
  dispatch,
  exchangeCode,
  authorizeUrl as ghAuthorizeUrl,
  linkRepoTx,
  userRepos,
  userToken,
  verifySignature,
} from "./github.ts";
import { authorizeUrl, exchange } from "./hca.ts";
import { html, notFound, page, type Safe } from "./html.ts";
import {
  appendEntries,
  badge,
  checkBatch,
  getHead,
  recordCommit,
  verifiedSeconds,
} from "./ledger.ts";
import { createLive, type Live } from "./live.ts";
import { log } from "./log.ts";
import {
  deleteAccount,
  homePage,
  profilePage,
  projectFragment,
  projectPage,
  saveProfile,
  settingsPage,
  viewerOf,
  visibleProject,
} from "./pages.ts";
import {
  bumpChange,
  constraint,
  createProject,
  projectHours,
  setHandle,
  unique,
} from "./projects.ts";
import {
  clientIp,
  createLimiter,
  LIMITS,
  type Limiter,
  type Limits,
  type Rate,
} from "./ratelimit.ts";
import { CommitBody, DeviceCodeBody, DeviceTokenBody, UploadBody } from "./schema.ts";
import {
  clearCookie,
  clearGhOauthCookie,
  clearOauthCookie,
  cookieValue,
  createSession,
  GH_OAUTH_COOKIE,
  getSession,
  ghOauthCookie,
  OAUTH_COOKIE,
  oauthCookie,
  requireCsrf,
  requireUser,
  same,
  sessionToken,
} from "./session.ts";
import { findUpdEntries, updLabel } from "./upd.ts";
import { rulesetVersion } from "./wasm.ts";

type Deps = {
  sql: SQL;
  port: number;
  ruleset?: string;
  limiter?: Limiter;
  /** Overrides for the LIMITS table; tests shrink or widen them. */
  limits?: Partial<Limits>;
  cfg: Config;
  /** GitHub calls; tests stub it. */
  fetchFn?: typeof fetch;
  /** SSE hub; main.ts passes its own so SIGTERM can close every stream. */
  live?: Live;
};

const b64url = (b: ArrayBuffer | Uint8Array) => Buffer.from(b as Uint8Array).toString("base64url");
const redirect = (to: string, setCookies: string[] = []) => {
  const headers = new Headers({ location: to });
  for (const c of setCookies) headers.append("set-cookie", c);
  return new Response(null, { status: 303, headers });
};

const STATIC = new Set(["style.css", "live.js", "review.js"]);

/** Same-site paths only. Printable ASCII: browsers strip tabs/newlines from URLs, so "/\t/evil" would become "//evil". */
const safeReturn = (rt: string | null) => (rt && /^\/(?![/\\])[\x21-\x7e]*$/.test(rt) ? rt : "/");

type Srv = Server<undefined>;

/** Browser routes get a page; /v1 keeps its JSON body. Never cached: the limit is per caller. */
const tooMany = () => {
  const r = page(
    "Too many requests",
    html`<h1>Too many requests</h1><p>Try again in a minute.</p>`,
    {
      status: 429,
    },
  );
  r.headers.set("retry-after", "60");
  r.headers.set("cache-control", "no-store");
  return r;
};

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
      else if (e instanceof HttpError && e.status === 429 && !route.startsWith("/v1"))
        res = tooMany();
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
  const fetchFn = deps.fetchFn ?? fetch;
  const live = deps.live ?? createLive(sql);
  const ruleset = deps.ruleset ?? rulesetVersion();
  const limiter = deps.limiter ?? createLimiter(); // one bucket map; unkeyed calls use plan 06's rate
  const rates: Limits = { ...LIMITS, ...deps.limits };
  const ip = (req: Request, server: Srv) => clientIp(req, server, cfg);
  const limit = (key: string, rate?: Rate) => {
    if (!limiter.take(key, rate)) throw new HttpError(429, { error: "rate_limited" });
  };
  const limitAuth = (req: Request, server: Srv) => limit(`auth:${ip(req, server)}`, rates.auth);
  const limitPage = (req: Request, server: Srv) => limit(`page:${ip(req, server)}`, rates.page);
  /** Token auth, then the per-token bucket; failed auth spends the caller's ip bucket instead. */
  const authed = async (req: Request, server: Srv, projectId: string) => {
    let tokenId: number;
    try {
      ({ tokenId } = await requireToken(sql, req, projectId));
    } catch (e) {
      if (e instanceof HttpError && e.status === 401) limit(`ip:${ip(req, server)}`);
      throw e;
    }
    limit(`tok:${tokenId}`);
  };
  /** Read the body once per request. */
  const formOf = (req: Request) => req.formData().catch(() => new FormData());
  /** Every session POST: the signed-in user, their form:<user_id> bucket, then CSRF. */
  const sessionForm = async (req: Request) => {
    const s = await requireUser(sql, req);
    limit(`form:${s.userId}`, rates.form);
    const form = await formOf(req);
    requireCsrf(cfg, req, s, form);
    return { s, form };
  };
  /** Runs a call made with the user's stored token; a 401 means it is dead, so drop it and reconnect. */
  const withGhToken = async <T>(userId: string, returnTo: string, f: () => Promise<T>) => {
    try {
      return await f();
    } catch (e) {
      if (e instanceof HttpError && e.status === 401 && e.body.error === "github_token_invalid") {
        await sql`delete from github_user_tokens where user_id = ${userId}`;
        throw new HttpError(
          303,
          { error: "github_required" },
          `/github/connect?return_to=${encodeURIComponent(returnTo)}`,
        );
      }
      throw e;
    }
  };
  /** The project at /p/:owner/:name if the session user owns it; otherwise 404 (never reveals existence). */
  const ownedProject = async (userId: string, owner: string, name: string) => {
    const [p] =
      await sql`select id, public_name from projects where lower(public_name) = lower(${`${owner}/${name}`}) and owner_id = ${userId}`;
    if (!p) throw new HttpError(404, { error: "not_found" });
    return p as { id: string; public_name: string };
  };
  /** full_name of the project's live linked GitHub repo, or null. */
  const liveRepoName = async (projectId: string) => {
    const [r] = await sql`select r.full_name from project_repos l
      join github_repos r on r.id = l.github_repo_id
      where l.project_id = ${projectId} and l.unlinked_at is null`;
    return (r?.full_name as string | undefined) ?? null;
  };
  return Bun.serve({
    port: deps.port,
    development: false,
    // Transport hard cap only: Bun resets the connection past it, so it sits above MAX_BODY_BYTES
    // and readJson answers the JSON 413 for everything in between (Content-Length is checked first).
    maxRequestBodySize: 2 * MAX_BODY_BYTES,
    routes: {
      "/static/:file": {
        GET: async (req) => {
          const f = req.params.file;
          if (!STATIC.has(f)) return notFound();
          const file = Bun.file(new URL(`../static/${f}`, import.meta.url));
          if (!(await file.exists())) return notFound();
          return new Response(file, {
            headers: {
              "cache-control": "public, max-age=3600",
              "x-content-type-options": "nosniff",
            },
          });
        },
      },
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
          limitAuth(req, server);
          const returnTo = safeReturn(new URL(req.url).searchParams.get("return_to"));
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
          limitAuth(req, server);
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
          await sessionForm(req);
          await sql`delete from sessions where token_hash = ${hashToken(sessionToken(req) as string)}`;
          return redirect("/", [clearCookie(cfg)]);
        }),
      },
      "/": {
        GET: handle("/", async (req) => homePage(sql, req, await getSession(sql, req))),
      },
      "/u/:handle": {
        GET: handle("/u/:handle", async (req, server) => {
          limitPage(req, server);
          return profilePage(sql, req.params.handle, await getSession(sql, req));
        }),
      },
      "/p/:owner/:name": {
        GET: handle("/p/:owner/:name", async (req, server) => {
          limitPage(req, server);
          const { owner, name } = req.params;
          return projectPage(sql, cfg, owner, name, await getSession(sql, req), fetchFn);
        }),
      },
      "/p/:owner/:name/fragment": {
        GET: handle("/p/:owner/:name/fragment", async (req, server) => {
          limitPage(req, server);
          const { owner, name } = req.params;
          return projectFragment(sql, cfg, owner, name, await getSession(sql, req), fetchFn);
        }),
      },
      "/p/:owner/:name/events": {
        GET: handle("/p/:owner/:name/events", async (req, server) => {
          limitPage(req, server); // opening costs queries; live.ts separately caps streams held per ip
          const { owner, name } = req.params;
          const p = await visibleProject(sql, owner, name, await getSession(sql, req));
          if (!p) return notFound();
          return live.stream(req, server, p.id, ip(req, server));
        }),
      },
      "/settings": {
        GET: handle("/settings", async (req) =>
          settingsPage(sql, req, await requireUser(sql, req)),
        ),
      },
      "/settings/profile": {
        POST: handle("/settings/profile", async (req) => {
          const { s, form } = await sessionForm(req);
          const err = await saveProfile(sql, s.userId, form);
          return redirect(err ? `/settings?error=${err}` : "/settings");
        }),
      },
      "/settings/delete": {
        POST: handle("/settings/delete", async (req) => {
          const { s, form } = await sessionForm(req);
          if (!(await deleteAccount(sql, s.userId, String(form.get("confirm") ?? ""))))
            return redirect("/settings?error=confirm");
          return redirect("/", [clearCookie(cfg)]);
        }),
      },
      "/settings/handle": {
        POST: handle("/settings/handle", async (req) => {
          const { s, form } = await sessionForm(req);
          const r = await setHandle(sql, s.userId, String(form.get("handle") ?? ""));
          return redirect(r === "ok" ? "/settings" : `/settings?error=${r}`);
        }),
      },
      "/v1/device/code": {
        POST: handle("/v1/device/code", async (req, server) => {
          limit(`device:${ip(req, server)}`, rates.device);
          const b = parse(DeviceCodeBody, await readJson(req));
          const c = await createDeviceCode(sql, b.device_name);
          return json(200, {
            ...c,
            verification_uri: `${cfg.publicBaseUrl}/device`,
            verification_uri_complete: `${cfg.publicBaseUrl}/device?code=${c.user_code}`,
            expires_in: 600,
            interval: 5,
          });
        }),
      },
      "/v1/device/token": {
        POST: handle("/v1/device/token", async (req) => {
          const b = parse(DeviceTokenBody, await readJson(req));
          const r = await pollDeviceToken(sql, b.device_code);
          return json(r.status, r.body, { "cache-control": "no-store" });
        }),
      },
      "/device": {
        GET: handle("/device", async (req) => {
          const s = await requireUser(sql, req);
          const code = new URL(req.url).searchParams.get("code") ?? "";
          const viewer = viewerOf(s);
          if (!code.trim())
            return page(
              "Link device",
              html`<h1>Link a device</h1><form method="get" action="/device"><label>Code from your terminal <input name="code" required autocomplete="off" placeholder="XXXX-XXXX"></label> <button>Continue</button></form>`,
              { viewer },
            );
          const name = await pendingDevice(sql, code);
          if (name === null)
            return page("Link device", html`<p>This code is no longer valid</p>`, {
              status: 400,
              viewer,
            });
          const c = normUserCode(code);
          return page(
            "Link device",
            html`<p>Link device '${name}' to your Tally account? Check that this code matches your terminal: <strong>${c}</strong></p>
<form method="post" action="/device"><input type="hidden" name="csrf" value="${s.csrf}"><input type="hidden" name="user_code" value="${c}">
<button name="decision" value="approve">Approve</button> <button name="decision" value="deny">Deny</button></form>`,
            { viewer },
          );
        }),
        POST: handle("/device", async (req) => {
          const { s, form } = await sessionForm(req);
          const d = form.get("decision");
          if (d !== "approve" && d !== "deny") throw new HttpError(400, { error: "bad_decision" });
          const viewer = viewerOf(s);
          if (!(await decide(sql, String(form.get("user_code") ?? ""), s.userId, d === "approve")))
            return page("Link device", html`<p>This code is no longer valid</p>`, {
              status: 400,
              viewer,
            });
          return page(
            "Link device",
            html`<p>${d === "approve" ? "Device linked. Return to your terminal." : "Denied."}</p>`,
            { viewer },
          );
        }),
      },
      "/settings/tokens/:id/revoke": {
        POST: handle("/settings/tokens/:id/revoke", async (req) => {
          const { s } = await sessionForm(req);
          const id = req.params.id;
          const rows = /^\d{1,15}$/.test(id)
            ? await sql`update device_tokens set revoked_at = now()
                where id = ${id} and revoked_at is null and (user_id = ${s.userId}
                  or project_id in (select id from projects where owner_id = ${s.userId})) returning id`
            : [];
          if (rows.length === 0) throw new HttpError(404, { error: "not_found" });
          return redirect("/settings");
        }),
      },
      "/projects": {
        POST: handle("/projects", async (req) => {
          const { s, form } = await sessionForm(req);
          const name = String(form.get("name") ?? "");
          const r = await createProject(sql, s.userId, name, () =>
            withGhToken(s.userId, "/settings", () => userRepos(sql, s.userId, fetchFn)),
          );
          return redirect("error" in r ? `/settings?error=${r.error}` : `/p/${name}`);
        }),
      },
      "/github/connect": {
        GET: handle("/github/connect", async (req, server) => {
          limitAuth(req, server);
          const s = await requireUser(sql, req);
          const state = newToken();
          const verifier = b64url(crypto.getRandomValues(new Uint8Array(32)));
          const challenge = b64url(
            await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier)),
          );
          await sql`delete from oauth_states where expires_at < now()`;
          await sql`insert into oauth_states (state_hash, verifier, nonce, return_to, expires_at)
            values (${hashToken(state)}, ${verifier}, ${s.userId}, ${safeReturn(new URL(req.url).searchParams.get("return_to"))}, now() + interval '10 minutes')`;
          return redirect(ghAuthorizeUrl(cfg, state, challenge), [ghOauthCookie(cfg, state)]);
        }),
      },
      "/github/callback": {
        GET: handle("/github/callback", async (req, server) => {
          limitAuth(req, server);
          const q = new URL(req.url).searchParams;
          const code = q.get("code");
          const state = q.get("state");
          // An install-time authorization may arrive with a code but no state of ours. Never
          // exchange it: discard it and start the normal stateful flow.
          if (code && !state && !cookieValue(req, GH_OAUTH_COOKIE))
            return redirect("/github/connect");
          const s = await requireUser(sql, req);
          if (!code || !state) throw new HttpError(400, { error: "bad_callback" });
          const bound = cookieValue(req, GH_OAUTH_COOKIE);
          if (!bound || !same(bound, state)) throw new HttpError(400, { error: "bad_state" });
          // nonce holds the user who started the flow: a state cannot be completed by someone else.
          const [st] =
            await sql`delete from oauth_states where state_hash = ${hashToken(state)} and nonce = ${s.userId} and expires_at > now() returning verifier, return_to`;
          if (!st) throw new HttpError(400, { error: "bad_state" });
          let g: Awaited<ReturnType<typeof exchangeCode>>;
          try {
            g = await exchangeCode(cfg, code, st.verifier, fetchFn);
          } catch (e) {
            log("warn", "github exchange failed", { reason: e instanceof Error ? e.message : "?" });
            throw new HttpError(502, { error: "github_failed" });
          }
          await sql.begin(async (tx) => {
            // One GitHub account maps to one Tally user: the newest proof wins.
            await tx`delete from github_user_tokens where github_user_id = ${g.githubUserId} and user_id <> ${s.userId}`;
            await tx`insert into github_user_tokens (user_id, github_user_id, access_token, expires_at)
              values (${s.userId}, ${g.githubUserId}, ${g.token}, now() + ${`${g.expiresIn} seconds`}::interval)
              on conflict (user_id) do update set github_user_id = excluded.github_user_id,
                access_token = excluded.access_token, expires_at = excluded.expires_at`;
          });
          return redirect(st.return_to, [clearGhOauthCookie(cfg)]);
        }),
      },
      "/github/setup": {
        GET: handle("/github/setup", async (req) => {
          // The query is an untrusted hint: log it, never act on it.
          log("info", "github setup", {
            setup_action: (new URL(req.url).searchParams.get("setup_action") ?? "").slice(0, 20),
          });
          return redirect("/github/connect");
        }),
      },
      "/github/install": {
        GET: handle("/github/install", async (req) => {
          await requireUser(sql, req);
          return redirect(`https://github.com/apps/${cfg.githubAppSlug}/installations/new`);
        }),
      },
      "/p/:owner/:name/link-repo": {
        POST: handle("/p/:owner/:name/link-repo", async (req) => {
          const { s, form } = await sessionForm(req);
          const p = await ownedProject(s.userId, req.params.owner, req.params.name);
          const token = await userToken(sql, s.userId);
          const back = `/p/${p.public_name}/settings`;
          if (!token)
            throw new HttpError(
              303,
              { error: "github_required" },
              `/github/connect?return_to=${encodeURIComponent(back)}`,
            );
          const raw = String(form.get("repo_id") ?? "");
          // Re-fetched from GitHub: the form only names a candidate.
          const repo = /^\d{1,15}$/.test(raw)
            ? (await withGhToken(s.userId, back, () => accessibleRepos(token, fetchFn))).find(
                (r) => r.id === Number(raw),
              )
            : undefined;
          if (!repo) throw new HttpError(403, { error: "forbidden" });
          try {
            await sql.begin(async (tx) => {
              await linkRepoTx(tx, p.id, repo);
              await bumpChange(tx, p.id);
            });
          } catch (e) {
            if (unique(e)) {
              const [mine] = await sql`select 1 as x from project_repos
                where github_repo_id = ${repo.id} and project_id = ${p.id} and unlinked_at is null`;
              throw new HttpError(409, {
                error: mine
                  ? "repo already linked to this project"
                  : constraint(e) === "project_repos_one_per_project"
                    ? "project already has a linked repo; unlink it first"
                    : "repo already linked to another project",
              });
            }
            throw e;
          }
          return redirect(`/p/${p.public_name}/settings`);
        }),
      },
      "/p/:owner/:name/unlink-repo": {
        POST: handle("/p/:owner/:name/unlink-repo", async (req) => {
          const { s } = await sessionForm(req);
          const p = await ownedProject(s.userId, req.params.owner, req.params.name);
          // project_repos is state, not evidence: this is its one allowed update.
          await sql.begin(async (tx) => {
            const r = await tx`update project_repos set unlinked_at = now()
              where project_id = ${p.id} and unlinked_at is null returning github_repo_id`;
            // A record matched the old repo; it must not outlive the link.
            if (r.length) {
              await tx`update projects set upd_record_id = null where id = ${p.id}`;
              await bumpChange(tx, p.id);
            }
          });
          return redirect(`/p/${p.public_name}/settings`);
        }),
      },
      "/p/:owner/:name/upd-link": {
        POST: handle("/p/:owner/:name/upd-link", async (req) => {
          const { s, form } = await sessionForm(req);
          const p = await ownedProject(s.userId, req.params.owner, req.params.name);
          const full = await liveRepoName(p.id);
          if (!full) throw new HttpError(400, { error: "link a GitHub repo first" });
          const id = String(form.get("record_id") ?? "");
          // Re-derived from the live list: the form only names a candidate.
          if (!(await findUpdEntries(full, fetchFn)).some((e) => e.id === id))
            throw new HttpError(400, { error: "not a match for the linked repo" });
          await sql.begin(async (tx) => {
            await tx`update projects set upd_record_id = ${id} where id = ${p.id}`;
            await bumpChange(tx, p.id);
          });
          return redirect(`/p/${p.public_name}/settings`);
        }),
      },
      "/p/:owner/:name/upd-unlink": {
        POST: handle("/p/:owner/:name/upd-unlink", async (req) => {
          const { s } = await sessionForm(req);
          const p = await ownedProject(s.userId, req.params.owner, req.params.name);
          await sql.begin(async (tx) => {
            const r = await tx`update projects set upd_record_id = null
              where id = ${p.id} and upd_record_id is not null returning id`;
            if (r.length) await bumpChange(tx, p.id);
          });
          return redirect(`/p/${p.public_name}/settings`);
        }),
      },
      "/p/:owner/:name/settings": {
        POST: handle("/p/:owner/:name/settings", async (req) => {
          const { s, form } = await sessionForm(req);
          const p = await ownedProject(s.userId, req.params.owner, req.params.name);
          const next = form.get("public") === "on";
          // A flip changes what badges and pages show, so it bumps change_seq (the badge etag).
          await sql.begin(async (tx) => {
            const r = await tx`update projects set public = ${next}
              where id = ${p.id} and public is distinct from ${next} returning id`;
            if (r.length) await bumpChange(tx, p.id);
          });
          return redirect(`/p/${p.public_name}/settings`);
        }),
        GET: handle("/p/:owner/:name/settings", async (req, server) => {
          limitPage(req, server);
          const s = await requireUser(sql, req);
          const p = await ownedProject(s.userId, req.params.owner, req.params.name);
          const [{ public: isPublic }] = await sql`select public from projects where id = ${p.id}`;
          const linked = await sql`select r.id, r.full_name from project_repos l
            join github_repos r on r.id = l.github_repo_id
            where l.project_id = ${p.id} and l.unlinked_at is null`;
          const csrf = html`<input type="hidden" name="csrf" value="${s.csrf}">`;
          const action = `/p/${p.public_name}`;
          const back = `${action}/settings`;
          let repos: Safe;
          if (linked.length) {
            repos = html`<p>Linked: ${linked[0].full_name}</p>
<form method="post" action="${action}/unlink-repo">${csrf}<button>Unlink</button></form>`;
          } else if (!(await userToken(sql, s.userId))) {
            repos = html`<p><a href="/github/connect?return_to=${encodeURIComponent(back)}">Connect GitHub</a> to link a repo (<a href="/github/install">install the App</a> first).</p>`;
          } else {
            const list = await withGhToken(s.userId, back, () => userRepos(sql, s.userId, fetchFn));
            repos = html`<ul>${list.map(
              (r) =>
                html`<li>${r.full_name} <form class="inline" method="post" action="${action}/link-repo">${csrf}<input type="hidden" name="repo_id" value="${r.id}"><button>Link</button></form></li>`,
            )}</ul><p><a href="/github/install">Install the App on more repos</a></p>`;
          }
          let upd = html``;
          if (linked.length) {
            const [{ upd_record_id: cur }] =
              await sql`select upd_record_id from projects where id = ${p.id}`;
            const curRow = cur
              ? html`<p>${(await updLabel(cur, linked[0].full_name, fetchFn)) ?? ""}</p>
<form method="post" action="${action}/upd-unlink">${csrf}<button>Unlink</button></form>`
              : html``;
            let found: Safe;
            try {
              found = html`<ul>${(await findUpdEntries(linked[0].full_name, fetchFn))
                .filter((e) => e.id !== cur)
                .map(
                  (e) =>
                    html`<li>Found in Unified Projects DB: ${e.ysws}, approved ${e.approved_at === null ? "(not yet)" : new Date(e.approved_at * 1000).toISOString().slice(0, 10)} <form class="inline" method="post" action="${action}/upd-link">${csrf}<input type="hidden" name="record_id" value="${e.id}"><button>Link</button></form></li>`,
                )}</ul>`;
            } catch {
              found = html`<p>Unified Projects DB is unavailable right now.</p>`;
            }
            upd = html`${curRow}${found}`;
          }
          return page(
            p.public_name,
            html`<h1>${p.public_name}</h1><p><a href="${action}">View the project page</a></p><p>server id: ${p.id}</p><pre>${`tally project link <path> --server-id ${p.id}`}</pre>
<form method="post" action="${back}">${csrf}<label><input type="checkbox" name="public"${isPublic ? html` checked` : html``}> Public project</label> <button>Save</button></form>${repos}${upd}`,
            { viewer: viewerOf(s) },
          );
        }),
      },
      "/github/webhook": {
        // No cookies and no rate limit (GitHub's addresses are shared): the signature is the auth.
        POST: handle("/github/webhook", async (req) => {
          const tooBig = () => new HttpError(413, { error: "body_too_large", max: MAX_BODY_BYTES });
          if (Number(req.headers.get("content-length")) > MAX_BODY_BYTES) throw tooBig();
          const raw = new Uint8Array(await req.arrayBuffer());
          if (raw.byteLength > MAX_BODY_BYTES) throw tooBig();
          if (
            !verifySignature(cfg.githubWebhookSecret, raw, req.headers.get("x-hub-signature-256"))
          )
            throw new HttpError(401, { error: "bad_signature" });
          const id = req.headers.get("x-github-delivery");
          const event = req.headers.get("x-github-event");
          if (!id || !event || id.length > 100 || event.length > 100)
            throw new HttpError(400, { error: "bad_headers" });
          let payload: unknown;
          try {
            payload = JSON.parse(new TextDecoder().decode(raw));
          } catch {
            throw new HttpError(400, { error: "invalid_json" });
          }
          // One transaction: a delivery is never recorded without its effects.
          return sql.begin(async (tx) => {
            const ins =
              await tx`insert into webhook_deliveries (delivery_id, event) values (${id}, ${event})
              on conflict do nothing returning delivery_id`;
            if (!ins.length) return json(200, { duplicate: true });
            return json(
              200,
              (await dispatch(tx, id, event, payload)) ? { ok: true } : { ignored: true },
            );
          });
        }),
      },
      // Public names contain a slash and Bun does not split a ".svg" suffix off a param: match it here.
      "/badge/:owner/:file": {
        GET: handle("/badge/:owner/:file", async (req, server) => {
          const m = /^(.+)\.(svg|json)$/.exec(req.params.file);
          if (!m) return notFound();
          if (!limiter.take(`badge:${ip(req, server)}`, rates.badge))
            return rateLimited(m[2] as "svg" | "json");
          return badgeResponse(
            sql,
            `${req.params.owner}/${m[1]}`,
            m[2] as "svg" | "json",
            req.headers.get("if-none-match"),
          );
        }),
      },
      "/v1/projects/:id/summary": {
        GET: handle("/v1/projects/:id/summary", async (req, server) => {
          limit(`ip:${ip(req, server)}`); // not in the table: plan 06's per-ip rate
          const [p] = await sql`select p.id, p.public_name, u.handle, u.profile_public
            from projects p join users u on u.id = p.owner_id
            where p.id = ${req.params.id} and p.public`;
          if (!p) throw new HttpError(404, { error: "not_found" });
          return json(
            200,
            {
              id: p.id,
              public_name: p.public_name,
              owner_handle: p.profile_public ? p.handle : null,
              ...(await projectHours(sql, p.id)),
              // Server time of the newest receipt, or null.
              last_commit_at: (
                await sql`select max(received_at) as t from push_receipts where project_id = ${p.id}`
              )[0].t,
            },
            { "cache-control": "public, max-age=60" },
          );
        }),
      },
      "/v1/projects/:id/badge.json": {
        GET: handle("/v1/projects/:id/badge.json", async (req, server) => {
          limit(`badge:${ip(req, server)}`, rates.badge);
          const cache = { "cache-control": "public, max-age=300" };
          const [p] = await sql`select public, owner_id from projects where id = ${req.params.id}`;
          if (!p) throw new HttpError(404, { error: "not_found" });
          // Owned and private (or the owner deleted their account): the same answer as /badge.
          // Unowned plan 06 projects keep serving hours.
          if (p.owner_id && !p.public)
            return json(200, { ...badge(0), message: "private", color: "lightgrey" }, cache);
          return json(200, badge((await verifiedSeconds(sql, req.params.id)) ?? 0), cache);
        }),
      },
    },
    fetch: () => Response.json({ error: "not_found" }, { status: 404 }),
  });
}
