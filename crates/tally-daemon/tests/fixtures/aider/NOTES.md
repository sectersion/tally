# aider spike notes (docs-only; spike NOT run)

Status: no fixture, no parser. Aider was not run here. Nothing below was observed on a real session.

## Claims
- OBSERVED (vendor docs, https://aider.chat/docs/config/options.html): `--chat-history-file` default `.aider.chat.history.md`; env `AIDER_CHAT_HISTORY_FILE`.
- OBSERVED (same URL): `--input-history-file` default `.aider.input.history`.
- OBSERVED (same URL): `--llm-history-file` has no default (docs show example `.aider.llm.history`); logs the raw LLM conversation.
- OBSERVED (same URL): `--analytics-log FILE` has no default; logs analytics events.
- OBSERVED (same URL): `--gitignore` adds `.aider*` to .gitignore.
- OBSERVED (https://aider.chat/docs/faq.html): users copy markdown logs from `.aider.chat.history.md`.
- UNVERIFIED: the files are written relative to the working directory / git root (docs give bare filenames only).
- UNVERIFIED: any per-OS global log location (nothing documented).
- UNVERIFIED: record format of `.aider.chat.history.md` (timestamps per session? per message?), whether token counts or timestamps appear at all, how sessions are delimited.
- UNVERIFIED: whether `.aider.llm.history` carries timestamps/usage; it is opt-in so cannot be relied on.
- UNVERIFIED: that history files are append-only and safe to tail.

## What a maintainer must do to run the real spike
1. Install aider (`pip install aider-chat`) and configure any model/API key.
2. In a scratch git repo run `aider --llm-history-file .aider.llm.history --analytics-log .aider.analytics.jsonl` and do 2 short sessions (one with an edit, one idle), quit each.
3. Capture `.aider.chat.history.md`, `.aider.input.history`, `.aider.llm.history`, `.aider.analytics.jsonl`; also check `~/.aider*` and platform config dirs for any other files.
4. Record: timestamp presence/format, session delimiters, token/cost lines, append behaviour, cwd-relative placement.
5. Scrub secrets, save as fixtures here, mark each claim above OBSERVED with the file as evidence, then write the adapter.
