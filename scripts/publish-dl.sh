#!/usr/bin/env bash
# Upload a release's files to the download bucket served at https://dl.screenforge.studio
# (Cloudflare R2, S3 API), then point latest.json at it.
#
#   scripts/publish-dl.sh v0.1.64 dir/with/files [--no-latest]
#
# Layout of the bucket:
#   v0.1.64/<file>          every file of the release, as built; only the newest DL_KEEP
#                           (default 3) versions are kept, older ones are deleted
#   latest/<stable name>    ScreenForge.dmg, ScreenForge-macOS.zip, ... (copies of the newest)
#   latest.json             the newest release, in the shape of GitHub's releases/latest API
#                           (tag_name, html_url, published_at, assets[name,size,digest,
#                           browser_download_url]); read by the app's updater and download.html
#   models-v1/<file>        Whisper and Kokoro models (scripts/backfill-dl.sh)
#
# Needs the aws CLI and R2_ACCOUNT_ID, R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY; R2_BUCKET
# defaults to screenforge-dl. --no-latest uploads the versioned files only (backfilling old
# releases) and prunes nothing. scripts/gen-releases.py offers per-version downloads for the same
# DL_KEEP newest releases; keep the two in step.
set -euo pipefail

TAG=${1:?usage: publish-dl.sh <tag> <dir> [--no-latest]}
DIR=${2:?usage: publish-dl.sh <tag> <dir> [--no-latest]}
LATEST=1
[ "${3:-}" = "--no-latest" ] && LATEST=0
BUCKET=${R2_BUCKET:-screenforge-dl}
BASE=${DL_BASE:-https://dl.screenforge.studio}
KEEP=${DL_KEEP:-3}
: "${R2_ACCOUNT_ID:?}" "${R2_ACCESS_KEY_ID:?}" "${R2_SECRET_ACCESS_KEY:?}"
export AWS_ACCESS_KEY_ID=$R2_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY=$R2_SECRET_ACCESS_KEY AWS_DEFAULT_REGION=auto
S3=(aws s3 --endpoint-url "https://$R2_ACCOUNT_ID.r2.cloudflarestorage.com")

# versioned files never change: cache them for a year; latest/* and latest.json move on every
# release, so the edge must revalidate them
FOREVER="public, max-age=31536000, immutable"
FRESH="public, max-age=60, must-revalidate"

shopt -s nullglob
files=("$DIR"/*)
[ ${#files[@]} -gt 0 ] || { echo "no files in $DIR" >&2; exit 1; }

for f in "${files[@]}"; do
  "${S3[@]}" cp "$f" "s3://$BUCKET/$TAG/$(basename "$f")" --cache-control "$FOREVER" --only-show-errors
  echo "uploaded $TAG/$(basename "$f")"
done

[ "$LATEST" = 1 ] || exit 0

# the stable names (no version in them) are the ones latest/ and the updater use
stable='^(ScreenForge\.dmg|ScreenForge-macOS\.zip|ScreenForge-linux-x86_64\.tar\.gz|screenforge_amd64\.deb|ScreenForge-windows-x86_64(-setup\.exe|\.zip))$'
assets=()
for f in "${files[@]}"; do
  name=$(basename "$f")
  if [[ $name =~ $stable ]]; then
    "${S3[@]}" cp "$f" "s3://$BUCKET/latest/$name" --cache-control "$FRESH" --only-show-errors
    echo "uploaded latest/$name"
  fi
  size=$(wc -c < "$f" | tr -d ' ')
  sum=$( (command -v sha256sum >/dev/null && sha256sum "$f" || shasum -a 256 "$f") | cut -d' ' -f1)
  assets+=("$(printf '{"name":"%s","size":%s,"digest":"sha256:%s","browser_download_url":"%s/%s/%s"}' "$name" "$size" "$sum" "$BASE" "$TAG" "$name")")
done

json=$(mktemp)
{
  printf '{"tag_name":"%s","name":"ScreenForge %s",' "$TAG" "$TAG"
  printf '"html_url":"https://screenforge.studio/changelog/%s/",' "$TAG"
  printf '"published_at":"%s",' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  printf '"assets":[%s]}\n' "$(IFS=,; echo "${assets[*]}")"
} > "$json"
python3 -m json.tool "$json" > /dev/null   # refuse to publish a broken file
"${S3[@]}" cp "$json" "s3://$BUCKET/latest.json" --content-type application/json --cache-control "$FRESH" --only-show-errors
rm -f "$json"
echo "latest.json -> $TAG"

# keep the newest $KEEP version folders (vX.Y.Z/ only; latest/ and models-v1/ are never touched)
old=$("${S3[@]}" ls "s3://$BUCKET/" | awk '$1 == "PRE" { sub("/$", "", $2); print $2 }' \
  | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | sort -V -r | tail -n +$((KEEP + 1)) || true)
for v in $old; do
  "${S3[@]}" rm "s3://$BUCKET/$v/" --recursive --only-show-errors
  echo "pruned $v/"
done
