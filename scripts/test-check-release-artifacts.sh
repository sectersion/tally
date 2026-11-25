#!/bin/sh
# Self-check for check-release-artifacts.sh: a good set passes; each kind of damage fails.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
chk="$here/check-release-artifacts.sh"
v=0.1.0
mk() { # dir, signed(0/1)
  mkdir -p "$1"; (cd "$1"
  for t in x86_64-pc-windows-msvc aarch64-pc-windows-msvc x86_64-apple-darwin aarch64-apple-darwin x86_64-unknown-linux-musl aarch64-unknown-linux-musl; do
    case $t in *windows*) e=zip ;; *) e=tar.gz ;; esac
    if [ "$2" = 1 ]; then n="tally-$v-$t.$e"; : > "$n.SIGNED"; else n="tally-$v-$t-unsigned.$e"; fi
    echo "bits $t" > "$n"
  done
  sha256sum tally-* | grep -v SIGNED | sed 's/ \*/  /' > SHA256SUMS)
}
t=$(mktemp -d); trap 'rm -rf "$t"' EXIT

mk "$t/unsigned" 0; "$chk" "$t/unsigned" $v >/dev/null || { echo "FAIL: good unsigned set rejected"; exit 1; }
mk "$t/signed" 1;   "$chk" "$t/signed" $v >/dev/null   || { echo "FAIL: good signed set rejected"; exit 1; }

mk "$t/missing" 0; rm "$t/missing/tally-$v-aarch64-apple-darwin-unsigned.tar.gz"
"$chk" "$t/missing" $v >/dev/null 2>&1 && { echo "FAIL: missing target accepted"; exit 1; }

mk "$t/tamper" 0; echo x >> "$t/tamper/tally-$v-x86_64-apple-darwin-unsigned.tar.gz"
"$chk" "$t/tamper" $v >/dev/null 2>&1 && { echo "FAIL: tampered archive accepted"; exit 1; }

mk "$t/fake" 1; rm "$t/fake/tally-$v-x86_64-pc-windows-msvc.zip.SIGNED"
"$chk" "$t/fake" $v >/dev/null 2>&1 && { echo "FAIL: signed name without marker accepted"; exit 1; }

mk "$t/nosum" 0; grep -v aarch64-apple "$t/nosum/SHA256SUMS" > "$t/nosum/s" && mv "$t/nosum/s" "$t/nosum/SHA256SUMS"
"$chk" "$t/nosum" $v >/dev/null 2>&1 && { echo "FAIL: archive missing from SHA256SUMS accepted"; exit 1; }

echo "check-release-artifacts self-test ok"
