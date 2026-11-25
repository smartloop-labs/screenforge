//! Audio → Find music: the sidebar's music browser. Searching Jamendo, downloading, credits and
//! the favourites library are in [`sf_services::music`] (shared with the agent and `sf mcp`);
//! this runs that work on background threads, and the sidebar polls [`Browser::poll`] every
//! frame.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

pub use sf_services::music::*;

#[derive(Debug, Clone, PartialEq)]
pub enum Job {
    Track(String),
    Link,
}

/// The tab of Audio → Background audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Source {
    /// The bundled tracks and "Add background audio…".
    #[default]
    Library,
    /// Jamendo search results (popular tracks for an empty search).
    Jamendo,
    Favorites,
    Recent,
    /// Import from a link.
    Link,
}

/// How many tracks "Recent" keeps.

pub struct Browser {
    pub query: String,
    pub instrumental: bool,
    /// Also offer CC BY-NC tracks: the user says the video earns nothing.
    pub noncommercial: bool,
    pub results: Vec<Track>,
    page: u32,
    total_pages: u32,
    pub error: Option<String>,
    /// The first page loads when the section is first shown.
    started: bool,
    search: Option<(u32, Receiver<Result<SearchReply, String>>)>,
    pub source: Source,
    /// Loaded when the section is first shown, and again when the agent changed it.
    pub library: Library,
    /// When the saved file was last changed as of the last load (`Some(None)`: no file yet).
    library_stamp: Option<Option<std::time::SystemTime>>,
    /// The Jamendo track downloading, to note in "Recent" once it is added.
    pending: Option<Track>,
    thumbs: HashMap<String, PathBuf>,
    thumb_rx: Vec<Receiver<(String, PathBuf)>>,
    /// The link typed in "Import from a link".
    pub link: String,
    pub link_error: Option<String>,
    pub applying: Option<Job>,
    apply_rx: Option<Receiver<Result<Applied, String>>>,
    /// Download progress as f32 bits: 0..1, or negative while the size is unknown.
    progress: Arc<AtomicU32>,
}

impl Default for Browser {
    fn default() -> Self {
        Self {
            query: String::new(),
            instrumental: true,
            noncommercial: false,
            results: vec![],
            page: 0,
            total_pages: 0,
            error: None,
            started: false,
            search: None,
            source: Source::Library,
            library: Library::default(),
            library_stamp: None,
            pending: None,
            thumbs: HashMap::new(),
            thumb_rx: vec![],
            link: String::new(),
            link_error: None,
            applying: None,
            apply_rx: None,
            progress: Default::default(),
        }
    }
}

impl Browser {
    pub fn is_busy(&self) -> bool {
        self.search.is_some() || self.apply_rx.is_some()
    }

    pub fn is_searching(&self) -> bool {
        self.search.is_some()
    }

    pub fn can_load_more(&self) -> bool {
        self.search.is_none() && self.page > 0 && self.page < self.total_pages
    }

    /// Load the popular tracks the first time the section is shown.
    pub fn ensure_started(&mut self, ctx: &egui::Context) {
        if !self.started {
            self.started = true;
            self.ensure_library();
            let saved: Vec<Track> = self.library.favorites.iter().chain(&self.library.recent).cloned().collect();
            self.fetch_thumbs(ctx, &saved);
            self.search(ctx, 1);
        }
    }

    /// Read the saved favourites and recent tracks, again whenever the file changed (the agent
    /// adds favourites too; a stale copy saved by the next heart would drop them).
    pub fn ensure_library(&mut self) {
        let stamp = Library::modified();
        if self.library_stamp != Some(stamp) {
            self.library_stamp = Some(stamp);
            self.library = Library::load();
        }
    }

    /// Search for `self.query` (empty: the month's popular tracks); page 1 replaces the results,
    /// later pages are appended.
    pub fn search(&mut self, ctx: &egui::Context, page: u32) {
        let (tx, rx) = channel();
        let (query, instrumental, noncommercial, ctx) = (self.query.trim().to_string(), self.instrumental, self.noncommercial, ctx.clone());
        std::thread::spawn(move || {
            let reply = search_page(&query, page, instrumental, noncommercial);
            let _ = tx.send(reply);
            ctx.request_repaint();
        });
        self.search = Some((page, rx));
        self.error = None;
    }

    pub fn load_more(&mut self, ctx: &egui::Context) {
        if self.can_load_more() {
            self.search(ctx, self.page + 1);
        }
    }

