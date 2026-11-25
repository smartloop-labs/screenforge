//! Cloud share links, free for every computer: an exported MP4 uploaded straight to the share
//! storage (Cloudflare R2, through URLs the `share` backend function signs) and answered with a
//! `screenforge.studio/s/<id>` link. A link belongs to the export's computer (and to its license
//! when one is signed in); a project has one link, so sharing it again replaces the video behind
//! the same URL. This is the part without UI, shared by the app's Export and `sf mcp`.

use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// A finished share: the page anyone can watch, and (when the server says) the video file and
/// poster behind it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shared {
    pub url: String,
    pub video: Option<String>,
    pub poster: Option<String>,
}

impl Shared {
    /// Just the page (its id is known before anything is uploaded; `link_url`).
    pub fn page(url: String) -> Shared {
        Shared { url, ..Default::default() }
    }
}

/// What the page behind a link says about the video: its title, a description (which may end
/// with the chapter list) and tags.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Meta {
    pub title: String,
    pub description: String,
    pub tags: Vec<String>,
}

/// What a cancelled share fails with (see [`is_cancel`]).
pub const CANCELLED: &str = "cancelled";

/// One `share` request. Its 404 means the link is gone (not that the service is missing).
fn call(req: serde_json::Value) -> Result<serde_json::Value, String> {
    sf_license::call_backend_status("share", req).map_err(|(code, e)| if code == 404 && e == "not found" { "That link no longer exists.".into() } else { e })
}

/// [`upload`] for a file Export wrote: its size and length are read from the file, which stays
/// where it is.
pub fn upload_file(project: &Path, video: &Path, meta: &Meta, cancel: &AtomicBool, progress: &dyn Fn(f32)) -> Result<Shared, String> {
    let info = sf_media::probe::probe(video).map_err(|e| format!("Could not read the exported video: {e:#}"))?;
    let v = info.video();
    let dims = (v.and_then(|v| v.width).unwrap_or(0), v.and_then(|v| v.height).unwrap_or(0), info.duration_ms().unwrap_or(0.0).max(0.0) as u64);
    upload(project, video, meta, dims, cancel, progress)
}

/// The project's QuickLook preview as a JPEG, for the link's poster and unfurl image. Rendered
/// first when the project has none yet (a recording that just stopped may still be writing it).
fn poster_jpeg(project: &Path) -> Option<Vec<u8>> {
    let png = project.join(sf_project::package::QUICKLOOK_PREVIEW);
    if !png.is_file() {
        let _ = sf_session::preview::write_quicklook_preview(project);
    }
    let img = image::open(&png).ok()?.to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85).encode_image(&img).ok()?;
    Some(out)
}

/// Upload `video` (an H.264 MP4 of `project`, `width`×`height` and `duration_ms` long) as the
/// project's share link, with its poster. `progress` hears how far the upload is (0–1).
pub fn upload(project: &Path, video: &Path, meta: &Meta, (width, height, duration_ms): (u32, u32, u64), cancel: &AtomicBool, progress: &dyn Fn(f32)) -> Result<Shared, String> {
    let auth = sf_license::owner_auth().ok_or("ScreenForge cannot identify this computer, so it cannot make a share link.")?;
    let bytes = std::fs::metadata(video).map_err(|e| format!("The rendered video is missing: {e}"))?.len();
    let poster = poster_jpeg(project);
    let mut req = auth.clone();
    req["action"] = "create".into();
    // the project's one link (the server reuses it)
    let project_id = sf_project::ProjectPackage::open(project).ok().map(|pkg| pkg.project.id);
    if let Some(id) = &project_id {
        req["project"] = id.clone().into();
    }
    req["title"] = meta.title.as_str().into();
    // older share servers ignore these
    if !meta.description.trim().is_empty() {
        req["description"] = meta.description.trim().into();
    }
    if !meta.tags.is_empty() {
        req["tags"] = meta.tags.clone().into();
    }
    req["bytes"] = bytes.into();
    req["width"] = width.into();
    req["height"] = height.into();
    req["duration_ms"] = duration_ms.into();
    if let Some(p) = &poster {
        req["poster"] = p.len().into();
    }
    let created = call(req)?;
    let id = created["id"].as_str().ok_or("The share server sent no link id.")?.to_string();
    let video_url = created["upload"]["video"].as_str().ok_or("The share server sent no upload address.")?;
    let agent = ureq::AgentBuilder::new().timeout_connect(std::time::Duration::from_secs(15)).timeout_read(std::time::Duration::from_secs(120)).timeout_write(std::time::Duration::from_secs(120)).build();
    // the poster is a nicety: a failed one leaves the link without it
    if let (Some(url), Some(jpg)) = (created["upload"]["poster"].as_str(), &poster) {
        let _ = agent.put(url).set("Content-Type", "image/jpeg").send_bytes(jpg);
    }
    let file = std::fs::File::open(video).map_err(|e| format!("Could not read the rendered video: {e}"))?;
    let reader = Counting { inner: file, sent: 0, total: bytes, cancel, progress };
    match agent.put(video_url).set("Content-Type", "video/mp4").set("Content-Length", &bytes.to_string()).send(reader) {
        Ok(_) => {}
        Err(_) if cancel.load(Ordering::Relaxed) => return Err(CANCELLED.into()),
        Err(ureq::Error::Status(code, _)) => return Err(format!("The upload was refused ({code}). Try sharing again.")),
        Err(e) => return Err(format!("The upload stopped ({}). Check your connection and try again.", e.kind())),
    }
    let mut req = auth;
    req["action"] = "complete".into();
    req["id"] = id.clone().into();
    let done = call(req)?;
    let url = done["url"].as_str().map(String::from).ok_or("The share server sent no link.")?;
    // the file and poster addresses (with the version that gets past the CDN's cached copy)
    // come from the public `get`; without them the dialog lists the page alone
    let media = call(serde_json::json!({ "action": "get", "id": id })).ok();
    let field = |k: &str| media.as_ref().and_then(|m| m[k].as_str()).filter(|s| s.starts_with("https://")).map(String::from);
    Ok(Shared { url, video: field("video"), poster: field("poster") })
}

