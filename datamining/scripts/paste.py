import json, os, re, collections
root = os.path.expanduser("~/.claude/projects")
loc = collections.Counter(); shape = collections.Counter(); ver = collections.Counter()
TAG = re.compile(r'</?pasted_content[^>\n]{0,30}>')
for d, _, fs in os.walk(root):
    for f in fs:
        if not f.endswith(".jsonl"): continue
        for line in open(os.path.join(d, f), encoding="utf-8", errors="replace"):
            if "pasted_content" not in line: continue
            try: v = json.loads(line)
            except Exception: continue
            c = v.get("message", {}).get("content") if isinstance(v.get("message"), dict) else None
            txt = c if isinstance(c, str) else json.dumps(c)
            where = "message.content" if TAG.search(txt or "") else "other"
            o = (v.get("origin") or {}).get("kind")
            loc[(v.get("type"), where, o, v.get("promptSource"), isinstance(c, str))] += 1
            if where == "message.content":
                for t in TAG.findall(txt): shape[re.sub(r'"?\d+"?', 'N', t)] += 1
                ver[v.get("version")] += 1
print(dict(loc)); print(dict(shape)); print(dict(ver))
