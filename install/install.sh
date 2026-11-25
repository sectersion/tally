#!/bin/sh
# Tally installer for macOS and Linux. Installs the binary only: no service, no tracking.
# Consent and the explainer come with `tally setup`.
#
#   curl -fsSL https://github.com/sectersion/tally/releases/latest/download/install.sh | sh
#   sh install.sh --uninstall        # removes the binary only; your data stays
#
# Environment: TALLY_VERSION (default: latest), TALLY_INSTALL_DIR (default: ~/.local/bin),
#   TALLY_RELEASE_BASE (flat directory or URL holding the archive and SHA256SUMS; needs TALLY_VERSION; for CI and offline installs).
set -eu

repo=sectersion/tally
dir=${TALLY_INSTALL_DIR:-$HOME/.local/bin}

die() { echo "tally install: $*" >&2; exit 1; }

home_dir() {
  if [ -n "${TALLY_HOME:-}" ]; then echo "$TALLY_HOME"; return; fi
  case $(uname -s) in
    Darwin) echo "$HOME/Library/Application Support/tally" ;;
    *) echo "${XDG_DATA_HOME:-$HOME/.local/share}/tally" ;;
  esac
}

if [ "${1:-}" = "--uninstall" ]; then
  echo "data kept in $(home_dir); to delete it run: tally service uninstall --purge"
  echo "(run that before removing the binary if you want it gone)"
  rm -f "$dir/tally"
  echo "removed $dir/tally"
  exit 0
fi

case $(uname -s) in
  Darwin) os=apple-darwin ;;
  Linux) os=unknown-linux-musl ;;
  *) die "unsupported OS $(uname -s); on Windows use install.ps1" ;;
esac
case $(uname -m) in
  arm64 | aarch64) arch=aarch64 ;;
  x86_64 | amd64) arch=x86_64 ;;
  *) die "unsupported architecture $(uname -m)" ;;
esac
target="$arch-$os"

if [ -n "${TALLY_RELEASE_BASE:-}" ]; then
  [ -n "${TALLY_VERSION:-}" ] || die "TALLY_RELEASE_BASE needs TALLY_VERSION"
  ver=$TALLY_VERSION
  base=${TALLY_RELEASE_BASE%/}
else
  ver=${TALLY_VERSION:-}
  if [ -z "$ver" ]; then
    final=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$repo/releases/latest") || die "cannot find the latest release"
    ver=${final##*/v}
  fi
  base="https://github.com/$repo/releases/download/v$ver"
fi

if [ -x "$dir/tally" ] && [ "$("$dir/tally" --version 2>/dev/null || true)" = "tally $ver" ]; then
  echo "tally $ver already installed in $dir"
  exit 0
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL "$base/SHA256SUMS" -o "$tmp/SHA256SUMS" || die "cannot download SHA256SUMS from $base"
# the archive is `-unsigned` when the release was built without signing secrets
line=$(grep -E "  tally-$ver-$target(-unsigned)?\.tar\.gz\$" "$tmp/SHA256SUMS" | head -1) || true
[ -n "$line" ] || die "no archive for $target in SHA256SUMS (version $ver)"
want=${line%% *}
name=${line##* }
curl -fsSL "$base/$name" -o "$tmp/$name" || die "cannot download $name"
if command -v sha256sum >/dev/null 2>&1; then got=$(sha256sum "$tmp/$name" | cut -d' ' -f1); else got=$(shasum -a 256 "$tmp/$name" | cut -d' ' -f1); fi
[ "$got" = "$want" ] || die "checksum mismatch for $name (expected $want, got $got); nothing installed"

tar -xzf "$tmp/$name" -C "$tmp" tally
mkdir -p "$dir"
was_running=0
if [ -x "$dir/tally" ] && "$dir/tally" service status --quiet >/dev/null 2>&1; then
  was_running=1
  # `tally stop`/`tally start`, not `tally service ...`: the daemon may have been started by a hook with no service
  # installed, and `service start` would fail (or leave the `paused` marker) in that case.
  "$dir/tally" stop >/dev/null 2>&1 || true
fi
mv -f "$tmp/tally" "$dir/tally.new" && chmod 755 "$dir/tally.new" && mv -f "$dir/tally.new" "$dir/tally"
if [ "$was_running" = 1 ]; then "$dir/tally" start >/dev/null 2>&1 || true; fi

echo "installed tally $ver to $dir"
case ":$PATH:" in *":$dir:"*) ;; *) echo "add $dir to your PATH (this script does not edit shell profiles)" ;; esac
echo 'Next: run "tally setup", then "tally service install" to start it at every login'
