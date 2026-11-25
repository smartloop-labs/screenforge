//! The editing tools' host outside the app: what `sf mcp` gives [`sf_agent::Workspace`] in
//! place of the editor. Catalogs, presets, files, stills and motion from the screen video, and
//! on-device narration. What needs the app's accounts (Unsplash, Jamendo, ElevenLabs, the
//! user's own voice) is left out.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use sf_agent::{Host, Workspace};
use sf_project::audio_clips::AudioTrack;
use sf_project::transcript::{Generator, TranscriptFile, TranscriptItem};
use sf_project::Project;
use sf_session::Session;

pub struct CliHost {
    options: Value,
    presets: Vec<(String, PathBuf)>,
    project_dir: PathBuf,
    /// The screen video (source time = its time) and its size.
    media: PathBuf,
    media_size: (u32, u32),
}

/// Audio recorded with the screen, as the editor offers it for detaching.
pub fn audio_tracks(session: &Session) -> Vec<AudioTrack> {
    let mut t = vec![];
    if !session.source.microphone.is_empty() {
        t.push(AudioTrack::Microphone);
    }
    if !session.source.system_audio.is_empty() {
        t.push(AudioTrack::SystemAudio);
    }
    t
}

impl CliHost {
    pub fn new(session: &Session) -> Self {
        let assets = &session.assets;
        let presets: Vec<(String, PathBuf)> = sf_project::Preset::default_folder().map(|f| sf_project::Preset::list(&f)).unwrap_or_default().into_iter().map(|p| (p.name(), p.path.clone())).collect();
        let mut looks = assets.list_luts();
        if let Ok(files) = std::fs::read_dir(session.pkg.path.join("assets")) {
            looks.extend(files.flatten().map(|f| f.file_name().to_string_lossy().into_owned()).filter(|n| n.to_ascii_lowercase().ends_with(".cube")));
        }
        let audio = &session.pkg.project.config.audio;
        let options = json!({
            "wallpapers": assets.list_backgrounds(),
            "music": assets.list_background_audio().iter().map(|n| json!({ "track": n, "label": sf_session::assets::background_audio_label(n) })).collect::<Vec<_>>(),
            "narrationLanguage": audio.narration_language(),
            "narrationVoice": audio.narration_voice.as_deref().filter(|v| is_local_voice(v)).map(|v| json!({ "voice": v, "name": sf_tts::voice_name(v).unwrap_or(v) })),
            "cursorSets": sf_session::cursor_sets::SETS.iter().map(|(id, label)| json!({ "id": id, "label": label })).collect::<Vec<_>>(),
            "cameraLooks": looks,
            "presets": presets.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            "recordedAudio": audio_tracks(session),
            "hasCamera": !session.source.camera.is_empty(),
            "hasTranscript": !session.captions.is_empty(),
            "note": "Outside the app: narration uses the on-device voices (am_*, af_*); Unsplash photos, Jamendo music, ElevenLabs and the user's own voice are only in the ScreenForge app.",
        });
        let media_size = (session.source.native_width.max(1), session.source.native_height.max(1));
        CliHost { options, presets, project_dir: session.pkg.path.clone(), media: session.source.media_path.clone(), media_size }
    }
}

/// The agent's view of an open project, as the editor builds it when a chat turn starts.
pub fn workspace(session: &Session, host: Arc<dyn Host>) -> Workspace {
    let t = &session.cursor_track;
    let clicks: Vec<f64> = t.clicks.iter().filter(|c| c.down).map(|c| c.time_ms).collect();
    let moves: Vec<f64> = t.samples.iter().map(|s| s.time_ms).collect();
    let activity = (!t.key_downs.is_empty() || !clicks.is_empty() || !moves.is_empty()).then(|| sf_agent::activity::Activity::from_events(&t.key_downs, &clicks, &moves, session.source.duration_ms));
    let (w, h) = session.content_size_at(0.0);
    let mut ws = Workspace::new(session.pkg.project.clone());
    ws.words = session.captions.lines.iter().flat_map(|l| l.words.iter().cloned()).collect();
    ws.auto_zooms = session.auto_zooms();
    ws.activity = activity;
    ws.content_px = (w as f64, h as f64);
    ws.audio_tracks = audio_tracks(session);
    ws.host = Some(host);
    ws
}

/// A Kokoro voice, read on this computer (not ElevenLabs or the user's own voice).
fn is_local_voice(id: &str) -> bool {
    sf_tts::voice(id).is_some()
}

/// Motion sections per screen video, measured once (decoding is the slow part).
type Sections = Vec<(f64, f64, sf_agent::motion::Motion)>;
static MOTION: Mutex<Vec<(PathBuf, Sections)>> = Mutex::new(Vec::new());
const MOTION_FPS: f64 = 2.0;
const MOTION_W: u32 = 640;

fn ffmpeg() -> Result<std::process::Command, String> {
    let path = sf_recording::ffmpeg::ffmpeg_path().map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(path);
    sf_recording::ffmpeg::hide_console(&mut cmd);
    Ok(cmd)
}

fn expand_home(path: &str) -> PathBuf {
    let src = PathBuf::from(path.trim());
    match src.strip_prefix("~") {
        Ok(rest) => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest)).unwrap_or(src.clone()),
        Err(_) => src,
    }
}

