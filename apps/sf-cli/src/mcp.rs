//! `sf mcp`: a Model Context Protocol server on stdin/stdout, so the user's own agent (Claude
//! Code, Codex, Cursor …) can record and edit ScreenForge projects. The editing tools are the
//! same ones the app's assistant uses (`sf_agent::tools`), plus recording, importing, rendering
//! a frame and exporting: no AI runs for them and ScreenForge spends no tokens. `ask_editor`
//! is the exception: it hands a whole job, described in plain words, to the app's own AI editor
//! (`sf_agent::run_turn`), so the exploring stays here and the calling agent only reads a short
//! report. That one runs on the user's plan, as the agent panel does. Of the rest everything is
//! free except `export`, which has the app's pay gate ([`export_allowed`]): the computer's free
//! exports, then a license. An export is uploaded and answered with its link.
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

use sf_services::host::LocalHost;

use crate::host;
use crate::record;

/// Protocol versions this server speaks, newest first.
const VERSIONS: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// App tools that make no sense outside the editor (the playhead, its export dialog).
const APP_ONLY: [&str; 2] = ["seek", "open_export"];

/// Tools that write files into the package besides project.json: the session is opened again
/// after them so it sees the new files.
const FILE_TOOLS: [&str; 9] = ["add_narration", "update_narration", "add_overlay", "import_file", "apply_preset", "add_music", "set_music", "use_photo", "use_music"];

const INSTRUCTIONS: &str = "ScreenForge records the screen and edits screen recordings into polished videos (backgrounds, zooms, cuts, speed, captions, camera, titles, narration). \
A project is a .screenforge package (a folder); pass its path as `project` to every editing tool. Edits are saved immediately and show up live in the ScreenForge app if the project is open there.

Getting a project: list_projects (recent ones in ~/ScreenForge Projects), start_recording + stop_recording (records the screen; the user does what they want to show in between), or import_video (an .mp4/.mov file, a .screenstudio project, or a public Loom link).

