# Cost

## Server

Budget line: PITCH.md "Cost" gives **~$50 to $80 per month for API, Postgres and the real-time dashboard** at 15,488 monthly active users (MAU) logging 109,759 hours per month. This section covers accounts, projects, profiles, badges, receipts and live pages (plan 09) on top of plan 06's ledger service.

**No new infrastructure.** One Bun process and one Postgres. No Redis, no queue, no second service, no paid CDN, as PITCH.md and plan 09 require. Live pages use Postgres `LISTEN/NOTIFY`; the rate limiter is in-process; badge caching relies on `cache-control` and `etag` headers that the readers' proxies (camo, shields, browsers) already honor.

### Measured (local, dev laptop; not a production number)

Machine: AMD Ryzen 5 5500GT (6 cores / 12 threads), 32 GB RAM, Windows 11 Pro 10.0.26300, Bun 1.4.5, Postgres 16 in Docker on the same machine. Date: 2026-11-23. The load generator (`server/scripts/load.ts`, 32 concurrent `fetch` loops, 5 s each) and the server (`server/scripts/loadserver.ts`, rate limits set to 1e9) ran on the same machine and competed for CPU, so the server's own ceiling is higher. The first runs used a small project (6 ledger entries, 20 push receipts); the project page was then re-measured on a bigger one (5,006 entries, 500 receipts over 30 days; `LOAD_ENTRIES=5000 LOAD_RECEIPTS=500` in `server/scripts/loadserver.ts`).

