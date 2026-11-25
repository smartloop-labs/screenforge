//! Recording without the app's window: `sf record` and the MCP recording tools. Records a
//! display or the area around a window, as the app's toolbar does, and makes a project of it.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use sf_project::schema::common::Rect;
use sf_recording::{permissions, CaptureTarget, Recorder, RecordingConfig};

/// What to record.
#[derive(Debug, Clone, Default)]
pub struct Request {
    /// A display id from [`sources`]; the main display when neither this nor a window is given.
    pub display: Option<u32>,
    /// A window id from [`sources`], or a word of its title or app name.
    pub window: Option<String>,
    /// A microphone id or name from [`sources`], or "default". `None`: no microphone.
    pub microphone: Option<String>,
    pub system_audio: bool,
    /// Record clicks, keys and the cursor (needs Input Monitoring on macOS).
    pub input: bool,
    /// Project name (default: "Recording <date time>").
    pub name: Option<String>,
}

/// A recording in progress.
pub struct Active {
    rec: Recorder,
    dir: PathBuf,
    pub name: String,
    pub what: String,
    started: Instant,
}

/// Where recordings are written before they become projects (the app's folder).
fn recordings_folder() -> PathBuf {
    directories::ProjectDirs::from("dev", "screenforge", "ScreenForge").map(|d| d.data_dir().join("Recordings")).unwrap_or_else(|| std::env::temp_dir().join("screenforge-recordings"))
}

fn status(s: permissions::PermissionStatus) -> &'static str {
    use permissions::PermissionStatus::*;
    match s {
        Granted => "granted",
        Denied => "denied",
        NotDetermined => "not asked yet",
        Unsupported => "not needed here",
    }
}

/// Displays, windows, microphones and the permissions recording needs.
pub fn sources() -> Value {
    let d = sf_recording::discover();
    json!({
        "displays": d.displays.iter().map(|x| json!({ "id": x.id, "name": x.name, "main": x.is_main, "width": x.bounds_points.width, "height": x.bounds_points.height })).collect::<Vec<_>>(),
        "windows": d.windows.iter().filter(|w| w.is_on_screen && !w.title.is_empty()).map(|w| json!({ "id": w.id, "app": w.app_name, "title": w.title, "width": w.bounds.width, "height": w.bounds.height })).collect::<Vec<_>>(),
        "microphones": d.microphones.iter().map(|m| json!({ "id": m.id, "name": m.name, "default": m.is_default })).collect::<Vec<_>>(),
        "permissions": {
            "screenRecording": status(permissions::screen_recording()),
            "inputMonitoring": status(permissions::input_monitoring()),
            "microphone": status(permissions::microphone()),
        },
        "note": "On macOS the permissions belong to the app that runs sf (your terminal, or the app hosting the MCP client).",
    })
}

