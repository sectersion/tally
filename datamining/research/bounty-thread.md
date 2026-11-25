# The bounty thread itself: NOT datamined

**Status: not done. I have no access to Slack from this environment**, so none of this was read first-hand. The only thing we know about the thread is a pasted summary from someone who read it:

- The thread sits under zrl's Nov 18 bounty post in #zrl-land, with 2,900+ replies, newest dated Nov 22.
- That summary covered 8 projects (4 with a live site and a public repo, 3 with no public link, plus the Kairos question from #hq).
- Slack search did not work for the person who wrote the summary, so it comes from reading the thread and recent #ship / #hq / #code history. Entries submitted by DM or in private channels are not covered.

## Why it is still the most valuable thing left to read
The code of the competing entries tells us how they work. The thread tells us what the **reviewers will accept**, and that decides whether any of our design choices matter:
- What exactly zrl and HQ said "good" looks like (hours per day caps, per-tool support, verification, privacy red lines).
- Whether Kairos (HQ's own repo, `hackclub/kairos`) is the official method. Someone asked in #hq; the answer was not in the summary.
- Entries posted after Nov 22, and entries that were submitted by DM or in private channels.
- Reviewer objections to existing entries (e.g. what they said about Steer's own admission that a rhythm-faking script earns 4-6 h/day).
- Any announced payout rules or deadlines.

## What to do
A person with Slack access should export the thread (or paste the replies from zrl, HQ staff and bounty reviewers) into a file here. Then re-run the datamine: extract reviewer requirements, list any new entries with links, and check each requirement against `docs/spec/scoring.md`.

Suggested checklist for the reader:
1. Search the thread for `zrl`, `hq`, `reviewer`, `approve`, `reject`, `hours`, `verify`, `privacy`.
2. Collect every GitHub or site link posted, not only the 8 already known.
3. Note any statement of what counts as a valid tracker (log parsing, hooks, editor plugin, self-report).
4. Note any deadline or payout rule.
