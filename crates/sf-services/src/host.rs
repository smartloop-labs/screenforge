//! The agent's host ([`sf_agent::Host`]) on this computer: the catalogs the pickers offer, presets,
//! files, stills and motion from the screen video, and what goes through ScreenForge's backend
//! (Unsplash, Jamendo, the ElevenLabs voices) or the app's data folder (favourite music, the
//! user's own voices). The app's agent panel and `sf mcp` both use it, so an agent can do the
//! same from either.

use std::path::PathBuf;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use sf_agent::Host;
use sf_project::audio_clips::AudioTrack;
use sf_project::Project;
use sf_session::Session;

use crate::{media, music, my_voice, narration, unsplash};

/// Called while a download progresses (the app repaints its window; `sf mcp` does nothing).
pub type Repaint = Arc<dyn Fn() + Send + Sync>;

pub struct LocalHost {
    options: Value,
    presets: Vec<(String, PathBuf)>,
    project_dir: PathBuf,
    /// The screen video (source time = its time) and its size.
    media: PathBuf,
    media_size: (u32, u32),
    /// Results of the last Unsplash search, for `download_photo`.
    photos: Mutex<Vec<unsplash::Photo>>,
    /// The user's favourite tracks, then the results of each Jamendo search, for `download_music`.
    tracks: Mutex<Vec<music::Track>>,
    repaint: Repaint,
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

impl LocalHost {
    /// The host for `session`'s project. Call [`narration::load_cloud_voices`] and
    /// [`my_voice::load_studio`] once a run before, so the ElevenLabs voices are listed.
    pub fn new(session: &Session, repaint: Repaint) -> Self {
        let assets = &session.assets;
        let presets: Vec<(String, PathBuf)> = sf_project::Preset::default_folder().map(|f| sf_project::Preset::list(&f)).unwrap_or_default().into_iter().map(|p| (p.name(), p.path.clone())).collect();
        let mut looks = assets.list_luts();
        if let Ok(files) = std::fs::read_dir(session.pkg.path.join("assets")) {
            looks.extend(files.flatten().map(|f| f.file_name().to_string_lossy().into_owned()).filter(|n| n.to_ascii_lowercase().ends_with(".cube")));
        }
        // the Jamendo tracks the user hearted, CC BY only: the agent cannot know whether the video
        // earns money, so a non-commercial favourite is left out
        let library = music::Library::load();
        let favorites: Vec<music::Track> = library.favorites.iter().filter(|t| t.kind == music::TrackKind::Jamendo && music::usable(t.license_url.as_deref(), false)).cloned().collect();
        let audio = &session.pkg.project.config.audio;
        let options = json!({
            "wallpapers": assets.list_backgrounds(),
            // bundled tracks the user hearted are marked, so the agent can prefer them
            "music": assets.list_background_audio().iter().map(|n| json!({ "track": n, "label": sf_session::assets::background_audio_label(n), "favorite": library.is_favorite(&music::library_id(n)) })).collect::<Vec<_>>(),
            "favoriteMusic": favorites.iter().map(track_value).collect::<Vec<_>>(),
            // the user's own voices, made on this computer, by the names they gave them
            "myVoices": my_voice::list().iter().map(|v| json!({ "voice": v.id, "name": v.name })).collect::<Vec<_>>(),
            // ElevenLabs voices beyond the built-in eight (native speakers of other languages),
            // and the language narration is in (absent: English)
            "cloudVoices": sf_tts::cloud::cloud_voices().iter().filter(|v| sf_agent::host::VOICES.iter().all(|(id, _)| *id != v.id)).map(|v| json!({ "voice": v.id, "name": v.name, "female": v.female, "style": v.style, "language": v.language }))
                // the user's own voice cloned by ElevenLabs
                .chain(sf_tts::cloud::studio().map(|v| json!({ "voice": v.id, "name": format!("{} (studio quality)", v.name), "style": v.style, "studio": true })))
                .collect::<Vec<_>>(),
            "narrationLanguage": audio.narration_language(),
            // the narrator the user selected in the Narration panel: add_narration reads with it
            // when no voice is given
            "narrationVoice": audio.narration_voice.as_deref().map(|v| json!({ "voice": v, "name": if sf_tts::clone::is_my_voice(v) { my_voice::name(v) } else { sf_tts::voice_name(v).unwrap_or(v).to_string() } })),
            "cursorSets": sf_session::cursor_sets::SETS.iter().map(|(id, label)| json!({ "id": id, "label": label })).collect::<Vec<_>>(),
            "cameraLooks": looks,
            "presets": presets.iter().map(|(n, _)| n).collect::<Vec<_>>(),
            "recordedAudio": audio_tracks(session),
            "hasCamera": !session.source.camera.is_empty(),
            "hasTranscript": !session.captions.is_empty(),
        });
        let media_size = (session.source.native_width.max(1), session.source.native_height.max(1));
        LocalHost { options, presets, project_dir: session.pkg.path.clone(), media: session.source.media_path.clone(), media_size, photos: Mutex::new(vec![]), tracks: Mutex::new(favorites), repaint }
    }

