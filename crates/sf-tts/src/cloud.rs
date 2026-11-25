//! ElevenLabs voices, read online through the backend's `voice` function (which holds the key
//! and keeps a daily allowance; the voices are free): the voices the editor offers, and turning a reply (16-bit PCM at
//! 24 kHz and ElevenLabs' character timing) into a [`Speech`] with its words for captions.

use std::sync::RwLock;

use anyhow::{bail, Result};

use crate::{Speech, SpokenWord, PEAK};

/// An ElevenLabs voice. Its id is the app's (`el_brian`); the backend maps it to ElevenLabs'.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloudVoice {
    pub id: &'static str,
    pub name: &'static str,
    pub female: bool,
    /// How it sounds, for the agent and tooltips.
    pub style: &'static str,
    /// A native speaker of this language (a [`Language`] code), offered only for narration in
    /// it; `None`: offered for every language.
    pub language: Option<&'static str>,
}

/// The voices until the backend's list is known ([`set_catalog`]), and offline: the backend's
/// CATALOG as it was at release. The first is the one picked when switching to ElevenLabs (and
/// the agent's choice for studio quality); the first [`CORE`] are the ones every list must keep.
pub const BUILT_IN: [CloudVoice; 59] = [
    CloudVoice { id: "el_eric", name: "Eric", female: false, style: "smooth, trustworthy", language: None },
    CloudVoice { id: "el_brian", name: "Brian", female: false, style: "deep, resonant narrator", language: None },
    CloudVoice { id: "el_george", name: "George", female: false, style: "warm British storyteller", language: None },
    CloudVoice { id: "el_daniel", name: "Daniel", female: false, style: "steady British broadcaster", language: None },
    CloudVoice { id: "el_sarah", name: "Sarah", female: true, style: "mature, confident", language: None },
    CloudVoice { id: "el_matilda", name: "Matilda", female: true, style: "knowledgeable, professional", language: None },
    CloudVoice { id: "el_alice", name: "Alice", female: true, style: "clear British educator", language: None },
    CloudVoice { id: "el_jessica", name: "Jessica", female: true, style: "playful, bright", language: None },
    CloudVoice { id: "el_hi_raghav", name: "Raghav", female: false, style: "calm, confident explainer", language: Some("hi") },
    CloudVoice { id: "el_hi_viraj", name: "Viraj", female: false, style: "energetic, clear narrator", language: Some("hi") },
    CloudVoice { id: "el_hi_monika", name: "Monika", female: true, style: "calm, natural", language: Some("hi") },
    CloudVoice { id: "el_hi_kanika", name: "Kanika", female: true, style: "soft, smooth professional", language: Some("hi") },
    CloudVoice { id: "el_es_alberto", name: "Alberto", female: false, style: "serious Latin American narrator", language: Some("es") },
    CloudVoice { id: "el_es_aaron", name: "Aarón", female: false, style: "clear, professional Castilian", language: Some("es") },
    CloudVoice { id: "el_es_tatiana", name: "Tatiana", female: true, style: "calm, neutral Latin American", language: Some("es") },
    CloudVoice { id: "el_es_jhenny", name: "Jhenny", female: true, style: "soft, educational", language: Some("es") },
    CloudVoice { id: "el_fr_nicolas", name: "Nicolas", female: false, style: "Parisian narrator", language: Some("fr") },
    CloudVoice { id: "el_fr_guillaume", name: "Guillaume", female: false, style: "warm educational voice-over", language: Some("fr") },
    CloudVoice { id: "el_fr_jeanne", name: "Jeanne", female: true, style: "young Parisian narrator", language: Some("fr") },
    CloudVoice { id: "el_fr_victoria", name: "Victoria", female: true, style: "fluid content creator", language: Some("fr") },
    CloudVoice { id: "el_de_johannes", name: "Johannes", female: false, style: "clear, neutral narrator", language: Some("de") },
    CloudVoice { id: "el_de_otto", name: "Otto", female: false, style: "calm, casual", language: Some("de") },
    CloudVoice { id: "el_de_leonie", name: "Leonie", female: true, style: "clear, engaging", language: Some("de") },
    CloudVoice { id: "el_de_annika", name: "Annika", female: true, style: "calm, confident", language: Some("de") },
    CloudVoice { id: "el_it_marco", name: "Marco", female: false, style: "warm, polished narrator", language: Some("it") },
    CloudVoice { id: "el_it_thomas", name: "Thomas", female: false, style: "energetic documentary voice", language: Some("it") },
    CloudVoice { id: "el_it_violetta", name: "Violetta", female: true, style: "bright, natural", language: Some("it") },
    CloudVoice { id: "el_it_manuela", name: "Manuela", female: true, style: "warm, versatile narrator", language: Some("it") },
    CloudVoice { id: "el_pt_sandro", name: "Sandro", female: false, style: "clear Brazilian narrator", language: Some("pt") },
    CloudVoice { id: "el_pt_matheus", name: "Matheus", female: false, style: "calm, confident Brazilian", language: Some("pt") },
    CloudVoice { id: "el_pt_amanda", name: "Amanda", female: true, style: "warm, neutral Brazilian", language: Some("pt") },
    CloudVoice { id: "el_pt_ana", name: "Ana", female: true, style: "smooth, engaging Brazilian", language: Some("pt") },
    CloudVoice { id: "el_nl_ido", name: "Ido", female: false, style: "friendly e-learning narrator", language: Some("nl") },
    CloudVoice { id: "el_nl_arjen", name: "Arjen", female: false, style: "calm, trustworthy", language: Some("nl") },
    CloudVoice { id: "el_nl_petra", name: "Petra", female: true, style: "energetic Flemish", language: Some("nl") },
    CloudVoice { id: "el_pl_pawel", name: "Paweł", female: false, style: "clear, optimistic narrator", language: Some("pl") },
    CloudVoice { id: "el_pl_adam", name: "Adam", female: false, style: "neutral, youthful narrator", language: Some("pl") },
    CloudVoice { id: "el_pl_joanna", name: "Joanna", female: true, style: "clear, welcoming narrator", language: Some("pl") },
    CloudVoice { id: "el_pl_magdalena", name: "Magdalena", female: true, style: "warm, lively", language: Some("pl") },
    CloudVoice { id: "el_tr_aliburak", name: "Ali Burak", female: false, style: "calm, explanatory", language: Some("tr") },
    CloudVoice { id: "el_tr_ahmet", name: "Ahmet", female: false, style: "steady, confident", language: Some("tr") },
    CloudVoice { id: "el_tr_derya", name: "Derya", female: true, style: "dynamic, friendly narrator", language: Some("tr") },
    CloudVoice { id: "el_tr_deniz", name: "Deniz", female: true, style: "soft, friendly narrator", language: Some("tr") },
    CloudVoice { id: "el_ar_anas", name: "Anas", female: false, style: "calm Modern Standard", language: Some("ar") },
    CloudVoice { id: "el_ar_ali", name: "Ali", female: false, style: "calm Saudi narrator", language: Some("ar") },
    CloudVoice { id: "el_ar_ghizlane", name: "Ghizlane", female: true, style: "smooth Modern Standard", language: Some("ar") },
    CloudVoice { id: "el_ar_sana", name: "Sana", female: true, style: "calm, soft Modern Standard", language: Some("ar") },
    CloudVoice { id: "el_ja_konoha", name: "Konoha", female: true, style: "professional explainer", language: Some("ja") },
    CloudVoice { id: "el_ja_otani", name: "Otani", female: false, style: "clear, measured narrator", language: Some("ja") },
    CloudVoice { id: "el_ja_toshi", name: "Toshi", female: false, style: "calm narrator", language: Some("ja") },
    CloudVoice { id: "el_ja_harune", name: "Harune", female: true, style: "calm, neutral narration", language: Some("ja") },
    CloudVoice { id: "el_ko_hyunbin", name: "Hyunbin", female: false, style: "clear corporate narrator", language: Some("ko") },
    CloudVoice { id: "el_ko_gale", name: "Gale", female: false, style: "educational videos", language: Some("ko") },
    CloudVoice { id: "el_ko_jiyoung", name: "Jiyoung", female: true, style: "warm, clear", language: Some("ko") },
    CloudVoice { id: "el_ko_annakim", name: "Anna Kim", female: true, style: "calm, clear narrator", language: Some("ko") },
    CloudVoice { id: "el_zh_adrian", name: "Adrian", female: false, style: "modern Mandarin narration", language: Some("zh") },
    CloudVoice { id: "el_zh_evan", name: "Evan", female: false, style: "warm, trustworthy Mandarin", language: Some("zh") },
    CloudVoice { id: "el_zh_annasu", name: "Anna Su", female: true, style: "clear Taiwanese Mandarin", language: Some("zh") },
    CloudVoice { id: "el_zh_susan", name: "Susan", female: true, style: "clear, calm storyteller", language: Some("zh") },
];

