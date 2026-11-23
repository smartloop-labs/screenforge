// What `voice` (narration read from a script) and `voice-enhance` (a recorded voice-over cleaned
// up or re-voiced) share: the app's ElevenLabs voice ids, and the gate and charge every
// ElevenLabs request goes through. Paid in agent credits (credits.ts): a request needs Pro (or
// staff, or trial credits) and a positive balance, its cost is taken after ElevenLabs answers,
// and it counts towards the agent's daily caps (agent_usage), the backstop on everything we pay.
import { createClient, SupabaseClient } from "https://esm.sh/@supabase/supabase-js@2.57.2";
import { MONTHLY_CREDITS, Payer, payerFor, remaining, summary, TRIAL_CREDITS } from "./credits.ts";

export const cors = { "Access-Control-Allow-Origin": "*", "Access-Control-Allow-Headers": "authorization, x-client-info, apikey, content-type" };
export const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { ...cors, "Content-Type": "application/json" } });
export const fail = (error: string, status: number, extra: Record<string, unknown> = {}) => json({ error, ...extra }, status);

/** An ElevenLabs voice the app offers. `id` is the app's (`el_brian`), `elevenlabs` the voice
 * at ElevenLabs (never sent to the app). `language` (ISO 639-1, or 639-3 where there is no
 * 639-1 code) marks a native speaker of that language, offered only when the project's
 * narration is in it; no language = offered for every language (v4 reads them all, in the
 * language's own accent). */
export type CatalogVoice = { id: string; elevenlabs: string; name: string; female: boolean; style: string; language?: string };

/** The voices the app offers, in order (the first is the default). The app fetches this list
 * (`voice` with action "list"), so voices are added here without an app release; its own copy
 * (sf_tts::cloud::BUILT_IN) is the fallback until the first fetch and must keep these eight.
 *
 * Native narrators come from the Voice Library: pick them with backend/find-narrators.sh. Take
 * only voices verified for the model (verified_languages), with a notice period of 30 days or
 * more (an owner who stops sharing a voice takes it away after that) and no custom rate
 * (fiat_rate), which would cost more than the credits charged. Library voices need no voice
 * slot. Ids are `el_<language>_<name>` so they never clash with the eight. */
