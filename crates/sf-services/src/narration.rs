//! AI voice narration for the Narration panel and the agent: downloading the voice model,
//! previewing a voice, and reading text into voice-over takes (with their words for captions),
//! off the UI thread. Kokoro's voices are read on this computer; ElevenLabs' online through the
//! backend's `voice` function, paid in credits. This is the part without UI, shared by the app
//! and `sf mcp`.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use sf_project::transcript::{Generator, TranscriptFile, TranscriptItem};
use sf_tts::clone::is_my_voice;
use sf_tts::Progress;

/// What the preview says (in English; an ElevenLabs voice says it in the narration's
/// language, [`sf_tts::cloud::Language::preview`]).
pub const PREVIEW: &str = sf_tts::cloud::LANGUAGES[0].preview;

/// Load the ElevenLabs voices the backend offers (native speakers of other languages beside
/// the built-in ones): at start the copy kept from the last run, then the backend's list in the
/// background. Until then, and offline, the voices built in are offered.
pub fn load_cloud_voices() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if let Some(kept) = catalog_file().and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) {
            sf_tts::cloud::set_catalog(&kept);
        }
        refresh_cloud_voices();
    });
}

fn catalog_file() -> Option<PathBuf> {
    previews_dir().map(|d| d.with_file_name("voice-catalog.json"))
}

/// Fetch the backend's voice list again (when the narration language changes, so voices added
/// since the app started show), at most once a minute.
pub fn refresh_cloud_voices() {
    static LAST: Mutex<Option<std::time::Instant>> = Mutex::new(None);
    {
        let Ok(mut last) = LAST.lock() else { return };
        if last.is_some_and(|t| t.elapsed().as_secs() < 60) {
            return;
        }
        *last = Some(std::time::Instant::now());
    }
    let file = catalog_file();
    std::thread::spawn(move || match sf_license::call_backend("voice", serde_json::json!({ "action": "list" })) {
        Ok(reply) if sf_tts::cloud::set_catalog(&reply) => {
            if let Some(f) = file {
                let _ = f.parent().map(std::fs::create_dir_all);
                let _ = std::fs::write(f, reply.to_string());
            }
        }
        Ok(_) => tracing::warn!("the backend's voice list lacks the core voices; keeping the current list"),
        Err(e) => tracing::info!("voice list not loaded: {e}"),
    });
}

/// The balance the last ElevenLabs reading left (the backend's `credits` summary), for the agent
/// panel's credits chip; taken by [`take_credits`].
static CREDITS: Mutex<Option<serde_json::Value>> = Mutex::new(None);

pub fn take_credits() -> Option<serde_json::Value> {
    CREDITS.lock().ok()?.take()
}

/// Keep the balance an ElevenLabs reply (`sf_credits`) left, for the credits chip.
pub fn note_credits(reply: &serde_json::Value) {
    if let Some(c) = reply.get("sf_credits").filter(|c| c.is_object()) {
        if let Ok(mut slot) = CREDITS.lock() {
            *slot = Some(c.clone());
        }
    }
}

/// May this computer use the ElevenLabs voices (Pro, staff or trial credits, as the backend
/// decides)? `None` until the agent panel has checked the plan; the backend refuses anyway.
static CLOUD_ALLOWED: Mutex<Option<bool>> = Mutex::new(None);

pub fn set_cloud_allowed(allowed: Option<bool>) {
    if let Ok(mut a) = CLOUD_ALLOWED.lock() {
        *a = allowed;
    }
}

/// The ElevenLabs voices are known to be unavailable here (not Pro).
pub fn cloud_locked() -> bool {
    CLOUD_ALLOWED.lock().ok().and_then(|a| *a) == Some(false)
}

/// A take read into the project's voice-over folder.