/// The voices for every language at the start of [`BUILT_IN`]: projects and the agent refer to
/// them, so a list from the backend without them is not taken.
pub const CORE: usize = 8;

/// The voices offered: the backend's list once known, else [`BUILT_IN`].
static CATALOG: RwLock<&'static [CloudVoice]> = RwLock::new(&BUILT_IN);

/// Every ElevenLabs voice offered, the default first.
pub fn cloud_voices() -> &'static [CloudVoice] {
    *CATALOG.read().unwrap_or_else(|e| e.into_inner())
}

/// An ElevenLabs voice by id: one offered, or the user's studio voice ([`STUDIO`]).
pub fn cloud_voice(id: &str) -> Option<&'static CloudVoice> {
    cloud_voices().iter().find(|v| v.id == id).or_else(|| studio().filter(|v| v.id == id))
}

/// The id of the user's own voice cloned by ElevenLabs ("studio quality"), made from one of
/// their voices on this computer. The backend knows it as `my_voice`.
pub const STUDIO: &str = "el_my_voice";

static STUDIO_VOICE: RwLock<Option<&'static CloudVoice>> = RwLock::new(None);

/// The user's studio voice, once they made one (named as they named it).
pub fn studio() -> Option<&'static CloudVoice> {
    *STUDIO_VOICE.read().unwrap_or_else(|e| e.into_inner())
}

