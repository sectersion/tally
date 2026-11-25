import json, os, re, sys, collections
root = os.path.expanduser("~/.claude/projects")
den = collections.Counter(); pd = collections.Counter(); tags = collections.Counter()
ver = collections.Counter()
for d, _, fs in os.walk(root):
    for f in fs:
        if not f.endswith(".jsonl"): continue
        try:
            for line in open(os.path.join(d, f), encoding="utf-8", errors="replace"):
                if "toolDenialKind" in line or "pasted_content" in line:
                    try: v = json.loads(line)
                    except Exception: continue
                    if "toolDenialKind" in v:
                        p = v.get("permissionDecision")
                        den[(v["toolDenialKind"], "pd=" + (f'{p.get("decision")}/{p.get("source")}' if p else "none"))] += 1
                    s = json.dumps(v.get("message", {}).get("content", ""))
                    for m in re.findall(r'<(/?pasted_content[^>\]*(?:\\"[^>]*)?)>', s):
                        tags[re.sub(r'\d+', 'N', m)[:40]] += 1
                    if "pasted_content" in s: ver[v.get("version")] += 1
                    ps = v.get("promptSource")
                    if "pasted_content" in s: tags["promptSource=" + str(ps)] += 1
        except Exception: pass
print("denials:", dict(den)); print("tags:", dict(tags)); print("versions:", dict(ver))
