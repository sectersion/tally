#!/bin/sh
# Bump the workspace version, commit, and tag: `sh scripts/release-bump.sh patch|minor`
# patch = a release, minor = breaking change (0.x SemVer rules). 1.0.0 is set by hand.
set -e
bump=$1; [ "$bump" = patch ] || [ "$bump" = minor ] || { echo "usage: $0 patch|minor"; exit 1; }
f=crates/tally-daemon/Cargo.toml
ver=$(sed -n 's/^version = "\(.*\)"/\1/p' "$f" | head -1)
[ -n "$ver" ] || { echo "no version in $f"; exit 1; }
case "$bump" in
  minor) y=$(echo "$ver" | cut -d. -f2); new="0.$((y + 1)).0" ;;
  patch) z=$(echo "$ver" | cut -d. -f3); new="${ver%.*}.$((z + 1))" ;;
esac
for f in crates/*/Cargo.toml; do
  sed -i "s/^version = \"$ver\"/version = \"$new\"/" "$f"
done
git diff --exit-code -- crates/tally-daemon/tests/fixtures >/dev/null || { echo "uncommitted test fixtures touched"; exit 1; }
cargo check -q || true # refresh Cargo.lock version pins
git add crates/*/Cargo.toml Cargo.lock
git commit -m "release: $new"
git tag "v$new"
echo "tagged v$new — push with: git push origin main \"v$new\""