/// The user has a studio voice called `name`, or none.
pub fn set_studio(name: Option<&str>) {
    if studio().map(|v| v.name) == name {
        return;
    }
    let voice = name.map(|n| &*Box::leak(Box::new(CloudVoice { id: STUDIO, name: Box::leak(n.to_string().into_boxed_str()), female: false, style: "your own voice, cloned by ElevenLabs", language: None })));
    if let Ok(mut s) = STUDIO_VOICE.write() {
        *s = voice;
    }
}

/// What the backend calls voice `id` (the studio voice is its `my_voice`).
pub fn backend_voice(id: &str) -> &str {
    if id == STUDIO {
        "my_voice"
    } else {
        id
    }
}

/// The default ElevenLabs voice for narration in `language` (`None`: English): its first
/// native speaker, else the first voice.
pub fn default_voice(language: Option<&str>) -> &'static CloudVoice {
    voices_for(language).into_iter().next().unwrap_or(&BUILT_IN[0])
}

/// The voices offered for narration in `language` (`None`: English): only its native
/// narrators. English's are the voices without a language (the original eight); a language
/// with no native narrator yet gets those too, as they read every language.
pub fn voices_for(language: Option<&str>) -> Vec<&'static CloudVoice> {
    let lang = language.unwrap_or("en");
    let all = cloud_voices();
    let natives: Vec<_> = all.iter().filter(|v| v.language == Some(lang)).collect();
    if natives.is_empty() || lang == "en" {
        natives.into_iter().chain(all.iter().filter(|v| v.language.is_none())).collect()
    } else {
        natives
    }
}

