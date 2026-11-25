//! "My voice": the voices the user made of their own voice on this computer (read by
//! Chatterbox Turbo, [`sf_tts::clone`]), and the studio voice ElevenLabs cloned from one of
//! them. This is the part without UI (finding, naming and reading the voices, and the studio
//! voice's backend calls), shared by the app and `sf mcp`; recording a voice is in the app.

use std::path::PathBuf;

use sf_tts::clone::MY_VOICE;
use sf_tts::SAMPLE_RATE;

pub fn voices_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "screenforge", "ScreenForge").map(|d| d.data_dir().join("my-voice-local"))
}

/// The folder of voice `id` (`my_voice:<key>`).
pub fn voice_dir(id: &str) -> Option<PathBuf> {
    let key = id.strip_prefix(MY_VOICE)?.strip_prefix(':')?;
    // keys are made here: letters and digits only, never a path
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    voices_dir().map(|d| d.join(key))
}

/// A voice made on this computer.
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceClone {
    /// `my_voice:<key>`
    pub id: String,
    pub name: String,
    pub seconds: f64,
    pub created_at: String,
}

fn read_voice(id: &str) -> Option<VoiceClone> {
    let dir = voice_dir(id)?;
    if !dir.join("reference.wav").is_file() {
        return None;
    }
    let v: serde_json::Value = std::fs::read_to_string(dir.join("voice.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    Some(VoiceClone {
        id: id.to_string(),
        name: v["name"].as_str().filter(|n| !n.trim().is_empty()).unwrap_or(sf_tts::clone_name()).to_string(),
        seconds: v["seconds"].as_f64().unwrap_or(0.0),
        created_at: v["createdAt"].as_str().unwrap_or_default().to_string(),
    })
}

/// The voices made, oldest first.
pub fn list() -> Vec<VoiceClone> {
    let Some(dir) = voices_dir() else { return vec![] };
    let mut out: Vec<VoiceClone> = std::fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|e| read_voice(&format!("{MY_VOICE}:{}", e.file_name().to_string_lossy()))).collect();
    out.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    out
}

/// Voice `id`: `my_voice` alone is the first one.
pub fn get(id: &str) -> Option<VoiceClone> {
    if id == MY_VOICE {
        return list().into_iter().next();
    }
    read_voice(id)
}

/// What voice `id` is called: the name the user gave it, or "My voice".
pub fn name(id: &str) -> String {
    get(id).map_or_else(|| sf_tts::clone_name().to_string(), |c| c.name)
}

/// The longest name a voice can have.
pub const NAME_MAX: usize = 40;

/// Keep a name being typed within [`NAME_MAX`] characters.
pub fn limit_name(name: &mut String) {
    if let Some((i, _)) = name.char_indices().nth(NAME_MAX) {
        name.truncate(i);
    }
}

/// `name` as kept: trimmed, at most [`NAME_MAX`] characters, "My voice" when empty.
pub fn clean_name(name: &str) -> String {
    let n: String = name.trim().chars().take(NAME_MAX).collect();
    if n.is_empty() {
        sf_tts::clone_name().to_string()
    } else {
        n
    }
}

pub fn save_meta(c: &VoiceClone) -> Result<(), String> {
    let dir = voice_dir(&c.id).ok_or("no folder for your voice")?;
    let json = serde_json::json!({ "name": c.name, "seconds": c.seconds, "createdAt": c.created_at });
    std::fs::write(dir.join("voice.json"), json.to_string()).map_err(|e| format!("Could not save your voice: {e}"))
}

/// Call voice `id` `name` from now on.
pub fn rename(id: &str, name: &str) -> Result<(), String> {
    let Some(mut c) = read_voice(id) else { return Ok(()) };
    c.name = clean_name(name);
    save_meta(&c)?;
    // its studio voice is called the same
    if studio_from().as_deref() == Some(id) {
        let name = c.name.clone();
        save_studio(id, &name);
        sf_tts::cloud::set_studio(Some(&name));
        std::thread::spawn(move || {
            if let Err(e) = call(serde_json::json!({ "action": "rename", "name": name }), 30) {
                tracing::warn!("studio voice not renamed: {e}");
            }
        });
    }
    Ok(())
}