| Measurement | Result |
|---|---|
| `/badge/:owner/:name.svg`, no `If-None-Match` (full 200, a cache miss for the proxy) | 3,884 req/s |
| same, with matching `If-None-Match` (304) | 6,652 req/s |
| `/p/:owner/:name` (full page), small project: 6 entries, 20 receipts | 1,670 req/s |
| `/p/:owner/:name/fragment`, small project | 1,813 req/s |
| `/p/:owner/:name`, 5,006 entries, 500 receipts, before the `commitWindows` fix (every receipt joined to every entry, no `(chain_id, ts)` index) | 17 req/s |
| same project, after the fix (newest 51 commits picked first, migration 105's index) | 485 req/s |
| `/p/:owner/:name/fragment`, same project: before / after | 16 / 407 req/s |
| Server RSS: after start / after the runs above / with 1,256 open SSE streams | 54 MB / 82 MB / 69 MB |
| Postgres transactions per second at idle with 1,256 streams | about 0 (2 transactions in 20 s, both from the measuring query itself) |

Notes: 1,256 streams is 1,000 from the final run (1,000 distinct `x-forwarded-for` values to get past the 4-per-ip cap, under `TRUST_PROXY=1`) plus 256 left from an aborted first attempt that Bun's fetch connection limit (256) stalled. RSS per stream is therefore about 12 KB at most. On the bigger project the page's slowest query is now `projectDaily` (about 8 ms), then `commitWindows` (about 4 ms, from 95 ms before the fix). Ledger upload throughput was not measured here (plan 06's tests cover correctness only). A real deployment adds TLS and network latency.

### Load model (assumptions marked A)

| Source | Model | Per month | Average rate |
|---|---|---|---|
| Ledger uploads (plan 06 figures, README "Monthly cost envelope check") | 109,759 h x 60 / 3 min per entry (A: 3 min mean); 4 h/day daemon uptime, 2 requests per 300 s (A) | 2.2 M entries, 1.49 M requests/day | about 17 req/s |
| Badge cache misses | 316 badges (A: about 1 in 49 users shows one) x 12 misses/hour (camo caches 300 s) x 730 h | 2.77 M | about 1.1 req/s |
| Page views | A: 4 views per user per month, x 10 headroom | about 620 k | about 0.2 req/s |
| Live viewers | A: 1% of MAU = 155 concurrent streams (cap 2,000 per process) | n/a | 155 streams = about 2 MB RSS |
| Webhook deliveries | A: 50% of users install the App (7,744), 5 pushes per day each | 1.16 M | about 0.45 req/s |
| Receipt rows | A: 3 commits per push | 3.48 M rows | n/a |

Total is about 19 req/s on average. Measured read-path ceilings on the dev laptop are 485 req/s (project page, 5,006-entry project) to 6.7 k req/s (badge 304), so one small instance has at least an order of magnitude of headroom. Peaks (a hackathon deadline) are assumed 10x the average: still under 200 req/s.

### Storage growth

| Table | Per month | 12 months |
|---|---|---|
| `ledger_entries` (plan 06: about 1 KB per entry with indexes) | 2.2 GB | 26.4 GB |
| `push_receipts`: 150 bytes per commit row (estimate) x 3.48 M | 0.52 GB | 6.3 GB |
| `webhook_deliveries`: A: about 100 bytes x 1.16 M | 0.12 GB | 1.4 GB |
| Total | about 2.8 GB | about 34 GB (use 35 GB) |

Measure the real bytes per row in the pilot (`pg_total_relation_size(...) / count(*)`).

### Prices (checked 2026-11-23; do not quote from memory)

Hetzner Cloud, https://www.hetzner.com/cloud/regular-performance/ (euros, excl. VAT; the page marked every plan "currently unavailable" on the check date, so availability is a risk):

- CPX22: 2 vCPU, 4 GB RAM, 80 GB NVMe, EUR 16.99/month (Helsinki), EUR 19.99/month (Nuremberg). CPX12: 1 vCPU, 2 GB, 40 GB, EUR 9.99 (HEL) / EUR 11.99 (FSN, NBG). Backup pricing: could not verify.
- Postgres runs on the same VM; no managed Postgres price was checked.

Fly.io, https://docs.fly.io/about/pricing (USD):

- Machine shared-cpu-2x, 2 GB: $13.39/month. Volumes $0.15/GB/month.
- Managed Postgres: "Basic" $38.00/month (CPU and RAM of that plan: could not verify); storage on non-Developer plans $0.28/GB/month by usage. The Developer plan ($4/month) is too small to assume.

### Envelope at month 12

| Item | Option A: Hetzner, Postgres on the VM | Option B: Fly.io, managed Postgres |
|---|---|---|
| App + database host | CPX22 (2 vCPU, 4 GB, 80 GB): EUR 19.99 | shared-cpu-2x 2 GB machine: $13.39 |
| Postgres | on the same VM (35 GB of the 80 GB disk) | Basic plan: $38.00 |
| Storage | included | 35 GB x $0.28 = $9.80 |
| Backups | could not verify; add `pg_dump` to object storage (PITCH.md budgets <$5 for storage) | not verified |
| **Total** | **EUR 19.99** (below $50 at any plausible exchange rate; no conversion applied) | **$61.19** |
| Fits ~$50 to $80 | **yes, if Hetzner has stock** (every plan was unavailable on the check date); well under the range | **yes, if the Basic plan is big enough** (its CPU and RAM could not be verified) |

At month 1 the storage line is $0.28 x 3 GB, so option B starts near $52 and grows by about $0.8 per month.

### Verdict

Fits ~$50 to $80: **yes, conditionally** (Fly.io with managed Postgres, $61.19, if the Basic plan's unverified CPU and RAM suffice; Hetzner is cheaper but self-managed and was out of stock on the check date). Memory is not the constraint (about 70 MB with 1,256 streams); the first cost is the database (ledger storage, $38 plan plus $9.80). If the budget is exceeded, the first thing to cut is **ledger retention growth** (archive entries older than N months to object storage); that trims only the storage line ($9.80 at month 12 on option B), not the $38 plan. Then the SSE stream cap (`PER_PROCESS` in `server/src/live.ts`), which saves almost nothing. Unverified: the Basic plan's CPU and RAM, Hetzner availability and backup cost, and ledger upload throughput under load.
