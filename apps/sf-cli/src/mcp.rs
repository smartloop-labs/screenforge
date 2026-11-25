//! `sf mcp`: a Model Context Protocol server on stdin/stdout, so the user's own agent (Claude
//! Code, Codex, Cursor …) can record and edit ScreenForge projects. No AI runs here and no
//! tokens are spent by ScreenForge: the editing tools are the same ones the app's assistant
//! uses (`sf_agent::tools`), plus recording, importing, rendering a frame and exporting.
//!
//! ```text
//! claude mcp add screenforge -- sf mcp
//! ```
//!
//! Every editing tool takes `project`, the path of a `.screenforge` package. An edit is saved
//! to the package straight away (with `project.json.bak` as before every save), and an open
//! ScreenForge editor picks it up from the disk. A project changed on disk by the app is read
//! again before the next tool runs.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde_json::{json, Value};
use sf_project::Project;
use sf_render::{Compositor, Gpu};
use sf_session::Session;

use crate::host::{self, CliHost};
use crate::record;

/// Protocol versions this server speaks, newest first.
const VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// App tools that make no sense outside the editor (the playhead, its export dialog) or need
/// the app's accounts.
const APP_ONLY: [&str; 7] = ["seek", "open_export", "search_photos", "use_photo", "search_music", "use_music", "favorite_music"];

/// Tools that write files into the package besides project.json: the session is opened again
/// after them so it sees the new files.
const FILE_TOOLS: [&str; 7] = ["add_narration", "update_narration", "add_overlay", "import_file", "apply_preset", "add_music", "set_music"];

const INSTRUCTIONS: &str = "ScreenForge records the screen and edits screen recordings into polished videos (backgrounds, zooms, cuts, speed, captions, camera, titles, narration). \
A project is a .screenforge package (a folder); pass its path as `project` to every editing tool. Edits are saved immediately and show up live in the ScreenForge app if the project is open there.

Getting a project: list_projects (recent ones in ~/ScreenForge Projects), start_recording + stop_recording (records the screen; the user does what they want to show in between), or import_video (an .mp4/.mov file, a .screenstudio project, or a public Loom link).

How to edit:
- Call get_project first, and again after the user may have edited by hand. Tool times are milliseconds on the edited timeline (what plays), not in the raw recording.
- Find moments with get_transcript (needs transcribe first), get_activity (typing, clicks, idle from the input log), get_motion and view_frames (stills of the raw screen).
- cut takes every range of one edit in a single call (ranges refer to the timeline before the call). Leave about 100 ms around neighbouring words when cutting speech.
- Zooms, masks and camera layouts each sit on one lane and cannot overlap. Zoom gently: 1.3–1.5× for text and typing, about 2× only for a small detail; a few seconds each.
- Settings (background, padding, corners, shadow, cursor, camera, captions, audio, aspect ratio) are changed with patch_project; get_options lists the values and where each setting lives, get_config shows current values.
- \"Auto edit\" / tighten: with a transcript cut long pauses and filler words; speed up stretches where nothing is said or clicked (2–4×); add zooms around the clicks (add_auto_zooms) if there are none.
- render_frame shows the finished look (background, zoom, captions) at a time: use it to check your work. export writes the video file.
- undo_answers reverts your earlier edits in this session.";

/// A project the server has open.
struct Open {
    session: Session,
    host: Arc<CliHost>,
    /// project.json's modification time when it was last read or written here.
    stamp: Option<SystemTime>,
    /// The project before each edit made through this server, for `undo_answers`.
    earlier: Vec<Project>,
}

struct Server {
    gpu: Option<Gpu>,
    open: HashMap<PathBuf, Open>,
    recording: Option<record::Active>,
}

fn stamp(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path.join(sf_project::package::PROJECT_FILE)).and_then(|m| m.modified()).ok()
}

fn expand(path: &str) -> PathBuf {
    let p = PathBuf::from(path.trim());
    let p = match p.strip_prefix("~") {
        Ok(rest) => std::env::var_os("HOME").map(|h| PathBuf::from(h).join(rest)).unwrap_or(p.clone()),
        Err(_) => p,
    };
    std::fs::canonicalize(&p).unwrap_or(p)
}