/// The recording voice `id` is read from, mono at [`SAMPLE_RATE`].
pub fn reference(id: &str) -> Result<Vec<f32>, String> {
    let c = get(id).ok_or("You have not made your voice yet. Make it under Narration → My voice.")?;
    let path = voice_dir(&c.id).map(|d| d.join("reference.wav")).ok_or("no folder for your voice")?;
    sf_media::audio::decode(&path, SAMPLE_RATE, 1).map(|c| c.samples.to_vec()).map_err(|e| format!("Could not read your voice: {e}"))
}

/// Delete voice `id` (takes already read keep their audio).
pub fn delete(id: &str) -> Result<(), String> {
    let Some(dir) = voice_dir(id).filter(|d| d.exists()) else { return Ok(()) };
    crate::narration::forget_previews(id);
    // the studio voice made from it goes too (it could not be made again without it)
    if studio_from().as_deref() == Some(id) {
        std::thread::spawn(|| {
            if let Err(e) = remove_studio() {
                tracing::warn!("studio voice not deleted: {e}");
            }
        });
    }
    std::fs::remove_dir_all(dir).map_err(|e| format!("Could not delete your voice: {e}"))
}

/// A new voice's id.
pub fn new_id() -> String {
    format!("{MY_VOICE}:{}", chrono::Utc::now().format("%Y%m%d%H%M%S%3f"))
}


// ── The studio voice: the user's voice cloned by ElevenLabs on ScreenForge's account ──
//
// Made from one of their voices on this computer (its recordings are uploaded to the backend's
// `my-voice`, which makes an Instant Voice Clone), one per license. It reads like the other
// ElevenLabs voices, free, as `sf_tts::cloud::STUDIO`. The backend puts away clones
// unused for a while (to free the account's voice slots); reading with one put away makes it
// again from the same recordings, here, and reads once more.

/// Most of the recordings uploaded for a studio voice (s): ElevenLabs suggests 1–2 minutes.
pub const STUDIO_MAX_S: f64 = 90.0;

/// What the backend says about studio voices here: (new ones are made, this computer may make
/// one). Until it answered: neither.
static STUDIO_OFFER: std::sync::Mutex<(bool, bool)> = std::sync::Mutex::new((false, false));

/// The panel offers to make a studio voice (the backend makes them, for every computer).
pub fn studio_offered() -> bool {
    STUDIO_OFFER.lock().map(|o| o.0 && o.1).unwrap_or(false)
}

/// Which voice the studio voice was made from, and its name: kept beside the voices.
fn studio_file() -> Option<PathBuf> {
    voices_dir().map(|d| d.with_file_name("my-voice-studio.json"))
}

/// The voice on this computer the studio voice was made from.
pub fn studio_from() -> Option<String> {
    let f = studio_file()?;
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(f).ok()?).ok()?;
    v["from"].as_str().map(str::to_string)
}

fn save_studio(from: &str, name: &str) {
    if let Some(f) = studio_file() {
        let _ = f.parent().map(std::fs::create_dir_all);
        let _ = std::fs::write(f, serde_json::json!({ "from": from, "name": name }).to_string());
    }
}

fn forget_studio() {
    if let Some(f) = studio_file() {
        let _ = std::fs::remove_file(f);
    }
    sf_tts::cloud::set_studio(None);
    crate::narration::forget_previews(sf_tts::cloud::STUDIO);
}

/// Call the backend's `my-voice`.
fn call(body: serde_json::Value, secs: u64) -> Result<serde_json::Value, String> {
    let device = sf_license::device_id().ok_or("ElevenLabs needs this computer's id, which could not be read")?;
    let mut body = body;
    body["device"] = device.into();
    sf_license::call_backend_within("my-voice", body, secs)
}

