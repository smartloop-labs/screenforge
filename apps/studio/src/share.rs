//! Cloud share links (free, for every computer): a project rendered to MP4 on a background
//! thread, uploaded straight to the share storage (Cloudflare R2, through URLs the `share`
//! backend function signs) and answered with a `screenforge.studio/s/<id>` link. Export with "Make a share link too" uploads
//! its file. A project has one link: sharing it again replaces the video behind the same URL.
//! The upload itself is [`sf_services::share`], which `sf mcp` uses too.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use sf_media::VideoCodec;

pub use sf_services::share::{is_cancel, link_url, Meta, Shared};
use sf_services::share::{upload, upload_file, CANCELLED};

/// How the shared video is rendered. Always H.264 MP4, the one format every browser plays.
#[derive(Clone, Copy, Debug)]
pub struct ShareSettings {
    pub fps: f64,
    pub max_height: u32,
    /// 0..1, as `sf_export::ExportOptions::quality`
    pub quality: f64,
    pub include_audio: bool,
}

/// What a share uploads: the project rendered for it, or a file Export just wrote (an H.264
/// MP4, so it is not rendered twice).
#[derive(Clone, Debug)]
pub enum Source {
    Render(ShareSettings),
    File(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stage {
    Rendering(f32),
    Uploading(f32),
}

/// One link to copy from the export dialog: what it is, and the text that goes on the clipboard.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkRow {
    pub label: &'static str,
    pub text: String,
    pub tip: &'static str,
}

/// Everything a share can be pasted as: the page, the video file, Markdown with the poster
/// (GitHub, Notion, docs) and HTML to embed the player.
pub fn link_rows(s: &Shared, title: &str) -> Vec<LinkRow> {
    let title = title.trim();
    let alt = if title.is_empty() { "Watch the video" } else { title };
    let alt_md = alt.replace(['[', ']'], "");
    let esc = |t: &str| t.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;");
    let mut rows = vec![LinkRow { label: "Page", text: s.url.clone(), tip: "The page anyone can watch: paste it in a message, email or ticket" }];
    if let Some(v) = &s.video {
        rows.push(LinkRow { label: "Video file", text: v.clone(), tip: "The MP4 itself, for players and tools that take a video address" });
    }
    let md = match &s.poster {
        Some(p) => format!("[![{alt_md}]({p})]({})", s.url),
        None => format!("[{alt_md}]({})", s.url),
    };
    rows.push(LinkRow { label: "Markdown", text: md, tip: "For GitHub, Notion and docs: the poster image, linked to the page" });
    if let Some(v) = &s.video {
        let poster = s.poster.as_ref().map(|p| format!(" poster=\"{}\"", esc(p))).unwrap_or_default();
        rows.push(LinkRow { label: "Embed", text: format!("<video src=\"{}\"{poster} controls playsinline width=\"640\" title=\"{}\"></video>", esc(v), esc(alt)), tip: "HTML for a web page: the video plays right there" });
    }
    rows
}

enum Msg {
    Stage(Stage),
    Done(Result<Shared, String>),
}

/// A share in progress (or just finished) for one project.
pub struct ShareJob {
    pub project: PathBuf,
    pub title: String,
    pub stage: Stage,
    /// The links, or why there are none. Set once the job is over.
    pub result: Option<Result<Shared, String>>,
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
}

impl ShareJob {
    /// Render `project` (or take the exported file) and upload it on a background thread.
    /// `repaint` wakes the UI as the job moves on.
    pub fn start(project: PathBuf, meta: Meta, source: Source, repaint: egui::Context) -> ShareJob {
        let (tx, rx) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let title = meta.title.clone();
        let (path, name, stop) = (project.clone(), meta, cancel.clone());
        let first = if matches!(source, Source::File(_)) { Stage::Uploading(0.0) } else { Stage::Rendering(0.0) };
        std::thread::Builder::new()
            .name("share-link".into())
            .spawn(move || {
                let send = |m: Msg| {
                    let _ = tx.send(m);
                    repaint.request_repaint();
                };
                let r = run(&path, &name, source, &stop, &|s| send(Msg::Stage(s)));
                send(Msg::Done(r));
            })
            .expect("spawn share job");
        ShareJob { project, title, stage: first, result: None, rx, cancel }
    }

    /// Take the thread's news. True once, when the job has just finished.
    pub fn poll(&mut self) -> bool {
        let mut finished = false;
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Stage(s) => self.stage = s,
                Msg::Done(r) => {
                    self.result = Some(r);
                    finished = true;
                }
            }
        }
        finished
    }

    pub fn is_running(&self) -> bool {
        self.result.is_none()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Progress over the whole job: rendering is the first 60 %, uploading the rest.
    pub fn fraction(&self) -> f32 {
        match self.stage {
            Stage::Rendering(f) => 0.6 * f,
            Stage::Uploading(f) => 0.6 + 0.4 * f,
        }
    }

    /// What the job is doing now, shown under its progress bar.
    pub fn label(&self) -> String {
        match self.stage {
            Stage::Rendering(_) => "Rendering the video…".to_string(),
            Stage::Uploading(_) => "Uploading…".to_string(),
        }
    }
}