/// Read `text` with `voice` into `project`'s voice-over folder, with its words saved as the
/// take's transcript (they are the captions; the take is never transcribed). `language` is
/// the narration's for an ElevenLabs voice (`None`: English); the voices on this computer read
/// English.
pub fn speak_into(project: &Path, text: &str, voice: &str, speed: f64, language: Option<&str>) -> Result<sf_agent::host::Spoken, String> {
    let cloud = sf_tts::cloud::cloud_voice(voice);
    let online = cloud.is_some();
    // a native narrator reads its own language
    let language = cloud.and_then(|c| c.language).or(language.filter(|_| online));
    let (speech, kind) = if online {
        (cloud_speak(text, voice, speed, language)?, "elevenlabs")
    } else if is_my_voice(voice) {
        (my_voice_speak(voice, text, &AtomicBool::new(false), |_| {})?, "chatterbox")
    } else {
        let v = sf_tts::voice(voice).ok_or_else(|| format!("unknown voice {voice}"))?;
        // the first narration downloads the model
        let models = sf_tts::ensure(&AtomicBool::new(false), |_| {}).map_err(|e| format!("{e:#}"))?;
        (sf_tts::Kokoro::shared(&models).and_then(|k| k.speak(text, v, speed as f32)).map_err(|e| format!("{e:#}"))?, "kokoro")
    };
    let dir = project.join(sf_project::v4::VoiceOver::DIR);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = format!("narration-{}.wav", chrono::Local::now().format("%Y%m%d-%H%M%S-%3f"));
    speech.write_wav(&dir.join(&name)).map_err(|e| format!("{e:#}"))?;
    let items = speech.words.iter().enumerate().map(|(i, w)| TranscriptItem::new(i as u32, w.start_ms, w.end_ms, w.text.clone())).collect();
    let generator = Generator { kind: kind.into(), model: Some(voice.into()), language: Some(language.unwrap_or("en").into()), prompt: None, extra: Default::default() };
    TranscriptFile::voice_over(&name, generator, items).save(project).map_err(|e| e.to_string())?;
    Ok(sf_agent::host::Spoken { file_name: name, duration_ms: speech.duration_ms() })
}

/// Read `text` with ElevenLabs voice `voice` through the backend (which charges the credits),
/// in `language` (`None`: ElevenLabs tells from the text).
fn cloud_speak(text: &str, voice: &str, speed: f64, language: Option<&str>) -> Result<sf_tts::Speech, String> {
    let device = sf_license::device_id().ok_or("ElevenLabs voices need this computer's id, which could not be read")?;
    let mut body = serde_json::json!({ "device": device, "voice": sf_tts::cloud::backend_voice(voice), "text": text, "speed": speed });
    if let Some(l) = language {
        body["language"] = l.into();
    }
    let reply = if voice == sf_tts::cloud::STUDIO {
        // put away by the backend after a while unused: made again, then read
        crate::my_voice::with_studio(|| sf_license::call_backend_status_within("voice", body.clone(), 180))?
    } else {
        sf_license::call_backend_within("voice", body, 180)?
    };
    note_credits(&reply);
    use base64::Engine;
    let pcm = base64::engine::general_purpose::STANDARD.decode(reply["audio"].as_str().ok_or("ElevenLabs returned no audio")?).map_err(|e| e.to_string())?;
    let alignment = sf_tts::cloud::Alignment::from_json(&reply["alignment"]);
    sf_tts::cloud::speech_from_pcm(&pcm, &alignment).map_err(|e| format!("{e:#}"))
}

/// Read `text` in the user's own voice `voice` on this computer (downloading the model first).
fn my_voice_speak(voice: &str, text: &str, cancel: &AtomicBool, progress: impl FnMut(Progress)) -> Result<sf_tts::Speech, String> {
    let reference = crate::my_voice::reference(voice)?;
    speak_with(text, &reference, cancel, progress)
}

fn speak_with(text: &str, reference: &[f32], cancel: &AtomicBool, progress: impl FnMut(Progress)) -> Result<sf_tts::Speech, String> {
    speak_reporting(text, reference, cancel, progress, |_| {})
}

/// [`speak_with`], telling `reading` how far the reading is (0–1).
fn speak_reporting(text: &str, reference: &[f32], cancel: &AtomicBool, progress: impl FnMut(Progress), reading: impl FnMut(f32)) -> Result<sf_tts::Speech, String> {
    let dir = sf_tts::clone::ensure(cancel, progress).map_err(|e| format!("{e:#}"))?;
    sf_tts::clone::Chatterbox::shared(&dir).and_then(|c| c.speak_with_progress(text, reference, cancel, reading)).map_err(|e| format!("{e:#}"))
}