/// Ask the backend whether the user has a studio voice and may make one, once a run (in the
/// background): the voice is offered from then on.
pub fn load_studio() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // the name kept here, until the backend answers
        if let Some(name) = studio_file().and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()).and_then(|v| v["name"].as_str().map(str::to_string)) {
            sf_tts::cloud::set_studio(Some(&name));
        }
        std::thread::spawn(|| match call(serde_json::json!({ "action": "status" }), 15) {
            Ok(reply) => {
                if let Ok(mut o) = STUDIO_OFFER.lock() {
                    *o = (reply["enabled"].as_bool() == Some(true), reply["pro"].as_bool() == Some(true));
                }
                match reply["voice"]["name"].as_str() {
                    Some(name) => sf_tts::cloud::set_studio(Some(name)),
                    // deleted elsewhere (another computer of the same license)
                    None => forget_studio(),
                }
            }
            Err(e) => tracing::info!("studio voice status not loaded: {e}"),
        });
    });
}

/// Voice `from`'s recordings for a studio voice: all of them when kept, else the part the voice
/// on this computer reads from (voices made before recordings were kept).
fn studio_samples(from: &str) -> Result<Vec<f32>, String> {
    let dir = voice_dir(from).ok_or("That voice is no longer on this computer.")?;
    let file = [dir.join("recordings.wav"), dir.join("reference.wav")].into_iter().find(|f| f.is_file()).ok_or("That voice is no longer on this computer.")?;
    let mut s = sf_media::audio::decode(&file, SAMPLE_RATE, 1).map_err(|e| format!("Could not read your voice: {e}"))?.samples.to_vec();
    s.truncate((STUDIO_MAX_S * SAMPLE_RATE as f64) as usize);
    Ok(s)
}

/// Make the studio voice from voice `from` (blocking).
pub fn make_studio(from: &str) -> Result<(), String> {
    let v = get(from).ok_or("That voice is no longer on this computer.")?;
    let samples = studio_samples(&v.id)?;
    use base64::Engine;
    let wav = base64::engine::general_purpose::STANDARD.encode(wav_bytes(SAMPLE_RATE, &samples));
    // consent: the user pressed Make a studio version under the text saying what it does; the
    // recordings were cleaned up when the voice was made
    let reply = call(serde_json::json!({ "action": "create", "consent": true, "clean": false, "name": v.name, "samples": [wav] }), 240)?;
    let name = reply["voice"]["name"].as_str().unwrap_or(&v.name).to_string();
    save_studio(&v.id, &name);
    crate::narration::forget_previews(sf_tts::cloud::STUDIO);
    sf_tts::cloud::set_studio(Some(&name));
    Ok(())
}

/// Delete the studio voice (blocking).
pub fn remove_studio() -> Result<(), String> {
    call(serde_json::json!({ "action": "delete" }), 30)?;
    forget_studio();
    Ok(())
}

/// Run `request` (a backend call reading with the studio voice, which answers with its HTTP
/// status on a refusal). When the backend had put the voice away (409), it is made again from
/// the voice it was made from and `request` runs once more.
pub fn with_studio(request: impl Fn() -> Result<serde_json::Value, (u16, String)>) -> Result<serde_json::Value, String> {
    match request() {
        Err((409, _)) => {
            let from = studio_from().ok_or("Your studio voice was put away after a while unused, and the voice it was made from is not on this computer. Make it again under Narration → My voice.")?;
            make_studio(&from).map_err(|e| format!("Could not make your studio voice again: {e}"))?;
            request().map_err(|(_, e)| e)
        }
        // gone at ElevenLabs, or deleted on another computer: no longer offered here
        Err((404, e)) => {
            forget_studio();
            Err(e)
        }
        r => r.map_err(|(_, e)| e),
    }
}


/// A mono 16-bit PCM WAV file of `samples` at `rate`.
pub fn wav_bytes(rate: u32, samples: &[f32]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}
