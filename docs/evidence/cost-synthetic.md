# Cost sheet (generated)

## Measured

From `docs/evidence/replay/steady-work.ledger.json`: 30 entries, 0.52 verified hours.

- Bytes per entry (mean): **337**
- Bytes per verified hour: **19590**
- Total: 10122 bytes

The PITCH sentence "tens of bytes per action record" is wrong by this measurement (mean above 100 bytes): replace it with the measured figure.

## Projected at PITCH scale (15488 users, 109759 hours per month)

- Entries per month: 6373103
- Upload per month: 2150.2 MB
- Stored after 12 months: 25802.1 MB

## Line items

| item | low USD/month | high USD/month | source |
|---|---|---|---|
| App host: Fly.io shared-cpu-2x 2 GB machine | 13.39 | 13.39 | docs/cost.md, citing https://docs.fly.io/about/pricing (checked 2026-11-23) |
| Postgres: Fly.io managed, Basic plan (CPU and RAM of the plan not verified) | 38.00 | 38.00 | docs/cost.md, citing https://docs.fly.io/about/pricing (checked 2026-11-23) |
| Ledger storage at $0.28/GB (3 GB at month 1, 35 GB at month 12) | 0.84 | 9.80 | docs/cost.md (storage sizes there are estimates; replace with the measured bytes from this sheet) |
| Self-hosted 8B judge, spot GPU, 20 to 40 GPU-hours per month | 20.00 | 80.00 | **UNSOURCED** |
| Object storage for backups and published ledgers | 0.00 | 5.00 | **UNSOURCED** |
| Code signing: Windows certificate and Apple Developer Program (see docs/release.md) | 0.00 | 20.00 | **UNSOURCED** |
| **Total of the items above** | **72.23** | **166.19** | only the numbers given in `cost-inputs.toml` |

Some prices are UNSOURCED: they are placeholders the maintainer must replace or source before this sheet is quoted.
