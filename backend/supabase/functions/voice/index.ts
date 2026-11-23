// ElevenLabs voices for Narration and the agent, so the ElevenLabs key never ships in the app: it
// is the ELEVENLABS_API_KEY secret here.
// { device, voice, text, speed, language? } → { audio: base64 16-bit PCM mono at 24 kHz, alignment, sf_credits }
//   `voice` is one of VOICES (the app's ids, "el_brian" …; the ElevenLabs voice ids stay in
//   _shared/elevenlabs.ts so they can change without an app release), or "my_voice", the
//   user's own clone (my-voice); `language` is the narration's language (ISO 639-1, "es"),
//   passed to ElevenLabs as `language_code` (left out: ElevenLabs tells from the text);
//   `alignment` is ElevenLabs' character timing, which the app turns into word timing for
//   captions.
// { action: "list" } → { voices: [{ id, name, female, style, language? }] }
//   the voices the app offers (CATALOG), free and open to everyone: the panel lists them
//   before it knows the plan.
//
// Paid in agent credits like the agent (see _shared/elevenlabs.ts): it costs what ElevenLabs
// charges for its characters at list price, 1 credit = US$0.01, taken after the audio is made.
import { serve } from "https://deno.land/std@0.190.0/http/server.ts";
import { charge, cors, elevenLabsFailed, fail, gate, isDevice, isLanguage, isVoice, json, publicCatalog, summary, voiceIdFor } from "../_shared/elevenlabs.ts";

const MODEL = Deno.env.get("ELEVENLABS_MODEL") ?? "eleven_v4";
/** ElevenLabs' list price for MODEL, US$ per 1,000 characters (v4: 0.08, v4 Turbo: 0.04). */
const USD_PER_1K_CHARS = Number(Deno.env.get("ELEVENLABS_USD_PER_1K_CHARS") ?? "0.08");
/** Longest text read at once (one narration take). */
const MAX_CHARS = 5000;

/** The request carries a service role token. The gateway has verified the token's signature
 * before the function runs (this function is deployed with JWT verification), so its `role`
 * claim can be trusted; the env key is compared too, for a token in the new key format. */
function isServiceRole(req: Request): boolean {
  const token = (req.headers.get("authorization") ?? "").replace(/^Bearer\s+/i, "");
  if (!token) return false;
  const env = Deno.env.get("SUPABASE_SERVICE_ROLE_KEY");
  if (env && token === env) return true;
  try {
    const part = token.split(".")[1].replace(/-/g, "+").replace(/_/g, "/");
    return JSON.parse(atob(part + "=".repeat((4 - (part.length % 4)) % 4))).role === "service_role";
  } catch {
    return false;
  }
}

/** Voice Library candidates for native narrators in `language` (backend/find-narrators.sh),
 * searched with the key held here. Staff only: the caller must send the service role key. */
async function library(req: Request, apiKey: string, language: unknown): Promise<Response> {
  if (!isServiceRole(req)) return fail("not allowed", 403);
  if (!isLanguage(language)) return fail("language required", 400);
  const q = new URLSearchParams({ language, use_cases: "narrative_story", min_notice_period_days: "30", sort: "usage_character_count_1y", page_size: "100" });
  const r = await fetch(`https://api.elevenlabs.io/v1/shared-voices?${q}`, { headers: { "xi-api-key": apiKey } }).catch(() => null);
  if (!r || !r.ok) return elevenLabsFailed(r, "search the Voice Library");
  // deno-lint-ignore no-explicit-any
  const { voices = [] } = (await r.json()) as { voices: any[] };
  const fit = voices
    .filter((v) => v.fiat_rate == null && v.free_users_allowed !== false)
    // deno-lint-ignore no-explicit-any
    .filter((v) => (v.verified_languages ?? []).some((l: any) => l.language === language && l.model_id === MODEL))
    .slice(0, 20)
    .map((v) => ({ voice_id: v.voice_id, name: v.name, gender: v.gender, age: v.age, accent: v.accent, notice_days: v.notice_period, usage_1y: v.usage_character_count_1y, description: v.description, preview_url: v.preview_url }));
  return json({ model: MODEL, voices: fit });
}

serve(async (req) => {
  if (req.method === "OPTIONS") return new Response(null, { headers: cors });
  const { action, device, voice, text, speed, language } = await req.json().catch(() => ({}));
  if (action === "list") return json({ voices: publicCatalog() });
  const apiKey = Deno.env.get("ELEVENLABS_API_KEY");
  if (!apiKey) return fail("ElevenLabs voices are not configured on the server yet.", 503);
  if (action === "library") return library(req, apiKey, language);
  if (!isDevice(device)) return fail("device required", 400);
  if (!isVoice(voice)) return fail(`unknown voice ${voice}`, 400);
  if (language != null && !isLanguage(language)) return fail(`unknown language ${language}`, 400);
  const script = typeof text === "string" ? text.trim() : "";
  if (!script) return fail("text required", 400);
  if (script.length > MAX_CHARS) return fail(`A take can be at most ${MAX_CHARS.toLocaleString("en-US")} characters; split it into shorter ones.`, 400);

  const gated = await gate(device);
  if (gated instanceof Response) return gated;
  const { db, payer } = gated;
  const voiceId = await voiceIdFor(db, payer, voice);
  if (voiceId instanceof Response) return voiceId;

  // ElevenLabs takes 0.7–1.2
  const pace = Math.min(Math.max(Number(speed) || 1, 0.7), 1.2);
  const r = await fetch(`https://api.elevenlabs.io/v1/text-to-speech/${voiceId}/with-timestamps?output_format=pcm_24000`, {
    method: "POST",
    headers: { "xi-api-key": apiKey, "Content-Type": "application/json" },
    body: JSON.stringify({ text: script, model_id: MODEL, voice_settings: { speed: pace }, ...(language ? { language_code: language } : {}) }),
  }).catch((e) => {
    console.error("elevenlabs", e);
    return null;
  });
  if (!r || !r.ok) return elevenLabsFailed(r, "read it");
  const out = await r.json();

  const usd = (script.length / 1000) * USD_PER_1K_CHARS;
  const credits = await charge(db, device, payer, usd, `voice:${r.headers.get("request-id") ?? crypto.randomUUID()}`);
  return json({ audio: out.audio_base64, alignment: out.alignment ?? null, credits, sf_credits: summary(payer) });
});