/// May this computer make share links? Every computer ScreenForge can identify may: a link
/// belongs to the export's computer, with or without a license.
pub fn allowed() -> bool {
    sf_license::device_id().is_some()
}

fn run(project: &Path, meta: &Meta, source: Source, cancel: &AtomicBool, stage: &dyn Fn(Stage)) -> Result<Shared, String> {
    let s = match source {
        // the exported file stays where the user saved it
        Source::File(video) => return upload_file(project, &video, meta, cancel, &|f| stage(Stage::Uploading(f))),
        Source::Render(s) => s,
    };
    let dir = directories::ProjectDirs::from("dev", "screenforge", "ScreenForge").map(|d| d.cache_dir().join("shares")).ok_or("no cache folder")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let video = dir.join(format!("{stamp}.mp4"));
    let result = (|| {
        let (width, height, duration_ms) = render(project, &video, s, cancel, stage)?;
        if cancel.load(Ordering::Relaxed) {
            return Err(CANCELLED.to_string());
        }
        stage(Stage::Uploading(0.0));
        upload(project, &video, meta, (width, height, duration_ms), cancel, &|f| stage(Stage::Uploading(f)))
    })();
    let _ = std::fs::remove_file(&video);
    result
}

/// Render `project` to `out` (H.264 MP4). Returns the video's width, height and length in ms.
fn render(project: &Path, out: &Path, s: ShareSettings, cancel: &AtomicBool, stage: &dyn Fn(Stage)) -> Result<(u32, u32, u64), String> {
    let go = || -> anyhow::Result<(u32, u32, u64)> {
        let gpu = sf_render::Gpu::headless()?;
        let mut session = sf_session::Session::open(project, gpu.clone())?;
        let mut comp = sf_render::Compositor::new(gpu);
        let size = session.output_size(s.max_height);
        let duration = session.duration_ms().max(0.0) as u64;
        let opts = sf_export::ExportOptions { output: out.to_path_buf(), fps: s.fps, max_height: s.max_height, codec: VideoCodec::H264, quality: s.quality, include_audio: s.include_audio, range_ms: None };
        sf_export::export_cancellable(&mut session, &mut comp, &opts, cancel, |p| stage(Stage::Rendering(p.frame as f32 / p.total_frames.max(1) as f32)))?;
        Ok((size.width, size.height, duration))
    };
    go().map_err(|e| {
        let e = format!("{e:#}");
        if e.contains("cancelled") {
            CANCELLED.to_string()
        } else {
            format!("Rendering the video failed: {e}")
        }
    })
}

/// Where "Make a share link too" is remembered.
fn setting_file() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "screenforge", "ScreenForge").map(|d| d.config_dir().join("share-with-export"))
}

/// Make a share link of every file Export writes. Off unless turned on.
pub fn with_export_enabled() -> bool {
    setting_file().and_then(|f| std::fs::read_to_string(f).ok()).is_some_and(|s| s.trim() == "on")
}

pub fn set_with_export(on: bool) {
    if let Some(f) = setting_file() {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let _ = std::fs::write(f, if on { "on" } else { "off" });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn job_progress_splits_render_and_upload() {
        let (_tx, rx) = channel();
        let mut job = ShareJob { project: PathBuf::new(), title: String::new(), stage: Stage::Rendering(0.5), result: None, rx, cancel: Arc::new(AtomicBool::new(false)) };
        assert!((job.fraction() - 0.3).abs() < 1e-6);
        job.stage = Stage::Uploading(1.0);
        assert!((job.fraction() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn link_rows_cover_page_file_markdown_and_embed() {
        let s = Shared { url: "https://screenforge.studio/s/abc".into(), video: Some("https://media.screenforge.studio/v/abc/video.mp4?v=2".into()), poster: Some("https://media.screenforge.studio/v/abc/poster.jpg?v=2".into()) };
        let rows = link_rows(&s, "My [demo] & more");
        let labels: Vec<_> = rows.iter().map(|r| r.label).collect();
        assert_eq!(labels, vec!["Page", "Video file", "Markdown", "Embed"]);
        assert_eq!(rows[2].text, "[![My demo & more](https://media.screenforge.studio/v/abc/poster.jpg?v=2)](https://screenforge.studio/s/abc)");
        assert!(rows[3].text.starts_with("<video src=\"https://media.screenforge.studio/v/abc/video.mp4?v=2\" poster="));
        assert!(rows[3].text.contains("title=\"My [demo] &amp; more\""));
        // only the page known: the page and a plain Markdown link
        let rows = link_rows(&Shared::page("https://screenforge.studio/s/abc".into()), "");
        assert_eq!(rows.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), vec!["https://screenforge.studio/s/abc", "[Watch the video](https://screenforge.studio/s/abc)"]);
    }


}