    /// Keep the photos and tracks `earlier` (the host before the project was read again) was
    /// shown, so an id from a search made then can still be used.
    pub fn keep_results(&self, earlier: &LocalHost) {
        if let (Ok(mut photos), Ok(old)) = (self.photos.lock(), earlier.photos.lock()) {
            if photos.is_empty() {
                *photos = old.clone();
            }
        }
        if let (Ok(mut tracks), Ok(old)) = (self.tracks.lock(), earlier.tracks.lock()) {
            for t in old.iter() {
                if !tracks.iter().any(|k| k.id == t.id) {
                    tracks.push(t.clone());
                }
            }
        }
    }
}

/// A Jamendo track as the agent sees it (a favourite or a search result).
fn track_value(t: &music::Track) -> Value {
    let credit = music::Credit { title: t.name.clone(), artist: t.artist.clone(), url: None, license_url: t.license_url.clone() };
    json!({ "id": t.id, "name": t.name, "artist": t.artist, "seconds": t.duration, "license": credit.license() })
}

/// Motion sections per screen video, measured once (decoding is the slow part).
type Sections = Vec<(f64, f64, sf_agent::motion::Motion)>;
static MOTION: Mutex<Vec<(PathBuf, Sections)>> = Mutex::new(Vec::new());

/// Frames the motion scan reads per second, and their width (wide enough that a typed
/// character changes a few pixels).
const MOTION_FPS: f64 = 2.0;
const MOTION_W: u32 = 640;

fn ffmpeg() -> Result<std::process::Command, String> {
    let path = sf_recording::ffmpeg::ffmpeg_path().map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(path);
    sf_recording::ffmpeg::hide_console(&mut cmd);
    Ok(cmd)
}

/// `path` with a leading `~` made the home folder.
fn expand_home(path: &str) -> PathBuf {
    let src = PathBuf::from(path.trim());
    match src.strip_prefix("~") {
        Ok(rest) => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest)).unwrap_or(src.clone()),
        Err(_) => src,
    }
}

impl Host for LocalHost {
    fn options(&self) -> Value {
        self.options.clone()
    }

    fn apply_preset(&self, name: &str, project: &mut Project) -> Result<(), String> {
        let (_, path) = self.presets.iter().find(|(n, _)| n.eq_ignore_ascii_case(name.trim())).ok_or_else(|| format!("no preset named {name}"))?;
        let preset = sf_project::Preset::open(path).map_err(|e| e.to_string())?;
        preset.apply(project, &self.project_dir).map_err(|e| e.to_string())
    }

    fn search_photos(&self, query: &str) -> Result<Value, String> {
        let photos = unsplash::search_blocking(query)?;
        let list = photos.iter().map(|p| json!({ "id": p.id, "description": p.description, "author": p.author, "color": p.color })).collect();
        *self.photos.lock().map_err(|_| "busy")? = photos;
        Ok(Value::Array(list))
    }