impl Active {
    pub fn start(req: &Request) -> Result<Active> {
        let d = sf_recording::discover();
        let display = match req.display {
            Some(id) => d.display(id).ok_or_else(|| anyhow!("no display {id}"))?,
            None => d.main_display().context("no display found")?,
        };
        let (target, follow, what) = match req.window.as_deref().map(str::trim).filter(|w| !w.is_empty()) {
            Some(w) => {
                let needle = w.to_lowercase();
                let win = d
                    .windows
                    .iter()
                    .filter(|x| x.is_on_screen)
                    .find(|x| x.id.to_string() == w)
                    .or_else(|| d.windows.iter().filter(|x| x.is_on_screen && !x.title.is_empty()).find(|x| x.title.to_lowercase().contains(&needle) || x.app_name.to_lowercase().contains(&needle)))
                    .ok_or_else(|| anyhow!("no window matches {w:?}; list them with `sf sources`"))?;
                let b = win.bounds;
                let (cx, cy) = (b.x + b.width / 2.0, b.y + b.height / 2.0);
                let disp = d.displays.iter().find(|x| { let r = x.bounds_points; cx >= r.x && cx < r.x + r.width && cy >= r.y && cy < r.y + r.height }).unwrap_or(display);
                let r = disp.bounds_points;
                let (x0, y0) = (b.x.max(r.x), b.y.max(r.y));
                let (x1, y1) = ((b.x + b.width).min(r.x + r.width), (b.y + b.height).min(r.y + r.height));
                let w = ((x1 - x0).max(32.0) / 2.0).floor() * 2.0;
                let h = ((y1 - y0).max(32.0) / 2.0).floor() * 2.0;
                let rect = Rect { x: x0.round(), y: y0.round(), width: w, height: h };
                (CaptureTarget::Area { display_id: disp.id, rect_points: rect }, Some(win.id), format!("window \"{}\" ({})", win.title, win.app_name))
            }
            None => (CaptureTarget::Display { display_id: display.id }, None, format!("display \"{}\"", display.name)),
        };
        let name = req.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("Recording {}", chrono::Local::now().format("%Y-%m-%d at %H.%M.%S")));
        let dir = recordings_folder().join(format!("{} {}", sanitize(&name), chrono::Local::now().format("%Y%m%d-%H%M%S")));
        std::fs::create_dir_all(dir.parent().expect("recordings folder"))?;
        let mut cfg = RecordingConfig::new(&dir, target);
        cfg.microphone = req.microphone.clone();
        cfg.capture_system_audio = req.system_audio;
        cfg.follow_window = follow;
        cfg.capture_input = req.input;
        cfg.capture_keystrokes = req.input;
        let rec = Recorder::start(cfg).map_err(|e| match e {
            sf_recording::Error::PermissionDenied(p) => anyhow!("missing permission: {p}. On macOS, allow it for the app running sf (System Settings → Privacy & Security), then restart that app"),
            e => anyhow!(e),
        })?;
        Ok(Active { rec, dir, name, what, started: Instant::now() })
    }

    pub fn elapsed_ms(&self) -> f64 {
        self.rec.elapsed_ms()
    }

    pub fn status(&self) -> Value {
        json!({ "recording": true, "name": self.name, "what": self.what, "seconds": (self.elapsed_ms() / 100.0).round() / 10.0, "wallSeconds": self.started.elapsed().as_secs(), "paused": self.rec.state() == sf_recording::RecorderState::Paused })
    }

    pub fn pause(&mut self) -> Result<()> {
        Ok(self.rec.pause()?)
    }

    pub fn resume(&mut self) -> Result<()> {
        Ok(self.rec.resume()?)
    }

    pub fn marker(&mut self) {
        self.rec.add_marker();
    }

    /// Stop and make a project in `folder` (default: ~/ScreenForge Projects). Returns its path.
    pub fn finish(self, folder: Option<&Path>, auto_zooms: bool) -> Result<PathBuf> {
        let markers = self.rec.markers();
        self.rec.finish().context("finishing the recording")?;
        let opts = sf_session::create::CreateOptions {
            name: self.name.clone(),
            projects_folder: folder.map(Path::to_path_buf).unwrap_or_else(sf_session::create::default_projects_folder),
            default_config: None,
            create_automatic_zooms: auto_zooms,
            markers_ms: markers,
            device_mockup: false,
        };
        let pkg = sf_session::create::create_project_from_recording(&self.dir, opts)?;
        let _ = sf_session::preview::write_quicklook_preview(&pkg.path);
        Ok(pkg.path)
    }

    /// Stop and throw the recording away.
    pub fn cancel(self) {
        let dir = self.dir.clone();
        self.rec.cancel();
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn sanitize(name: &str) -> String {
    name.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '-' } else { c }).collect::<String>().trim().to_string()
}

/// A video file (or a public Loom link) as a new project in `folder`.
pub fn import_video(src: &str, folder: Option<&Path>) -> Result<PathBuf> {
    let folder = folder.map(Path::to_path_buf).unwrap_or_else(sf_session::create::default_projects_folder);
    if sf_recording::loom::is_link(src) {
        return crate::import_loom(src, &folder);
    }
    let path = PathBuf::from(src);
    if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("screenstudio")) {
        return Ok(sf_project::package::import_screen_studio(&path, &folder)?);
    }
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let probe = sf_recording::import::probe(&path)?;
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Video".into());
    let dir = recordings_folder().join(format!("Import {} {}", sanitize(&name), chrono::Local::now().format("%Y%m%d-%H%M%S")));
    sf_recording::import::import_video(&path, &dir, |_| {}, &cancel)?;
    let tall = probe.height as f64 >= probe.width as f64 * 1.6;
    let opts = sf_session::create::CreateOptions { name, projects_folder: folder, default_config: None, create_automatic_zooms: false, markers_ms: vec![], device_mockup: tall };
    let pkg = sf_session::create::create_project_from_recording(&dir, opts)?;
    let _ = sf_session::preview::write_quicklook_preview(&pkg.path);
    Ok(pkg.path)
}