The short way: ask_editor does a whole job from a description (\"make this a 30 s vertical TikTok with narration by Eric, a mountain photo behind and music from my favourites\"). ScreenForge's own AI editor looks through the recording, edits, narrates and styles it, and answers with a short report, so the transcript, the activity log and the frames never enter your context. Use it first for anything that would take more than two or three editing calls, check with render_frame, then export. It runs on the user's ScreenForge plan; when that does not include the AI editor the tool says so, and you edit with the tools below, which are free.

How to edit yourself:
- Call get_project first, and again after the user may have edited by hand. Tool times are milliseconds on the edited timeline (what plays), not in the raw recording.
- Find moments with get_transcript (needs transcribe first), get_activity (typing, clicks, idle from the input log), get_motion and view_frames (stills of the raw screen).
- cut takes every range of one edit in a single call (ranges refer to the timeline before the call). Leave about 100 ms around neighbouring words when cutting speech.
- Zooms, masks and camera layouts each sit on one lane and cannot overlap. Zoom gently: 1.3–1.5× for text and typing, about 2× only for a small detail; a few seconds each.
- Settings (background, padding, corners, shadow, cursor, camera, captions, audio, aspect ratio) are changed with patch_project; get_options lists the values and where each setting lives, get_config shows current values.
- \"Auto edit\" / tighten: with a transcript cut long pauses and filler words; speed up stretches where nothing is said or clicked (2–4×); add zooms around the clicks (add_auto_zooms) if there are none.
- render_frame shows the finished look (background, zoom, captions) at a time: use it to check your work. export writes the video file, uploads it and returns its link (a page anyone can watch: give it to the user); pass it a title, description and tags you write (ScreenForge has no model of its own here to write them). Export is the one paid step: a computer without a license has a few free exports, and when they are used the tool says how to get Pro or Creator; tell the user that rather than trying again.
- undo_answers reverts your earlier edits in this session.";

/// A project the server has open.
struct Open {
    session: Session,
    host: Arc<LocalHost>,
    /// project.json's modification time when it was last read or written here.
    stamp: Option<SystemTime>,
    /// The project before each edit made through this server, for `undo_answers`.
    earlier: Vec<Project>,
    /// The chat with the app's AI editor (`ask_editor`), so a follow-up request continues it.
    chat: sf_agent::Conversation,
}

/// What the AI editor is told in place of the app's playhead and selection.
const EDITOR_CONTEXT: &str = "Asked by the user's own agent through ScreenForge's MCP server, not in the app: there is no playhead and nothing is selected. Nobody can answer a question during this turn, so make the sensible choice yourself and say what you chose. The agent that asked exports the video afterwards; do not open the export dialog.";

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
    // the ElevenLabs voices on offer (and the user's studio voice), as the app loads them
    sf_services::narration::load_cloud_voices();
    sf_services::my_voice::load_studio();
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
            let host = Arc::new(host::new_host(&session));
            // keep the undo history across a reload (the edits are still ours), and what the
            // last searches found
            let before = self.open.remove(path);
            if let Some(o) = &before {
                host.keep_results(&o.host);
            }
            let (earlier, chat) = before.map(|o| (o.earlier, o.chat)).unwrap_or_default();
            self.open.insert(path.to_path_buf(), Open { session, host, stamp: now, earlier, chat });
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
                    "ask_editor" => self.ask_editor(&path, &args, progress),
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

    /// Hand a request in plain words to the app's AI editor and answer with its report: what it
    /// said, the steps it took and the new length. The project is saved after each turn. When
    /// the editor asks for a transcript, the recording is transcribed here and the editor is
    /// asked once more to carry on, since nobody else would ask it again.
    fn ask_editor(&mut self, path: &Path, args: &Value, progress: &Value) -> Result<Vec<Value>> {
        let request = args["request"].as_str().map(str::trim).filter(|r| !r.is_empty()).ok_or_else(|| anyhow!("`request` is required: what the video should become, in plain words"))?.to_string();
        let device = sf_license::device_id().ok_or_else(|| anyhow!("ScreenForge cannot identify this computer, so its AI editor is not available. Edit with the other tools."))?;
        let route = sf_agent::Route { url: format!("{}/functions/v1/agent", sf_license::SUPABASE_URL), anon_key: sf_license::SUPABASE_ANON_KEY.into(), device };
        // the model chosen in the app's agent panel
        let settings = sf_license::key_file().map(|p| sf_agent::Settings::load(&p.with_file_name("agent.json"))).unwrap_or_default();
        if args["new_chat"].as_bool().unwrap_or(false) {
            self.project(path)?.chat = sf_agent::Conversation::default();
        }
        let (mut said, mut steps, mut failed): (Vec<String>, Vec<String>, Vec<String>) = (vec![], vec![], vec![]);
        let (mut stopped, mut credits, mut changed) = (None, Value::Null, false);
        let mut ask = request.clone();
        for round in 0..2 {
            let open = self.project(path)?;
            let mut ws = host::workspace(&open.session, open.host.clone());
            ws.earlier = open.earlier.clone();
            let before = ws.project.clone();
            let (mut edited, mut transcript) = (false, None);
            let chat = sf_agent::run_turn(&route, &settings, std::mem::take(&mut open.chat), &mut ws, &ask, EDITOR_CONTEXT, &std::sync::atomic::AtomicBool::new(false), |e| match e {
                sf_agent::Event::Text(t) => said.push(t),
                sf_agent::Event::Step { summary, failed: true } => failed.push(summary),
                sf_agent::Event::Step { summary, .. } => {
                    if !progress.is_null() {
                        send(&json!({ "jsonrpc": "2.0", "method": "notifications/progress", "params": { "progressToken": progress, "progress": steps.len() + 1, "message": summary } }));
                    }
                    steps.push(summary);
                }
                sf_agent::Event::Changed(_) => edited = true,
                sf_agent::Event::Action(sf_agent::Action::Transcribe { language }) => transcript = Some(language),
                sf_agent::Event::Credits(c) => credits = c,
                sf_agent::Event::Error(e) => stopped = Some(e),
                _ => {}
            });
            open.chat = chat;
            if edited {
                if ws.earlier.len() < open.earlier.len() {
                    open.earlier = ws.earlier.clone(); // the editor undid earlier answers
                } else {
                    open.earlier.push(before);
                }
                open.session.pkg.project = ws.project.clone();
                open.session.pkg.save()?;
                open.session.rebuild();
                open.stamp = None; // narration, photos and music are new files: read again
                changed = true;
            }
            let Some(language) = transcript.filter(|_| round == 0 && stopped.is_none()) else { break };
            let n = transcribe(path, language)?;
            self.open.get_mut(path).expect("open").stamp = None;
            steps.push(format!("Transcribed on this computer: {n} words"));
            ask = format!("The recording is transcribed now ({n} words; get_transcript has them). Carry on with the request: {request}");
        }
        if let (Some(e), false, true) = (&stopped, changed, steps.is_empty()) {
            // refused before anything ran (the plan does not include the editor, no connection …)
            return Err(anyhow!("ScreenForge's AI editor did not run: {e} Edit with the other tools instead; they need no plan."));
        }
        let open = self.project(path)?;
        let mut out = json!({ "answer": said.last().cloned().unwrap_or_default(), "steps": steps, "changed": changed, "durationMs": open.session.duration_ms().round() });
        if !failed.is_empty() {
            out["failedSteps"] = json!(failed);
        }
        if let Some(e) = stopped {
            out["stopped"] = json!(e);
        }
        if let Some(left) = credits["credits"].as_f64() {
            // only the balance: the account and the price list are the app's business
            out["aiCreditsLeft"] = json!(left.floor());
        }
        out["next"] = json!("Check the look with render_frame, ask again here for changes (the chat continues), then export.");
        Ok(vec![text(out.to_string())])
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
        // the pay gate first: nothing is rendered for an export that may not be made
        let free = export_allowed()?;
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
        let input = sf_export::details_input(&open.session, opts.range_ms);
        let mut details = written_details(args, &input);
        // who the licences ask to be credited: under the description, and in the answer
        let credits = credit_lines(&open.session.pkg.project, path);
        details.description = with_credits(&details.description, &credits);
        let dtxt = sf_export::details_txt_path(&result);
        std::fs::write(&dtxt, details.to_text())?;
        let mut out = json!({ "file": result, "file_url": file_url(&result), "seconds": started.elapsed().as_secs(), "details": dtxt });
        if !credits.is_empty() {
            out["credits"] = json!(credits);
            out["credits_note"] = json!("The music and photo licences ask for these lines wherever the video is published: give them to the user and put them in the caption or description as they are.");
        }
        if free {
            // counted once the file is written (kept on this computer until the server hears)
            match sf_license::record_free_export() {
                Ok(left) => out["free_exports_left"] = json!(left),
                Err(e) => tracing::warn!("free export not counted yet: {e}"),
            }
        }
        // the link: the page that plays the video, which takes the MP4 every browser plays
        let share = args["share"].as_bool().unwrap_or(true);
        if share && codec != sf_media::VideoCodec::H264 {
            out["link_note"] = json!("No link: a share link plays MP4 (H.264). Export with format mp4 to get one.");
        } else if share {
            let meta = sf_services::share::Meta { title: details.title.clone(), description: details.description.clone(), tags: details.tags.clone() };
            let uploading = |f: f32| {
                if !progress.is_null() {
                    send(&json!({ "jsonrpc": "2.0", "method": "notifications/progress", "params": { "progressToken": progress, "progress": (f * 100.0).round(), "total": 100, "message": "Uploading" } }));
                }
            };
            match sf_services::share::upload_file(path, &result, &meta, &std::sync::atomic::AtomicBool::new(false), &uploading) {
                Ok(shared) => {
                    out["link"] = json!(shared.url);
                    if let Some(v) = shared.video {
                        out["video_url"] = json!(v);
                    }
                }
                // the file is written all the same
                Err(e) => out["link_error"] = json!(format!("The video was exported, but no link was made: {e}")),
            }
        }
        if !details.by_ai {
            out["note"] = json!(if details.description.trim().is_empty() {
                "The details file has no description (there is no transcript to start one from). Write a title, description and tags from what the video shows and call export again with them."
            } else {
                "The details file is a template: the opening of the transcript and words from it. Pass title, description and tags to export for better ones."
            });
        }
        Ok(vec![text(out.to_string())])
    }
}

