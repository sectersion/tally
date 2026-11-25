## Hindsight (mixid_mbb): site only

**The repo link doesn't work.** `github.com/MIXIDtheSilly/Hindsight` returns 404 (not in the owner's public repo list), so it is private or renamed. The site says "open source AGPLv3" and points to GitHub releases, so this is based on the landing page only. I read no code.

**Model** (numbers quoted from the site):
- *Hands-on*: each prompt, edit, diff review or commit keeps the clock running **35 s**.
- *Watching*: an agent run counts if you react within **20 s** of it finishing (reply, scroll, open diff).
- *Check-in*: a mascot ("Byte") adds **5 min** per interaction and asks "what's the agent doing?" before it expires. Answering extends it.
- *Unattended*: runs while away show as hatched blocks on the timeline and are excluded from the total.
- Quick replies like "continue" earn about 1 min, not 5.

**Capture:** installer hooks Claude Code, OpenCode, VS Code, Cursor. Zed works with no setup. Git.

**Privacy claim:** only timing metadata leaves. The sample event is `prompt_submitted` with `text_len`, `typed_permille` and `content_hash`.

**Ideas for Tally**
- **`typed_permille` (steal).** The share of a prompt that was typed vs pasted is a cheap human-presence signal we don't have. A pasted 2 kB prompt and a typed one get different substance caps. Check whether it can be taken from harness logs alone. Probably not, so it likely needs a hook or an editor extension. It could be an optional feature.
- **`content_hash` (adapt).** Lets the server detect duplicate or replayed prompts without seeing text. We could add a salted hash as a numeric text feature, but check it against the privacy list in the README first. A hash of short text is brute-forceable, so use a per-user salt.
- **Hatched "unattended" timeline (steal for the dashboard).** Showing the excluded time instead of dropping it makes the subtraction visible. It matches our *unverified* bucket idea.
- **Check-in prompt (avoid).** It is a presence challenge that interrupts the user, and it is easy to script. It also contradicts "passive capture". Our OS idle timer plus action evidence is the better approach.
- **Flat per-action windows (35 s / 20 s / 5 min).** These are simpler than our claim-since-previous-action model, but they pay for activity and not for decisions. This is exactly what the bounty post complains about, so I'd keep our model.

Verdict: steal `typed_permille` and the unattended-timeline idea. Adapt the hash. Avoid the check-in.