/// Take the backend's list (`{ voices: [{ id, name, female, style, language? }] }`, the reply
/// of `voice` with action "list"). Ignored, keeping the current list, unless it has the [`CORE`]
/// voices; returns whether it was taken.
pub fn set_catalog(reply: &serde_json::Value) -> bool {
    let Some(list) = reply.get("voices").and_then(|v| v.as_array()) else { return false };
    // the strings live for the rest of the run: the list is read once or twice a run
    let keep = |s: &str| -> &'static str { Box::leak(s.to_string().into_boxed_str()) };
    let mut voices: Vec<CloudVoice> = vec![];
    for v in list {
        let (Some(id), Some(name)) = (v.get("id").and_then(|x| x.as_str()), v.get("name").and_then(|x| x.as_str())) else { continue };
        if !id.starts_with("el_") || id == STUDIO || voices.iter().any(|c| c.id == id) {
            continue;
        }
        let language = v.get("language").and_then(|x| x.as_str()).filter(|l| !l.is_empty()).map(keep);
        let style = v.get("style").and_then(|x| x.as_str()).unwrap_or_default();
        voices.push(CloudVoice { id: keep(id), name: keep(name), female: v.get("female").and_then(|x| x.as_bool()).unwrap_or(false), style: keep(style), language });
    }
    if !BUILT_IN[..CORE].iter().all(|b| voices.iter().any(|v| v.id == b.id)) {
        return false;
    }
    if let Ok(mut c) = CATALOG.write() {
        *c = Box::leak(voices.into_boxed_slice());
    }
    true
}

/// A language ElevenLabs narration can be in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Language {
    /// ElevenLabs' `language_code` (ISO 639-1).
    pub code: &'static str,
    /// Its English name, for the panel (the app's fonts lack some scripts).
    pub name: &'static str,
    /// What a voice's preview says in it.
    pub preview: &'static str,
}

/// The languages offered for ElevenLabs narration, English first (the default). Eleven v4
/// reads 90+ languages; these are the major ones.
pub const LANGUAGES: [Language; 14] = [
    Language { code: "en", name: "English", preview: "Hi! This is how I sound reading your video's narration." },
    Language { code: "es", name: "Spanish", preview: "¡Hola! Así sueno al leer la narración de tu vídeo." },
    Language { code: "fr", name: "French", preview: "Bonjour ! Voici ma voix quand je lis la narration de votre vidéo." },
    Language { code: "de", name: "German", preview: "Hallo! So klinge ich, wenn ich den Sprechertext deines Videos lese." },
    Language { code: "it", name: "Italian", preview: "Ciao! Ecco come suono quando leggo la narrazione del tuo video." },
    Language { code: "pt", name: "Portuguese", preview: "Olá! É assim que eu soo lendo a narração do seu vídeo." },
    Language { code: "nl", name: "Dutch", preview: "Hallo! Zo klink ik als ik de voice-over van je video voorlees." },
    Language { code: "pl", name: "Polish", preview: "Cześć! Tak brzmię, czytając narrację do Twojego filmu." },
    Language { code: "tr", name: "Turkish", preview: "Merhaba! Videonuzun anlatımını okurken sesim böyle duyuluyor." },
    Language { code: "ar", name: "Arabic", preview: "مرحبًا! هكذا يبدو صوتي عندما أقرأ التعليق الصوتي لمقطع الفيديو الخاص بك." },
    Language { code: "hi", name: "Hindi", preview: "नमस्ते! आपके वीडियो का नैरेशन पढ़ते हुए मेरी आवाज़ ऐसी लगती है।" },
    Language { code: "ja", name: "Japanese", preview: "こんにちは！あなたの動画のナレーションを読むと、こんな声になります。" },
    Language { code: "ko", name: "Korean", preview: "안녕하세요! 영상 내레이션을 읽으면 이런 목소리가 나요." },
    Language { code: "zh", name: "Chinese", preview: "你好！这就是我朗读你视频旁白时的声音。" },
];

/// The language with `code`.
pub fn language(code: &str) -> Option<&'static Language> {
    LANGUAGES.iter().find(|l| l.code == code)
}

/// ElevenLabs' character timing of a reading (the `alignment` of a `with-timestamps` reply).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Alignment {
    pub characters: Vec<String>,
    pub starts: Vec<f64>,
    pub ends: Vec<f64>,
}

impl Alignment {
    pub fn from_json(v: &serde_json::Value) -> Alignment {
        let list = |k: &str| v.get(k).and_then(|a| a.as_array()).cloned().unwrap_or_default();
        Alignment {
            characters: list("characters").iter().map(|c| c.as_str().unwrap_or_default().to_string()).collect(),
            starts: list("character_start_times_seconds").iter().map(|t| t.as_f64().unwrap_or(0.0)).collect(),
            ends: list("character_end_times_seconds").iter().map(|t| t.as_f64().unwrap_or(0.0)).collect(),
        }
    }