/// The export pay gate, as the app's Export dialog has it: a license on this computer exports
/// (`Ok(false)`); without one the computer has [`sf_license::FREE_EXPORTS`] free exports, counted
/// by the server (`Ok(true)`: this export is one of them, to be recorded when it is written).
/// The error says how to keep exporting.
fn export_allowed() -> Result<bool> {
    let mut license = sf_license::saved_status();
    if matches!(license, sf_license::Status::Expired(_)) {
        // a monthly key that renewed since the app last ran
        let _ = sf_license::check_in();
        license = sf_license::saved_status();
    }
    if license.is_valid() {
        return Ok(false);
    }
    match sf_license::free_exports_left() {
        Ok(left) if left > 0 => Ok(true),
        Ok(_) => Err(anyhow!(
            "Export needs a license: the {} free exports on this computer are used. Pro ({}) exports without limit and includes the AI agent in the app; Creator ({}) exports without limit. The user can buy either at {} or in the ScreenForge app (Export), sign in there with the email used at checkout, then export again. Recording and editing stay free.",
            sf_license::FREE_EXPORTS,
            sf_license::PRICE_PRO,
            sf_license::PRICE_LIFETIME,
            sf_license::PURCHASE_URL
        )),
        Err(e) => Err(anyhow!("Without a license, export needs the license server to count this computer's free exports, and it did not answer: {e}")),
    }
}