fn text(s: impl Into<String>) -> Value {
    json!({ "type": "text", "text": s.into() })
}

fn image(bytes: &[u8], mime: &str) -> Value {
    json!({ "type": "image", "data": base64::engine::general_purpose::STANDARD.encode(bytes), "mimeType": mime })
}

/// Run the server until stdin closes.
pub fn serve() -> Result<()> {
    let mut server = Server { gpu: None, open: HashMap::new(), recording: None };
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                send(&json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {e}") } }));
                continue;
            }
        };
        // a batch (2025-03-26) or one message
        let msgs = match msg {
            Value::Array(list) => list,
            m => vec![m],
        };
        for m in msgs {
            if let Some(reply) = server.handle(&m) {
                send(&reply);
            }
        }
    }
    if let Some(rec) = server.recording.take() {
        // the client went away mid-recording: keep what was recorded as a project
        let _ = rec.finish(None, true);
    }
    Ok(())
}

fn send(v: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{v}");
    let _ = out.flush();
}

impl Server {
    /// Answer one JSON-RPC message (`None` for notifications and responses).
    fn handle(&mut self, m: &Value) -> Option<Value> {
        let method = m.get("method")?.as_str()?;
        let id = m.get("id").cloned();
        let params = m.get("params").cloned().unwrap_or(json!({}));
        let result: Result<Value, (i64, String)> = match method {
            "initialize" => {
                let asked = params["protocolVersion"].as_str().unwrap_or(VERSIONS[0]);
                let version = VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(VERSIONS[0]);
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "screenforge", "title": "ScreenForge", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": INSTRUCTIONS,
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tool_list() })),
            "tools/call" => {
                let name = params["name"].as_str().unwrap_or_default().to_string();
                let args = params.get("arguments").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
                let progress = params["_meta"]["progressToken"].clone();
                Ok(match self.call(&name, args, &progress) {
                    Ok(content) => json!({ "content": content, "isError": false }),
                    Err(e) => json!({ "content": [text(format!("{e:#}"))], "isError": true }),
                })
            }
            "resources/list" => Ok(json!({ "resources": [] })),
            "prompts/list" => Ok(json!({ "prompts": [] })),
            _ if id.is_none() => return None, // notifications/initialized, cancelled …
            _ => Err((-32601, format!("method not found: {method}"))),
        };
        let id = id?;
        Some(match result {
            Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
        })
    }

    fn gpu(&mut self) -> Result<Gpu> {
        if self.gpu.is_none() {
            self.gpu = Some(Gpu::headless()?);
        }
        Ok(self.gpu.clone().expect("gpu"))
    }

    /// The project at `path`, opened (again) when it is not open yet or changed on disk.
    fn project(&mut self, path: &Path) -> Result<&mut Open> {
        let now = stamp(path);
        if now.is_none() {
            return Err(anyhow!("no ScreenForge project at {} (a .screenforge package; list_projects shows recent ones)", path.display()));
        }
        let fresh = self.open.get(path).is_some_and(|o| o.stamp == now);
        if !fresh {
            let gpu = self.gpu()?;
            let session = Session::open(path, gpu).with_context(|| format!("opening {}", path.display()))?;
            let host = Arc::new(CliHost::new(&session));
            // keep the undo history across a reload: the edits are still ours
            let earlier = self.open.remove(path).map(|o| o.earlier).unwrap_or_default();
            self.open.insert(path.to_path_buf(), Open { session, host, stamp: now, earlier });
        }
        Ok(self.open.get_mut(path).expect("open"))
    }

    fn call(&mut self, name: &str, mut args: Value, progress: &Value) -> Result<Vec<Value>> {
        match name {
            "list_projects" => Ok(vec![text(list_projects(args["folder"].as_str().map(expand))?.to_string())]),
            "list_capture_sources" => Ok(vec![text(record::sources().to_string())]),
            "start_recording" => {
                if let Some(r) = &self.recording {
                    return Err(anyhow!("already recording: {} (stop_recording or cancel_recording first)", r.what));
                }
                let req = record::Request {
                    display: args["display_id"].as_u64().map(|d| d as u32),
                    window: args["window"].as_str().map(str::to_string).or_else(|| args["window"].as_u64().map(|w| w.to_string())),
                    microphone: args["microphone"].as_str().map(str::to_string),
                    system_audio: args["system_audio"].as_bool().unwrap_or(false),
                    input: args["capture_input"].as_bool().unwrap_or(true),
                    name: args["name"].as_str().map(str::to_string),
                };
                let rec = record::Active::start(&req)?;
                let msg = format!("Recording {} as \"{}\". Call stop_recording when the user is done (or pause_recording / resume_recording).", rec.what, rec.name);
                self.recording = Some(rec);
                Ok(vec![text(msg)])
            }
            "recording_status" => Ok(vec![text(self.recording.as_ref().map(|r| r.status()).unwrap_or_else(|| json!({ "recording": false })).to_string())]),
            "pause_recording" | "resume_recording" | "add_recording_marker" => {
                let rec = self.recording.as_mut().ok_or_else(|| anyhow!("not recording"))?;
                match name {
                    "pause_recording" => rec.pause()?,
                    "resume_recording" => rec.resume()?,
                    _ => rec.marker(),
                }
                Ok(vec![text(rec.status().to_string())])
            }
            "stop_recording" => {
                let rec = self.recording.take().ok_or_else(|| anyhow!("not recording"))?;
                let secs = rec.elapsed_ms() / 1000.0;
                let path = rec.finish(args["folder"].as_str().map(expand).as_deref(), args["auto_zooms"].as_bool().unwrap_or(true))?;
                Ok(vec![text(json!({ "project": path, "seconds": (secs * 10.0).round() / 10.0 }).to_string())])
            }
            "cancel_recording" => {
                let rec = self.recording.take().ok_or_else(|| anyhow!("not recording"))?;
                rec.cancel();
                Ok(vec![text("Recording deleted.")])
            }
            "import_video" => {
                let src = args["source"].as_str().ok_or_else(|| anyhow!("`source` is required"))?;
                let src = if sf_recording::loom::is_link(src) { src.to_string() } else { expand(src).to_string_lossy().into_owned() };
                let path = record::import_video(&src, args["folder"].as_str().map(expand).as_deref())?;
                Ok(vec![text(json!({ "project": path }).to_string())])
            }
            _ => {
                let path = expand(args["project"].as_str().ok_or_else(|| anyhow!("`project` (the path of a .screenforge package) is required"))?);
                if let Value::Object(m) = &mut args {
                    m.remove("project");
                }
                match name {
                    "render_frame" => self.render_frame(&path, &args),
                    "export" => self.export(&path, &args, progress),
                    _ => self.edit(&path, name, &args),
                }
            }
        }
    }

    /// One of the app assistant's tools.
    fn edit(&mut self, path: &Path, name: &str, args: &Value) -> Result<Vec<Value>> {
        if APP_ONLY.contains(&name) || !sf_agent::tools::definitions().as_array().is_some_and(|l| l.iter().any(|t| t["name"] == name)) {
            return Err(anyhow!("unknown tool {name}"));
        }
        let open = self.project(path)?;
        let mut ws = host::workspace(&open.session, open.host.clone());
        ws.earlier = open.earlier.clone();
        let before = ws.project.clone();
        let outcome = ws.run(name, args).map_err(|e| anyhow!(e))?;
        let mut content = vec![text(format!("{}\n{}", outcome.summary, outcome.result))];
        content.extend(outcome.images.iter().map(|jpeg| image(jpeg, "image/jpeg")));
        if outcome.changed {
            if name == "undo_answers" {
                open.earlier = ws.earlier.clone();
            } else {
                open.earlier.push(before);
            }
            open.session.pkg.project = ws.project.clone();
            open.session.pkg.save()?;
            open.session.rebuild();
            open.stamp = stamp(path);
            if FILE_TOOLS.contains(&name) {
                open.stamp = None; // read again next time, with the new files
            }
        }
        for action in std::mem::take(&mut ws.actions) {
            if let sf_agent::Action::Transcribe { language } = action {
                let n = transcribe(path, language)?;
                self.open.get_mut(path).expect("open").stamp = None;
                content.push(text(format!("Transcribed on this computer: {n} words. get_transcript has them now.")));
            }
        }
        Ok(content)
    }

    /// The finished look at an output time, as a JPEG.
    fn render_frame(&mut self, path: &Path, args: &Value) -> Result<Vec<Value>> {
        let at_ms = args["at_ms"].as_f64().unwrap_or(0.0).max(0.0);
        let height = args["height"].as_u64().unwrap_or(720).clamp(144, 2160) as u32;
        let gpu = self.gpu()?;
        let open = self.project(path)?;
        let session = &mut open.session;
        let at_ms = at_ms.min((session.duration_ms() - 1.0).max(0.0));
        let size = session.output_size(height);
        // warm the springs before the requested time so the frame looks like playback
        let mut t = (at_ms - 2000.0).max(0.0);
        while t < at_ms {
            session.frame_desc_at(t, size)?;
            t += 1000.0 / 60.0;
        }
        let mut comp = Compositor::new(gpu);
        session.render(&mut comp, at_ms, size)?;
        let px = comp.read_pixels()?;
        let rgba = image::RgbaImage::from_raw(size.width, size.height, px).ok_or_else(|| anyhow!("bad frame"))?;
        let rgb = image::DynamicImage::ImageRgba8(rgba).to_rgb8();
        let mut jpeg = vec![];
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85).encode_image(&rgb)?;
        Ok(vec![text(format!("Frame at {} ({}x{})", sf_agent::tools::clock(at_ms), size.width, size.height)), image(&jpeg, "image/jpeg")])
    }

    fn export(&mut self, path: &Path, args: &Value, progress: &Value) -> Result<Vec<Value>> {
        let codec = match args["format"].as_str().unwrap_or("mp4") {
            "mp4" | "h264" => sf_media::VideoCodec::H264,
            "hevc" => sf_media::VideoCodec::Hevc,
            "prores" | "mov" => sf_media::VideoCodec::ProRes4444,
            "gif" => sf_media::VideoCodec::Gif,
            "webm" | "vp9" => sf_media::VideoCodec::Vp9,
            f => return Err(anyhow!("unknown format {f} (mp4, hevc, prores, gif, webm)")),
        };
        let ext = match codec {
            sf_media::VideoCodec::ProRes4444 => "mov",
            sf_media::VideoCodec::Gif => "gif",
            sf_media::VideoCodec::Vp9 => "webm",
            _ => "mp4",
        };
        let gpu = self.gpu()?;
        let open = self.project(path)?;
        let output = match args["out"].as_str() {
            Some(o) => expand(o),
            None => path.with_extension(ext),
        };
        let range = args["range"].as_array().filter(|r| r.len() == 2).and_then(|r| Some((r[0].as_f64()?, r[1].as_f64()?)));
        let opts = sf_export::ExportOptions {
            output,
            fps: args["fps"].as_f64().unwrap_or(60.0).clamp(10.0, 120.0),
            max_height: args["height"].as_u64().unwrap_or(1080).clamp(144, 4320) as u32,
            codec,
            quality: args["quality"].as_f64().unwrap_or(0.85).clamp(0.1, 1.0),
            include_audio: !args["no_audio"].as_bool().unwrap_or(false),
            range_ms: range,
        };
        let mut comp = Compositor::new(gpu);
        let started = std::time::Instant::now();
        let result = sf_export::export(&mut open.session, &mut comp, &opts, |p| {
            if !progress.is_null() && (p.frame % 60 == 0 || p.frame == p.total_frames) {
                send(&json!({ "jsonrpc": "2.0", "method": "notifications/progress", "params": { "progressToken": progress, "progress": p.frame, "total": p.total_frames, "message": "Exporting" } }));
            }
        })?;
        let details = sf_engine::details::template(&sf_export::details_input(&open.session, opts.range_ms));
        let dtxt = sf_export::details_txt_path(&result);
        std::fs::write(&dtxt, details.to_text())?;
        Ok(vec![text(json!({ "file": result, "seconds": started.elapsed().as_secs(), "details": dtxt }).to_string())])
    }
}

