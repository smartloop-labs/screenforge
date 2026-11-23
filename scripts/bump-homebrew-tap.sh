#!/usr/bin/env bash
# Point zot24/homebrew-tap's ScreenForge cask at the build just published to
# https://dl.screenforge.studio. Called from .github/workflows/release.yml after
# scripts/publish-dl.sh.
#
#   scripts/bump-homebrew-tap.sh v0.1.65 dir/with/files
#
# The cask URLs are already versioned (dl.screenforge.studio/v#{version}/...), so
# this only bumps the version and the two sha256 lines. Needs HOMEBREW_TAP_DEPLOY_KEY,
# the private half of a write deploy key on zot24/homebrew-tap.
set -euo pipefail

TAG=${1:?usage: bump-homebrew-tap.sh <tag> <dir>}
DIR=${2:?usage: bump-homebrew-tap.sh <tag> <dir>}
VER=${TAG#v}
[[ "$VER" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "tag must be vX.Y.Z, got $TAG" >&2; exit 1; }
: "${HOMEBREW_TAP_DEPLOY_KEY:?HOMEBREW_TAP_DEPLOY_KEY is not set}"

MAC_FILE="$DIR/ScreenForge-$VER.dmg"
LINUX_FILE="$DIR/ScreenForge-$VER-linux-x86_64.tar.gz"
[ -f "$MAC_FILE" ] || { echo "missing $MAC_FILE" >&2; exit 1; }
[ -f "$LINUX_FILE" ] || { echo "missing $LINUX_FILE" >&2; exit 1; }

sha() { sha256sum "$1" | awk '{print $1}'; }
MAC=$(sha "$MAC_FILE")
LINUX=$(sha "$LINUX_FILE")

KEY=$(mktemp)
WORKDIR=$(mktemp -d)
trap 'rm -f "$KEY"; rm -rf "$WORKDIR"' EXIT
printf '%s\n' "$HOMEBREW_TAP_DEPLOY_KEY" > "$KEY"
chmod 600 "$KEY"

mkdir -p ~/.ssh
# GitHub's published ed25519 host key. Pinned so the first connection is not learned from the network.
grep -q 'AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl' ~/.ssh/known_hosts 2>/dev/null \
  || echo 'github.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl' >> ~/.ssh/known_hosts
export GIT_SSH_COMMAND="ssh -i $KEY -o IdentitiesOnly=yes -o StrictHostKeyChecking=yes"

git clone --depth 1 git@github.com:zot24/homebrew-tap.git "$WORKDIR"
CASK="$WORKDIR/Casks/screenforge.rb"
CUR=$(sed -nE 's/^  version "(.*)"/\1/p' "$CASK")
if [ "$VER" = "$CUR" ]; then
  echo "homebrew cask already at $VER"
  exit 0
fi

perl -pi -e "s/^  version \".*\"/  version \"$VER\"/; s/(arm: +)\"[0-9a-f]{64}\"/\${1}\"$MAC\"/; s/(x86_64_linux: +)\"[0-9a-f]{64}\"/\${1}\"$LINUX\"/" "$CASK"
grep -q "version \"$VER\"" "$CASK"
grep -q "$MAC" "$CASK"
grep -q "$LINUX" "$CASK"

git -C "$WORKDIR" diff
git -C "$WORKDIR" config user.name "github-actions[bot]"
git -C "$WORKDIR" config user.email "41898282+github-actions[bot]@users.noreply.github.com"
git -C "$WORKDIR" commit -am "screenforge $VER"
git -C "$WORKDIR" push origin HEAD:main
echo "homebrew-tap updated to screenforge $VER"
