#!/usr/bin/env bash
# One-off: copy the newest DL_KEEP (default 3) GitHub releases and the models-v1 release to the
# download bucket, so the site, the changelog's download buttons and the app's model downloads
# work once the repo is private. Run before the repo goes private; safe to run again.
#
#   R2_ACCOUNT_ID=... R2_ACCESS_KEY_ID=... R2_SECRET_ACCESS_KEY=... scripts/backfill-dl.sh
#
# Needs gh (logged in) and the aws CLI. The newest release also becomes latest/ and latest.json.
set -euo pipefail

REPO=${REPO:-mehfuzh/screenforge}
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# models-v1 is marked a pre-release, so name it explicitly
tags="models-v1 $(gh release list -R "$REPO" -L "${DL_KEEP:-3}" --exclude-drafts --exclude-pre-releases --json tagName --jq '.[].tagName')"
newest=$(gh release view -R "$REPO" --json tagName --jq .tagName)

for tag in $tags; do
  d="$WORK/$tag"
  mkdir -p "$d"
  gh release download "$tag" -R "$REPO" -D "$d"
  if [ "$tag" = models-v1 ]; then
    # the models keep their path: dl.screenforge.studio/models-v1/<file>
    "$HERE/publish-dl.sh" models-v1 "$d" --no-latest
  elif [ "$tag" = "$newest" ]; then
    "$HERE/publish-dl.sh" "$tag" "$d"
  else
    "$HERE/publish-dl.sh" "$tag" "$d" --no-latest
  fi
  rm -rf "$d"
done
