# Scripts

Structure-only measurements over local Claude Code session logs (`~/.claude/projects`). They print key paths, counts, lengths and enum-like values. **They never print prompt or response text.** Run with `python -I` (isolated mode, nothing imported from the current directory).

| Script | Question it answers |
|---|---|
| `shape.py` | Where does `toolDenialKind` appear, and does the entry carry a `permissionDecision`? |
| `where.py` | Which entry types and key paths carry `toolDenialKind`; how many log lines mention `pasted_content` |
| `paste.py` | Which tag shapes wrap pastes in real 2.1.x logs, and in which entry kinds |
| `measure.py` | How many human-classified prompts are really injected (compaction summaries, meta flags) or paste-dominated |
| `git_hours_baseline.py` | Commit-gap hours estimate (git-hours algorithm) for a repo, to compare against the ledger |

Findings from these runs: `../implementation/steal-writeups.md`, items 1 to 3. Re-run them on another machine before relying on the numbers: they describe one person's logs.
