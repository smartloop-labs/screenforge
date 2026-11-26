#!/bin/sh
# Draw the picture each template shows in the app's Templates view: its first beat (headline
# and card) from a project made with the template's stand-in content, cropped to the card's
# shape. Run after changing a template's look: scripts/gen-template-previews.sh
# Needs a built sf (cargo build -p sf-cli) and ffmpeg.
set -e
cd "$(dirname "$0")/.."
SF=${SF:-target/debug/sf}
OUT=apps/studio/assets/templates
TMP=$(mktemp -d)
mkdir -p "$OUT"
for key in feature-short how-to-steps announcement top-three quick-tip did-you-know before-after testimonial sale; do
  project=$("$SF" template "$key" --folder "$TMP" | tail -1)
  "$SF" frame "$project" --at-ms 4300 --height 1920 -o "$TMP/$key.png" >/dev/null
  # the band from well above the headline (room over the text) into the card, in the view's 8:5
  # (800 px of the frame's height fits the headline and the card's main figure whole; the sides
  # are filled with the same band, blurred, so the picture keeps the view's proportions)
  ffmpeg -v error -y -i "$TMP/$key.png" -filter_complex "[0]crop=1080:800:0:205,split[a][b];[a]scale=528:330,boxblur=24:2[bg];[b]scale=-2:330:flags=lanczos[fg];[bg][fg]overlay=(W-w)/2:0" "$OUT/$key.png"
  echo "$OUT/$key.png"
done
echo "made in $TMP (safe to delete)"
