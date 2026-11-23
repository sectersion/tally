# Server Accounts, Projects and Public Profiles Implementation Plan (plan 09)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Users sign in to the Tally server with Hack Club Auth and link a daemon with `tally login` (a device-code flow). They create projects with unique public names, link each one to a GitHub repo and, where possible, to the Hack Club Unified Projects DB, and install the Tally GitHub App so the server records its own receive time for every push. The server then serves public profile and project pages, an SVG and JSON badge service, and a project page that updates live over server-sent events.

**Architecture:** New modules inside plan 06's `server/` service: one Bun process using `Bun.serve` routes and `Bun.sql`. There is no new process and no frontend build. Pages are server-rendered HTML strings plus one small static script for SSE. Postgres `LISTEN/NOTIFY` delivers "project changed" events to whichever server instance holds the SSE connection. Every write that changes public numbers (plan 06's ledger append, and push receipts from this plan) runs `pg_notify('tally_project', <project id>)` in the same transaction.

**Tech Stack:** Bun (version pinned by plan 06), TypeScript strict, `Bun.sql`, and `zod` (from plan 06). The one new runtime dependency is `jose`, which verifies Hack Club RS256 ID tokens against their JWKS; hand-rolled JWT verification is easy to get wrong. `node:crypto` covers HMAC, `timingSafeEqual` and random bytes. Tests use `bun test` against `DATABASE_URL`, as in plan 06.

**Spec:** `PITCH.md` ("Trust anchors", "Privacy", "Cost", "The path to 1,000 weighted projects"), `docs/superpowers/plans/README.md` (C7, C8, C9), `docs/superpowers/plans/2026-11-23-06-server-core.md` (all of it), and `docs/superpowers/plans/2026-11-23-07-daemon-sync.md` (decisions 3 and 5, on `tally login` and server ids). Read them before every task.

## Assumes from earlier plans (code on `main` wins; rename here, never duplicate)

From plan 06 (`docs/superpowers/plans/2026-11-23-06-server-core.md`):
- `server/src/app.ts`: `createServer(deps: { sql: SQL; port: number }): Server`, using the `Bun.serve` `routes` table and a JSON 404 fallback. This plan adds routes there.
- `server/src/db.ts`: `connect(url)` and `migrate(sql, dir)`, which apply `server/migrations/NNN_*.sql` in name order. This plan's migrations are `100_*.sql` to `103_*.sql` (three digits; `103_` only if Task 4 needs it), so they sort after any plan 06 file `0NN_`. Plan 10 continues at `104_submissions.sql` (ruling 4).
- `server/migrations/001_init.sql` tables:
  - `projects(id text pk, public_name text not null unique, owner_id text null, created_at)`. `owner_id` is created by plan 06 (nullable); this plan fills it and adds the foreign key. URL `{project}` segments are the server `projects.id` (cross-plan ruling 11), never the public name.
  - `chains(id, project_id, ruleset, head_hash, count, verified_s)`, where `verified_s` is a running sum.
  - `ledger_entries(chain_id, seq, hash, prev_hash, action_id, ts, kind, verified_s, unverified_s, entry json, received_at)`. Note there is no `project_id`; join through `chains`.
  - `commits(id, project_id, hash, ts, received_at)`. These are C7 **hash commits** of the ledger head, not git commits. This plan calls git pushes **receipts** to keep the two apart.
  - `device_tokens(id bigserial, project_id text not null, token_hash text unique, label, created_at, revoked_at)`.
  - The trigger function `tally_append_only()`.