export const CATALOG: CatalogVoice[] = [
  { id: "el_eric", elevenlabs: "cjVigY5qzO86Huf0OWal", name: "Eric", female: false, style: "smooth, trustworthy" },
  { id: "el_brian", elevenlabs: "nPczCjzI2devNBz1zQrb", name: "Brian", female: false, style: "deep, resonant narrator" },
  { id: "el_george", elevenlabs: "JBFqnCBsd6RMkjVDRZzb", name: "George", female: false, style: "warm British storyteller" },
  { id: "el_daniel", elevenlabs: "onwK4e9ZLuTAKqWW03F9", name: "Daniel", female: false, style: "steady British broadcaster" },
  { id: "el_sarah", elevenlabs: "EXAVITQu4vr4xnSDxMaL", name: "Sarah", female: true, style: "mature, confident" },
  { id: "el_matilda", elevenlabs: "XrExE9yKIg1WjnnlVkGX", name: "Matilda", female: true, style: "knowledgeable, professional" },
  { id: "el_alice", elevenlabs: "Xb7hH8MSUJpSbSDYk0k2", name: "Alice", female: true, style: "clear British educator" },
  { id: "el_jessica", elevenlabs: "cgSgspJ2msm6clMCkdW9", name: "Jessica", female: true, style: "playful, bright" },
  // native narrators (Voice Library, verified for eleven_v4, 730-day notice, no custom rate);
  // the first of a language is its default
  { id: "el_hi_raghav", elevenlabs: "4BoDaQ6aygOP6fpsUmJe", name: "Raghav", female: false, style: "calm, confident explainer", language: "hi" },
  { id: "el_hi_viraj", elevenlabs: "FmBhnvP58BK0vz65OOj7", name: "Viraj", female: false, style: "energetic, clear narrator", language: "hi" },
  { id: "el_hi_monika", elevenlabs: "1qEiC6qsybMkmnNdVMbK", name: "Monika", female: true, style: "calm, natural", language: "hi" },
  { id: "el_hi_kanika", elevenlabs: "H6QPv2pQZDcGqLwDTIJQ", name: "Kanika", female: true, style: "soft, smooth professional", language: "hi" },
  { id: "el_es_alberto", elevenlabs: "l1zE9xgNpUTaQCZzpNJa", name: "Alberto", female: false, style: "serious Latin American narrator", language: "es" },
  { id: "el_es_aaron", elevenlabs: "t9LRTh3y1ioN00e9wsNh", name: "Aarón", female: false, style: "clear, professional Castilian", language: "es" },
  { id: "el_es_tatiana", elevenlabs: "2rigMbVWLdqtBSCahJFX", name: "Tatiana", female: true, style: "calm, neutral Latin American", language: "es" },
  { id: "el_es_jhenny", elevenlabs: "2Lb1en5ujrODDIqmp7F3", name: "Jhenny", female: true, style: "soft, educational", language: "es" },
  { id: "el_fr_nicolas", elevenlabs: "aQROLel5sQbj1vuIVi6B", name: "Nicolas", female: false, style: "Parisian narrator", language: "fr" },
  { id: "el_fr_guillaume", elevenlabs: "Qrl71rx6Yg8RvyPYRGCQ", name: "Guillaume", female: false, style: "warm educational voice-over", language: "fr" },
  { id: "el_fr_jeanne", elevenlabs: "txtf1EDouKke753vN8SL", name: "Jeanne", female: true, style: "young Parisian narrator", language: "fr" },
  { id: "el_fr_victoria", elevenlabs: "O31r762Gb3WFygrEOGh0", name: "Victoria", female: true, style: "fluid content creator", language: "fr" },
  { id: "el_de_johannes", elevenlabs: "r8MyP4qUsq5WFFSkPdfV", name: "Johannes", female: false, style: "clear, neutral narrator", language: "de" },
  { id: "el_de_otto", elevenlabs: "FTNCalFNG5bRnkkaP5Ug", name: "Otto", female: false, style: "calm, casual", language: "de" },
  { id: "el_de_leonie", elevenlabs: "uvysWDLbKpA4XvpD3GI6", name: "Leonie", female: true, style: "clear, engaging", language: "de" },
  { id: "el_de_annika", elevenlabs: "ViKqgJNeCiWZlYgHiAOO", name: "Annika", female: true, style: "calm, confident", language: "de" },
  { id: "el_it_marco", elevenlabs: "W71zT1VwIFFx3mMGH2uZ", name: "Marco", female: false, style: "warm, polished narrator", language: "it" },
  { id: "el_it_thomas", elevenlabs: "CITWdMEsnRduEUkNWXQv", name: "Thomas", female: false, style: "energetic documentary voice", language: "it" },
  { id: "el_it_violetta", elevenlabs: "gfKKsLN1k0oYYN9n2dXX", name: "Violetta", female: true, style: "bright, natural", language: "it" },
  { id: "el_it_manuela", elevenlabs: "oVJbgLwL0s5pk9e2U6QH", name: "Manuela", female: true, style: "warm, versatile narrator", language: "it" },
  { id: "el_pt_sandro", elevenlabs: "qPfM2laM0pRL4rrZtBGl", name: "Sandro", female: false, style: "clear Brazilian narrator", language: "pt" },
  { id: "el_pt_matheus", elevenlabs: "oArP4WehPe3qjqvCwHNo", name: "Matheus", female: false, style: "calm, confident Brazilian", language: "pt" },
  { id: "el_pt_amanda", elevenlabs: "oi8rgjIfLgJRsQ6rbZh3", name: "Amanda", female: true, style: "warm, neutral Brazilian", language: "pt" },
  { id: "el_pt_ana", elevenlabs: "MZxV5lN3cv7hi1376O0m", name: "Ana", female: true, style: "smooth, engaging Brazilian", language: "pt" },
  { id: "el_nl_ido", elevenlabs: "dLPO5AsXc3FZDbTh1IKa", name: "Ido", female: false, style: "friendly e-learning narrator", language: "nl" },
  { id: "el_nl_arjen", elevenlabs: "62klqbsYqbynbr66ypRt", name: "Arjen", female: false, style: "calm, trustworthy", language: "nl" },
  { id: "el_nl_petra", elevenlabs: "ANHrhmaFeVN0QJaa0PhL", name: "Petra", female: true, style: "energetic Flemish", language: "nl" },
  { id: "el_pl_pawel", elevenlabs: "zzBTsLBFM6AOJtkr1e9b", name: "Paweł", female: false, style: "clear, optimistic narrator", language: "pl" },
  { id: "el_pl_adam", elevenlabs: "B9cNwbQXN3s6l3nU6fqz", name: "Adam", female: false, style: "neutral, youthful narrator", language: "pl" },
  { id: "el_pl_joanna", elevenlabs: "H3IcxEgdFxIEJAqMc0Bc", name: "Joanna", female: true, style: "clear, welcoming narrator", language: "pl" },
  { id: "el_pl_magdalena", elevenlabs: "fEfGdiGJK4l3imI70mtC", name: "Magdalena", female: true, style: "warm, lively", language: "pl" },
  { id: "el_tr_aliburak", elevenlabs: "5ANiIbDLbNMQ65tBPPDe", name: "Ali Burak", female: false, style: "calm, explanatory", language: "tr" },
  { id: "el_tr_ahmet", elevenlabs: "ZsYcqahfiS2dy4J6XYC5", name: "Ahmet", female: false, style: "steady, confident", language: "tr" },
  { id: "el_tr_derya", elevenlabs: "NNn9dv8zq2kUo7d3JSGG", name: "Derya", female: true, style: "dynamic, friendly narrator", language: "tr" },
  { id: "el_tr_deniz", elevenlabs: "KAGDtM2gzDrjWlUp2KNe", name: "Deniz", female: true, style: "soft, friendly narrator", language: "tr" },
  { id: "el_ar_anas", elevenlabs: "R6nda3uM038xEEKi7GFl", name: "Anas", female: false, style: "calm Modern Standard", language: "ar" },
  { id: "el_ar_ali", elevenlabs: "MI88rOZjXbH22N8KHXUo", name: "Ali", female: false, style: "calm Saudi narrator", language: "ar" },
  { id: "el_ar_ghizlane", elevenlabs: "u0TsaWvt0v8migutHM3M", name: "Ghizlane", female: true, style: "smooth Modern Standard", language: "ar" },
  { id: "el_ar_sana", elevenlabs: "mRdG9GYEjJmIzqbYTidv", name: "Sana", female: true, style: "calm, soft Modern Standard", language: "ar" },
  { id: "el_ja_konoha", elevenlabs: "T7yYq3WpB94yAuOXraRi", name: "Konoha", female: true, style: "professional explainer", language: "ja" },
  { id: "el_ja_otani", elevenlabs: "3JDquces8E8bkmvbh6Bc", name: "Otani", female: false, style: "clear, measured narrator", language: "ja" },
  { id: "el_ja_toshi", elevenlabs: "TzUI53GPXnGDRdeLAWZ4", name: "Toshi", female: false, style: "calm narrator", language: "ja" },
  { id: "el_ja_harune", elevenlabs: "c2XJrw7TvNGtOc6r0ijG", name: "Harune", female: true, style: "calm, neutral narration", language: "ja" },
  { id: "el_ko_hyunbin", elevenlabs: "s07IwTCOrCDCaETjUVjx", name: "Hyunbin", female: false, style: "clear corporate narrator", language: "ko" },
  { id: "el_ko_gale", elevenlabs: "K3qo7ugXmpT87FDhLBbN", name: "Gale", female: false, style: "educational videos", language: "ko" },
  { id: "el_ko_jiyoung", elevenlabs: "AW5wrnG1jVizOYY7R1Oo", name: "Jiyoung", female: true, style: "warm, clear", language: "ko" },
  { id: "el_ko_annakim", elevenlabs: "uyVNoMrnUku1dZyVEXwD", name: "Anna Kim", female: true, style: "calm, clear narrator", language: "ko" },
  { id: "el_zh_adrian", elevenlabs: "agczkAUlHLowaNnL72Cc", name: "Adrian", female: false, style: "modern Mandarin narration", language: "zh" },
  { id: "el_zh_evan", elevenlabs: "MI36FIkp9wRP7cpWKPTl", name: "Evan", female: false, style: "warm, trustworthy Mandarin", language: "zh" },
  { id: "el_zh_annasu", elevenlabs: "r6qgCCGI7RWKXCagm158", name: "Anna Su", female: true, style: "clear Taiwanese Mandarin", language: "zh" },
  { id: "el_zh_susan", elevenlabs: "kAIqZ7fZv234ClKXwzDx", name: "Susan", female: true, style: "clear, calm storyteller", language: "zh" },
];