/// The video file as the upload body, reporting progress and stopping when cancelled.
struct Counting<'a> {
    inner: std::fs::File,
    sent: u64,
    total: u64,
    cancel: &'a AtomicBool,
    progress: &'a dyn Fn(f32),
}

impl Read for Counting<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other(CANCELLED));
        }
        let n = self.inner.read(buf)?;
        let before = self.sent * 100 / self.total.max(1);
        self.sent += n as u64;
        // a repaint per whole percent is plenty
        if self.sent * 100 / self.total.max(1) != before {
            (self.progress)(self.sent as f32 / self.total.max(1) as f32);
        }
        Ok(n)
    }
}

/// The page a link id opens.
pub const PAGE_URL: &str = "https://screenforge.studio/s/";

/// A project's link id, the same every time: 12 base62 characters of
/// SHA-256("screenforge-share/" + project id). The app shows a project's link before anything
/// is uploaded; the server gives a new link this id (`linkIdFor` in the share function), and
/// links made before keep theirs. Unguessable as long as the project id is (10 random base62
/// characters, kept in the project file).
pub fn link_id(project: &str) -> String {
    use sha2::Digest;
    const ABC: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let d = sha2::Sha256::digest(format!("screenforge-share/{project}"));
    d[..12].iter().map(|b| ABC[*b as usize % 62] as char).collect()
}

pub fn link_url(project: &str) -> String {
    format!("{PAGE_URL}{}", link_id(project))
}

/// Is the job's error just a cancel (nothing to tell the user)?
pub fn is_cancel(e: &str) -> bool {
    e == CANCELLED
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io::Write;

    fn file_with(bytes: usize) -> (tempfile::TempDir, std::fs::File) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v.mp4");
        std::fs::File::create(&path).unwrap().write_all(&vec![7u8; bytes]).unwrap();
        (dir, std::fs::File::open(&path).unwrap())
    }

    #[test]
    fn upload_body_reports_progress_to_the_end() {
        let (_dir, file) = file_with(10_000);
        let seen = RefCell::new(Vec::new());
        let cancel = AtomicBool::new(false);
        let report = |f: f32| seen.borrow_mut().push(f);
        let mut r = Counting { inner: file, sent: 0, total: 10_000, cancel: &cancel, progress: &report };
        let mut out = Vec::new();
        r.read_to_end(&mut out).unwrap();
        assert_eq!(out.len(), 10_000);
        let seen = seen.borrow();
        assert_eq!(*seen.last().unwrap(), 1.0);
        assert!(seen.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn upload_body_stops_when_cancelled() {
        let (_dir, file) = file_with(100);
        let cancel = AtomicBool::new(true);
        let mut r = Counting { inner: file, sent: 0, total: 100, cancel: &cancel, progress: &|_| {} };
        let err = r.read(&mut [0u8; 64]).unwrap_err();
        assert!(is_cancel(&err.to_string()));
    }

    #[test]
    fn link_id_is_the_servers() {
        // the same values as `linkIdFor` in backend/supabase/functions/share/index.ts
        assert_eq!(link_id("iv5Pww9n2l"), "v3BwvryVLJ5B");
        assert_eq!(link_id("abc"), "LyVS9w6Jh3S9");
        assert_eq!(link_url("abc"), "https://screenforge.studio/s/LyVS9w6Jh3S9");
    }

    /// Against the live backend with this computer's license: one project, one link.
    /// `cargo test -p studio share_again_keeps_the_link -- --ignored`
    #[test]
    #[ignore]
    fn share_again_keeps_the_link() {
        let auth = sf_license::owner_auth().expect("a device id");
        let project = format!("e2e-{}", std::process::id());
        let put = |bytes: &[u8]| {
            let mut req = auth.clone();
            req["action"] = "create".into();
            req["project"] = project.clone().into();
            req["title"] = "e2e replace".into();
            req["bytes"] = bytes.len().into();
            let c = call(req).unwrap();
            ureq::put(c["upload"]["video"].as_str().unwrap()).set("Content-Type", "video/mp4").send_bytes(bytes).unwrap();
            let mut req = auth.clone();
            req["action"] = "complete".into();
            req["id"] = c["id"].clone();
            call(req).unwrap();
            c["id"].as_str().unwrap().to_string()
        };
        let first = put(&[1u8; 1000]);
        let get = |id: &str| call(serde_json::json!({ "action": "get", "id": id })).unwrap();
        let v1 = get(&first)["video"].as_str().unwrap().to_string();
        let second = put(&[2u8; 2000]);
        let v2 = get(&second)["video"].as_str().unwrap().to_string();
        let served = ureq::get(&v2).call().ok().and_then(|r| r.header("content-length").map(str::to_string));
        let mut req = auth.clone();
        req["action"] = "delete".into();
        req["id"] = first.clone().into();
        call(req).unwrap();
        assert_eq!(first, second, "same link");
        assert_eq!(first, link_id(&project), "the project's derived id");
        assert_ne!(v1, v2, "new video address");
        assert_eq!(served.as_deref(), Some("2000"), "the new video is served");
    }
}