/// Where the preview of My voice `v` is kept (per voice and per making of it).
fn my_voice_preview_file(v: &crate::my_voice::VoiceClone) -> Option<PathBuf> {
    let made: String = v.created_at.chars().filter(char::is_ascii_alphanumeric).collect();
    previews_dir().map(|d| d.join(format!("{}-{made}.pcm", preview_key(&v.id))))
}

fn keep_preview(file: &Path, speech: &sf_tts::Speech) {
    let pcm: Vec<u8> = speech.samples.iter().flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
    let _ = file.parent().map(std::fs::create_dir_all);
    let _ = std::fs::write(file, pcm);
}

/// Read My voice `v`'s preview from `reference` and keep it, as the voice is made: the voice is
/// known to work, and its Preview plays at once.
/// `reading` hears how far it is (0–1).
pub fn prepare_my_voice(v: &crate::my_voice::VoiceClone, reference: &[f32], reading: impl FnMut(f32)) -> Result<(), String> {
    forget_previews(&v.id);
    let speech = speak_reporting(PREVIEW, reference, &AtomicBool::new(false), |_| {}, reading).map_err(|e| format!("Could not read with your voice: {e}"))?;
    if let Some(f) = my_voice_preview_file(v) {
        keep_preview(&f, &speech);
    }
    Ok(())
}

/// The preview of My voice `voice`: the one kept when it was made, or read now.
pub fn my_voice_preview(voice: &str, cancel: &AtomicBool, progress: impl FnMut(Progress)) -> Result<sf_tts::Speech, String> {
    let v = crate::my_voice::get(voice).ok_or("You have not made your voice yet. Make it under Narration → My voice.")?;
    let file = my_voice_preview_file(&v);
    if let Some(pcm) = file.as_ref().and_then(|f| std::fs::read(f).ok()) {
        return sf_tts::cloud::speech_from_pcm(&pcm, &Default::default()).map_err(|e| format!("{e:#}"));
    }
    let speech = my_voice_speak(&v.id, PREVIEW, cancel, progress)?;
    if let Some(f) = file {
        keep_preview(&f, &speech);
    }
    Ok(speech)
}

/// A voice id as part of a file name (`my_voice:123` has a colon, which Windows refuses).
fn preview_key(voice: &str) -> String {
    voice.replace(':', "_")
}

fn previews_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "screenforge", "ScreenForge").map(|d| d.cache_dir().join("voice-previews"))
}

/// Drop the kept previews of `voice` (My voice was made again or deleted).
pub fn forget_previews(voice: &str) {
    let Some(dir) = previews_dir() else { return };
    for f in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if f.file_name().to_string_lossy().starts_with(&format!("{}-", preview_key(voice))) {
            let _ = std::fs::remove_file(f.path());
        }
    }
}

/// The preview of ElevenLabs voice `voice` at `speed` in `language` (`None`: English), read
/// once and kept in the cache folder.
pub fn cloud_preview(voice: &str, speed: f64, language: Option<&str>) -> Result<sf_tts::Speech, String> {
    let lang = language.and_then(sf_tts::cloud::language).filter(|l| l.code != "en");
    // English previews keep the name they had before narration had a language
    let name = match lang {
        Some(l) => format!("{voice}-{}-{:.0}.pcm", l.code, speed * 100.0),
        None => format!("{voice}-{:.0}.pcm", speed * 100.0),
    };
    let file = previews_dir().map(|d| d.join(name));
    if let Some(pcm) = file.as_ref().and_then(|f| std::fs::read(f).ok()) {
        return sf_tts::cloud::speech_from_pcm(&pcm, &Default::default()).map_err(|e| format!("{e:#}"));
    }
    let speech = cloud_speak(lang.map_or(PREVIEW, |l| l.preview), voice, speed, lang.map(|l| l.code))?;
    if let Some(f) = file {
        let pcm: Vec<u8> = speech.samples.iter().flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
        let _ = f.parent().map(std::fs::create_dir_all);
        let _ = std::fs::write(f, pcm);
    }
    Ok(speech)
}

