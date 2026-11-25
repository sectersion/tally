import json, os, collections
root = os.path.expanduser("~/.claude/projects")
def walk(o, path, key, out):
    if isinstance(o, dict):
        for k, v in o.items():
            if k == key: out.append((path + "/" + k, v if not isinstance(v, (dict, list)) else "<obj>"))
            walk(v, path + "/" + k, key, out)
    elif isinstance(o, list):
        for x in o: walk(x, path + "[]", key, out)
paths = collections.Counter(); kinds = collections.Counter(); pc = collections.Counter()
for d, _, fs in os.walk(root):
    for f in fs:
        if not f.endswith(".jsonl"): continue
        for line in open(os.path.join(d, f), encoding="utf-8", errors="replace"):
            if "toolDenialKind" in line:
                try: v = json.loads(line)
                except Exception: continue
                out = []; walk(v, "", "toolDenialKind", out)
                for p, val in out:
                    paths[(v.get("type"), p, val, "pd" if "permissionDecision" in line else "nopd")] += 1
            if "pasted_content" in line: pc[line.count("pasted_content")] += 1
print(dict(paths)); print("lines mentioning pasted_content:", dict(pc))
