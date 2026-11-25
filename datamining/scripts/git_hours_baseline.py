"""Re-implementation of the kimmobrunfeldt/git-hours estimate (MIT), stdlib only.

Algorithm (per author, then summed, as git-hours does):
  sort commit times; for each consecutive pair, gap = next - prev.
  gap < max_diff  -> add the gap          (same "session")
  gap >= max_diff -> add first_add        (new session; flat guess)
  The oldest commit of an author adds nothing on its own; fewer than 2
  commits -> 0 hours. Sessions = 1 + number of gaps >= max_diff.
  git-hours rounds each author's hours to an integer; this script does not.

Differences from git-hours: it reads one `git log --no-merges` (git-hours
includes merge commits by default and walks all local branches, deduped by
sha), and groups by raw author email (no aliases).

Limits: only commit timestamps exist, so it cannot see agent runs, review time
or idle time; a long gap is a flat constant whatever happened in it.
Usage: python -I git_hours_baseline.py REPO [--author EMAIL] [--max-diff-min 120] [--first-add-min 120]
"""
import argparse
import subprocess
from collections import defaultdict


def estimate(times, max_diff, first_add):
    """times: unix seconds for one author. Returns (hours, sessions)."""
    t = sorted(times)
    if len(t) < 2:
        return 0.0, len(t)
    mins, sessions = 0.0, 1
    for a, b in zip(t, t[1:]):
        gap = (b - a) / 60
        if gap < max_diff:
            mins += gap
        else:
            mins += first_add
            sessions += 1
    return mins / 60, sessions


def main():
    p = argparse.ArgumentParser()
    p.add_argument("repo")
    p.add_argument("--author")
    p.add_argument("--max-diff-min", type=float, default=120)
    p.add_argument("--first-add-min", type=float, default=120)
    a = p.parse_args()
    out = subprocess.run(
        ["git", "-C", a.repo, "log", "--no-merges", "--format=%ct %ae"],
        capture_output=True, text=True, check=True).stdout
    by = defaultdict(list)
    for line in out.splitlines():
        ts, _, email = line.partition(" ")
        if not a.author or email.lower() == a.author.lower():
            by[email].append(int(ts))
    hours = sessions = commits = 0
    for ts in by.values():
        h, s = estimate(ts, a.max_diff_min, a.first_add_min)
        hours += h; sessions += s; commits += len(ts)
    print(f"hours={hours:.2f} commits={commits} sessions={sessions} "
          f"authors={len(by)} max_diff={a.max_diff_min:g} first_add={a.first_add_min:g}")


if __name__ == "__main__":
    # self-check: gaps 30m, 30m, 300m with 120/120 -> 0.5+0.5? (30+30+120)/60 = 3.0h, 2 sessions
    assert estimate([0, 1800, 3600, 3600 + 18000], 120, 120) == (3.0, 2)
    main()
