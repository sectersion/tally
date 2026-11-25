import json, os, re, collections, statistics
root = os.path.expanduser("~/.claude/projects")
PFX = ["<task-notification>","<local-command-stdout>","<local-command-stderr>","<bash-stdout>","<bash-stderr>","Caveat:","This session is being continued from a previous conversation"]
flags = collections.Counter(); pref = collections.Counter(); humans = 0
P = re.compile(r'<pasted_content id="?[A-Za-z0-9]{1,32}"?>.*?</pasted_content(?: id="?[A-Za-z0-9]{1,32}"?)?>', re.S)
rows = []
for d, _, fs in os.walk(root):
    if "subagents" in d: continue
    for f in fs:
        if not f.endswith(".jsonl"): continue
        for line in open(os.path.join(d, f), encoding="utf-8", errors="replace"):
            if '"type":"user"' not in line.replace(" ", ""): continue
            try: v = json.loads(line)
            except Exception: continue
            if v.get("type") != "user" or v.get("isSidechain"): continue
            o = (v.get("origin") or {}).get("kind")
            human = (o == "human") if "origin" in v else (v.get("turnOrigin") == "human")
            c = (v.get("message") or {}).get("content")
            if not human or not isinstance(c, (str, list)): continue
            if v.get("toolUseResult") is not None: continue
            t = c if isinstance(c, str) else "".join(i.get("text", "") for i in c if isinstance(i, dict))
            humans += 1
            for k in ("isMeta","isCompactSummary","isVisibleInTranscriptOnly","isVirtual"):
                if v.get(k): flags[k] += 1
            for p in PFX:
                if t.lstrip().startswith(p): pref[p[:24]] += 1
            pasted = sum(len(m.group(0)) for m in P.finditer(t))
            if pasted: rows.append((len(t), len(t) - pasted))
print("human-classified prompts:", humans); print("flagged:", dict(flags)); print("injected prefixes:", dict(pref))
print("prompts with paste blocks:", len(rows))
if rows:
    print("median total chars:", statistics.median(r[0] for r in rows), " median typed chars:", statistics.median(r[1] for r in rows))
    print("typed < 20 chars (would turn trivial if pastes were ignored):", sum(1 for r in rows if r[1] < 20))
    print("pasted share of chars overall: %.0f%%" % (100 * (1 - sum(r[1] for r in rows) / sum(r[0] for r in rows))))