/// Transcribe the project with Whisper on this computer: the number of words.
fn transcribe(path: &Path, language: Option<String>) -> Result<usize> {
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let opts = sf_transcribe::Options { language, prompt: None };
    let files = sf_transcribe::transcribe_project(path, sf_transcribe::Model::from_id("base").expect("base model"), &opts, cancel, |_| {})?;
    Ok(files.iter().map(|(_, n)| n).sum())
}

/// Projects in `folder` (default: ~/ScreenForge Projects), newest first.
fn list_projects(folder: Option<PathBuf>) -> Result<Value> {
    let folder = folder.unwrap_or_else(sf_session::create::default_projects_folder);
    let mut list: Vec<(SystemTime, Value)> = std::fs::read_dir(&folder)
        .with_context(|| format!("reading {}", folder.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == sf_project::PACKAGE_EXTENSION))
        .filter_map(|p| {
            let modified = stamp(&p)?;
            let pkg = sf_project::ProjectPackage::open(&p).ok()?;
            let duration = pkg.project.scenes.first().map(|s| sf_engine::timeline::Timeline::from_scene(s).duration_ms()).unwrap_or(0.0);
            let saved = chrono::DateTime::<chrono::Local>::from(modified).format("%Y-%m-%d %H:%M").to_string();
            Some((modified, json!({ "project": p, "name": pkg.project.name, "seconds": (duration / 100.0).round() / 10.0, "saved": saved })))
        })
        .collect();
    list.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    list.truncate(50);
    Ok(json!({ "folder": folder, "projects": list.into_iter().map(|(_, v)| v).collect::<Vec<_>>() }))
}