/** The app's ids → ElevenLabs' voices. */
export const VOICES: Record<string, string> = Object.fromEntries(CATALOG.map((v) => [v.id, v.elevenlabs]));

/** The catalog as the app sees it (without ElevenLabs' ids). */
export const publicCatalog = () => CATALOG.map(({ elevenlabs: _, ...v }) => v);

/** A language code ElevenLabs' `language_code` takes (ISO 639-1, or 639-3). */
export const isLanguage = (code: unknown): code is string => typeof code === "string" && /^[a-z]{2,3}$/.test(code);

/** The app's id for the user's own voice ("My voice"), resolved per license by [`voiceIdFor`]. */
export const MY_VOICE = "my_voice";

/** A voice the app may ask for: one of VOICES, or the user's own. */
export const isVoice = (voice: unknown): voice is string => typeof voice === "string" && (voice in VOICES || voice === MY_VOICE);

/** ElevenLabs' id for `voice` (see [`isVoice`]) as `payer` may use it; or the response refusing it. */
export async function voiceIdFor(db: SupabaseClient, payer: Payer, voice: string): Promise<string | Response> {
  if (voice !== MY_VOICE) return VOICES[voice];
  if (!payer.email) return fail("My voice needs ScreenForge Pro activated on this computer.", 402, { code: "pro_required" });
  const email = payer.email.toLowerCase();
  const { data } = await db.from("voice_clones").select("voice_id").eq("email", email).maybeSingle();
  if (!data) return fail("You have not made your voice yet. Create it under Narration → My voice.", 404, { code: "no_clone" });
  // put away by the cleanup (my-voice): the app makes it again from its recordings and retries
  if (!data.voice_id) return fail("Your voice was put away after a while unused. Make it again under Narration → My voice.", 409, { code: "voice_parked" });
  // how recently it was used decides which clones the cleanup puts away
  const { error } = await db.from("voice_clones").update({ last_used_at: new Date().toISOString() }).eq("email", email);
  if (error) console.error("voice_clones last_used_at", error.message);
  return data.voice_id;
}

