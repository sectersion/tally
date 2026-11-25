#!/usr/bin/env bash
# Cross-file checks for the editor integrations. Needs node (no jq).
set -u
cd "$(dirname "$0")/.."
fail=0
bad() { echo "FAIL: $*"; fail=1; }
js() { node -e "const f=require('fs');const j=JSON.parse(f.readFileSync(process.argv[1],'utf8'));$2" "$1"; }

want=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/tally-daemon/Cargo.toml | head -n1)
[ -n "$want" ] || bad "no version in crates/tally-daemon/Cargo.toml"

# (a) marketplace source resolves to a plugin directory
mp=.claude-plugin/marketplace.json
if [ -f "$mp" ]; then
  src=$(js "$mp" 'console.log(j.plugins[0].source)') || bad "unreadable $mp"
  [ -f "${src:-/nonexistent}/.claude-plugin/plugin.json" ] || bad "(a) marketplace source '$src' has no .claude-plugin/plugin.json"
else bad "(a) $mp missing"; fi

# (b) plugin version equals the crate version
pj=integrations/claude-code/.claude-plugin/plugin.json
if [ -f "$pj" ]; then
  got=$(js "$pj" 'console.log(j.version)')
  [ "$got" = "$want" ] || bad "(b) plugin.json version '$got' != Cargo.toml '$want'"
else bad "(b) $pj missing"; fi

# (c) every file hooks.json names exists and is executable in git
hj=integrations/claude-code/hooks/hooks.json
if [ -f "$hj" ]; then
  cmds=$(js "$hj" 'for(const e of j.hooks.SessionStart)for(const h of e.hooks)for(const k of ["command","powershell"])if(h[k])console.log(h[k])')
  paths=$(printf '%s\n' "$cmds" | grep -o '{CLAUDE_PLUGIN_ROOT}/[^" ]*' | sed 's#^{CLAUDE_PLUGIN_ROOT}#integrations/claude-code#' | sort -u)
  [ -n "$paths" ] || bad "(c) hooks.json names no \${CLAUDE_PLUGIN_ROOT} file"
  for p in $paths integrations/claude-code/hooks/session-start; do
    [ -f "$p" ] || { bad "(c) $p missing"; continue; }
    mode=$(git ls-files -s -- "$p" | cut -c1-6)
    [ "$mode" = 100755 ] || bad "(c) $p is not executable in git (mode '$mode'; git update-index --chmod=+x)"
  done
else bad "(c) $hj missing"; fi
vs=integrations/vscode/package.json
if [ -f "$vs" ] && [ -f crates/tally-daemon/src/setup/vscode.rs ]; then
  id=$(sed -n 's/.*EXTENSION_ID[^"]*"\([^"]*\)".*/\1/p' crates/tally-daemon/src/setup/vscode.rs | head -n1)
  pn=$(js "$vs" 'console.log(j.publisher+"."+j.name)')
  [ "$id" = "$pn" ] || bad "(c) EXTENSION_ID '$id' != package.json '$pn'"
fi
# (d)
if [ -f "$vs" ]; then
  v=$(js "$vs" 'console.log(j.version)')
  [ "$v" = "$want" ] || bad "(d) vscode package.json version '$v' != '$want'"
fi

# (f) one license everywhere: the crate, the plugin and the extension
lic=$(sed -n 's/^license = "//p' crates/tally-daemon/Cargo.toml | head -n1 | tr -d '"')
for f in "$pj" "$vs"; do
  [ -f "$f" ] || continue
  l=$(js "$f" 'console.log(j.license)')
  [ "$l" = "$lic" ] || bad "(f) $f license '$l' != Cargo.toml '$lic'"
done
[ ! -f "$vs" ] || cmp -s LICENSE integrations/vscode/LICENSE || bad "(f) integrations/vscode/LICENSE differs from LICENSE"

# (e) each command file has description front matter
n=0
for f in integrations/claude-code/commands/*.md; do
  [ -f "$f" ] || continue
  n=$((n+1))
  head -n 5 "$f" | grep -q '^description: .' || bad "(e) $f has no description front matter"
done
[ "$n" -gt 0 ] || bad "(e) no command files"

[ "$fail" = 0 ] && echo "check-integrations: ok"
exit "$fail"
