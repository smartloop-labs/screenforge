//! Background → Unsplash: the sidebar's photo browser. Searching and downloading are in
//! [`sf_services::unsplash`] (shared with the agent and `sf mcp`); this shows thumbnails and
//! runs that work on background threads, and the sidebar polls [`Browser::poll`] every frame.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

pub use sf_services::unsplash::*;

#[derive(Default)]
pub struct Browser {
    pub query: String,
    pub results: Vec<Photo>,
    page: u32,
    total_pages: u32,
    pub error: Option<String>,
    /// The first page loads when the Unsplash tab is first shown.
    started: bool,
    search: Option<(u32, Receiver<Result<SearchReply, String>>)>,
    thumbs: HashMap<String, PathBuf>,
    thumb_rx: Option<Receiver<(String, PathBuf)>>,
    /// Photo id being downloaded.
    pub applying: Option<String>,
    apply_rx: Option<Receiver<Result<Applied, String>>>,
    /// Download progress of `applying` as f32 bits: 0..1, or negative while the size is unknown.
    progress: Arc<AtomicU32>,
}

impl Browser {
    pub fn is_busy(&self) -> bool {
        self.search.is_some() || self.apply_rx.is_some()
    }

    pub fn can_load_more(&self) -> bool {
        self.search.is_none() && self.page > 0 && self.page < self.total_pages
    }

    /// Load the wallpaper topic the first time the tab is shown.
    pub fn ensure_started(&mut self, ctx: &egui::Context) {
        if !self.started {
            self.started = true;
            self.search(ctx, 1);
        }
    }

    /// Search for `self.query` (empty: Unsplash's wallpapers); page 1 replaces the results,
    /// later pages are appended.
    pub fn search(&mut self, ctx: &egui::Context, page: u32) {
        let (tx, rx) = channel();
        let (query, ctx) = (self.query.trim().to_string(), ctx.clone());
        std::thread::spawn(move || {
            let reply = sf_license::call_backend("unsplash", serde_json::json!({ "query": query, "page": page })).and_then(|v| serde_json::from_value::<SearchReply>(v).map_err(|e| format!("unexpected reply: {e}")));
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

    /// Local thumbnail of a result, once downloaded.
    pub fn thumb(&self, id: &str) -> Option<&Path> {
        self.thumbs.get(id).map(PathBuf::as_path)
    }

    /// Fraction of the photo being applied that has downloaded, `None` while its size is not
    /// known yet.
    pub fn progress(&self) -> Option<f32> {
        let v = f32::from_bits(self.progress.load(Ordering::Relaxed));
        (v >= 0.0).then_some(v.min(1.0))
    }

    /// Download `photo` into `assets_dir` (registering the download with Unsplash first).
    pub fn apply(&mut self, ctx: &egui::Context, photo: Photo, assets_dir: PathBuf) {
        if self.apply_rx.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        self.applying = Some(photo.id.clone());
        self.progress.store((-1f32).to_bits(), Ordering::Relaxed);
        let progress = self.progress.clone();
        std::thread::spawn(move || {
            let _ = tx.send(download_photo(photo, &assets_dir, &progress, &|| ctx.request_repaint()));
            ctx.request_repaint();
        });
        self.apply_rx = Some(rx);
        self.error = None;
    }

    /// Collect finished work; returns a photo that finished downloading.
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
        if let Some(rx) = &self.thumb_rx {
            while let Ok((id, path)) = rx.try_recv() {
                self.thumbs.insert(id, path);
            }
        }
        let rx = self.apply_rx.as_ref()?;
        let done = rx.try_recv().ok()?;
        self.apply_rx = None;
        self.applying = None;
        match done {
            Ok(a) => Some(a),
            Err(e) => {
                self.error = Some(e);
                None
            }
        }
    }

    fn fetch_thumbs(&mut self, ctx: &egui::Context, photos: &[Photo]) {
        let Some(dir) = cache_dir() else { return };
        let (tx, rx) = channel();
        let todo: Vec<(String, String)> = photos.iter().filter(|p| !self.thumbs.contains_key(&p.id)).map(|p| (p.id.clone(), p.thumb.clone())).collect();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = std::fs::create_dir_all(&dir);
            for (id, url) in todo {
                let path = dir.join(format!("{}.jpg", safe_id(&id)));
                if path.is_file() || fetch(&url, &path, 4 * 1024 * 1024, None).is_ok() {
                    let _ = tx.send((id, path));
                    ctx.request_repaint();
                }
            }
        });
        // replaces an older batch's channel: its thumbnails still land on disk for next time
        self.thumb_rx = Some(rx);
    }
}