impl Host for CliHost {
    fn options(&self) -> Value {
        self.options.clone()
    }

    fn apply_preset(&self, name: &str, project: &mut Project) -> Result<(), String> {
        let (_, path) = self.presets.iter().find(|(n, _)| n.eq_ignore_ascii_case(name.trim())).ok_or_else(|| format!("no preset named {name}"))?;
        let preset = sf_project::Preset::open(path).map_err(|e| e.to_string())?;
        preset.apply(project, &self.project_dir).map_err(|e| e.to_string())
    }

    fn search_photos(&self, _query: &str) -> Result<Value, String> {
        Err("Unsplash photos are only in the ScreenForge app; use import_file with an image instead".into())
    }

    fn download_photo(&self, _id: &str) -> Result<Value, String> {
        Err("Unsplash photos are only in the ScreenForge app".into())
    }

    fn import_file(&self, path: &str) -> Result<String, String> {
        let src = expand_home(path);
        if !src.is_file() {
            return Err(format!("no file at {}", src.display()));
        }
        let name = src.file_name().ok_or("not a file")?.to_string_lossy().into_owned();
        let dest = self.project_dir.join("assets").join(&name);
        std::fs::create_dir_all(dest.parent().expect("assets dir")).map_err(|e| e.to_string())?;
        std::fs::copy(&src, &dest).map_err(|e| format!("could not copy it into the project: {e}"))?;
        Ok(name)
    }

    fn screen_motion(&self) -> Result<Sections, String> {
        if let Some((_, s)) = MOTION.lock().map_err(|_| "busy")?.iter().find(|(p, _)| *p == self.media) {
            return Ok(s.clone());
        }
        let h = ((MOTION_W as f64 * self.media_size.1 as f64 / self.media_size.0 as f64 / 2.0).round() as u32 * 2).max(2);
        let mut child = ffmpeg()?
            .args(["-v", "error", "-nostdin", "-i"])
            .arg(&self.media)
            .args(["-vf", &format!("fps={MOTION_FPS},scale={MOTION_W}:{h},format=gray"), "-f", "rawvideo", "-"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("could not read the video: {e}"))?;
        let mut stdout = child.stdout.take().ok_or("could not read the video")?;
        let size = (MOTION_W * h) as usize;
        let frames = std::iter::from_fn(|| {
            use std::io::Read;
            let mut f = vec![0u8; size];
            stdout.read_exact(&mut f).ok().map(|_| f)
        });
        let sections = sf_agent::motion::sections(frames, MOTION_W as usize, MOTION_FPS);
        let _ = child.wait();
        if sections.is_empty() {
            return Err("could not read the video".into());
        }
        MOTION.lock().map_err(|_| "busy")?.push((self.media.clone(), sections.clone()));
        Ok(sections)
    }

    fn screen_frames(&self, source_ms: &[f64]) -> Result<Vec<Vec<u8>>, String> {
        source_ms
            .iter()
            .map(|t| {
                let out = ffmpeg()?
                    .args(["-v", "error", "-nostdin", "-ss", &format!("{:.3}", t / 1000.0), "-i"])
                    .arg(&self.media)
                    .args(["-frames:v", "1", "-vf", "scale=768:-2", "-q:v", "5", "-f", "image2", "-c:v", "mjpeg", "pipe:1"])
                    .output()
                    .map_err(|e| format!("could not read the video: {e}"))?;
                if out.stdout.is_empty() {
                    return Err(format!("no frame at {:.1} s", t / 1000.0));
                }
                Ok(out.stdout)
            })
            .collect()
    }

    fn speak(&self, text: &str, voice: &str, speed: f64, _language: Option<&str>) -> Result<sf_agent::host::Spoken, String> {
        let v = sf_tts::voice(voice).ok_or_else(|| format!("voice {voice} is only in the ScreenForge app; outside it use an on-device voice (am_fenrir, am_michael, am_puck, af_heart, af_bella, af_nicole)"))?;
        // the first narration downloads the model
        let models = sf_tts::ensure(&AtomicBool::new(false), |_| {}).map_err(|e| format!("{e:#}"))?;
        let speech = sf_tts::Kokoro::shared(&models).and_then(|k| k.speak(text, v, speed as f32)).map_err(|e| format!("{e:#}"))?;
        write_voice_over(&self.project_dir, speech, voice)
    }
}

/// Save a take in the project's voice-over folder with its words, as the app does.
fn write_voice_over(project: &Path, speech: sf_tts::Speech, voice: &str) -> Result<sf_agent::host::Spoken, String> {
    let dir = project.join(sf_project::v4::VoiceOver::DIR);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = format!("narration-{}.wav", chrono::Local::now().format("%Y%m%d-%H%M%S-%3f"));
    speech.write_wav(&dir.join(&name)).map_err(|e| format!("{e:#}"))?;
    let items = speech.words.iter().enumerate().map(|(i, w)| TranscriptItem::new(i as u32, w.start_ms, w.end_ms, w.text.clone())).collect();
    let generator = Generator { kind: "kokoro".into(), model: Some(voice.into()), language: Some("en".into()), prompt: None, extra: Default::default() };
    TranscriptFile::voice_over(&name, generator, items).save(project).map_err(|e| e.to_string())?;
    Ok(sf_agent::host::Spoken { file_name: name, duration_ms: speech.duration_ms() })
}
