#!/usr/bin/env bash
# Native narrators for a language, from ElevenLabs' Voice Library, to add to CATALOG in
# supabase/functions/_shared/elevenlabs.ts. Lists the most used narration voices that are
# verified for the model, keep a notice period of at least 30 days and have no custom rate.
#
#   ELEVENLABS_API_KEY=… backend/find-narrators.sh es [eleven_v4]
#
# Filtering the library needs a logged-in key (any key of the account works; voices_read is
# enough). Listen to preview_url before adding one.
set -euo pipefail
lang="${1:?usage: find-narrators.sh <language code, e.g. es> [model]}"
model="${2:-eleven_v4}"
: "${ELEVENLABS_API_KEY:?set ELEVENLABS_API_KEY}"
command -v jq >/dev/null || { echo "needs jq" >&2; exit 1; }

curl -sf -G "https://api.elevenlabs.io/v1/shared-voices" \
  -H "xi-api-key: $ELEVENLABS_API_KEY" \
  --data-urlencode "language=$lang" \
  --data-urlencode "use_cases=narrative_story" \
  --data-urlencode "min_notice_period_days=30" \
  --data-urlencode "sort=usage_character_count_1y" \
  --data-urlencode "page_size=100" |
  jq -r --arg lang "$lang" --arg model "$model" '
    .voices[]
    | select(.fiat_rate == null and (.free_users_allowed // true))
    | select([.verified_languages[]? | select(.language == $lang and .model_id == $model)] | length > 0)
    | [.voice_id, .name, .gender, (.accent // "-"), (.notice_period // 0 | tostring) + "d", (.usage_character_count_1y // 0 | tostring), (.description // "" | gsub("\\s+"; " ") | .[0:60]), .preview_url]
    | @tsv' |
  head -n 20 |
  column -t -s $'\t'