- `server/src/auth.ts`: `newToken()` (`"tly_" + base64url(32 bytes)`), `hashToken(token)` (hex SHA-256), and `requireToken(sql, req, projectId): Promise<{ tokenId: number }>` (`401` unknown or revoked, `403` wrong project).
- `server/src/errors.ts`: `HttpError`, `json(status, body)`, `readJson(req)`, `parse(schema, value)`.
- `server/src/ledger.ts`: `appendEntries(sql, projectId, body, ruleset?)` and `getHead(sql, projectId, ruleset?)` (plan 06's amended signatures: a trailing optional ruleset parameter, default the current `rulesetVersion()`; this plan calls both with the default). `appendEntries` runs in one `sql.begin` transaction that locks the chain row. Also `verifiedSeconds(...)`. The badge function `badge(seconds)` follows C8 with blue color, and its message is `floor(verified_s / 360) / 10` hours, formatted with one decimal: floor to a tenth, never above verified time (ruling 9).
- `server/src/ratelimit.ts`: an in-memory token bucket. This plan reuses it with new keys; it does not write a second limiter.
- `server/src/log.ts`: `log(level, msg, fields)`, with redaction of named fields.
- `server/test/helpers.ts`: `seed(sql, publicName?)` returns `{ projectId, token, publicName }`, and `start(sql)` returns `{ url, stop }`.
- Spec §6, as amended by plan 05 (Step 4b; plan 06 only verifies the amendment): an entry's `project` is the **public name**, and the hash covers it. A project's public name therefore can never change once entries exist.

From plan 07: `tally login` reads a token from stdin, and the user gives `tally project link <path> --server-id <id>` from the web UI. This plan supplies that id on the project settings page, and adds the device flow (Task 4) that `tally login --browser` (plan 12) uses, alongside plan 07's `tally login`, without changing plan 07's storage.

## Global Constraints

- **C7 is unchanged:** routes, bodies and status codes stay as they are. Browsers authenticate with a session cookie and daemons with a device token. A device token never works as a session, and a session never authorizes `/v1/ledger/*` writes.
- **Secrets at rest are hashes:** sessions, device tokens and device codes are stored as `hashToken(value)` (plan 06's function), never raw, and every value comes from `newToken()`. Configuration comes from environment variables, added to plan 06's `loadConfig`: `PUBLIC_BASE_URL`, `HCA_CLIENT_ID`, `HCA_CLIENT_SECRET`, `GITHUB_APP_SLUG`, `GITHUB_APP_CLIENT_ID`, `GITHUB_APP_CLIENT_SECRET`, and `GITHUB_WEBHOOK_SECRET`. If one is missing, startup throws `"<NAME> is required"`. The HCA redirect URI is always `PUBLIC_BASE_URL + "/auth/callback"`.
- **Cookie:** `tally_session=<token>; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=2592000` (30 days). `Secure` is omitted only when `PUBLIC_BASE_URL` starts with `http://localhost`.
- **CSRF:** every state-changing browser route (a `POST` authenticated by the session cookie) requires two things. The `Origin` header must equal `PUBLIC_BASE_URL`, and the form field `csrf` must equal the session's CSRF token, compared with `timingSafeEqual`. If either fails, the route returns `403` and writes nothing. Webhooks and `/v1` bearer routes do not read cookies, so they are exempt.
- **Public pages show only public data:** project public name, linked GitHub repo full name, Unified Projects DB link, verified and unverified seconds (total and per UTC day), and per push the commit SHA, the server receive time and the payload's claimed commit time; plus the user's `handle` and `display_name`. **Never public:** email, Hack Club `sub`, Slack id, verification status, ledger `reasons` and `features`, token labels, commit messages, author names or emails. Commit messages and author fields are dropped from payloads **before** storage.
- **Unverified seconds are shown separately, labelled "not counted", and never added** to any total, badge, JSON hour field or SSE number. Every surface reads one function, `projectHours(sql, projectId)`.
- **Server-witnessed time:** `push_receipts.received_at` is Postgres `now()` at insert, never a payload value. Payload times are stored as `claimed_ts` and always labelled "claimed".
- **Append-only:** `push_receipts` and `webhook_deliveries` get the `tally_append_only()` triggers. Revocation and rename are recorded on other tables, never by editing a receipt.
- **HTML:** server-rendered. Every interpolated value passes through the single `esc()` from Task 8, so a raw value cannot reach a template. HTML responses carry `content-type: text/html; charset=utf-8` and the CSP `default-src 'none'; style-src 'self'; script-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'`. There are no inline scripts or handlers. The only script is `/static/live.js`, under 40 lines. Pages work fully without JavaScript; JS only adds live updates. (Plan 06's "every response is JSON" now applies to `/v1/*` only. Task 8 records this.)
- **Cost:** no new infrastructure: no Redis, no queue, no second service, no paid CDN. Task 13 checks this against PITCH.md.
- TypeScript `strict`, no `any` in new code, `bun run typecheck` and `bun run lint` clean. License `AGPL-3.0-only`.

## Review Focus

1. **A user who revokes the GitHub App** (uninstalls it, removes the repo from the installation, or revokes their own user authorization). Expected: `installation` `deleted`/`suspend` sets `github_installations.revoked_at`, and `installation_repositories` `removed` sets `project_repos.unlinked_at` (append a row to `repo_events`; never edit receipts). Old receipts stay visible under "Pushes witnessed while the App was installed (until <date>)". New pushes stop, because GitHub stops sending them. A push that still arrives for an unlinked repo is stored with `project_id NULL` and never shown on a page. `github_app_authorization` `revoked` deletes the stored user-to-server token. Ledger hours are unaffected in every case. Test in Task 6.
2. **A repo renamed or transferred.** Expected: repos are keyed by GitHub's numeric `repository.id`, never by name. `repository` `renamed`/`transferred` updates `github_repos.full_name` and appends to `repo_events`, and receipts keep their link. The project's Tally `public_name` does **not** change, because it is hashed into every ledger entry. The page shows "GitHub repo: <new full name> (was <old>)". `GET /badge/{owner}/{repo}.svg` resolves by Tally `public_name`, so an existing README badge keeps working after a rename. Test in Tasks 6 and 10.
3. **A webhook replay.** Expected: a missing or wrong `X-Hub-Signature-256` returns `401` and writes nothing. A valid signature with an `X-GitHub-Delivery` already stored returns `200 {"duplicate":true}` and writes nothing, so the first receive time stands. GitHub's "Redeliver" reuses the delivery id, so it is a duplicate. A *new* delivery repeating a push already stored (same repo and `after` SHA, which happens with force-push loops or a replay from a captured payload with a fresh id) needs the secret to sign. It is stored, and the page shows only the **earliest** receipt per commit SHA, so a replay can never make a commit look later or earlier than first witnessed. Test in Task 6.
4. **A project linked by two users.** Expected: a GitHub repo links to at most one project (`unique (github_repo_id)` where `unlinked_at is null`), and a second user gets `409 "repo already linked to another project"`. Linking needs proof: the repo must be visible to the signed-in user through a user-to-server token for an installation that includes it (Task 5). Public names are unique case-insensitively (`unique (lower(public_name))`), and a name of the form `owner/repo` must match a repo the user has proved access to. Otherwise the name must start with the user's own `handle/`. So nobody can claim `torvalds/linux` without access to it. A project has one owner. Test in Tasks 3 and 5.
5. **SSE connections surviving a deploy.** Expected: the stream starts with `retry: 3000`, and each event carries `id: <projects.change_seq>`. When the process exits, the browser reconnects with `Last-Event-ID`. The new process sends a snapshot immediately if `change_seq` differs, and nothing if it is equal. No state lives only in the old process. On `SIGTERM` the server closes all streams before exiting, so a deploy drains in seconds. A `: ping` comment every 25 s keeps proxies from closing idle streams. Test in Task 11.
6. **A public page for a project with zero verified hours.** Expected: status `200`. The page shows "0.0 h verified" and, if there are any unverified seconds, "N min unverified (not counted)". An empty push list says "No pushes witnessed yet", and the daily chart is empty with no error. The badge shows `0.0 h` (C8 format and color), never an error. A project with no ledger, no repo and no UPD link also renders. Test in Tasks 9 and 10.
7. **A private profile.** Expected: with `profile_public = false`, `/u/{handle}` returns the same `404` page as a nonexistent handle (not `403`, which would confirm the account exists). The user is also removed from the profile link on project pages. A project's own `public` flag is separate: if it is true, the project page and badge stay up, so a README badge never breaks. If it is false, the project page is `404` and the badge shows "private". Test in Task 9.
8. **Abuse of the device-code flow.** Expected: a `user_code` can be approved once, within 10 minutes, by a signed-in user with a valid CSRF token. Polling faster than every 5 s returns `slow_down`. The token is returned exactly once, and only its hash is stored. Test in Task 4.

## File Structure

```
server/migrations/100_accounts.sql        users, sessions, oauth_states, device_codes; device_tokens.user_id
server/migrations/101_projects.sql        projects: owner fk, public flag, upd link, change_seq, lower(name) unique
server/migrations/102_github.sql          github_installations, github_repos, project_repos, repo_events,
                                          github_user_tokens, webhook_deliveries, push_receipts
server/src/hca.ts                         Hack Club Auth: authorize URL, code exchange, ID token check
server/src/session.ts                     sessions, cookie, CSRF, requireUser()
server/src/device.ts                      device-code flow (RFC 8628 shape)
server/src/projects.ts                    create projects, name rules, projectHours(), projectDaily(), bumpChange()
server/src/upd.ts                         Unified Projects DB lookup (shape fixed by the Task 7 spike)
server/src/github.ts                      user OAuth, repo access check, webhook verify and dispatch, receipts
server/src/html.ts                        esc(), page(), the page renderers
server/src/pages.ts                       browser routes: /, /u/:handle, /p/:owner/:name, /settings...
server/src/badge.ts                       SVG badge and the /badge/ routes
server/src/live.ts                        LISTEN bridge and the SSE endpoint
server/static/live.js                     EventSource client
server/static/style.css                   minimal styles
server/test/<module>.test.ts              one test file per module
server/test/fixtures/hca/*.json           Task 1 samples (values replaced)
server/test/fixtures/github/*.json        Task 5 captured webhook payloads (values replaced)
docs/spikes/hack-club-auth.md             Task 1 findings
docs/spikes/github-app.md                 Task 5 findings
docs/spikes/unified-projects-db.md        Task 7 findings
docs/cost.md                              Task 13 cost envelope ("Server" section; create the file if absent)
```

---

### Task 1: Spike: Hack Club Auth flow, recorded with sources

The design below rests on what the docs said on 2026-11-22. Confirm it against the live service before writing any code, because the docs leave several things unstated.

**Files:**
- Create: `docs/spikes/hack-club-auth.md`, `server/test/fixtures/hca/discovery.json`, `server/test/fixtures/hca/id_token_claims.json` (claims only, with real values replaced)

**What the docs already state (verify each, then record it with the URL and the date checked):**

| Fact | Source |
|---|---|
| Issuer `https://auth.hackclub.com`; discovery at `/.well-known/openid-configuration` | https://auth.hackclub.com/docs/oidc-guide, https://auth.hackclub.com/.well-known/openid-configuration |
| `authorization_endpoint` `https://auth.hackclub.com/oauth/authorize`, `token_endpoint` `https://auth.hackclub.com/oauth/token`, `userinfo_endpoint` `https://auth.hackclub.com/oauth/userinfo`, `jwks_uri` `https://auth.hackclub.com/oauth/discovery/keys`, plus `revocation_endpoint` `/oauth/revoke` | discovery document |
| `response_types_supported: ["code"]`; `grant_types_supported: ["authorization_code","client_credentials","refresh_token"]` (no device grant); `token_endpoint_auth_methods_supported: ["client_secret_basic","client_secret_post"]` | discovery document |
| ID token is an RS256 JWT; `sub` looks like `ident!abc123` | oidc-guide |
| Community apps may request `openid profile email name slack_id verification_status`; other scopes need HQ approval | oidc-guide, https://auth.hackclub.com/docs/oauth-guide |
| `verification_status` scope adds the `verification_status` claim (`needs_submission`, `pending`, `verified`, `ineligible`) and `ysws_eligible` | oidc-guide, https://auth.hackclub.com/docs/api |
| Apps are registered at `https://auth.hackclub.com/developer/apps`; new apps start at trust level `community_untrusted`, which shows users a warning-heavy consent screen | oidc-guide, oauth-guide |
| `GET /api/v1/me` with the access token returns `{identity: {id, first_name, ..., slack_id, verification_status, ysws_eligible}, scopes}`; it has no GitHub username | https://auth.hackclub.com/docs/api |

**Not stated anywhere (each is a spike step; do not assume):**
- PKCE: `code_challenge_methods_supported` is absent from discovery. Test whether `/oauth/authorize` accepts `code_challenge` with `S256` and whether `/oauth/token` enforces `code_verifier`.
- `nonce`: whether the ID token echoes a `nonce` sent on the authorize request.
- `state`: whether it is returned unchanged (expected by OAuth 2.0, but confirm).
- Whether the token response includes `id_token` when `openid` is requested.
- ID token and access token lifetimes (the docs show only an example token).
- Logout: there is no `end_session_endpoint`. Confirm, and record that logout is local only (Task 2).

- [ ] **Step 1:** Register a development app at `https://auth.hackclub.com/developer/apps` with redirect URI `http://localhost:8080/auth/callback` and scopes `openid profile email verification_status`. Record its trust level in the spike doc. Keep the secret out of the repo.
- [ ] **Step 2:** `curl -s https://auth.hackclub.com/.well-known/openid-configuration > server/test/fixtures/hca/discovery.json`. Diff it against the table above, and update the table if anything changed.
- [ ] **Step 3:** In a browser, open `/oauth/authorize?client_id=...&redirect_uri=...&response_type=code&scope=openid%20profile%20email%20verification_status&state=s1&nonce=n1&code_challenge=<S256 of a known verifier>&code_challenge_method=S256`. Record whether the consent screen appears and whether `state=s1` comes back.
- [ ] **Step 4:** Exchange the code with `curl -u client_id:secret -d grant_type=authorization_code -d code=... -d redirect_uri=... -d code_verifier=<verifier>`. Then repeat with a wrong verifier. Record whether PKCE is enforced, and the response fields (`id_token` present or not, `expires_in`).
- [ ] **Step 5:** Decode the ID token payload (without the signature). Write the claim names, with values replaced, to `id_token_claims.json`. Record whether `nonce` equals `n1` and what `exp - iat` is.
- [ ] **Step 6:** Write `docs/spikes/hack-club-auth.md` covering: the tables above with source URLs and check date, each answer from Steps 3 to 5, and a "Decision" line for each unknown. Defaults if the spike cannot settle one: send PKCE anyway (harmless if ignored); require `nonce` only if it is echoed; use `/oauth/userinfo` for claims if the token response has no `id_token`.
- [ ] **Step 7:** Commit: `docs(server): hack club auth spike`.

**Done when:** each unknown above has a recorded answer or an explicit "could not verify; fallback is X".

---


### Task 2: Accounts, sign-in, sessions, CSRF, logout

**Files:**
- Create: `server/migrations/100_accounts.sql`, `server/src/hca.ts`, `server/src/session.ts`, `server/test/hca.test.ts`, `server/test/session.test.ts`
- Modify: `server/src/config.ts` (new env vars), `server/src/app.ts` (routes), `server/package.json` (add `jose`), `server/src/log.ts` (add `cookie`, `csrf`, `code`, `client_secret` to the redacted field names)

**Interfaces:**
- `100_accounts.sql`:

```sql
create table users (
  id text primary key,                          -- crypto.randomUUID()
  hca_sub text not null unique,                 -- private
  email text, name text,                        -- private, refreshed each sign-in
  verification_status text,                     -- private ("verified", "pending", ...)
  handle text,                                  -- public, set in settings; null until chosen
  display_name text,                            -- public
  profile_public boolean not null default true,
  role text not null default 'user' check (role in ('user','reviewer','admin')),  -- plan 10 reviewer pages
  created_at timestamptz not null default now()
);
create unique index users_handle_lower on users (lower(handle));
create table sessions (
  token_hash text primary key,
  user_id text not null references users(id) on delete cascade,
  csrf text not null,                           -- newToken(), sent in forms, not a secret from the user
  expires_at timestamptz not null
);
create table oauth_states (
  state_hash text primary key,
  verifier text not null, nonce text not null, return_to text not null,
  expires_at timestamptz not null               -- now() + 10 min
);
```

- `hca.ts` (endpoints come from the Task 1 spike doc, as constants):
  - `authorizeUrl(cfg, state, nonce, challenge): string` builds `https://auth.hackclub.com/oauth/authorize` with `response_type=code`, `scope=openid profile email verification_status`, `redirect_uri`, `state`, `nonce`, `code_challenge`, and `code_challenge_method=S256`.
  - `exchangeCode(cfg, code, verifier, nonce, fetchFn = fetch): Promise<HcaUser>`, where `HcaUser = { sub: string; email: string | null; name: string | null; verification_status: string | null }`. It posts the form to `/oauth/token` with `client_secret_post`. It verifies `id_token` with `jwtVerify(t, createRemoteJWKSet(new URL("https://auth.hackclub.com/oauth/discovery/keys")), { issuer: "https://auth.hackclub.com", audience: cfg.hcaClientId })`, and checks `nonce` when Task 1 found it echoed. If Task 1 found no `id_token`, it uses `GET /oauth/userinfo` with the access token instead. The access and refresh tokens are **not stored**: Tally needs identity only.
  - For tests, the JWKS source is injectable as an optional `jwks` parameter.
- `session.ts`:
  - `createSession(sql, userId): Promise<{ setCookie: string; csrf: string }>`.
  - `getSession(sql, req): Promise<{ userId: string; csrf: string } | null>`, which ignores expired rows.
  - `requireUser(sql, req): Promise<{ userId: string; csrf: string }>`. With no session it throws `HttpError`: a `303` to `/auth/login?return_to=<path>` for `GET`, and `401` otherwise. Add an `HttpError` variant for a redirect (`location` header) if plan 06's has none.
  - `requireCsrf(cfg, req, s, form: FormData): void`, which throws `HttpError(403, {error:"csrf"})`.
  - `requireRole(user: { role: string }, role: "reviewer" | "admin"): void` throws `HttpError(404)` unless `user.role === role` or `user.role === "admin"`. `requireUser` also returns `role`. Plan 10 calls it for reviewer pages.
  - Granting a role: admin CLI `bun run user:role <handle> reviewer` (script `server/scripts/user-role.ts`, package.json script `user:role`; values `user|reviewer|admin`; updates `users.role` by lower(handle)). There is no web UI for roles. Add a test that `requireRole` rejects `user` and admits `admin`.
- Routes:
  - `GET /auth/login?return_to=`: `return_to` must match `^/(?!/)` or becomes `/`. Inserts the `oauth_states` row and sends a `303` to `authorizeUrl`.
  - `GET /auth/callback?code&state`: `delete from oauth_states where state_hash = $1 and expires_at > now() returning *`. No row means `400`. Then `exchangeCode`, then `insert ... on conflict (hca_sub) do update set email, name, verification_status`. Then `createSession`, and a `303` to `return_to`.
  - `POST /auth/logout` (CSRF): deletes the session row, sets `tally_session=; Max-Age=0`, and sends a `303` to `/`. This is local logout only. Hack Club Auth has no end-session endpoint (Task 1), so the HCA session stays, which is standard for OIDC clients without RP-initiated logout.

- [ ] **Step 1: Write the failing tests.** In `hca.test.ts`, create a key pair with `jose.generateKeyPair("RS256")`, sign ID tokens with `SignJWT`, pass a local JWKS, and stub `fetchFn` to return `{ id_token, access_token }`:

```ts
test("valid id token gives the user", async () => { expect((await exchange(tok({}))).sub).toBe("ident!abc"); });
test("wrong audience is rejected", async () => { await expect(exchange(tok({ aud: "other" }))).rejects.toThrow(); });
test("wrong issuer is rejected", async () => { await expect(exchange(tok({ iss: "https://evil" }))).rejects.toThrow(); });
test("expired token is rejected", async () => { await expect(exchange(tok({ exp: 1 }))).rejects.toThrow(); });
test("wrong nonce is rejected", async () => { await expect(exchange(tok({ nonce: "x" }))).rejects.toThrow(); }); // drop if Task 1 found no nonce
```

  In `session.test.ts`, run against a started server (`start(sql)`), with `exchangeCode` stubbed through a module-level setter `setHcaExchangeForTests`:

```ts
test("unknown state is 400 and creates no user", ...);
test("a state works once", ...);                         // second callback with the same state: 400
test("return_to cannot leave the site", ...);            // //evil.example and https://evil both become "/"
test("cookie flags", ...);                               // HttpOnly; SameSite=Lax; Path=/; Max-Age=2592000
test("sessions store only the hash", ...);               // token_hash == hashToken(cookie value)
test("logout without csrf is 403 and keeps the session", ...);
test("logout with a foreign Origin is 403 even with the right csrf", ...);
test("logout with both deletes the session", ...);
test("expired session redirects GET /settings to /auth/login", ...);
test("a session cookie does not authorize POST /v1/ledger/:id/entries", ...); // 401
```

- [ ] **Step 2:** Run `cd server && bun test hca session`. Expected: FAIL.
- [ ] **Step 3:** Write the migration and the config additions, and add `jose` (pin an exact version, `bun add jose@<x.y.z>`).
- [ ] **Step 4:** Implement `hca.ts`, `session.ts` and the three routes.
- [ ] **Step 5:** Run `bun test`. Expected: all PASS, plan 06 included.
- [ ] **Step 6:** Manual check with the Task 1 dev app: start the server with real `HCA_*` values, open `http://localhost:8080/auth/login`, sign in, and confirm a `users` row and the cookie. Record the result in the spike doc.
- [ ] **Step 7:** Commit: `feat(server): hack club auth sign-in, sessions and csrf`.

---

### Task 3: Projects: ownership, public names, `projectHours`

**Files:**
- Create: `server/migrations/101_projects.sql`, `server/src/projects.ts`, `server/test/projects.test.ts`
- Modify: `server/src/app.ts`, `server/src/ledger.ts` (call `bumpChange` inside `appendEntries`'s transaction when something was inserted), `server/scripts/token.ts` (add `claim <project-id> <user-handle>`)

**Interfaces:**
- `101_projects.sql`:

```sql
alter table projects
  add constraint projects_owner_fk foreign key (owner_id) references users(id),
  add column public boolean not null default true,
  add column upd_record_id text,                     -- Task 7; null if unlinked
  add column change_seq bigint not null default 0;   -- SSE event id (Task 11)
create unique index projects_public_name_lower on projects (lower(public_name));
```

  Projects that plan 06's token CLI created have `owner_id` null. Their pages are `404` until the maintainer runs `bun run token claim <id> <handle>`.
- Name rules (`projects.ts`):
  - `validName(name): boolean`: two segments `^[A-Za-z0-9](?:[A-Za-z0-9-]{0,38})\/[A-Za-z0-9._-]{1,100}$`. These are GitHub's owner and repo character sets, so a repo name always fits.
  - `nameAllowed(sql, userId, name): Promise<boolean>`. The first segment, case-insensitively, must equal the user's `handle`. Alternatively the whole name must equal the `full_name` of a GitHub repo the user has proved access to: Task 5 adds that branch, and until then it returns false.
  - Handles: `setHandle(sql, userId, handle): Promise<"ok" | "bad" | "taken" | "reserved">`. A handle matches `^[a-z0-9](?:[a-z0-9-]{0,37}[a-z0-9])?$` and is stored lowercase. Reserved handles: `auth api v1 badge static settings device github admin u p healthz`. It can be set once (renaming would orphan `handle/` project names, which are permanent). Uniqueness comes from catching the unique violation `23505`, never from a pre-check.
- `createProject(sql, userId, name): Promise<{ id: string } | { error: "bad_name" | "not_allowed" | "taken" }>` inserts with `id = crypto.randomUUID()`, `owner_id = userId`. A `23505` is `taken`, which covers a different-case duplicate.
- `projectHours(sql, projectId): Promise<{ verified_s: number; unverified_s: number }>`: `select coalesce(sum(e.verified_s),0)::bigint, coalesce(sum(e.unverified_s),0)::bigint from ledger_entries e join chains c on c.id = e.chain_id where c.project_id = $1`, converted with `Number()`. This sums across rulesets, per C7. Plan 06's badge keeps reading `chains.verified_s`. A test asserts the two agree, so neither drifts.
- `projectDaily(sql, projectId, days = 90): Promise<{ day: string; verified_s: number; unverified_s: number }[]>`, grouped by `to_char(to_timestamp(e.ts / 1000.0) at time zone 'UTC', 'YYYY-MM-DD')`, for days with any entries only.
- `bumpChange(tx, projectId): Promise<void>`: `update projects set change_seq = change_seq + 1 where id = $1`, then `select pg_notify('tally_project', $1)`. Callers run it inside their own transaction, so the notify fires only on commit.
- Routes (session; POST also CSRF):
  - `POST /settings/handle` with form `handle`.
  - `POST /projects` with form `name`, which sends a `303` to `/p/{name}`.
  - `POST /p/:owner/:name/settings` with form `public=on|off` (owner only; otherwise `404`).
  - `GET /v1/projects/:id/summary` (public projects only, else `404`; read-only JSON, `cache-control: public, max-age=60`): `{ id, public_name, owner_handle (null if the owner's profile is private), verified_s, unverified_s, last_commit_at (server `received_at` of the newest `push_receipts` row for the project, or null) }`, built from `projectHours`. Plan 10 should use this and the server-rendered project page `/p/:owner/:name`, not new queries. Add a test for private `404` and the seconds values.
  - `GET /p/:owner/:name/settings` (owner only) shows the server id with the exact command `tally project link <path> --server-id <id>` (plan 07), plus the device tokens for this project (Task 4).

- [ ] **Step 1: Write the failing tests** in `server/test/projects.test.ts`. The helpers `user(sql, handle)` and `ledgerRows(sql, projectId, rows)` insert directly. Each test uses a random handle, so the database never needs cleaning.

```ts
test("names are unique ignoring case", async () => {
  const u = await user(sql, h());
  expect(await createProject(sql, u.id, `${u.handle}/Tally`)).toHaveProperty("id");
  expect(await createProject(sql, u.id, `${u.handle}/tally`)).toEqual({ error: "taken" });
});
test("concurrent creates of one name: exactly one wins", async () => {
  const u = await user(sql, h());
  const r = await Promise.all([createProject(sql, u.id, `${u.handle}/x`), createProject(sql, u.id, `${u.handle}/x`)]);
  expect(r.filter(x => "id" in x).length).toBe(1);
});
test("a name under someone else's handle is not allowed", async () => {
  const a = await user(sql, h()), b = await user(sql, h());
  expect(await createProject(sql, b.id, `${a.handle}/x`)).toEqual({ error: "not_allowed" });
  expect(await createProject(sql, b.id, "torvalds/linux")).toEqual({ error: "not_allowed" });
});
test("handles: reserved, malformed, set once", async () => {
  const u = await user(sql, null);
  expect(await setHandle(sql, u.id, "badge")).toBe("reserved");
  expect(await setHandle(sql, u.id, "-x")).toBe("bad");
  expect(await setHandle(sql, u.id, `Ok-${h()}`)).toBe("ok");
  expect(await setHandle(sql, u.id, h())).toBe("bad");          // already set
});
test("projectHours keeps unverified apart and sums across chains", async () => {
  // chain A (ruleset 2026.11-0): rows (100 verified, 50 unverified); chain B (old ruleset, inserted by SQL): (0, 30)
  expect(await projectHours(sql, p)).toEqual({ verified_s: 100, unverified_s: 80 });
});
test("projectHours agrees with chains.verified_s after real uploads", ...);   // upload a fixture chain via HTTP
test("empty project is zeros, not null", async () => {
  expect(await projectHours(sql, p)).toEqual({ verified_s: 0, unverified_s: 0 });
});
test("an upload that inserts bumps change_seq once; a replayed upload does not", ...);
```

- [ ] **Step 2:** Run `bun test projects`. Expected: FAIL.
- [ ] **Step 3:** Write the migration, then `projects.ts`, then the routes. Add `bumpChange` to `appendEntries`: one line inside the existing transaction, only when the insert count is above 0. Add the `claim` subcommand to `scripts/token.ts`.
- [ ] **Step 4:** Run `bun test`. Expected: PASS, plan 06's tests included.
- [ ] **Step 5:** Commit: `feat(server): project ownership, public names and hours`.

---

### Task 4: Device linking for `tally login` (device-code flow)

Hack Club Auth offers no device grant. Its `grant_types_supported` is `authorization_code`, `client_credentials` and `refresh_token` (Task 1). So Tally runs its own flow shaped like RFC 8628, and the user signs in to Tally in the browser as usual. The flow issues a **user** token. Plan 06 tokens are scoped to one project, so this task widens `requireToken` as described below. Plan 06's project-scoped tokens keep working unchanged.

**Files:**
- Create: `server/src/device.ts`, `server/test/device.test.ts`
- Modify: `server/migrations/100_accounts.sql` if Task 2 is not merged yet (otherwise add `103_device.sql`), `server/src/auth.ts`, `server/src/app.ts`

**Interfaces:**
- SQL:

```sql
alter table device_tokens
  alter column project_id drop not null,
  add column user_id text references users(id),
  add constraint device_tokens_scope check (project_id is not null or user_id is not null);
create table device_codes (
  code_hash text primary key,
  user_code text not null unique,          -- 8 letters from BCDFGHJKLMNPQRSTVWXZ, shown as XXXX-XXXX
  device_name text not null,               -- from the daemon, 1..64 chars; shown on the approval page
  user_id text references users(id),       -- set on approval
  denied boolean not null default false,
  consumed boolean not null default false,
  last_poll_at timestamptz,
  expires_at timestamptz not null          -- now() + 10 min
);
```

- `auth.ts`, `requireToken(sql, req, projectId)`: a token with `project_id = projectId` passes, as before. A token with `project_id is null` passes only if `user_id` is the project's `owner_id` (`projectId` is the server `projects.id`, ruling 11). Anything else is `403`. It is still one query (join `projects`).
- `POST /v1/device/code` with `{ "device_name": "..." }` (zod strict) returns `200 { device_code, user_code, verification_uri: "<base>/device", verification_uri_complete: "<base>/device?code=<user_code>", expires_in: 600, interval: 5 }`.
- `POST /v1/device/token` with `{ "device_code": "..." }`, all in one transaction with `select ... for update`:
  - unknown code, expired, or `consumed`: `400 {"error":"expired_token"}`
  - `denied`: `400 {"error":"access_denied"}` (and set `consumed`)
  - `last_poll_at > now() - interval '5 s'`: `400 {"error":"slow_down"}`; `last_poll_at` is updated on every poll
  - `user_id is null`: `400 {"error":"authorization_pending"}`
  - otherwise `200 {"access_token": newToken(), "token_type": "Bearer"}`, inserting `device_tokens(user_id, label = device_name, token_hash)` and setting `consumed = true`
- `GET /device?code=` (session): "Link device '<device_name>' to your Tally account? Check that this code matches your terminal: XXXX-XXXX", with Approve and Deny buttons and the `csrf` field.
- `POST /device` with form `user_code`, `decision=approve|deny`, `csrf` (session plus CSRF). An unknown, expired, consumed or already decided code returns `400` "This code is no longer valid".
- `GET /settings` (Task 9) lists the user's tokens (`label`, `created_at`, scope), each with `POST /settings/tokens/:id/revoke` (CSRF; owner only), which sets `revoked_at`.
- Plan 07 hand-off: this one device-code flow serves both `tally login` (plan 07) and `tally login --browser` (planned in plan 12, ruling 12). `--browser` calls `/v1/device/code`, prints `user_code` and `verification_uri_complete`, polls `/v1/device/token` every `interval` s (adding 5 s on `slow_down`), and stores the token exactly where plan 07's stdin `tally login` stores it.

- [ ] **Step 1: Write the failing tests** in `server/test/device.test.ts`. The helpers are `poll(code)`, `approveAs(user, userCode)` (signs in through a test session and posts the form with csrf and Origin), and `rewindPoll(code)` (`update device_codes set last_poll_at = now() - interval '6 s'`).

```ts
test("code, pending, approve, token exactly once", async () => {
  const c = await post("/v1/device/code", { device_name: "lap" });
  expect(c.user_code).toMatch(/^[BCDFGHJKLMNPQRSTVWXZ]{4}-[BCDFGHJKLMNPQRSTVWXZ]{4}$/);
  expect((await poll(c.device_code)).error).toBe("authorization_pending");
  const u = await user(sql, h()); await approveAs(u, c.user_code); await rewindPoll(c.device_code);
  const t = await poll(c.device_code);
  expect(t.token_type).toBe("Bearer");
  await rewindPoll(c.device_code);
  expect((await poll(c.device_code)).error).toBe("expired_token");
});
test("a user token uploads to the user's projects only", ...);  // own project 200, other user's project 403
test("plan 06 project tokens still work", ...);                 // seed() token on its project: 200
test("polling within 5 s is slow_down", ...);
test("approval without csrf is 403 and the code stays pending", ...);
test("expired code cannot be approved", ...);                   // set expires_at in the past; POST /device -> 400
test("deny gives access_denied", ...);
test("only the token hash is stored", ...);                     // token_hash == hashToken(access_token)
test("a revoked user token is 401", ...);
```

- [ ] **Step 2:** Run `bun test device`. Expected: FAIL.
- [ ] **Step 3:** Implement the migration, `device.ts`, the `requireToken` change and the routes. Generate `user_code` with rejection sampling over the 20-letter alphabet (bytes `< 240`, then `% 20`), and retry on `23505`. Rate-limit `/v1/device/code` per IP with plan 06's limiter, key `device:<ip>`, 10 per minute.
- [ ] **Step 4:** Run `bun test`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(server): device-code flow and user-scoped device tokens`.

---

### Task 5: Spike, then GitHub App install and repo linking

Tally needs only two things from GitHub: webhooks, and proof that the signed-in user can access a repo. So it uses a **user-to-server token** to list the repos the user can reach through the App, and **no installation access tokens** at all. That means no App private key is stored and no App JWT is signed.

**Part A, the spike.** Facts from GitHub's docs, to verify and record in `docs/spikes/github-app.md` with each URL and the date checked:

| Fact | Source |
|---|---|
| Deliveries carry `X-GitHub-Event`, `X-GitHub-Delivery` (a GUID, reused on redelivery) and `X-Hub-Signature-256: sha256=<hex HMAC-SHA256 of the raw body with the webhook secret>`. Compare in constant time. | https://docs.github.com/en/webhooks/using-webhooks/validating-webhook-deliveries |
| The `push` payload has `ref`, `before`, `after`, `forced`, `deleted`, `repository.id`, `repository.full_name`, `installation.id`, `commits[]` (`id`, `timestamp`, `message`, `author`, ...; at most 20 commits, later ones need the compare API) and `head_commit` | https://docs.github.com/en/webhooks/webhook-events-and-payloads#push |
| `installation` (`created`, `deleted`, `suspend`, `unsuspend`), `installation_repositories` (`added`, `removed`), `repository` (`renamed`, `transferred`) and `github_app_authorization` (`revoked`) events | same page, sections per event |
| User access tokens: `https://github.com/login/oauth/authorize?client_id=...&state=...`, exchanged at `POST https://github.com/login/oauth/access_token`. They expire (8 h by default) and have refresh tokens. | https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-user-access-token-for-a-github-app |
| `GET /user/installations` and `GET /user/installations/{installation_id}/repositories` list only what the user can access | https://docs.github.com/en/rest/apps/installations |
| The App's "Setup URL" receives `installation_id` and `setup_action` after an install. These are untrusted query values, used only as a hint. | https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/about-the-setup-url |

To confirm with real deliveries:
- Which permission makes GitHub deliver `push`. The expected answer is repository "Contents: read-only", plus "Metadata: read-only", which every App has.
- Whether `repository` `renamed` reaches an App that subscribes only to `push`.
- Whether `transferred` keeps `repository.id`.

- [ ] **Step 1:** Create a development GitHub App "Tally (dev)". Set webhook URL to a tunnel to `localhost:8080/github/webhook`, a random webhook secret, Setup URL `http://localhost:8080/github/setup`, and Callback URL `http://localhost:8080/github/callback`. Check "Request user authorization (OAuth) during installation". Permissions: Contents read-only, Metadata read-only. Events: Push, Repository.
- [ ] **Step 2:** Install the App on a scratch repo. Push twice and force-push once, rename the repo, transfer it to a second account, remove it from the installation, then uninstall. Save each delivery's headers and body from the App's "Advanced" tab to `server/test/fixtures/github/<event>.<action>.json` as `{ "headers": {...}, "body": "<raw string>" }`. Keep the raw body exactly, since signatures cover it. Re-sign the fixtures with the test secret `test-secret` (a one-off script, not committed), and replace emails and names in the body **before** re-signing.
- [ ] **Step 3:** Write `docs/spikes/github-app.md`: the table above with each fact confirmed or corrected, the answers to the three questions, and the App settings to reproduce in production.
- [ ] **Step 4:** Commit: `docs(server): github app spike and webhook fixtures`.

**Part B, linking.**

**Files:**
- Create: `server/migrations/102_github.sql`, `server/src/github.ts`, `server/test/github-link.test.ts`
- Modify: `server/src/app.ts`, `server/src/projects.ts` (`nameAllowed` repo branch)

**Interfaces:**
- `102_github.sql`:

```sql
create table github_installations (
  id bigint primary key,                 -- installation.id
  account_login text not null,
  revoked_at timestamptz                 -- installation deleted or suspended
);
create table github_repos (
  id bigint primary key,                 -- repository.id; never key by name
  full_name text not null,               -- current name, updated on rename/transfer
  installation_id bigint references github_installations(id)
);
create table project_repos (
  project_id text not null references projects(id),
  github_repo_id bigint not null references github_repos(id),
  linked_at timestamptz not null default now(),
  unlinked_at timestamptz,
  primary key (project_id, github_repo_id, linked_at)
);
create unique index project_repos_one_live on project_repos (github_repo_id) where unlinked_at is null;
create table repo_events (                -- audit trail shown on the page: renamed, transferred, removed, uninstalled
  id bigserial primary key, github_repo_id bigint not null, kind text not null,
  detail text not null, at timestamptz not null default now()
);
create table github_user_tokens (         -- short-lived proof of access; deleted on github_app_authorization.revoked
  user_id text primary key references users(id) on delete cascade,
  github_user_id bigint not null unique,
  access_token text not null,            -- needed raw to call GitHub; expires per GitHub (8 h)
  expires_at timestamptz not null
);
create table webhook_deliveries (
  delivery_id text primary key, event text not null, received_at timestamptz not null default now()
);
create table push_receipts (
  delivery_id text not null references webhook_deliveries(delivery_id),
  sha text not null,
  github_repo_id bigint not null,
  project_id text references projects(id),   -- null when the repo had no live link at receive time
  ref text not null,
  is_head boolean not null,
  forced boolean not null,
  claimed_ts bigint,                         -- commit timestamp from the payload, ms; untrusted
  received_at timestamptz not null default now(),
  primary key (delivery_id, sha)
);
create index push_receipts_project on push_receipts (project_id, received_at);
-- append-only, as plan 06 does for ledger_entries and commits
create trigger push_receipts_no_update before update or delete on push_receipts for each row execute function tally_append_only();
create trigger push_receipts_no_truncate before truncate on push_receipts for each statement execute function tally_append_only();
create trigger webhook_deliveries_no_update before update or delete on webhook_deliveries for each row execute function tally_append_only();
create trigger webhook_deliveries_no_truncate before truncate on webhook_deliveries for each statement execute function tally_append_only();
```

  `github_user_tokens.access_token` is the one raw secret this plan stores, because GitHub needs it. It is short-lived and used only for the linking check. Do **not** request or store the refresh token: an expired token just means "connect GitHub again".
- Routes (session; POST also CSRF):
  - `GET /github/connect` sends a `303` to GitHub's authorize URL with `state` (an `oauth_states` row, as in Task 2).
  - `GET /github/callback?code&state` exchanges the code, calls `GET /user` for `id`, and upserts `github_user_tokens`.
  - `GET /github/setup` sends a `303` to `/github/connect`; its query is ignored apart from logging.
  - `GET /github/install` sends a `303` to `https://github.com/apps/<GITHUB_APP_SLUG>/installations/new`.
  - `GET /p/:owner/:name/settings` (owner) lists the user's accessible repos from `GET /user/installations` and then `GET /user/installations/{id}/repositories`, each with a Link button.
  - `POST /p/:owner/:name/link-repo` with form `repo_id`:
    1. Look up the user's token. If it is missing or expired, send a `303` to `/github/connect`.
    2. Re-fetch the accessible repos from GitHub; never trust the form. A `repo_id` not among them is `403`.
    3. In one transaction, upsert `github_installations` and `github_repos` from GitHub's response, insert `project_repos`, and run `bumpChange`. A `23505` on `project_repos_one_live` is `409` "repo already linked to another project".
  - `POST /p/:owner/:name/unlink-repo` sets `unlinked_at = now()` on the live link. This is the one update allowed on `project_repos`, which is not append-only, because the link is state, not evidence.
- `nameAllowed` repo branch: the name equals, case-insensitively, the `full_name` of a repo in the user's accessible list, fetched live as in step 2 of linking. `POST /projects` with such a name also links that repo in the same transaction.
- GitHub calls go through one `gh(token, path, fetchFn = fetch)` helper. It sets `Accept: application/vnd.github+json`, `X-GitHub-Api-Version: 2022-11-28` and `User-Agent: tally-server`, follows `Link: rel="next"` pagination up to 10 pages, and throws `HttpError(502)` on a non-2xx response.

- [ ] **Step 5: Write the failing tests** in `server/test/github-link.test.ts`, with `fetchFn` stubbed by a map from URL to response:

```ts
test("linking a repo the user can access works and bumps change_seq", ...);
test("a repo id not in the user's GitHub list is 403 (form is not trusted)", ...);
test("second project linking the same repo is 409", ...);      // Review Focus 4, two users
test("unlink then link to another project works", ...);         // partial unique index frees the repo
test("project named owner/repo needs access to that repo", ...); // not in list -> not_allowed; in list -> created and linked
test("expired github token redirects to /github/connect", ...);
test("link without csrf is 403", ...);
```

- [ ] **Step 6:** Run `bun test github-link`. Expected: FAIL.
- [ ] **Step 7:** Implement the migration, `github.ts` (linking half) and the routes.
- [ ] **Step 8:** Run `bun test`. Expected: PASS. Then repeat the Step 2 install manually against the dev App, and link the scratch repo.
- [ ] **Step 9:** Commit: `feat(server): github app install and repo linking`.

---

### Task 6: Webhooks and server-witnessed pushes

**Files:**
- Create: `server/test/webhook.test.ts`
- Modify: `server/src/github.ts` (webhook half), `server/src/app.ts` (`POST /github/webhook`)

**Interfaces:**
- `verifySignature(secret: string, raw: Uint8Array, header: string | null): boolean`. The header must start with `sha256=`, followed by 64 hex characters. Compute HMAC-SHA256 over the **raw bytes** (`await req.arrayBuffer()` before any JSON parse) and compare with `timingSafeEqual` on equal-length buffers. Never read `X-Hub-Signature` (SHA-1).
- `POST /github/webhook`:
  1. Read the body, capped at 1 MiB (plan 06's `MAX_BODY_BYTES`). Over the cap is `413`.
  2. Verify the signature. A failure is `401 {"error":"bad_signature"}`, with nothing written and nothing parsed.
  3. `insert into webhook_deliveries (delivery_id, event) values ($1, $2) on conflict do nothing returning delivery_id`. No row means `200 {"duplicate":true}`. Steps 3 and 4 run in **one transaction**, so a crash between them cannot record a delivery without its effects. GitHub then redelivers, and the redelivery is processed.
  4. Dispatch on `X-GitHub-Event` and `action`:
     - `push`: if `deleted` is true (a branch was deleted), store nothing more. Otherwise find the live `project_repos` link for `repository.id` (null if none). Insert one `push_receipts` row per commit in `commits[]` with `is_head = false`, and one for `after` with `is_head = true` (skip the duplicate if `after` is also in `commits[]`; then mark that row as head). Each row gets `ref`, `forced`, and `claimed_ts = Date.parse(commit.timestamp)`, or null if it does not parse. **Do not store** `message`, `author`, `committer` or `pusher`. If a project was found, run `bumpChange`.
     - `installation` `deleted`/`suspend`: set `github_installations.revoked_at = now()`, set `unlinked_at = now()` on every live `project_repos` row of its repos, and append a `repo_events` row for each (`kind = "uninstalled"`). `unsuspend` clears `revoked_at` but does **not** relink; the user relinks.
     - `installation_repositories` `removed`: unlink each repo, with a `repo_events` row (`kind = "removed"`).
     - `repository` `renamed`/`transferred`: `update github_repos set full_name = repository.full_name where id = repository.id`, plus a `repo_events` row (`kind`, `detail = "<old> -> <new>"`). If the repo is unknown, do nothing.
     - `github_app_authorization` `revoked`: `delete from github_user_tokens where github_user_id = sender.id`.
     - Anything else: `200 {"ignored":true}` (the delivery row still records it).
  5. Return `200 {"ok":true}`. Keep the handler fast, with no outbound calls: GitHub times out deliveries after 10 s.
- Earliest receipt per commit (used by Task 9): `witnessedCommits(sql, projectId, limit = 50)` runs `select distinct on (sha) sha, received_at, claimed_ts, ref, forced from push_receipts where project_id = $1 order by sha, received_at`, then sorts by `received_at desc` and applies `limit`.

- [ ] **Step 1: Write the failing tests** in `server/test/webhook.test.ts`, using the Task 5 fixtures (`deliver(name, overrides?)` posts a fixture's raw body and headers to a started server):

```ts
test("bad signature is 401 and writes nothing", ...);           // flip one byte of the body
test("missing signature is 401", ...);
test("push to a linked repo stores receipts with server time, no message or author", async () => {
  const before = Date.now();
  await deliver("push.json");
  const rows = await sql`select * from push_receipts where delivery_id = ${fixtureDeliveryId}`;
  expect(rows.length).toBeGreaterThan(0);
  expect(rows[0].received_at.getTime()).toBeGreaterThanOrEqual(before - 1000);
  expect(JSON.stringify(rows)).not.toContain("message");
});
test("redelivery with the same delivery id is a duplicate and keeps the first time", ...);
test("same push under a new delivery id: page shows the earliest receipt", ...); // witnessedCommits
test("push for an unlinked repo is stored with null project and is not on the page", ...);
test("installation deleted unlinks repos, keeps receipts, records repo_events", ...);  // Review Focus 1
test("repository renamed updates full_name by id and keeps receipts", ...);           // Review Focus 2
test("transferred keeps the link", ...);
test("branch deletion push stores no receipt", ...);
test("receipts cannot be updated or deleted", ...);              // triggers raise 'append-only'
test("a crash after the delivery row rolls back both", ...);     // make the dispatch throw; delivery row absent
```

- [ ] **Step 2:** Run `bun test webhook`. Expected: FAIL.
- [ ] **Step 3:** Implement `verifySignature`, the route and the dispatch.
- [ ] **Step 4:** Run `bun test`. Expected: PASS. Then push to the dev App's scratch repo, and confirm a receipt row has a `received_at` within a few seconds of the push.
- [ ] **Step 5:** Commit: `feat(server): verified github webhooks and server-witnessed push receipts`.

---

### Task 7: Spike, then the Unified Projects DB link

The bounty counts "weighted projects in the Hack Club Unified Projects DB" (PITCH.md). How that database is reached, and what "weighted" means, is **not publicly documented**. What was found on 2026-11-22:

| Known | Source |
|---|---|
| `https://ships.hackclub.com/api/v1/ysws_entries` is a public JSON list of YSWS project entries, cached for 5 min. Fields include `id` (an Airtable record id), `ysws` (program), `approved_at` (Unix time, nullable), `code_url`, `demo_url`, `github_username` and `hours`. It needs no auth. | https://ships.hackclub.com/docs, https://github.com/hackclub/ships |
| `GET /api/v1/me` on Ships requires a Ships session cookie (web login), so a third-party server cannot call it for a user | https://ships.hackclub.com/docs |

| Unknown (spike steps; do not guess) |
|---|
| Whether Ships `ysws_entries` **is** the Unified Projects DB or a view of part of it (approved only? all programs?) |
| How a program **writes** to the Unified DB (Airtable base access is granted by HQ; no public API was found) |
| What "weighted" means and who computes it |
| Whether a Unified DB record can carry a Tally project URL or hours, and in which field |
| The size of the `ysws_entries` response (it decides whether fetching it on demand is acceptable) |

- [ ] **Step 1:** Run `curl -s https://ships.hackclub.com/api/v1/ysws_entries -o /tmp/ysws.json` and record the size, the entry count, and one entry with personal fields replaced, in `server/test/fixtures/upd/ysws_entry.json`. Record whether `code_url` values are GitHub URLs, and in what forms (`https://github.com/o/r`, `.git` suffix, trailing slash, subpaths).
- [ ] **Step 2:** Write `docs/spikes/unified-projects-db.md`: both tables above, the Step 1 numbers, and a list of **questions for Hack Club HQ** (the four unknowns). Mark the maintainer as the person who asks.
- [ ] **Step 3:** Commit: `docs(server): unified projects db spike`.

**Part B: read-only link (what can be built without HQ).** Matching uses the linked GitHub repo, which is the only identifier both sides share.

**Files:**
- Create: `server/src/upd.ts`, `server/test/upd.test.ts`
- Modify: `server/src/app.ts`

**Interfaces:**
- `normalizeRepoUrl(url: string): string | null`: for `github.com` URLs only, returns lowercase `owner/repo`, with `.git`, a trailing `/`, and `/tree/...` or `/blob/...` removed. Anything else gives `null`.
- `findUpdEntries(fullName: string, fetchFn = fetch): Promise<{ id: string; ysws: string; approved_at: number | null }[]>` fetches `ysws_entries` and returns entries whose `normalizeRepoUrl(code_url)` equals `fullName.toLowerCase()`. It keeps a module-level cache for 10 minutes (Ships caches for 5 minutes anyway). If Step 1 found the response over 20 MB, stop here: write a nightly script that stores the needed columns in an `upd_entries` table instead. Do not build it unless that is the case.
- `POST /p/:owner/:name/upd-link` (owner, CSRF) with form `record_id`. It must be one of `findUpdEntries(current github full_name)`. It sets `projects.upd_record_id` and runs `bumpChange`. With no linked repo, the response is `400` "link a GitHub repo first". `POST /p/:owner/:name/upd-unlink` clears it.
- The project settings page lists the matches as "Found in Unified Projects DB: <ysws>, approved <date>", each with a Link button.
- The public project page shows "Unified Projects DB: <ysws> (record <id>)" when linked. It shows only the program name and record id, never other fields.

- [ ] **Step 4: Write the failing tests** in `server/test/upd.test.ts`:

```ts
test("normalizeRepoUrl", () => {
  expect(normalizeRepoUrl("https://github.com/Owner/Repo.git")).toBe("owner/repo");
  expect(normalizeRepoUrl("https://github.com/owner/repo/tree/main/x")).toBe("owner/repo");
  expect(normalizeRepoUrl("https://gitlab.com/owner/repo")).toBeNull();
  expect(normalizeRepoUrl("not a url")).toBeNull();
});
test("findUpdEntries matches on the repo only", ...);        // fixture list with 1 match, 1 near miss (owner/repo2)
test("a record id not matching the linked repo is 400", ...);
test("upd link needs a linked repo", ...);
```

- [ ] **Step 5:** Run `bun test upd`. Expected: FAIL.
- [ ] **Step 6:** Implement `upd.ts` and the routes.
- [ ] **Step 7:** Run `bun test`. Expected: PASS.
- [ ] **Step 8:** Commit: `feat(server): read-only unified projects db link`.

---

### Task 8: HTML foundation: escaping, layout, security headers

**Files:**
- Create: `server/src/html.ts`, `server/static/style.css`, `server/test/html.test.ts`
- Modify: `server/src/app.ts` (`/static/*`, served from the `server/static/` directory only)

**Interfaces:**
- `esc(v: string | number): string` replaces `& < > " '` with `&amp; &lt; &gt; &quot; &#39;`.
- `` html`...` `` is a tagged template that escapes every interpolation unless it is a `Safe` value (`{ __html: string }`, produced only by `html` itself). Pages are built only with this tag, so forgetting `esc()` is impossible.
- `page(title: string, body: Safe, opts?: { status?: number; live?: string }): Response`. It returns a full HTML document with `<link rel="stylesheet" href="/static/style.css">`, a header (sign in, or the user's handle and a logout form with csrf), and `<script src="/static/live.js" defer>` only when `opts.live` is set. It sets the CSP from Global Constraints, `x-content-type-options: nosniff`, `referrer-policy: same-origin`, and `cache-control: no-store` for signed-in pages or `public, max-age=60` for public ones.
- `formatHours(s: number): string` is plan 06's badge rule: `floor(s / 360) / 10` hours with one decimal, `"12.3 h"` (never above verified time). Import or export it from plan 06's badge code; do not reimplement it. `formatMinutes(s: number): string` gives `"N min"`, floored.
- `notFound(): Response` is one 404 page for every missing or private thing (Review Focus 7).
- Static files: only `style.css`, `live.js` and `review.js`, by exact name, with `cache-control: public, max-age=3600`. `review.js` is supplied by plan 10 (this plan only allowlists it; a missing file is `404`). The CSP forbids inline script, so plan 10's reviewer UI loads `/static/review.js` with `<script src defer>`. Any other path under `/static/` is `404`, so there is no directory traversal.
- Amend plan 06's "every response body is JSON" wording in `server/README.md` (if present) or `docs/spikes/github-app.md` (if not) to "every `/v1` response". This is a one-line note, not a rewrite.

- [ ] **Step 1: Write the failing tests** in `server/test/html.test.ts`:

```ts
test("html tag escapes interpolations", () => {
  expect(html`<p>${"<script>x</script>"}</p>`.__html).toBe("<p>&lt;script&gt;x&lt;/script&gt;</p>");
  expect(html`<a title="${`" onmouseover="x`}">`.__html).not.toContain('" onmouseover');
});
test("nested html is not double-escaped", () => {
  expect(html`<div>${html`<b>${"&"}</b>`}</div>`.__html).toBe("<div><b>&amp;</b></div>");
});
test("page sets CSP and nosniff", async () => { /* headers contain "default-src 'none'" and nosniff */ });
test("/static/../package.json is 404", ...);
```

- [ ] **Step 2:** Run `bun test html`. Expected: FAIL.
- [ ] **Step 3:** Implement it. Write `style.css` with system fonts, one column at most 720 px wide, and readable tables, in under 80 lines.
- [ ] **Step 4:** Run `bun test`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(server): html escaping, layout and security headers`.

---

### Task 9: Public profile, project pages and settings

**How receipts relate to the ledger** (shown on the project page and explained there in one sentence). Ledger entries carry no commit SHA: the `commit` kind's features hold only `lines_changed`, and a SHA cannot be an integer feature. So receipts and ledger entries are joined by **time**, never by identity. For each witnessed commit, ordered by first `received_at`, the page shows the verified seconds of ledger entries whose `ts` falls after the previous witnessed push and at or before this one: "verified time since the previous witnessed push". That is the PITCH.md claim exactly: the receipt anchors the clock (this much verified time existed before the server saw this push), and it says nothing about effort. Only the push times are the server's: entry `ts` values are claimed by the device, so the page labels the column "ledger-claimed verified time" and says so in its sentence. Plan 06's C7 hash commits (`commits` table) are the other anchor. The page shows "ledger head last committed <time>" so a reader sees how fresh the chain is. Plan 10 may later add SHA-level linking with a ruleset change; that is not done here.

**Files:**
- Create: `server/src/pages.ts`, `server/test/pages.test.ts`
- Modify: `server/src/app.ts`, `server/src/projects.ts` (`commitWindows`)

**Interfaces:**
- `commitWindows(sql, projectId, limit = 50): Promise<{ sha: string; full_name: string; received_at: Date; claimed_ts: number | null; verified_s: number; flag: "claimed_after_receipt" | "pushed_late" | null }[]>`. It is one SQL query: the earliest receipt per SHA (Task 6's `witnessedCommits`), then `lag(received_at)` ordered by `received_at`, then a left join summing `ledger_entries.verified_s` where `ts > lag_ms and ts <= received_ms`. The first window is open at the start. Rows come back newest first. Flags: `claimed_ts > received_at + 5 min` is `claimed_after_receipt` (the commit claims to be from the future), and `received_at - claimed_ts > 7 days` is `pushed_late`. The flags are informational and never change hours.
- `GET /` shows signed-out text with a "Sign in with Hack Club" link. Signed in, it lists the user's projects with a create form (`POST /projects`) and a "Set your handle first" form when `handle` is null.
- `GET /u/:handle` returns `notFound()` when the handle is unknown or `profile_public = false`. Otherwise it shows `display_name` (or the handle) and a table of the user's `public = true` projects: name, verified h, unverified min "(not counted)", and last witnessed push. Nothing else about the user appears.
- `GET /p/:owner/:name` is matched case-insensitively on `public_name`. It returns `notFound()` when the project is missing, has no owner, or has `public = false` (unless the viewer is the owner, who sees a "private" banner). The page shows:
  - A `<section id="numbers" data-project="<id>" data-seq="<change_seq>">` with "<formatHours(verified)> verified" and, on its own line, "<formatMinutes(unverified)> unverified (not counted)" only when unverified is above 0.
  - "Owner: <a href=/u/handle>" only when the owner's profile is public.
  - The GitHub repo (current `full_name`, plus "(was <old>)" from the latest rename event), and the UPD link (Task 7).
  - Daily verified time for the last 90 days as a table with one native `<progress max=<day max> value=<v>>` per row. There are no inline styles, because the CSP forbids them.
  - The witnessed pushes table from `commitWindows`. Short SHAs link to `https://github.com/<full_name>/commit/<sha>`. Columns are "received by server" (UTC), "claimed", "ledger-claimed verified time since the previous push", and a flag text. If there are none, the table reads "No pushes witnessed yet".
  - Revocation notes from `repo_events` ("GitHub App removed on <date>; pushes after this are not witnessed").
  - "Ledger head last committed <time>" from `commits`, or "never".
  - The badge Markdown, ready to copy: `![tracked with Tally](<base>/badge/<public_name>.svg)` (Task 10).
  - `page(..., { live: id })` so Task 11's script runs.
- `GET /settings` (session) has forms for `display_name` (up to 64 chars, plain text), `profile_public` (checkbox), and handle (if unset); the device and user tokens with revoke buttons; "Connect GitHub" and "Install the GitHub App" links; and "Delete my account" (see below). `POST /settings/profile` (CSRF) saves the first two.
- `POST /settings/delete` (CSRF, plus a typed confirmation equal to the handle) deletes sessions, tokens and `github_user_tokens`, sets every project `public = false`, and blanks `email`, `name`, `display_name` and `verification_status`. Ledger entries and receipts are append-only evidence and stay, but nothing public shows them afterwards. Full erasure of ledger rows would need a decision on append-only data (listed under "Decisions").
- Every number on these pages comes from `projectHours`, `projectDaily` or `commitWindows`. No route sums seconds itself.

- [ ] **Step 1: Write the failing tests** in `server/test/pages.test.ts`:

```ts
test("zero-hour project page renders", async () => {        // Review Focus 6
  const { name } = await projectWithOwner();
  const r = await get(`/p/${name}`);
  expect(r.status).toBe(200);
  const t = await r.text();
  expect(t).toContain("0.0 h verified"); expect(t).toContain("No pushes witnessed yet");
  expect(t).not.toContain("unverified (not counted)");
});
test("unverified shown separately and never added", async () => {
  // ledger rows: verified 3600, unverified 1800
  const t = await (await get(`/p/${name}`)).text();
  expect(t).toContain("1.0 h verified"); expect(t).toContain("30 min unverified (not counted)");
  expect(t).not.toContain("1.5 h");
});
test("private profile is the same 404 as a missing one", async () => {   // Review Focus 7
  const a = await get(`/u/${privateHandle}`), b = await get(`/u/nobody-${h()}`);
  expect(a.status).toBe(404); expect(await a.text()).toBe(await b.text());
});
test("public project of a private profile stays up, without the owner link", ...);
test("private project is 404 to others and visible to the owner", ...);
test("project page never contains email, sub, slack id or verification status", ...); // seed distinctive values, assert absent
test("commit windows split verified time at each witnessed push", async () => {
  // entries at t=10s (60 s), t=100s (120 s); receipts received at t=50s (sha a), t=200s (sha b)
  // expect a: 60, b: 120
});
test("a replayed push does not move the commit's received time", ...);  // two deliveries, same sha: earliest wins
test("renamed repo shows the new name and 'was' the old", ...);
test("names with markup are escaped", ...);                             // display_name "<img src=x>"
test("settings without session redirects to sign-in", ...);
test("delete account hides everything public", ...);
```

- [ ] **Step 2:** Run `bun test pages`. Expected: FAIL.
- [ ] **Step 3:** Implement `commitWindows`, `pages.ts` and the routes.
- [ ] **Step 4:** Run `bun test`. Expected: PASS. Open the pages in a browser with JavaScript disabled, and confirm everything reads correctly.
- [ ] **Step 5:** Commit: `feat(server): public profiles, project pages and settings`.

---

### Task 10: Badge service

**Files:**
- Create: `server/src/badge.ts`, `server/test/badge-svg.test.ts`
- Modify: `server/src/app.ts`

**Interfaces:**
- Routes (public, no auth):
  - `GET /badge/:owner/:repo.svg` returns an SVG.
  - `GET /badge/:owner/:repo.json` returns C8 JSON (shields.io endpoint format), identical to `/v1/projects/:id/badge.json` for the same project.
  - `:owner/:repo` is the Tally `public_name`, matched case-insensitively. It resolves by public name, not by GitHub name, so a README badge survives a GitHub rename (Review Focus 2).
- Results:
  - Known public project: `200`, using plan 06's `badge(verifiedSeconds)` for the message. The color is always C8's `blue`; do not invent a zero color.
  - Private project: `200`, message `private`, color `lightgrey`.
  - Unknown name: `404` with a badge whose message is `unknown`, color `lightgrey`. It is still an SVG, so READMEs show something.
  - Any database error: `503` with message `unavailable`, plus `cache-control: no-store`.
- `renderSvg(label: string, message: string, color: "blue" | "lightgrey"): string` is a flat badge in shields.io's style: two `<rect>`s and two `<text>`s. Width is estimated as `6.5 px` per character plus `10 px` padding on each side. Both strings go through `esc()`. There is no external font or image. `content-type: image/svg+xml; charset=utf-8`.
- Caching: `cache-control: public, max-age=300, stale-while-revalidate=3600`, and `etag: "<project id>-<change_seq>"`. A request with a matching `if-none-match` gets `304` with no body. GitHub's camo image proxy honors `cache-control`, so the README badge refreshes within about 5 minutes and costs one cheap query per cache miss.
- One query per request: `select id, public, change_seq from projects where lower(public_name) = lower($1)`, then plan 06's `verifiedSeconds`. On a `304` the second query is skipped.

- [ ] **Step 1: Write the failing tests:**

```ts
test("svg and json agree with C8", async () => {
  // project with 3 h verified, 2 h unverified
  const j = await (await get(`/badge/${name}.json`)).json();
  expect(j).toEqual({ schemaVersion: 1, label: "tracked with Tally", message: "3.0 h", color: "blue" });
  const s = await (await get(`/badge/${name}.svg`)).text();
  expect(s).toContain("3.0 h"); expect(s).not.toContain("5.0 h");
});
test("zero hours is 0.0 h, 200, blue", ...);                 // Review Focus 6
test("unknown is a 404 svg with 'unknown'", ...);
test("private is 'private' and lightgrey", ...);
test("etag round trip gives 304 and changes after an upload", ...);
test("case-insensitive name", ...);
test("cache-control is public max-age=300", ...);
test("old github name after rename still serves the Tally name's badge", ...);  // name unchanged by rename
```

- [ ] **Step 2:** Run `bun test badge-svg`. Expected: FAIL.
- [ ] **Step 3:** Implement `badge.ts` and the routes.
- [ ] **Step 4:** Run `bun test`. Expected: PASS. Embed `![t](http://localhost:8080/badge/<name>.svg)` in a local Markdown preview to check it renders.
- [ ] **Step 5:** Commit: `feat(server): svg and json badge service with caching`.

---

### Task 11: Real-time project page (server-sent events)

**Choice: Postgres `LISTEN/NOTIFY`, not an in-process bus.** At the cost target the service is a single process most of the time. During a rolling deploy, though, two processes run at once: an upload handled by the new one must still reach SSE clients held by the old one, and an in-process bus would miss it. `NOTIFY` costs nothing extra (the same Postgres, one listening connection per process) and payloads are only a project id. There is no Redis.

**Files:**
- Create: `server/src/live.ts`, `server/static/live.js`, `server/test/live.test.ts`
- Modify: `server/src/app.ts`, `server/src/main.ts` (start the listener, handle `SIGTERM`), `server/src/pages.ts` (a fragment route)

**Interfaces:**
- **Spike step first (Step 1):** check whether the pinned Bun's `Bun.sql` supports `LISTEN`. Search the Bun docs for "listen", and try `await sql.listen?.("tally_project", cb)` in a scratch script.
  - If it does, use it.
  - If it does not, use a 2 s poll instead, with no new dependency: `select id, change_seq from projects where id = any($1)` over the project ids that currently have streams. That is one cheap query every 2 s per process, whatever the number of viewers. Record which one was used in `live.ts`'s header comment, and keep `pg_notify` in `bumpChange` either way, because it is harmless.
- `live.ts`:
  - `subscribe(projectId, send: (seq: number) => void): () => void` keeps an in-process `Map<projectId, Set<send>>` for the streams this process holds.
  - `startListener(sql)` runs `LISTEN tally_project`. On a notify, it reads that project's `change_seq` and calls each subscriber. If the connection drops, it reconnects with 1 s, 2 s, 5 s, then 10 s backoff, and after reconnecting it sends every subscribed project its current seq, because notifies may have been missed.
  - `GET /p/:owner/:name/events` (public; the same visibility rule as the page, else `404`) returns `text/event-stream` with `cache-control: no-store` and `x-accel-buffering: no`.
    - First write: `retry: 3000\n\n`.
    - If the request's `Last-Event-ID` (or `?seq=` from the page) differs from the current `change_seq`, it then sends `id: <seq>\nevent: changed\ndata: <seq>\n\n` straight away.
    - It sends `: ping\n\n` every 25 s.
    - Use a `ReadableStream`, and call `server.timeout(req, 0)` so Bun's idle timeout (10 s by default) does not cut the stream. Check that the pinned Bun has `server.timeout`; if not, lower the ping interval below `idleTimeout`.
    - On client abort (`req.signal`), unsubscribe and clear the timer.
  - Limits: at most 4 open streams per IP and 2000 per process. Over the limit is `429` with `retry-after: 30`.
  - `closeAll()` on `SIGTERM` ends every stream and then lets plan 06's shutdown continue. The browser reconnects to the new process after `retry` (Review Focus 5).
- `GET /p/:owner/:name/fragment` returns only the `<main>` inner HTML of the project page, built by the same renderer as Task 9 (`cache-control: no-store`).
- `static/live.js` (under 40 lines): `const el = document.querySelector("[data-project]")`. Open `new EventSource(location.pathname + "/events?seq=" + el.dataset.seq)`. On `changed`, wait 500 ms to debounce, `fetch(location.pathname + "/fragment")`, replace `main`'s contents with the response (same-origin HTML that the server already escaped), and update `data-seq`. The browser reconnects by itself, and `?seq` stays the value from page load, but `Last-Event-ID` takes precedence on reconnects.

- [ ] **Step 2: Write the failing tests** in `server/test/live.test.ts`, which read the stream with `fetch` plus `body.getReader()` against a started server:

```ts
test("stream starts with retry and sends changed after an upload", async () => {
  const s = await openStream(`/p/${name}/events?seq=0`);
  expect(await s.next()).toContain("retry: 3000");
  await uploadFixture(projectId);                         // plan 06 route; bumpChange notifies
  expect(await s.nextWithin(3000)).toMatch(/id: 1\nevent: changed/);
});
test("a push receipt also triggers changed", ...);
test("reconnect with Last-Event-ID equal to current sends nothing; older sends changed at once", ...); // Review Focus 5
test("a second server process delivers the change to the first's stream", ...);  // start(sql) twice; upload via B, stream on A
test("closeAll ends open streams", ...);
test("fifth stream from one IP is 429", ...);
test("events for a private project are 404", ...);
test("fragment escapes like the page", ...);
```

- [ ] **Step 3:** Run `bun test live`. Expected: FAIL.
- [ ] **Step 4:** Implement `live.ts`, the routes, `live.js`, and the `SIGTERM` handling in `main.ts`.
- [ ] **Step 5:** Run `bun test`. Expected: PASS. Then by hand: open a project page, upload with `tally sync` (plan 07) or `curl`, and watch the numbers change without a reload. Restart the server and watch the page reconnect and stay current.
- [ ] **Step 6:** Commit: `feat(server): live project page over server-sent events`.

---

### Task 12: Rate limits and the privacy check

**Files:**
- Create: `server/test/privacy.test.ts`
- Modify: `server/src/app.ts` (limiter keys), `server/src/ratelimit.ts` (only if plan 06's limiter cannot take a key and limit per call)

**Interfaces:**
- Use plan 06's token bucket with these keys and limits, as constants in `ratelimit.ts`. Over a limit is `429` with `retry-after`. HTML routes get a small 429 page; `/v1` and JSON routes get `{"error":"rate_limited"}`.

| Key | Limit | Why |
|---|---|---|
| `auth:<ip>` (`/auth/login`, `/auth/callback`, `/github/connect`, `/github/callback`) | 20/min | sign-in spam |
| `device:<ip>` (`/v1/device/code`) | 10/min | code farming |
| `devpoll:<device_code_hash>` | covered by `slow_down` | RFC 8628 |
| `form:<user_id>` (all session POSTs) | 60/min | abuse from a signed-in account |
| `page:<ip>` (`/u/*`, `/p/*`, fragments) | 120/min | scraping |
| `badge:<ip>` | 600/min | camo and shields fetch from few IPs, so keep this high |
| webhooks | none | signed by GitHub; unsigned ones are rejected before any work |

- The client IP comes from the `x-forwarded-for` header only when `TRUST_PROXY=1` (then the last hop the proxy appended), otherwise from `server.requestIP(req)`. Add `TRUST_PROXY` to config with default `0`.
- `privacy.test.ts` is a whole-surface check. It seeds a user with distinctive private values (`email = "leak-check@example.invalid"`, `sub = "ident!LEAKCHECK"`, `slack_id`, `verification_status = "verified-leakcheck"`, a ledger entry with feature key `leakcheck_feature`, a token label `leakcheck-label`, a push payload with message `leakcheck-message` and author email `leakcheck-author@example.invalid`). It then fetches every public route (`/`, `/u/:h`, `/p/:n`, `/p/:n/fragment`, `/p/:n/events` first event, `/badge/:n.svg`, `/badge/:n.json`, `/v1/projects/:id/badge.json`) and asserts that none of the strings appear. It also asserts that `push_receipts` and `webhook_deliveries` contain neither `leakcheck-message` nor the author email.

- [ ] **Step 1:** Write `privacy.test.ts` and a rate-limit test for each key in the table (send limit + 1 requests and expect the last to be `429`).
- [ ] **Step 2:** Run `bun test privacy ratelimit`. Expected: FAIL on the limits. The privacy test should already pass; if it fails, fix the leak in the module that caused it.
- [ ] **Step 3:** Wire the limiter keys.
- [ ] **Step 4:** Run `bun test`. Expected: PASS.
- [ ] **Step 5:** Commit: `feat(server): rate limits and a public-surface privacy check`.

---

### Task 13: Cost envelope, deploy notes, CI

**Files:**
- Create or modify: `docs/cost.md` ("Server" section)
- Modify: `.github/workflows/server.yml` (only if new test env vars are needed), `server/README.md` (or plan 06's deploy doc) for the new env vars and the GitHub App settings

**Steps:**
- [ ] **Step 1:** Measure locally with `bun test`'s helpers, or a 30-line `scripts/load.ts` using `fetch` in a loop. Record each number with the machine it ran on:
  - Requests per second for `/badge/:n.svg` on cache miss, and on `304`.
  - Requests per second for `/p/:n`.
  - The process's resident memory with 1000 open SSE streams (from `process.memoryUsage().rss`).
  - Postgres queries per second at idle with 1000 streams. This should be about 0 with LISTEN, or 0.5 per process with the 2 s poll.
- [ ] **Step 2:** Write the envelope in `docs/cost.md`, against PITCH.md's "~$50–80 for API, Postgres, real-time dashboard" at 15,488 monthly users:
  - Load model: badge hits (camo caches for 300 s, so at most 12 cache misses per hour per badge, about 2.7M per month for 316 badges if 1 in 50 users shows one, which is negligible), page views, SSE streams (peak concurrent viewers, guessed at 1% of MAU, about 155), webhook deliveries (pushes per user per day, times users with the App), and ledger uploads (plan 06's figure).
  - Resources: 1 small VM or container (2 vCPU, 2 to 4 GB) plus managed Postgres (smallest tier with 10 to 20 GB) or Postgres on the same VM. List current prices from two providers, with links and the date checked. Do not quote from memory.
  - Storage growth: receipts are about 150 bytes per commit row. Estimate commits per month and the 12-month total.
  - Result: a table of items with monthly cost, a total, and a "fits ~$50–80: yes/no" line. If it does not fit, name the first thing to cut (for example, the SSE stream cap).
- [ ] **Step 3:** Document the production settings: every env var, the GitHub App permissions and events from the Task 5 spike, the Hack Club Auth app's redirect URI and requested scopes, `TRUST_PROXY`, and a note that the proxy must not buffer `text/event-stream` (for nginx, `proxy_buffering off` on `/p/*/events`).
- [ ] **Step 4:** Make sure CI runs every new test with `NODE_ENV=test` and the dummy env values that `loadConfig` accepts in tests. Run `bun run typecheck && bun run lint && bun test`. Expected: all clean.
- [ ] **Step 5:** Commit: `docs(server): cost envelope and deploy settings for accounts and profiles`.

---

## Done when

- A new user signs in with Hack Club Auth, sets a handle, creates `handle/name` (or `owner/repo` after connecting GitHub), installs the App, links the repo, and runs `tally login --browser` (plan 12) or plain `tally login` (plan 07, token flow), both using this plan's device-code flow. A push to the repo appears on the project page with the server's receive time and the verified seconds since the previous push, without a reload.
- The badge at `/badge/<name>.svg` shows the same verified hours as plan 06's `/v1/projects/:id/badge.json`, and unverified time appears only as a separate "not counted" line on the page.
- Every Review Focus item has a passing test, and the privacy test passes.
- `docs/spikes/hack-club-auth.md`, `docs/spikes/github-app.md` and `docs/spikes/unified-projects-db.md` record sources and unknowns, and `docs/cost.md` has the server envelope.

## Not in this plan

- Evidence bundles, the judge, reviewer views, submissions (plan 10).
- Writing to the Unified Projects DB, and "weighted" project counting. Both need answers from Hack Club HQ (Task 7).
- Repo-delta caps from GitHub data (PITCH.md trust anchors). This would need the compare API and installation tokens; it belongs with plan 10.
- SHA-level linking of ledger `commit` claims to receipts. This needs a ruleset change to carry a commit hash.
- Handle renames, co-owned projects, organizations, admin moderation UI, and email.
- `tally login --browser` in the daemon (plan 12 adds it against Task 4's routes).

## Decisions I made (maintainer to confirm)

1. **Tally runs its own device-code flow.** Hack Club Auth has no device grant (discovery lists `authorization_code`, `client_credentials`, `refresh_token`). The flow issues a **user-scoped** device token, and `requireToken` accepts it for any project the user owns. Plan 06's per-project tokens still work.
2. **Public names are `handle/name`, or `owner/repo` only with proven GitHub access.** They are unique ignoring case and can never change (they are hashed into ledger entries). Badges and pages resolve by Tally name, so GitHub renames do not break READMEs. The cost is that after a transfer, the Tally name keeps the old owner.
3. **Receipts link to the ledger by time, not by SHA.** "Ledger-claimed verified time since the previous witnessed push" makes no effort claim, and only the push times are the server's. Follow-up for the maintainer: a stronger anchor would also require `e.received_at <= push received_at` (the entry reached the server before the push did). Only the earliest receipt per SHA is shown, so replays cannot shift it.
4. **No GitHub installation tokens and no App private key.** A short-lived user-to-server token proves repo access, and webhooks carry everything else. The repo-delta cap would need installation tokens later.
5. **`LISTEN/NOTIFY` for SSE** (with a 2 s `change_seq` poll if `Bun.sql` lacks LISTEN). The page re-fetches one HTML fragment per change, so rendering lives in one place and the script stays under 40 lines.
6. **Account deletion hides rather than erases** ledger entries and receipts, because both are append-only evidence. Is that acceptable for a teen user base, or must deletion erase them?
7. Only `jose` is added as a dependency. Access and refresh tokens from Hack Club are not stored: identity is read once at sign-in.
