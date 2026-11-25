#!/usr/bin/env bash
# Tests for integrations/claude-code/hooks/session-start with a fake tally and a fake vendored installer.
cd "$(dirname "$0")/.." || exit 1
SRC=$PWD/integrations/claude-code
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
VER=$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' "$SRC/.claude-plugin/plugin.json")
pass=0; failn=0
ok() { pass=$((pass+1)); echo "ok   $1"; }
no() { failn=$((failn+1)); echo "FAIL $1: $2"; }

# Fresh world: plugin root copy (optionally with a fake installer), fake tally bin, home.
world() { # $1 name, $2 fake tally version or "" for none, $3 installer body or "" for none
  W=$T/$1; rm -rf "$W"; mkdir -p "$W/root/hooks" "$W/root/.claude-plugin" "$W/bin" "$W/home" "$W/proj/My Code é"
  cp "$SRC/hooks/session-start" "$W/root/hooks/"; cp "$SRC/.claude-plugin/plugin.json" "$W/root/.claude-plugin/"
  if [ -n "$2" ]; then # fake tally: "$W/noversion" = no --version flag; "$W/mode" = garbage | partial | sleep
    cat > "$W/bin/tally" <<'FAKE'
#!/bin/sh
W=@W@
if [ "$1" = --version ]; then [ -f "$W/noversion" ] && exit 2; echo "tally @V@"; exit 0; fi
for a in "$@"; do printf '[%s]\n' "$a"; done >> "$W/calls.log"
case "$(cat "$W/mode" 2>/dev/null)" in
  garbage) echo "not json at all" ;;
  fail) echo "boom: no database" >&2; exit 1 ;;
  partial) printf '{"systemMessage": "x'; exit 1 ;;
  sleep) sleep 30 & echo $! > "$W/sleeper.pid"; wait ;;
esac
FAKE
    sed -i "s|@W@|$W|; s|@V@|$2|" "$W/bin/tally"
    chmod +x "$W/bin/tally"
  fi
  if [ -n "$3" ]; then mkdir -p "$W/root/install"; printf '#!/bin/sh\n%s\n' "$3" > "$W/root/install/install.sh"; chmod +x "$W/root/install/install.sh"; fi
}
run() { # runs the hook; sets RC OUT ERR
  local start=$(date +%s%N)
  OUT=$(PATH="$W/bin:/usr/bin:/bin" CLAUDE_PLUGIN_ROOT="$W/root" CLAUDE_PROJECT_DIR="$W/proj/My Code é" TALLY_HOME="${HOME_OVERRIDE:-$W/home}" \
    bash "$W/root/hooks/session-start" session-start 2>"$W/err"); RC=$?
  ERR=$(cat "$W/err"); ELAPSED_MS=$((($(date +%s%N)-start)/1000000)); ELAPSED=$((ELAPSED_MS/1000))
}
valid() { [ -z "$OUT" ] || printf '%s' "$OUT" | node -e 'JSON.parse(require("fs").readFileSync(0,"utf8"))' 2>/dev/null; }
common() { [ "$RC" = 0 ] || { no "$1" "exit $RC"; return 1; }; valid || { no "$1" "stdout not JSON: $OUT"; return 1; }; }
waitfor() { for _ in $(seq 1 40); do grep -q "$2" "$1" 2>/dev/null && return 0; sleep 0.1; done; return 1; }