    /// Fraction of the file being downloaded, `None` while its size is not known yet.
    pub fn progress(&self) -> Option<f32> {
        let v = f32::from_bits(self.progress.load(Ordering::Relaxed));
        (v >= 0.0).then_some(v.min(1.0))
    }

    /// The tracks the current view lists.
    pub fn tracks(&self) -> &[Track] {
        match self.source {
            Source::Favorites => &self.library.favorites,
            Source::Recent => &self.library.recent,
            _ => &self.results,
        }
    }

    /// Download Jamendo `track` into `assets_dir`, with its credit file.
    pub fn apply_track(&mut self, ctx: &egui::Context, track: Track, assets_dir: PathBuf) {
        if self.apply_rx.is_some() {
            return;
        }
        self.pending = Some(track.clone());
        self.start(ctx, Job::Track(track.id.clone()), move |progress, ctx| download_track(&track, &assets_dir, progress, &|| ctx.request_repaint()));
        self.error = None;
    }

    /// Download `self.link` into `assets_dir`.
    pub fn import_link(&mut self, ctx: &egui::Context, assets_dir: PathBuf) {
        let link = self.link.trim().to_string();
        self.start(ctx, Job::Link, move |progress, ctx| download_link(&link, &assets_dir, progress, &|| ctx.request_repaint()));
        self.link_error = None;
    }

    fn start(&mut self, ctx: &egui::Context, job: Job, work: impl FnOnce(&AtomicU32, &egui::Context) -> Result<Applied, String> + Send + 'static) {
        if self.apply_rx.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        self.applying = Some(job);
        self.progress.store((-1f32).to_bits(), Ordering::Relaxed);
        let progress = self.progress.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work(&progress, &ctx));
            ctx.request_repaint();
        });
        self.apply_rx = Some(rx);
    }

    /// Local album cover of a result, once downloaded.
    pub fn thumb(&self, id: &str) -> Option<&Path> {
        self.thumbs.get(id).map(PathBuf::as_path)
    }

    fn fetch_thumbs(&mut self, ctx: &egui::Context, tracks: &[Track]) {
        let Some(dir) = directories::ProjectDirs::from("dev", "screenforge", "ScreenForge").map(|d| d.cache_dir().join("jamendo")) else { return };
        let todo: Vec<(String, String)> = tracks.iter().filter(|t| !self.thumbs.contains_key(&t.id)).filter_map(|t| Some((t.id.clone(), t.image.clone().filter(|u| is_jamendo(u))?))).collect();
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = std::fs::create_dir_all(&dir);
            for (id, url) in todo {
                let path = dir.join(format!("{}.jpg", safe_name(&id)));
                let ok = path.is_file() || fetch(&url, 2 * 1024 * 1024, &AtomicU32::new(0), &|| ctx.request_repaint()).and_then(|(b, _)| write(&path, &b)).is_ok();
                if ok {
                    let _ = tx.send((id, path));
                    ctx.request_repaint();
                }
            }
        });
        self.thumb_rx.push(rx);
    }

    /// Collect finished work; returns a track that finished downloading.
    pub fn poll(&mut self, ctx: &egui::Context) -> Option<Applied> {
        if let Some((page, rx)) = &self.search {
            if let Ok(reply) = rx.try_recv() {
                let page = *page;
                self.search = None;
                match reply {
                    Ok(r) => {
                        if page == 1 {
                            self.results.clear();
                        }
                        self.page = page;
                        self.total_pages = r.total_pages;
                        self.fetch_thumbs(ctx, &r.results);
                        self.results.extend(r.results);
                    }
                    Err(e) => self.error = Some(e),
                }
            }
        }
        let thumbs = &mut self.thumbs;
        self.thumb_rx.retain(|rx| loop {
            match rx.try_recv() {
                Ok((id, path)) => {
                    thumbs.insert(id, path);
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break true,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break false,
            }
        });
        let rx = self.apply_rx.as_ref()?;
        let done = rx.try_recv().ok()?;
        self.apply_rx = None;
        let job = self.applying.take();
        let pending = self.pending.take();
        match done {
            Ok(a) => {
                match (&job, pending) {
                    (Some(Job::Link), _) => self.link.clear(),
                    (_, Some(track)) => self.library.note_used(&track),
                    _ => {}
                }
                Some(a)
            }
            Err(e) => {
                match job {
                    Some(Job::Link) => self.link_error = Some(e),
                    _ => self.error = Some(e),
                }
                None
            }
        }
    }
}