const DEVICE_DAILY_USD = Number(Deno.env.get("AGENT_DEVICE_DAILY_USD") ?? "0");
const TOTAL_DAILY_USD = Number(Deno.env.get("AGENT_TOTAL_DAILY_USD") ?? "20");

export const isDevice = (device: unknown): device is string => typeof device === "string" && /^[0-9a-f]{64}$/.test(device);

const outOfCredits = (p: Payer) =>
  p.pro
    ? "This month's credits are used up. Top up to keep using ElevenLabs, or pick a voice that runs on this computer."
    : `You're out of credits. Top up, or upgrade to Pro for ${MONTHLY_CREDITS.toLocaleString("en-US")} credits a month.`;

/** The database, and who pays for `device`'s request; or the response refusing it. */
export async function gate(device: string): Promise<{ db: SupabaseClient; payer: Payer } | Response> {
  const db = createClient(Deno.env.get("SUPABASE_URL") ?? "", Deno.env.get("SUPABASE_SERVICE_ROLE_KEY") ?? "", { auth: { persistSession: false } });
  const { data: today, error: usageError } = await db.rpc("agent_usage_today", { p_device: device }).maybeSingle();
  if (usageError) return fail(usageError.message, 500);
  // deno-lint-ignore no-explicit-any
  const spent = today as any;
  if (DEVICE_DAILY_USD > 0 && Number(spent?.device_usd ?? 0) >= DEVICE_DAILY_USD) return fail("Today's allowance on this computer is used up. It starts again tomorrow.", 402);
  if (TOTAL_DAILY_USD > 0 && Number(spent?.total_usd ?? 0) >= TOTAL_DAILY_USD) return fail("ElevenLabs is very busy today. Please try again tomorrow.", 503);
  const payer = await payerFor(db, device);
  if (!payer.allowed) {
    return fail(`ElevenLabs is part of Pro: US$20/month for ${MONTHLY_CREDITS.toLocaleString("en-US")} credits a month, shared with the agent. Already on Pro? Activate it with your email in Export.`, 402, { code: "pro_required", sf_credits: summary(payer) });
  }
  if (remaining(payer) <= 0) return fail(outOfCredits(payer), 402, { code: "no_credits", sf_credits: summary(payer) });
  return { db, payer };
}