    fn download_photo(&self, id: &str) -> Result<Value, String> {
        let photo = self.photos.lock().map_err(|_| "busy")?.iter().find(|p| p.id == id).cloned().ok_or("search_photos first: that id is not in the last results")?;
        let applied = unsplash::download_photo(photo, &self.project_dir.join("assets"), &AtomicU32::new(0), &*self.repaint)?;
        let mut value = unsplash::background_value(&applied);
        value["credit"] = json!(unsplash::credit(&value));
        Ok(value)
    }

    fn search_music(&self, query: &str, instrumental: bool) -> Result<Value, String> {
        let tracks = music::search_blocking(query, instrumental)?;
        let list = tracks.iter().map(track_value).collect();
        // keep the favourites (and earlier results): any id the agent was shown stays usable
        let mut known = self.tracks.lock().map_err(|_| "busy")?;
        for t in tracks {
            if !known.iter().any(|k| k.id == t.id) {
                known.push(t);
            }
        }
        Ok(Value::Array(list))
    }

    fn download_music(&self, id: &str) -> Result<Value, String> {
        let track = self.tracks.lock().map_err(|_| "busy")?.iter().find(|t| t.id == id).cloned().ok_or("that id is not in get_options favoriteMusic or a search_music result")?;
        let assets = self.project_dir.join("assets");
        let applied = music::download_track(&track, &assets, &AtomicU32::new(0), &*self.repaint)?;
        let credit = music::credit(&assets, &applied.file_name).map(|c| c.text());
        Ok(json!({ "fileName": applied.file_name, "name": track.name, "artist": track.artist, "credit": credit }))
    }

    fn favorite_music(&self, track: &str, favorite: bool) -> Result<Value, String> {
        use music::{Library, Track};
        // the file on disk, not a copy loaded earlier: the panel may have changed it since
        let mut library = Library::load();
        let name = track.trim();
        let bundled: Vec<String> = self.options["music"].as_array().into_iter().flatten().filter_map(|m| m["track"].as_str().map(str::to_string)).collect();
        let assets = self.project_dir.join("assets");
        let base = name.rsplit('/').next().unwrap_or(name);
        let known: Vec<Track> = self.tracks.lock().map_err(|_| "busy")?.iter().chain(&library.recent).cloned().collect();
        let title = if let Some(n) = bundled.iter().find(|n| n.as_str() == name || sf_session::assets::background_audio_key(n) == sf_session::assets::background_audio_key(name)) {
            let t = Track::library(n);
            library.set_favorite(&t, favorite);
            t.name
        } else if let Some(t) = known.iter().find(|t| t.id == name) {
            library.set_favorite(t, favorite);
            t.name.clone()
        } else if !base.is_empty() && assets.join(base).is_file() {
            library.set_file(&assets, base, &known, favorite)?
        } else {
            return Err(format!("no track \"{name}\": use a name from get_options music, a Jamendo id from favoriteMusic or search_music, or a music file in the project"));
        };
        Ok(json!({ "name": title, "favorite": favorite }))
    }

    fn import_url(&self, url: &str) -> Result<String, String> {
        music::download_link(url, &self.project_dir.join("assets"), &AtomicU32::new(0), &*self.repaint).map(|a| a.file_name)
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
        // one frame at a time: a long video never sits in memory
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

    fn speak(&self, text: &str, voice: &str, speed: f64, language: Option<&str>) -> Result<sf_agent::host::Spoken, String> {
        narration::speak_into(&self.project_dir, text, voice, speed, language)
    }

    fn import_media(&self, path: &str) -> Result<sf_project::tracks::MediaItem, String> {
        let src = expand_home(path);
        if !src.is_file() {
            return Err(format!("no file at {}", src.display()));
        }
        media::import(&self.project_dir, &src)
    }
}
