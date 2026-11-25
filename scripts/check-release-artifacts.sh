#!/bin/sh
# Usage: check-release-artifacts.sh <dir> <version>
# Checks the release artifact set in <dir>: exactly the six expected archives, each listed in SHA256SUMS,
# `sha256sum -c` passes, and an archive without `-unsigned` in its name has a `<archive>.SIGNED` marker
# (written by the job that actually ran the signing step), so an unsigned build is never presented as signed.
set -eu
dir=${1:?dir}
ver=${2:?version}
cd "$dir"
fail=0
targets="x86_64-pc-windows-msvc aarch64-pc-windows-msvc x86_64-apple-darwin aarch64-apple-darwin x86_64-unknown-linux-musl aarch64-unknown-linux-musl"
found=""
for t in $targets; do
  case $t in *windows*) ext=zip ;; *) ext=tar.gz ;; esac
  signed="tally-$ver-$t.$ext"
  unsigned="tally-$ver-$t-unsigned.$ext"
  if [ -f "$signed" ] && [ -f "$unsigned" ]; then echo "both signed and unsigned archives for $t" >&2; fail=1; continue; fi
  if [ -f "$signed" ]; then
    f=$signed
    [ -f "$f.SIGNED" ] || { echo "$f has no signed-by-the-job marker $f.SIGNED" >&2; fail=1; }
  elif [ -f "$unsigned" ]; then
    f=$unsigned
  else
    echo "missing archive for $t" >&2; fail=1; continue
  fi
  found="$found $f"
  grep -q "  $f\$" SHA256SUMS || { echo "$f is not in SHA256SUMS" >&2; fail=1; }
done
# nothing else may sit in the set (other than the sums file and markers)
for f in *; do
  case $f in SHA256SUMS | *.SIGNED) continue ;; esac
  case " $found " in *" $f "*) ;; *) echo "unexpected file $f" >&2; fail=1 ;; esac
done
sha256sum -c SHA256SUMS >/dev/null || { echo "sha256sum -c failed" >&2; fail=1; }
[ "$fail" = 0 ] && echo "release artifacts ok ($(echo $found | wc -w) archives)"
exit "$fail"