    /// The words, each from its first character's start to its last one's end (ms).
    pub fn words(&self) -> Vec<SpokenWord> {
        let mut out: Vec<SpokenWord> = vec![];
        let mut current: Option<SpokenWord> = None;
        let n = self.characters.len().min(self.starts.len()).min(self.ends.len());
        for i in 0..n {
            let c = &self.characters[i];
            if c.trim().is_empty() {
                out.extend(current.take());
                continue;
            }
            let (s, e) = (self.starts[i] * 1000.0, self.ends[i] * 1000.0);
            // Chinese and Japanese have no spaces: each character is a word, and their
            // punctuation stays with the one before it
            let first = c.chars().next().unwrap_or(' ');
            if is_cjk_punctuation(first) {
                if let Some(w) = current.as_mut().or(out.last_mut()) {
                    w.text.push_str(c);
                    w.end_ms = e.max(w.end_ms);
                    continue;
                }
            } else if is_cjk_letter(first) {
                out.extend(current.take());
                out.push(SpokenWord { start_ms: s, end_ms: e, text: c.clone() });
                continue;
            } else if current.as_ref().is_some_and(|w| w.text.chars().last().is_some_and(is_cjk_letter)) {
                out.extend(current.take());
            }
            match current.as_mut() {
                Some(w) => {
                    w.text.push_str(c);
                    w.end_ms = e.max(w.end_ms);
                }
                None => current = Some(SpokenWord { start_ms: s, end_ms: e, text: c.clone() }),
            }
        }
        out.extend(current);
        out
    }
}