t=calls_ensure_with_the_cwd; world $t "$VER" ""; run
if common $t; then
  waitfor "$W/calls.log" hook-json
  want=$(printf '[ensure]
[--client]
[claude-code]
[--register-cwd]
[%s]
[--hook-json]' "$W/proj/My Code é")
  [ "$(cat "$W/calls.log" 2>&1)" = "$want" ] && ok $t || no $t "log: $(cat "$W/calls.log" 2>&1)"
fi

t=old_binary_is_replaced_not_called; world $t 0.0.9 'env | grep "^TALLY_" > "$(dirname "$0")/../../inst.env"; echo "$@" >> "$(dirname "$0")/../../inst.env"'; run
if common $t; then
  waitfor "$W/inst.env" binary-only
  if grep -q "TALLY_VERSION=$VER" "$W/inst.env" && grep -q -- --binary-only "$W/inst.env" && ! grep -qF '[ensure]' "$W/calls.log" 2>/dev/null \
     && grep -q installing <<<"$OUT"; then ok $t; else no $t "env: $(cat "$W/inst.env" 2>&1) calls: $(cat "$W/calls.log" 2>&1) out: $OUT"; fi
fi

t=offline_install_fails_quietly; world $t "" 'exit 1'; run
if common $t; then
  waitfor "$W/home/ensure.log" install
  if [ "$ELAPSED" -lt 5 ] && grep -q 'systemMessage' <<<"$OUT" && grep -q installing <<<"$OUT" && grep -qi 'install' "$W/home/ensure.log" 2>/dev/null; then ok $t
  else no $t "elapsed $ELAPSED out $OUT log $(cat "$W/home/ensure.log" 2>&1)"; fi
fi

t=unwritable_home_is_fine; world $t "$VER" ""; : > "$W/file"; HOME_OVERRIDE=$W/file/sub run; unset HOME_OVERRIDE
if common $t; then [ -z "$ERR" ] && ok $t || no $t "stderr: $ERR"; fi

t=no_tally_and_no_installer_is_fine; world $t "" ""; run
if common $t; then grep -qi 'reinstall' <<<"$OUT" && grep -q systemMessage <<<"$OUT" && ok $t || no $t "out: $OUT"; fi

t=no_version_flag_without_ensure_is_stale; world $t 0.1.0 'echo "$@" >> "$(dirname "$0")/../../inst.env"'; touch "$W/noversion"; run
if common $t; then waitfor "$W/inst.env" binary-only
  if grep -q -- --binary-only "$W/inst.env" && ! grep -qF '[ensure]' "$W/calls.log" 2>/dev/null; then ok $t; else no $t "inst: $(cat "$W/inst.env" 2>&1) calls: $(cat "$W/calls.log" 2>&1)"; fi; fi

t=no_version_flag_with_ensure_is_still_stale; world $t 0.1.0 'echo "$@" >> "$(dirname "$0")/../../inst.env"'; touch "$W/noversion"; run
if common $t; then waitfor "$W/inst.env" binary-only
  if grep -q -- --binary-only "$W/inst.env" && grep -q installing <<<"$OUT"; then ok $t; else no $t "out: $OUT inst: $(cat "$W/inst.env" 2>&1)"; fi; fi

t=exact_plugin_version_is_accepted; world $t "$VER" 'echo ran >> "$(dirname "$0")/../../inst.env"'; run
if common $t; then waitfor "$W/calls.log" hook-json
  if grep -qF '[ensure]' "$W/calls.log" && [ ! -e "$W/inst.env" ] && ! grep -q installing <<<"$OUT"; then ok $t; else no $t "out: $OUT"; fi; fi

t=ensure_garbage_is_dropped; world $t "$VER" ""; echo garbage > "$W/mode"; run
if common $t; then [ -z "$OUT" ] && grep -q 'non-JSON' "$W/home/ensure.log" && ok $t || no $t "out: $OUT log: $(cat "$W/home/ensure.log" 2>&1)"; fi

t=ensure_partial_json_is_dropped; world $t "$VER" ""; echo partial > "$W/mode"; run
if common $t; then [ -z "$OUT" ] && grep -q 'ensure exited' "$W/home/ensure.log" && ok $t || no $t "out: $OUT log: $(cat "$W/home/ensure.log" 2>&1)"; fi

t=hanging_ensure_is_bounded; world $t "$VER" ""; echo sleep > "$W/mode"; run
[ -f "$W/sleeper.pid" ] && kill "$(cat "$W/sleeper.pid")" 2>/dev/null
if common $t; then [ "$ELAPSED_MS" -lt 4900 ] && [ -z "$OUT" ] && ok $t || no $t "elapsed ${ELAPSED_MS}ms out $OUT"; fi

t=slash_command_shows_a_failing_tally_error; world $t "$VER" ""; echo fail > "$W/mode"
OUT=$(PATH="$W/bin:/usr/bin:/bin" CLAUDE_PLUGIN_ROOT="$W/root" TALLY_HOME="$W/home" bash "$W/root/hooks/session-start" tally status --brief 2>&1); RC=$?
if grep -q 'boom: no database' <<<"$OUT" && [ "$(sed -n 1p "$W/calls.log")" = "[status]" ] && grep -q '\[--brief\]' "$W/calls.log"; then ok $t; else no $t "rc $RC out: $OUT"; fi

echo "$pass passed, $failn failed"; [ "$failn" = 0 ]