/// The credit lines the project's licences ask for: its Creative Commons music (Jamendo tracks
/// carry a credit file in the package) and an Unsplash background, as the app's sidebar shows.
fn credit_lines(project: &Project, package: &Path) -> Vec<String> {
    let p = serde_json::to_value(project).unwrap_or(Value::Null);
    let assets = package.join("assets");
    let mut files: Vec<String> = vec![];
    if !p["config"]["audio"]["muteBackgroundAudio"].as_bool().unwrap_or(false) {
        files.extend(p["config"]["audio"]["backgroundAudioFileName"].as_str().map(str::to_string));
    }
    // music parts that play their own track
    for clip in p["scenes"][0]["sfMusicClips"].as_array().into_iter().flatten() {
        files.extend(clip["track"].as_str().or(clip["fileName"].as_str()).map(str::to_string));
    }
    let mut lines: Vec<String> = vec![];
    for f in files {
        if let Some(c) = sf_services::music::credit(&assets, &f).map(|c| c.text()) {
            if !lines.contains(&c) {
                lines.push(c);
            }
        }
    }
    let bg = &p["config"]["styles"]["background"];
    if bg["type"] == "unsplash" && bg["unsplash"].is_object() {
        lines.push(sf_services::unsplash::credit(&bg["unsplash"]));
    }
    lines
}

/// `description` with the credits under it, leaving out one the writer already gave (told by
/// its opening, `Music: "Title" by Artist` or `Photo by Name`, since links may differ).
fn with_credits(description: &str, credits: &[String]) -> String {
    let mut out = description.trim_end().to_string();
    let missing: Vec<&String> = credits.iter().filter(|c| !description.contains(c.split(" (").next().unwrap_or(c))).collect();
    for (i, c) in missing.iter().enumerate() {
        out += if i == 0 && !out.is_empty() { "\n\n" } else if out.is_empty() { "" } else { "\n" };
        out += c;
    }
    out
}

