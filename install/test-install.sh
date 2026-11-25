#!/bin/sh
# Usage: test-install.sh <path-to-built-tally-binary>
# Runs install.sh against a fake release directory: install, install again (no-op), corrupt checksum (installs nothing),
# uninstall (binary removed, data kept).
set -eu
bin=${1:?path to the tally binary}
here=$(cd "$(dirname "$0")" && pwd)
ver=$("$bin" --version | awk '{print $2}')
case $(uname -s) in Darwin) os=apple-darwin ;; *) os=unknown-linux-musl ;; esac
case $(uname -m) in arm64 | aarch64) arch=aarch64 ;; *) arch=x86_64 ;; esac
name="tally-$ver-$arch-$os.tar.gz"

t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
mkdir -p "$t/rel" "$t/stage" "$t/home"
cp "$bin" "$t/stage/tally"
tar -C "$t/stage" -czf "$t/rel/$name" tally
if command -v sha256sum >/dev/null 2>&1; then h=$(sha256sum "$t/rel/$name" | cut -d' ' -f1); else h=$(shasum -a 256 "$t/rel/$name" | cut -d' ' -f1); fi
echo "$h  $name" > "$t/rel/SHA256SUMS"

export TALLY_RELEASE_BASE="file://$t/rel" TALLY_VERSION="$ver" TALLY_HOME="$t/home"
inst() { TALLY_INSTALL_DIR="$1" sh "$here/install.sh" "${2:-}"; }
fail() { echo "FAIL: $*" >&2; exit 1; }

# (a) first run installs a working binary
inst "$t/bin" >/dev/null || fail "first install exited non-zero"
[ "$("$t/bin/tally" --version)" = "tally $ver" ] || fail "installed binary does not report tally $ver"

# (b) second run is a no-op and leaves the binary untouched
before=$(ls -l --time-style=+%s "$t/bin/tally" 2>/dev/null || ls -l -D %s "$t/bin/tally" 2>/dev/null || stat -f %m "$t/bin/tally")
sleep 1
out=$(inst "$t/bin") || fail "second install exited non-zero"
echo "$out" | grep -q "already installed" || fail "second install did not say 'already installed': $out"
after=$(ls -l --time-style=+%s "$t/bin/tally" 2>/dev/null || ls -l -D %s "$t/bin/tally" 2>/dev/null || stat -f %m "$t/bin/tally")
[ "$before" = "$after" ] || fail "binary changed on the second run"

# (c) a corrupt checksum installs nothing
echo "0000000000000000000000000000000000000000000000000000000000000000  $name" > "$t/rel/SHA256SUMS"
if inst "$t/clean" >/dev/null 2>&1; then fail "corrupt checksum was accepted"; fi
[ ! -e "$t/clean/tally" ] || fail "binary installed despite a bad checksum"

# (d) uninstall removes the binary and keeps the data
echo data > "$t/home/tally.db"
inst "$t/bin" --uninstall >/dev/null || fail "uninstall exited non-zero"
[ ! -e "$t/bin/tally" ] || fail "uninstall left the binary"
[ -f "$t/home/tally.db" ] || fail "uninstall touched the data directory"

echo "install.sh ok"