/** Take `usd` (at 1 credit = US$0.01) from `payer` for `device`; `ref` keeps a retried
 * request from being charged twice. Returns the credits charged; `payer` is updated. */
export async function charge(db: SupabaseClient, device: string, payer: Payer, usd: number, ref: string): Promise<number> {
  const { error } = await db.rpc("agent_usage_add", { p_device: device, p_cost: usd });
  if (error) console.error("agent_usage_add", error.message);
  const credits = usd * 100;
  if (payer.email) {
    const { data, error: spendError } = await db.rpc("credits_spend", { p_email: payer.email, p_credits: credits, p_ref: ref }).maybeSingle();
    if (spendError) console.error("credits_spend", spendError.message);
    // deno-lint-ignore no-explicit-any
    const b = data as any;
    if (b) Object.assign(payer, { monthly: Number(b.monthly), topup: Number(b.topup) });
  } else {
    const { data, error: trialError } = await db.rpc("credits_trial_spend", { p_device: device, p_credits: credits });
    if (trialError) console.error("credits_trial_spend", trialError.message);
    payer.trial = Math.max(TRIAL_CREDITS - Number(data ?? 0), 0);
  }
  return credits;
}

/** An ElevenLabs reply that is not 2xx, as the app's error. */
export async function elevenLabsFailed(r: Response | null, what: string): Promise<Response> {
  const detail = r ? await r.text().catch(() => "") : "";
  console.error("elevenlabs", r?.status, detail.slice(0, 500));
  if (r?.status === 429) return fail("Too many requests right now. Wait a moment and try again.", 429);
  // ElevenLabs' own reason ({ detail: { status } }), so a failure that keeps happening says why
  let reason = "";
  try {
    const d = JSON.parse(detail)?.detail;
    reason = typeof d?.code === "string" ? d.code : typeof d?.status === "string" ? d.status : typeof d === "string" ? d.slice(0, 80) : "";
  } catch {
    // not JSON
  }
  // ElevenLabs flagged the voice (a clone it wants verified): only made-again recordings help
  if (r && /voice_access_denied/.test(reason)) return fail("ElevenLabs has blocked this voice: it flagged the clone for verification. Make your voice again under Narration → My voice with other recordings of only you speaking, or pick another voice.", 403, { code: "voice_blocked" });
  if (r && /voice_not_found/.test(reason)) return fail("ElevenLabs no longer has this voice. Make your voice again under Narration → My voice.", 404, { code: "no_clone" });
  if (r && /quota_exceeded|insufficient|payment|subscription/i.test(reason)) return fail(`ElevenLabs could not ${what}: ScreenForge's voice service is out of capacity. Please try again later. (${reason})`, 503);
  const code = r ? ` (ElevenLabs ${r.status}${reason ? `: ${reason}` : ""})` : " (ElevenLabs did not answer)";
  return fail(`ElevenLabs could not ${what} right now. Try again in a moment.${code}`, 502);
}

/** Seconds of 16-bit mono PCM in a WAV file, or an error for anything else. */
export function wavSeconds(b: Uint8Array): number | string {
  const v = new DataView(b.buffer, b.byteOffset, b.byteLength);
  const tag = (at: number) => String.fromCharCode(...b.subarray(at, at + 4));
  if (b.length < 44 || tag(0) !== "RIFF" || tag(8) !== "WAVE") return "audio must be a WAV file";
  let rate = 0;
  for (let at = 12; at + 8 <= b.length;) {
    const id = tag(at);
    const size = v.getUint32(at + 4, true);
    if (id === "fmt ") {
      if (v.getUint16(at + 8, true) !== 1 || v.getUint16(at + 10, true) !== 1 || v.getUint16(at + 22, true) !== 16) return "audio must be 16-bit PCM mono";
      rate = v.getUint32(at + 12, true);
    } else if (id === "data") {
      if (!rate) return "audio has no format";
      return Math.min(size, b.length - at - 8) / (rate * 2);
    }
    at += 8 + size + (size % 2);
  }
  return "audio has no samples";
}

export { summary };