/// A Han character or kana (written without spaces between words).
fn is_cjk_letter(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

/// CJK punctuation (、。「」！？ and their full-width forms).
fn is_cjk_punctuation(c: char) -> bool {
    matches!(c as u32, 0x3000..=0x303F | 0xFF01..=0xFF0F | 0xFF1A..=0xFF20 | 0xFF3B..=0xFF40 | 0xFF5B..=0xFF65)
}

/// A reading from 16-bit little-endian mono PCM at [`crate::SAMPLE_RATE`], brought to the same
/// peak level as Kokoro's takes.
pub fn speech_from_pcm(pcm: &[u8], alignment: &Alignment) -> Result<Speech> {
    if pcm.len() < 2 {
        bail!("ElevenLabs returned no audio");
    }
    let mut samples: Vec<f32> = pcm.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect();
    let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
    if peak > 0.0 {
        let gain = PEAK / peak;
        samples.iter_mut().for_each(|s| *s *= gain);
    }
    Ok(Speech { samples, words: alignment.words() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn characters_make_words_between_spaces() {
        let chars = "Hi, there  you".chars().map(String::from).collect::<Vec<_>>();
        let n = chars.len();
        let a = Alignment { characters: chars, starts: (0..n).map(|i| i as f64 * 0.1).collect(), ends: (0..n).map(|i| i as f64 * 0.1 + 0.1).collect() };
        let w = a.words();
        assert_eq!(w.iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["Hi,", "there", "you"]);
        assert!(w[0].start_ms == 0.0 && (w[0].end_ms - 300.0).abs() < 1e-6);
        assert!((w[2].start_ms - 1100.0).abs() < 1e-6);
    }

    #[test]
    fn chinese_characters_are_words_and_keep_their_punctuation() {
        let text = "你好，世界。OK";
        let chars = text.chars().map(String::from).collect::<Vec<_>>();
        let n = chars.len();
        let a = Alignment { characters: chars, starts: (0..n).map(|i| i as f64 * 0.1).collect(), ends: (0..n).map(|i| i as f64 * 0.1 + 0.1).collect() };
        assert_eq!(a.words().iter().map(|w| w.text.as_str()).collect::<Vec<_>>(), ["你", "好，", "世", "界。", "OK"]);
    }

    #[test]
    fn alignment_is_read_from_the_reply() {
        let v = serde_json::json!({ "characters": ["O", "k"], "character_start_times_seconds": [0.0, 0.2], "character_end_times_seconds": [0.2, 0.5] });
        let a = Alignment::from_json(&v);
        assert_eq!(a.words(), vec![SpokenWord { start_ms: 0.0, end_ms: 500.0, text: "Ok".into() }]);
    }

    #[test]
    fn pcm_is_decoded_and_levelled() {
        let pcm: Vec<u8> = [0i16, 8192, -16384].iter().flat_map(|s| s.to_le_bytes()).collect();
        let s = speech_from_pcm(&pcm, &Alignment::default()).unwrap();
        assert_eq!(s.samples.len(), 3);
        assert!((s.samples[2] + PEAK).abs() < 1e-6 && (s.samples[1] - PEAK / 2.0).abs() < 1e-6);
        assert!(speech_from_pcm(&[], &Alignment::default()).is_err());
    }

    #[test]
    fn ids_do_not_clash_with_kokoro() {
        assert!(BUILT_IN.iter().all(|c| crate::voice(c.id).is_none()));
    }

    #[test]
    fn the_backend_list_is_taken_only_with_every_built_in_voice() {
        let mut list: Vec<serde_json::Value> = BUILT_IN[..CORE].iter().map(|v| serde_json::json!({ "id": v.id, "name": v.name, "female": v.female, "style": v.style })).collect();
        assert!(!set_catalog(&serde_json::json!({ "voices": list[1..] })), "a list missing Eric is ignored");
        assert_eq!(cloud_voices().len(), BUILT_IN.len(), "the built-in list stays");
        list.push(serde_json::json!({ "id": "el_es_lucia", "name": "Lucía", "female": true, "style": "warm", "language": "es" }));
        list.push(serde_json::json!({ "id": "am_fenrir", "name": "Not ElevenLabs" }));
        assert!(set_catalog(&serde_json::json!({ "voices": list })));
        assert_eq!(cloud_voices().len(), CORE + 1, "only el_ ids are taken");
        assert_eq!(cloud_voice("el_es_lucia").and_then(|v| v.language), Some("es"));

        // Spanish: only its native narrator
        let es = voices_for(Some("es"));
        assert_eq!(es.iter().map(|v| v.id).collect::<Vec<_>>(), ["el_es_lucia"]);
        assert_eq!(default_voice(Some("es")).id, "el_es_lucia");
        // English and French (no native narrator yet) get the eight, never the Spanish one
        assert!(voices_for(None).iter().chain(voices_for(Some("fr")).iter()).all(|v| v.language.is_none()));
        assert_eq!(voices_for(Some("fr")).len(), CORE);
        assert_eq!(default_voice(Some("fr")).id, "el_eric");
    }

    #[test]
    fn the_studio_voice_is_found_by_id_but_not_offered_with_the_others() {
        assert!(cloud_voice(STUDIO).is_none());
        set_studio(Some("Mehfuz"));
        assert_eq!(cloud_voice(STUDIO).map(|v| v.name), Some("Mehfuz"));
        assert!(cloud_voices().iter().all(|v| v.id != STUDIO) && voices_for(None).iter().all(|v| v.id != STUDIO));
        assert_eq!((backend_voice(STUDIO), backend_voice("el_eric")), ("my_voice", "el_eric"));
        set_studio(None);
        assert!(cloud_voice(STUDIO).is_none());
    }

    #[test]
    fn every_language_has_native_narrators_after_the_core_voices() {
        assert!(BUILT_IN[..CORE].iter().all(|v| v.language.is_none()));
        assert!(BUILT_IN[CORE..].iter().all(|v| v.language.is_some_and(|l| language(l).is_some())));
        assert!(BUILT_IN.iter().enumerate().all(|(i, v)| BUILT_IN.iter().skip(i + 1).all(|w| w.id != v.id)), "ids are unique");
        // every non-English language has a narrator of each gender (Dutch: one woman so far)
        for l in LANGUAGES.iter().skip(1) {
            let natives: Vec<_> = BUILT_IN.iter().filter(|v| v.language == Some(l.code)).collect();
            assert!(natives.iter().any(|v| v.female) && natives.iter().any(|v| !v.female), "{}", l.code);
        }
    }

    #[test]
    fn languages_are_unique_and_english_comes_first() {
        assert_eq!(LANGUAGES[0].code, "en");
        assert!(LANGUAGES.iter().enumerate().all(|(i, l)| LANGUAGES.iter().skip(i + 1).all(|m| m.code != l.code)));
        assert_eq!(language("ja").map(|l| l.name), Some("Japanese"));
    }
}
