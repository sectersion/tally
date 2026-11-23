# Spike: Unified Projects DB

Checked 2026-11-23. One unauthenticated `GET https://ships.hackclub.com/api/v1/ysws_entries` and the public docs; no personal or project details were sent. The response was saved outside the repo. `server/test/fixtures/upd/ysws_entry.json` is ONE real entry's shape with every personal field replaced by a placeholder (`id`, `code_url`, `demo_url`, `github_username`, `description`, `display_name`, `slack_id`, `screenshot_url`, `country`, `archived_*` are all invented).

## Known (found 2026-11-22)

| Known | Source |
|---|---|
| `https://ships.hackclub.com/api/v1/ysws_entries` is a public JSON list of YSWS project entries, cached for 5 min. Fields include `id` (an Airtable record id), `ysws` (program), `approved_at` (Unix time, nullable), `code_url`, `demo_url`, `github_username` and `hours`. It needs no auth. | https://ships.hackclub.com/docs, https://github.com/hackclub/ships |
| `GET /api/v1/me` on Ships requires a Ships session cookie (web login), so a third-party server cannot call it for a user | https://ships.hackclub.com/docs |

## Step 1 numbers (2026-11-23)

| Measure | Result |
|---|---|
| HTTP status | 200 |
| Size | 13,551,319 bytes (about 13.6 MB, under the 20 MB limit) |
| Shape | a top-level JSON array of 12,224 entries |
| Keys per entry | `id, ysws, approved_at, code_url, country, demo_url, description, github_username, hours, screenshot_url, github_stars, display_name, slack_id, archived_demo, archived_repo` |
| `approved_at` null | 0 of 12,224 (this list looks approved-only) |
| `code_url` null | 0 |
| `code_url` on `github.com` | 10,923 (about 89%) |
| ... with a `.git` suffix | 338 |
| ... with a trailing `/` | 340 |
| ... with a subpath (`/tree/...`, `/blob/...`, etc.) | 722 |
| `code_url` elsewhere | 1,301, across 38 hosts (codeberg.org, gitlab.com, tangled.org, gist.github.com, self-hosted git, printables, onshape, strudel.cc, ...) plus some that do not parse as URLs |

Decision: the response is well under 20 MB, so fetching on demand with a 10-minute in-process cache is acceptable. The nightly `upd_entries` script is not built. Each list is about 13.6 MB parsed per cache miss; revisit if the list grows several-fold or if many instances run.

Note: the entry carries `slack_id`, `display_name`, `country` and `description`. Tally keeps only `id`, `ysws` and `approved_at` and shows only the program name and record id publicly.

## Unknown (do not guess)

| Unknown | Status |
|---|---|
| Whether Ships `ysws_entries` **is** the Unified Projects DB or a view of part of it | Open. The list looks approved-only (no null `approved_at`); whether it covers all programs is not known. |
| How a program **writes** to the Unified DB (Airtable base access is granted by HQ; no public API was found) | Open |
| What "weighted" means and who computes it | Open |
| Whether a Unified DB record can carry a Tally project URL or hours, and in which field | Open |
| Size of the `ysws_entries` response | Answered above: 13.6 MB, 12,224 entries |

## Questions for Hack Club HQ

The maintainer asks these (Tally does not contact HQ itself).

1. Is Ships `ysws_entries` the full Unified Projects DB, or only a view (approved only? all programs? are other states hidden)?
2. How does a program write to the Unified DB: is there an API or Airtable access we can be granted, and who grants it?
3. What does "weighted projects" mean for the bounty, who computes the weight, and from which fields?
4. Can a Unified DB record carry a Tally project URL or tracked hours, and in which field? Can a public stable field of a record (such as `hours`) be set by Tally data?

## What was built without HQ

Read-only link: a project owner links one entry whose `code_url` normalizes to the project's linked GitHub repo (`owner/repo`, lowercase; `.git`, trailing `/`, `/tree/...`, `/blob/...` dropped; non-GitHub hosts never match). Matching on the repo is the only identifier both sides share. The choice is stored in `projects.upd_record_id`.