/// `path` as a file:// link.
fn file_url(path: &Path) -> String {
    let p = path.to_string_lossy().replace('\\', "/");
    let mut url = String::from(if p.starts_with('/') { "file://" } else { "file:///" });
    for b in p.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' | b':' => url.push(b as char),
            _ => url.push_str(&format!("%{b:02X}")),
        }
    }
    url
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
            "name": "ask_editor",
            "description": "Hand a whole job to ScreenForge's own AI editor in plain words and get a short report back. It looks through the recording itself (transcript, typing and clicks, motion, frames), then cuts, speeds up, zooms, writes and reads the narration, picks the background and music, sets the aspect ratio, captions and titles. Use it first for anything that would take more than two or three editing calls (\"make this a 30 s vertical TikTok with narration by Eric, a mountain photo behind and music from my favourites\"): the exploring and the frames stay inside ScreenForge and not in your context, so it costs you far fewer tokens than editing call by call. Put everything the user asked for in `request` (length, format such as 9:16 for TikTok, voice, background, music, tone, what must stay in): the editor cannot ask back. Later calls continue the same chat (\"make the zooms gentler\"); new_chat: true starts over. It runs on the user's ScreenForge plan (the AI agent that comes with Pro, or the trial allowance): when the plan does not include it the tool says so, and you edit with the other tools, which stay free. It does not export: check with render_frame, then call export. Takes a minute or more.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": project,
                    "request": { "type": "string", "description": "What the video should become, in plain words, with every wish of the user in it" },
                    "new_chat": { "type": "boolean", "description": "Forget the earlier requests to the editor for this project (default false)" }
                },
                "required": ["project", "request"]
            }
        },
        {
            "name": "render_frame",
            "description": "Render the finished video at a timeline time (background, padding, zoom, cursor, captions, titles, camera) and return it as an image. Use it to check how an edit looks.",
            "inputSchema": { "type": "object", "properties": { "project": project, "at_ms": { "type": "number" }, "height": { "type": "integer", "description": "Image height, default 720" } }, "required": ["project", "at_ms"] },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "export",
            "description": "Export the edited video to a file (default: next to the project, named after it), upload it and return its link: `link` is the page anyone can watch (screenforge.studio/s/…, the same link every time the project is exported), `file` and `file_url` the file on this computer. Give the user the link. Takes about as long as the video for 1080p60, plus the upload. Export is the one paid step: without a license a computer has a few free exports (free_exports_left in the result), then the tool answers with how to get Pro or Creator. Also writes <name>.details.txt with a title, description and tags: write those yourself from what the video shows and pass them, since ScreenForge runs no AI model here. Leave music and photo credits out of your description: ScreenForge adds the lines the licences ask for (Creative Commons music, Unsplash photos) under it and returns them as `credits`; give those to the user as they are for the caption wherever the video is published.",
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
                    "share": { "type": "boolean", "description": "Upload the video and return its link (default true; mp4 only). false: only the file" },
                    "range": { "type": "array", "items": { "type": "number" }, "minItems": 2, "maxItems": 2, "description": "[start_ms, end_ms] of the timeline only" },
                    "title": { "type": "string", "description": "Title for the details file (e.g. a YouTube title, under 70 characters). Default: the project name" },
                    "description": { "type": "string", "description": "Description for the details file: a few sentences on what the video shows. The chapter list is added under it. Default: the transcript's opening, or empty without a transcript" },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "Up to 12 tags for the details file. Default: words from the transcript and title" }
                },
                "required": ["project"]
            }
        }
    ]);
    tools.extend(extra.as_array().cloned().unwrap_or_default());
    tools
}

