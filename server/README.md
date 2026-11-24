# tally server

The tally ledger service: a Bun HTTP server over Postgres that accepts hash-chained ledger entries from devices, verifies each chain with the same WASM build of `crates/tally-scoring` the daemon uses, stores entries append-only (database triggers reject `UPDATE`/`DELETE`), and serves a public badge of verified hours. API contract: C7 in [`docs/superpowers/plans/README.md`](../docs/superpowers/plans/README.md).

## Run locally

Needs [Bun](https://bun.sh) 1.4.5 and Postgres 16.

```sh
docker run --rm -e POSTGRES_PASSWORD=postgres -p 5432:5432 postgres:16
export DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres
# Required (see Production settings); these placeholders are enough to start, but sign-in and GitHub need real values.
export PUBLIC_BASE_URL=http://localhost:8080
export HCA_CLIENT_ID=dev HCA_CLIENT_SECRET=dev
export GITHUB_APP_SLUG=dev GITHUB_APP_CLIENT_ID=dev GITHUB_APP_CLIENT_SECRET=dev GITHUB_WEBHOOK_SECRET=dev
cd server
bun install
bun run wasm     # needs Rust stable, the wasm32-unknown-unknown target and wasm-pack
bun run start    # migrations run at start; PORT defaults to 8080, LOG_LEVEL to info
curl localhost:8080/healthz   # {"ok":true}
```

The `docker run` line stays in the foreground: run the rest in a second terminal. `server/wasm/` is build output and is not committed. If port 8080 is taken, set `PORT` (and match it in `PUBLIC_BASE_URL`).

## Create a device token

```sh
bun run token:create sectersion/tally laptop
```

Prints three lines:

- `project_id`: the server's id for the project. It goes in the URL (`/v1/ledger/{project_id}/...`), not the public name, which contains `/`.
- `token_id`: the number to pass to `token:revoke`.
- `token`: the bearer secret (`tly_...`). It is shown once; only its SHA-256 is stored.

Revoke with `bun run token:revoke <token_id>` (prints `revoked`, or `no such active token` and exits 1).

## Admin scripts

- `bun run token:claim <project_id> <handle>` gives a project (for example one made by `token:create`, which has no owner) to the user with that handle. If the project already has an owner it refuses and exits 1; add `--force` to replace the owner. Project-bound device tokens from `token:create` keep working after a claim (the new owner can revoke them in /settings).
- `bun run user:role <handle> user|reviewer|admin` sets a user's role (prints `<handle> is now <role>`, or `no such handle` and exits 1).

## Migrations

`server/migrations/` runs in name order. Number `103` has two files (`103_device.sql`, `103_one_repo_per_project.sql`; both apply, never rename an applied migration), `104` is reserved for plan 10 (`104_submissions.sql`), `105_ledger_entries_chain_ts.sql` already exists, and `106` has three files (`106_chains_unverified_s.sql`, `106_repo_gh_updated_at.sql`, `106_unique_action_id.sql`; independent, all apply). New migrations start at `107`.

## Test

```sh
bun run db:reset && bun test
bun run typecheck
bun run lint
```

`db:reset` drops and recreates the `public` schema and needs `psql` on PATH. Without it, run the same SQL through your container, for example `docker exec <container> psql -U postgres -c "drop schema public cascade; create schema public"`. `db:reset` drops the whole schema: never point it at production. Migrations are re-applied by `bun run migrate`, the next server start, or the next `bun test`.

Regenerate fixtures with `bun run fixture` whenever `crates/tally-scoring` hashing changes (CI fails on a stale fixture).

## Deploy

One container or VM running `bun run start` behind a TLS-terminating proxy (C7 requires HTTPS), a managed Postgres, and `DATABASE_URL` from the platform's secret store. Migrations run at start. A Dockerfile is not part of this plan (add with the deploy target, plan 09).

## Production settings

Environment (read once at start by `loadConfig` in `src/config.ts`; a missing required value stops the server):

| Variable                                           | Required           | Meaning                                                                                                                                                                                  |
| -------------------------------------------------- | ------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `DATABASE_URL`                                     | yes                | Postgres 16 connection string                                                                                                                                                            |
| `PUBLIC_BASE_URL`                                  | yes                | Public origin, no trailing slash, for example `https://tally.example`. Used for cookies (`Secure` unless it starts with `http://localhost`), the CSRF `Origin` check and OAuth redirects |
| `HCA_CLIENT_ID`, `HCA_CLIENT_SECRET`               | yes                | Hack Club Auth app credentials                                                                                                                                                           |
| `GITHUB_APP_SLUG`                                  | yes                | The App's URL slug (install link)                                                                                                                                                        |
| `GITHUB_APP_CLIENT_ID`, `GITHUB_APP_CLIENT_SECRET` | yes                | The App's user-to-server OAuth credentials                                                                                                                                               |
| `GITHUB_WEBHOOK_SECRET`                            | yes                | Webhook secret, checked on every `POST /github/webhook`                                                                                                                                  |
| `PORT`                                             | no, default `8080` | Listen port                                                                                                                                                                              |
| `LOG_LEVEL`                                        | no, default `info` | `debug`, `info`, `warn` or `error`                                                                                                                                                       |
| `TRUST_PROXY`                                      | no, default `0`    | `1` when exactly one proxy appends the client address to `x-forwarded-for` (see Limits)                                                                                                  |

**Hack Club Auth app** (https://auth.hackclub.com/developer/apps): redirect URI `PUBLIC_BASE_URL + /auth/callback`; scopes `openid profile email verification_status`. Identity is read once at sign-in and no tokens are stored.

**GitHub App** (settings from `docs/spikes/github-app.md`; webhook URL `PUBLIC_BASE_URL + /github/webhook`):

- Setup URL `PUBLIC_BASE_URL/github/setup`, Callback URL `PUBLIC_BASE_URL/github/callback`.
- Leave "Request user authorization (OAuth) during installation" **unchecked**; leave "Expire user authorization tokens" on.
- Permissions: Contents read-only, Metadata read-only. Events: Push, Repository (installation events are always delivered).
- The server holds no App private key and makes no installation-token calls.

**Reverse proxy:**

- Terminate TLS and forward to `PORT`.
- `text/event-stream` must not be buffered. The server sends `x-accel-buffering: no`; for nginx also set `proxy_buffering off;` (and a `proxy_read_timeout` above the 25 s ping, for example `3600s`) in the `location ~ ^/p/[^/]+/[^/]+/events$` block.
- With `TRUST_PROXY=1` the proxy must append the client address to `x-forwarded-for` (nginx: `proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;`). Without it, every client shares the proxy's rate-limit buckets; with it but no proxy, clients choose their own ip.

**Load check:** `bun scripts/loadserver.ts` (scratch server with unlimited rate limits; needs `DATABASE_URL`, never deploy it) and `bun scripts/load.ts <url>` measure requests per second. Numbers are in [`docs/cost.md`](../docs/cost.md).

## Limits

- Every `/v1` response is JSON; pages outside `/v1` are server-rendered HTML.
- Body 1 MiB, 1000 entries per batch. A larger body is a JSON `413 body_too_large` (checked by `Content-Length`, and while streaming for chunked bodies); Bun itself only cuts off at 256 MiB with an empty 413.
- 120 requests/min per token (burst 30), single instance (in-process limiter).
- Per minute: sign-in and GitHub connect 20 per ip, `/v1/device/code` 10 per ip, session form posts 60 per user, `/u` and `/p` pages 120 per ip, badges 600 per ip (table: `LIMITS` in `src/ratelimit.ts`). Webhooks are not limited. Over a limit is `429` with `retry-after: 60`.
- Failed auth is limited per client ip. Behind a proxy, set `TRUST_PROXY=1` so the client ip is the last `x-forwarded-for` hop (the one the proxy appended); otherwise every client shares the proxy's address. Never set it without a proxy that appends that header: clients could then pick their own ip.
- `TRUST_PROXY=1` assumes exactly one proxy in front, and that proxy appends the client address. With a chain of proxies the last hop is the inner proxy, and every client shares its bucket.
- Limits are per ip, so users behind one shared NAT (a school, an event) share the sign-in and page buckets. IPv6 addresses are keyed by their /64 and IPv4-mapped IPv6 by the IPv4 address.

## Monthly cost envelope check

Against `PITCH.md` "Cost": API + Postgres + dashboard is about $50 to $80 of the ~$100 to $185 total, budget $500. Replace each number below with a pilot measurement.

- **Write volume:** 109,759 h/month x 60 / 3 credited min per action = 2,195,180, about 2.2 M entries/month. The 3 minutes is an assumed mean; measure it in the pilot.
- **Storage:** an entry row (verbatim `json` about 450 to 600 bytes, indexed columns, primary key and hash index) is roughly 1 KB, so about 2.2 GB/month of growth, 26 GB/year. Measure with `select pg_total_relation_size('ledger_entries') / count(*) from ledger_entries`.
- **Requests:** 15,488 MAU, each daemon about 2 requests per 300 s while running. At a generous 4 h/day of daemon uptime that is 4 x 3600 / 300 x 2 = 96 requests per user per day, so 15,488 x 96 = 1,486,848 (about 1.5 M) requests/day, / 86,400 s = about 17/s average. One small instance handles this; verification cost is a SHA-256 per entry.
- **Verdict:** fits the $50 to $80 line on one small instance plus a managed Postgres with about 30 GB, if the measured entry size and action rate are within 2x of these assumptions. Re-check at the pilot.
