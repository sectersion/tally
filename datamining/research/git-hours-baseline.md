# git-hours baseline for Tally (sectersion/tally)

## Summary
The canonical estimator is kimmobrunfeldt/git-hours (816 stars, MIT). It counts gaps between commits, nothing else. On this repo (164 non-merge commits, one author, all within about 27 wall-clock hours) it gives 20.1 h at defaults, 17.0 h at 30/30 and 19.1 h at 60/60. The README badge text carries no number. The local copy of `origin/tally-data` badge.json read "0.0 h" (may be stale). A reviewer will see a gap-based number far from a per-action number, and the reason is structural.

## Findings
1. **Algorithm (read from `src/index.js`, cloned to `<scratch>/git-hours/src`, never run).** Defaults: `maxCommitDiffInMinutes` 120, `firstCommitAdditionInMinutes` 120. Per author, sort commit dates. For each consecutive pair: gap < 120 min adds the gap; otherwise adds a flat 120 min. So hours = sum of small gaps + (sessions-1) x 120 min. The last commit adds nothing, and an author with fewer than 2 commits gets 0. Each author is rounded to an integer, then summed.
2. **Scope.** Authors are grouped by author email (optional `-e` aliases). It walks all local `refs/heads/*` branches, dedupes by sha, and uses author date. Merge commits are INCLUDED by default (`mergeRequest: true`; only skipped if you pass `-m false`, which matches messages starting "Merge "). It refuses shallow clones. Remote-only branches and tags are ignored.
3. **Other estimators (verified by `gh`/README).** Ruin0x11/jikyuu (25 stars, Rust port of git-hours, same 120/120 defaults). luigitni/git-estimate (36 stars, Go; a baseline-per-session value defaulting to 2 h, with "session" or "day" methods; exact gap constant UNVERIFIED). Both are variants of the same idea; only the constants differ.
4. **My script** `datamining/scripts/git_hours_baseline.py` mirrors git-hours with two deliberate differences: `git log --no-merges` only, and no per-author rounding. Results on this checkout (`python -I`):

| max-diff / first-add | hours | commits | sessions |
|---|---|---|---|
| 120 / 120 (default) | 20.11 | 164 | 2 |
| 60 / 60 | 19.11 | 164 | 2 |
| 30 / 30 | 16.99 | 164 | 7 |

   There is one author (164 commits), so "all authors" and "top author" are identical. First to last commit is 26.8 h of wall clock, so the estimate cannot exceed about that plus one session add.
5. **Where it disagrees with a per-action ledger.**
   - Many tiny commits: gaps under the threshold are counted in full, so a commit every 20 min credits 100% of the time regardless of who was present. Here 164 commits in about 27 h gives roughly 6 min per commit.
   - Long agent runs between commits: a 3 h agent run with no commit is one flat 120 min add (undercount if the human was present, overcount if the human was asleep). A 100 min run is credited in full, though Tally subtracts watch time.
   - Squash-merge or one-big-commit workflows: history collapses to a few timestamps, so git-hours returns near 0 or just first-adds. Tally's ledger is unaffected.
   - Bursts: the threshold cliff makes results unstable. Moving 120 to 30 changed sessions from 2 to 7 and hours by 16%, and a burst after a 119 min gap is credited entirely.
   - Per-commit time is also affected by commit-author vs. committer date and rebases (UNVERIFIED on this repo).
6. **What a reviewer will conclude.** Within about 2x either way, they will treat the two as consistent, since the estimator is crude. At 3x to 10x they will assume one is inflated, and the usual assumption is the one with the more complex method (Tally). Tally counts only from 2026-11-23 and only human-decision time, so for a repo this young a gap-based number is plausibly higher than Tally's, which looks like deflation, not inflation, and is the safer direction.

## Recommendation
Publish the git-hours figure next to the Tally figure in the submission, with the commands and the 30/30, 60/60 and 120/120 range (about 17 to 20 h here). State plainly that git-hours measures commit cadence, not attention, and that Tally's lower figure subtracts agent time by design. Put the ratio in a short "why these differ" note covering the cases in finding 5, so a reviewer finds the explanation before the discrepancy.

## Not verified
- UNVERIFIED: git-estimate's exact gap constants. jikyuu's defaults come from its README only.
- UNVERIFIED: whether the live badge on `tally-data` matches the local `origin/tally-data` ("0.0 h"); I did not fetch.
- UNVERIFIED: what Hack Club reviewers actually use; git-hours is an assumption.
- My script was not diffed against real git-hours output (not run). Merge commits, multiple branches and per-author rounding are the known differences.
- Tally's ledger total was not compared, since no figure was readable offline.