/// The details file's contents: the agent's own title, description and tags where it gave
/// them, the template for the rest. The chapter list goes under the agent's description too.
fn written_details(args: &Value, input: &sf_engine::details::Input) -> sf_engine::details::Details {
    let mut d = sf_engine::details::template(input);
    let given = |k: &str| args[k].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    if let Some(title) = given("title") {
        d.title = title;
        d.by_ai = true;
    }
    if let Some(description) = given("description") {
        d.description = sf_engine::details::with_chapters(&description, &input.chapter_list);
        d.by_ai = true;
    }
    if let Some(tags) = args["tags"].as_array() {
        let mut list: Vec<String> = vec![];
        for t in tags.iter().filter_map(|t| t.as_str()).map(|t| t.trim().trim_start_matches('#').to_string()) {
            if !t.is_empty() && list.len() < sf_engine::details::MAX_TAGS && !list.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
                list.push(t);
            }
        }
        if !list.is_empty() {
            d.tags = list;
            d.by_ai = true;
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_details_replace_the_template() {
        let input = sf_engine::details::Input { name: "MCP demo".into(), transcript: String::new(), chapters: vec![], chapter_list: "00:00 Intro\n00:12 Export".into(), duration_ms: 24_000.0 };
        let plain = written_details(&json!({}), &input);
        assert!(!plain.by_ai);
        assert_eq!(plain.title, "MCP demo");
        let d = written_details(&json!({ "title": " Edit videos from Claude Code ", "description": "Claude records the screen.", "tags": ["#mcp", "MCP", "claude code", ""] }), &input);
        assert!(d.by_ai);
        assert_eq!(d.title, "Edit videos from Claude Code");
        assert_eq!(d.description, "Claude records the screen.\n\nChapters\n00:00 Intro\n00:12 Export");
        assert_eq!(d.tags, vec!["mcp", "claude code"]);
    }

    #[test]
    fn credits_go_under_the_description_once() {
        let credits = vec!["Music: \"Born Free\" by Pokki DJ (https://www.jamendo.com/track/1179546), licensed under CC BY 3.0".to_string(), "Photo by Ann (https://unsplash.com/@ann) on Unsplash (https://unsplash.com/photos/abc)".to_string()];
        assert_eq!(with_credits("A demo.", &credits), format!("A demo.\n\n{}\n{}", credits[0], credits[1]));
        assert_eq!(with_credits("", &credits[..1]), credits[0]);
        // a credit the writer gave already (other link) is not repeated
        assert_eq!(with_credits("A demo.\nPhoto by Ann on Unsplash", &credits), format!("A demo.\nPhoto by Ann on Unsplash\n\n{}", credits[0]));
        assert_eq!(with_credits("A demo.", &[]), "A demo.");
        // no music and a plain background: nothing to credit
        assert!(credit_lines(&Project::new("T"), Path::new("/nonexistent.screenforge")).is_empty());
    }

    #[test]
    fn file_links_escape_what_a_url_cannot_hold() {
        assert_eq!(file_url(Path::new("/home/me/ScreenForge Projects/My demo.mp4")), "file:///home/me/ScreenForge%20Projects/My%20demo.mp4");
        assert_eq!(file_url(Path::new("C:\\Users\\me\\a.mp4")), "file:///C:/Users/me/a.mp4");
    }

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
        // the AI editor needs a request and a project before anything is sent anywhere
        let r = s.handle(&json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "ask_editor", "arguments": { "project": "/nonexistent.screenforge", "request": " " } } })).unwrap();
        assert_eq!(r["result"]["isError"], true);
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("`request` is required"));
        assert!(tool_list().iter().any(|t| t["name"] == "ask_editor" && t["inputSchema"]["required"][1] == "request"));
        let r = s.handle(&json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "cut", "arguments": { "project": "/nonexistent.screenforge", "ranges": [] } } })).unwrap();
        assert_eq!(r["result"]["isError"], true);
    }
}
