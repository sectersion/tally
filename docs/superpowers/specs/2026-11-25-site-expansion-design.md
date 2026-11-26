# Site expansion (Hackatime-modeled) design

Status: approved in conversation 2026-11-25. Size: a project — six pieces, one subagent each, built in order on the `site-updates` branch. Data-shape checks come first where a piece depends on ledger fields that may not exist.

## Intent and why

Tally's site is Hackatime's shape without Hackatime's depth: it has a landing page, project pages, badges and profiles, but no signed-in dashboard, no leaderboards, no public read API, no per-harness setup docs, no export. Hackatime is the guide for how the site expands: dashboard, profiles with stats, leaderboards, read API, editor docs, export. The maintainer approved all six pieces in order, subagent per piece.

## Goals

- A signed-in user can see their own totals and activity at a glance.
- Visitors can compare projects (leaderboard) and see per-user totals.
- Programmatic consumers can read public stats over HTTP, Hackatime-style.
- A new user can find setup instructions for their harness.
- Anyone can export a project's ledger and re-derive its hours.

## Non-goals

- API key management UI (device tokens cover this), Slack integration, OAuth apps.
- Editor-plugin style per-file/language breakdowns unless the ledger already carries the data.
- Anything that breaks the CSP (`default-src 'none'`, no inline styles/scripts, self-hosted assets only) or the static-allowlist serving model.
- The signed-in app restyle (later piece).

## Constraints

- Server-rendered HTML through `page()` in `server/src/html.ts`; styles in `server/static/style.css`; static files via the `STATIC` allowlist in `server/src/app.ts`.
- The ledger stores verified/unverified seconds per entry (`ledger_entries`, `chains`); daily rollups exist in `projectDaily` (server/src/projects.ts). Language/editor data is not in the schema — breakdowns are cut unless a check finds it in `entry` JSON.
- Rate limits: page buckets (`rates.page`) for HTML, per-IP buckets for read API, same pattern as `/v1/projects/:id/summary`.
- Tests run with `DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres bun test` (Postgres 16 local). wasm is already built in `server/wasm/`.
- Branch: `site-updates`. One piece per subagent; pieces land as separate commits so review stays small.

## Pieces (approved order)

1. **Signed-in dashboard `/dashboard`** — totals (today, 7d, all-time verified), current streak, per-project verified hours table, recent activity. The core gap; used to shape the home page link.
2. **Profile upgrade `/u/:handle`** — totals across public projects, streak, richer than today's project table. Cut language/editor breakdowns if the `entry` JSON check (below) finds nothing.
3. **Leaderboards** — `/leaderboard` daily (UTC day) and weekly (ISO week) verified minutes across public projects. Only verified seconds count — this is the honesty story.
4. **Public read API** — `/api/v1/users/:handle/stats`, `/api/v1/users/:handle/projects`, `/api/v1/leaderboard?range=day|week`, `/api/v1/users/:handle/export` (JSON ledger). Read-only, no auth, per-IP rate limited, `cache-control: public, max-age=60`.
5. **Docs expansion** — one section per harness (Claude Code, Codex, Cursor, Aider, Copilot, editor plugins via local `api_url`), from PITCH.md and the daemon's setup modules (`crates/tally-daemon/src/setup/*`).
6. **JSON export** — per-project ledger download (entries with ts/kind/verified_s/unverified_s/head hash chain), linked from project pages and settings.

## Data-shape check (before piece 2)

One agent inspects `entry` JSON in `ledger_entries` rows from fixtures (`server/test/fixtures`, `dashboard/fixtures/seed.ndjson`) and the daemon's upload code (`crates/tally-daemon/src/submit/upload.rs`) for language/editor/harness fields. Report: what fields exist per entry kind. Pieces 2–4 use the answer; missing fields cut the corresponding breakdown, no migration is invented for it.

## Left to the builder

Exact copy, section layout, chart form (reuse the SVG-bar trend pattern), endpoint response shapes (keep them small and stable), and whether the leaderboard splits verified-only vs claimed (default: verified only).