/// Tool definitions in MCP's shape: the app assistant's editing tools (each with `project`
/// added) and the tools only this server has.
pub fn tool_list() -> Vec<Value> {
    let project = json!({ "type": "string", "description": "Path of the .screenforge project package" });
    let mut tools: Vec<Value> = sf_agent::tools::definitions()
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|t| !APP_ONLY.contains(&t["name"].as_str().unwrap_or_default()))
        .map(|t| {
            let name = t["name"].as_str().unwrap_or_default().to_string();
            let mut schema = t["input_schema"].clone();
            schema["properties"]["project"] = project.clone();
            let mut required: Vec<Value> = schema["required"].as_array().cloned().unwrap_or_default();
            required.insert(0, json!("project"));
            schema["required"] = Value::Array(required);
            let description = match name.as_str() {
                "transcribe" => "Transcribe what was said with Whisper on this computer (downloads the model the first time; a minute or more for a long recording). Replaces an existing transcript; get_transcript and captions use it.".to_string(),
                "undo_answers" => "Undo your own earlier edits to this project made through this server: the project goes back to how it was before the last `count` edits (default 1), or before the first with all: true.".to_string(),
                _ => t["description"].as_str().unwrap_or_default().to_string(),
            };
            let read_only = matches!(name.as_str(), "get_project" | "get_transcript" | "get_activity" | "get_motion" | "view_frames" | "get_config" | "get_options" | "get_chapters");
            json!({ "name": name, "description": description, "inputSchema": schema, "annotations": { "readOnlyHint": read_only } })
        })
        .collect();
    let folder = json!({ "type": "string", "description": "Projects folder (default ~/ScreenForge Projects)" });
    let extra = json!([
        {
            "name": "list_projects",
            "description": "Recent ScreenForge projects (newest first) with their paths, names and lengths.",
            "inputSchema": { "type": "object", "properties": { "folder": folder } },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "list_capture_sources",
            "description": "What can be recorded: displays, on-screen windows and microphones, and whether the recording permissions are granted.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "start_recording",
            "description": "Start recording the screen (the main display unless a display or window is given), with the mouse and keyboard log that automatic zooms use. Recording continues until stop_recording; the user does what they want to show in between. Returns at once.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "display_id": { "type": "integer", "description": "From list_capture_sources" },
                    "window": { "type": "string", "description": "A window id from list_capture_sources, or a word of its title or app name; the area around the window is recorded and follows it" },
                    "microphone": { "type": "string", "description": "A microphone id or name, or \"default\"; omit for no microphone" },
                    "system_audio": { "type": "boolean", "description": "Record the computer's sound too" },
                    "capture_input": { "type": "boolean", "description": "Record clicks, keys and the cursor (default true)" },
                    "name": { "type": "string", "description": "Project name" }
                }
            }
        },
        {
            "name": "recording_status",
            "description": "Whether a recording is running, and for how long.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "readOnlyHint": true }
        },
        { "name": "pause_recording", "description": "Pause the recording.", "inputSchema": { "type": "object", "properties": {} } },
        { "name": "resume_recording", "description": "Resume a paused recording.", "inputSchema": { "type": "object", "properties": {} } },
        { "name": "add_recording_marker", "description": "Mark this moment of the recording (it becomes a chapter marker in the project).", "inputSchema": { "type": "object", "properties": {} } },
        {
            "name": "stop_recording",
            "description": "Stop recording and make a ScreenForge project of it. Returns the project path to edit.",
            "inputSchema": { "type": "object", "properties": { "auto_zooms": { "type": "boolean", "description": "Add zooms around the clicks (default true)" }, "folder": folder } }
        },
        { "name": "cancel_recording", "description": "Stop recording and delete it.", "inputSchema": { "type": "object", "properties": {} }, "annotations": { "destructiveHint": true } },
        {
            "name": "import_video",
            "description": "Make a new project from a video file (.mp4, .mov …), a Screen Studio project (.screenstudio, copied; the original is untouched) or a public Loom link. Returns the project path.",
            "inputSchema": { "type": "object", "properties": { "source": { "type": "string", "description": "File path or loom.com/share link" }, "folder": folder }, "required": ["source"] }
        },
        {
            "name": "render_frame",
            "description": "Render the finished video at a timeline time (background, padding, zoom, cursor, captions, titles, camera) and return it as an image. Use it to check how an edit looks.",
            "inputSchema": { "type": "object", "properties": { "project": project, "at_ms": { "type": "number" }, "height": { "type": "integer", "description": "Image height, default 720" } }, "required": ["project", "at_ms"] },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "export",
            "description": "Export the edited video to a file (default: next to the project, named after it). Takes about as long as the video for 1080p60. Also writes <name>.details.txt with a title, description and tags.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": project,
                    "out": { "type": "string", "description": "Output file path" },
                    "format": { "type": "string", "enum": ["mp4", "hevc", "prores", "gif", "webm"], "description": "Default mp4 (H.264)" },
                    "height": { "type": "integer", "description": "Default 1080" },
                    "fps": { "type": "number", "description": "Default 60" },
                    "quality": { "type": "number", "description": "0.1–1, default 0.85" },
                    "no_audio": { "type": "boolean" },
                    "range": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2, "description": "[start_ms, end_ms] of the timeline only" }
                },
                "required": ["project"]
            }
        }
    ]);
    tools.extend(extra.as_array().cloned().unwrap_or_default());
    tools
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_are_valid_and_unique() {
        let tools = tool_list();
        let mut names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        let n = names.len();
        names.sort();
        names.dedup();
        assert_eq!(n, names.len(), "duplicate tool names");
        for t in &tools {
            assert_eq!(t["inputSchema"]["type"], "object", "{}", t["name"]);
            assert!(t["description"].as_str().is_some_and(|d| !d.is_empty()));
            assert!(t.get("input_schema").is_none());
        }
        for app_only in APP_ONLY {
            assert!(!names.contains(&app_only));
        }
        let cut = tools.iter().find(|t| t["name"] == "cut").unwrap();
        assert_eq!(cut["inputSchema"]["required"][0], "project");
    }

    #[test]
    fn handshake_and_errors() {
        let mut s = Server { gpu: None, open: HashMap::new(), recording: None };
        let r = s.handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } } })).unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
        assert!(s.handle(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).is_none());
        let r = s.handle(&json!({ "jsonrpc": "2.0", "id": 2, "method": "nope" })).unwrap();
        assert_eq!(r["error"]["code"], -32601);
        let r = s.handle(&json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "cut", "arguments": { "project": "/nonexistent.screenforge", "ranges": [] } } })).unwrap();
        assert_eq!(r["result"]["isError"], true);
    }
}
