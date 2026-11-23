# tally server

The tally ledger service: a Bun HTTP server over Postgres that accepts hash-chained ledger entries from devices, verifies each chain with the same WASM build of `crates/tally-scoring` the daemon uses, stores entries append-only (database triggers reject `UPDATE`/`DELETE`), and serves a public badge of verified hours. API contract: C7 in [`docs/superpowers/plans/README.md`](../docs/superpowers/plans/README.md).

## Run locally

Needs [Bun](https://bun.sh) 1.4.5 and Postgres 16.

```sh
docker run --rm -e POSTGRES_PASSWORD=postgres -p 5432:5432 postgres:16
export DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres
cd server
bun install
bun run wasm     # needs Rust stable, the wasm32-unknown-unknown target and wasm-pack
bun run start    # migrations run at start; PORT defaults to 8080, LOG_LEVEL to info
curl localhost:8080/healthz   # {"ok":true}
```

`server/wasm/` is build output and is not committed. If port 8080 is taken, set `PORT`.

## Create a device token

```sh
bun run token:create sectersion/tally laptop
```

Prints three lines:

- `project_id`: the server's id for the project. It goes in the URL (`/v1/ledger/{project_id}/...`), not the public name, which contains `/`.
- `token_id`: the number to pass to `token:revoke`.
- `token`: the bearer secret (`tly_...`). It is shown once; only its SHA-256 is stored.

Revoke with `bun run token:revoke <token_id>` (prints `revoked`, or `no such active token` and exits 1).

## Test

```sh
bun run db:reset && bun test
bun run typecheck
bun run lint
```

`db:reset` drops and recreates the `public` schema and needs `psql` on PATH. Without it, run the same SQL through your container, for example `docker exec <container> psql -U postgres -c "drop schema public cascade; create schema public"`. Migrations are re-applied by `bun run migrate`, the next server start, or the next `bun test`.

Regenerate fixtures with `bun run fixture` whenever `crates/tally-scoring` hashing changes (CI fails on a stale fixture).

## Deploy

One container or VM running `bun run start` behind a TLS-terminating proxy (C7 requires HTTPS), a managed Postgres, and `DATABASE_URL` from the platform's secret store. Migrations run at start. A Dockerfile is not part of this plan (add with the deploy target, plan 09).

## Limits

- Body 1 MiB, 1000 entries per batch.
- 120 requests/min per token (burst 30), single instance (in-process limiter).
- Behind a proxy, the per-ip rate-limit buckets share the proxy's address (single instance, in-process limiter).

## Monthly cost envelope check

Against `PITCH.md` "Cost": API + Postgres + dashboard is about $50 to $80 of the ~$100 to $185 total, budget $500. Replace each number below with a pilot measurement.

- **Write volume:** 109,759 h/month x 60 / 3 credited min per action = 2,195,180, about 2.2 M entries/month. The 3 minutes is an assumed mean; measure it in the pilot.
- **Storage:** an entry row (verbatim `json` about 450 to 600 bytes, indexed columns, primary key and hash index) is roughly 1 KB, so about 2.2 GB/month of growth, 26 GB/year. Measure with `select pg_total_relation_size('ledger_entries') / count(*) from ledger_entries`.
- **Requests:** 15,488 MAU, each daemon about 2 requests per 300 s while running. At a generous 4 h/day of daemon uptime that is 4 x 3600 / 300 x 2 = 96 requests per user per day, so 15,488 x 96 = 1,486,848 (about 1.5 M) requests/day, / 86,400 s = about 17/s average. One small instance handles this; verification cost is a SHA-256 per entry.
- **Verdict:** fits the $50 to $80 line on one small instance plus a managed Postgres with about 30 GB, if the measured entry size and action rate are within 2x of these assumptions. Re-check at the pilot.
