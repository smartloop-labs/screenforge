//! Background → Unsplash: search photos through the ScreenForge backend's `unsplash` proxy
//! (the access key stays on the server), show thumbnails, and download the chosen photo into
//! the project's `assets/` so it renders and exports offline like any image background.
//!
//! This is the part without UI, shared by the app's sidebar and agent and by `sf mcp`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use serde::Deserialize;

/// Largest photo we accept (bytes).
const MAX_IMAGE_BYTES: u64 = 40 * 1024 * 1024;
/// Referral parameters Unsplash asks for on every link back to it.
const UTM: &str = "utm_source=screenforge&utm_medium=referral";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Photo {
    pub id: String,
    pub thumb: String,
    pub full: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub author_url: Option<String>,
    #[serde(default)]
    pub photo_url: Option<String>,
    pub download_location: String,
    /// What the photo shows (Unsplash's alt text), when the server sends it.
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchReply {
    pub results: Vec<Photo>,
    #[serde(default)]
    pub total_pages: u32,
}

/// A downloaded photo, ready to become the background.
pub struct Applied {
    pub photo: Photo,
    /// File name inside the project's `assets/`.
    pub file_name: String,
}


/// One page of results for `query`, blocking (the agent runs on its own thread).
pub fn search_blocking(query: &str) -> Result<Vec<Photo>, String> {
    let v = sf_license::call_backend("unsplash", serde_json::json!({ "query": query.trim(), "page": 1 }))?;
    serde_json::from_value::<SearchReply>(v).map(|r| r.results).map_err(|e| format!("unexpected reply: {e}"))
}

/// Register the download with Unsplash, then fetch the image into `assets_dir`.
/// `repaint` is called as the download progresses.
pub fn download_photo(photo: Photo, assets_dir: &Path, progress: &AtomicU32, repaint: &dyn Fn()) -> Result<Applied, String> {
    let v = sf_license::call_backend("unsplash", serde_json::json!({ "download": photo.download_location }))?;
    // the download endpoint hands back the original; ask the CDN for a 4K-wide JPEG instead
    let url = v.get("url").and_then(|u| u.as_str()).map(|u| format!("{u}{}w=3840&fm=jpg&q=85", if u.contains('?') { "&" } else { "?" })).unwrap_or_else(|| photo.full.clone());
    std::fs::create_dir_all(assets_dir).map_err(|e| e.to_string())?;
    let file_name = format!("unsplash-{}.jpg", safe_id(&photo.id));
    fetch(&url, &assets_dir.join(&file_name), MAX_IMAGE_BYTES, Some((progress, repaint)))?;
    Ok(Applied { photo, file_name })
}

/// GET an image from Unsplash's CDN into `dest` (written via a temp file, at most `max` bytes).
/// With `progress`, the downloaded fraction is stored there as it arrives (when the server
/// sends the length) and `repaint` is called.
pub fn fetch(url: &str, dest: &Path, max: u64, progress: Option<(&AtomicU32, &dyn Fn())>) -> Result<(), String> {
    if !url.starts_with("https://images.unsplash.com/") {
        return Err("not an Unsplash image".into());
    }
    let resp = ureq::get(url).timeout(std::time::Duration::from_secs(60)).call().map_err(|e| format!("download failed: {e}"))?;
    let total = resp.header("Content-Length").and_then(|v| v.parse::<u64>().ok()).filter(|n| *n > 0);
    let mut reader = resp.into_reader().take(max + 1);
    let mut bytes = Vec::with_capacity(total.unwrap_or(0).min(max) as usize);
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut chunk).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..n]);
        if let (Some((p, repaint)), Some(total)) = (progress, total) {
            p.store((bytes.len() as f32 / total as f32).to_bits(), Ordering::Relaxed);
            repaint();
        }
    }
    if bytes.len() as u64 > max {
        return Err("image too large".into());
    }
    let tmp = dest.with_extension("part");
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dest).map_err(|e| e.to_string())
}

/// Where thumbnails are kept.
pub fn cache_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "screenforge", "ScreenForge").map(|d| d.cache_dir().join("unsplash"))
}

/// Photo ids are `[A-Za-z0-9_-]`; anything else is dropped before it reaches a file name.
pub fn safe_id(id: &str) -> String {
    id.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect()
}

/// `url` with Unsplash's referral parameters.
pub fn referral(url: &str) -> String {
    format!("{url}{}{UTM}", if url.contains('?') { "&" } else { "?" })
}

/// The project-file record of an Unsplash background (`config.styles.background.unsplash`).
pub fn background_value(a: &Applied) -> serde_json::Value {
    serde_json::json!({
        "image": a.file_name,
        "id": a.photo.id,
        "author": a.photo.author,
        "authorUrl": a.photo.author_url,
        "photoUrl": a.photo.photo_url,
        "color": a.photo.color,
    })
}

/// The credit to paste into a video description for an Unsplash background (`value` as from
/// [`background_value`]): the photographer and the photo, each with its link.
pub fn credit(value: &serde_json::Value) -> String {
    let get = |k: &str| value.get(k).and_then(|v| v.as_str()).filter(|s| !s.is_empty());
    let mut s = format!("Photo by {}", get("author").unwrap_or("an Unsplash photographer"));
    if let Some(url) = get("authorUrl") {
        s += &format!(" ({})", referral(url));
    }
    s += &format!(" on Unsplash ({})", referral(get("photoUrl").unwrap_or("https://unsplash.com/")));
    s
}

#[cfg(test)]
mod tests {
    #[test]
    fn credit_names_the_photographer() {
        let v = serde_json::json!({ "author": "Ann", "authorUrl": "https://unsplash.com/@ann", "photoUrl": "https://unsplash.com/photos/abc" });
        assert_eq!(super::credit(&v), "Photo by Ann (https://unsplash.com/@ann?utm_source=screenforge&utm_medium=referral) on Unsplash (https://unsplash.com/photos/abc?utm_source=screenforge&utm_medium=referral)");
        assert_eq!(super::credit(&serde_json::json!({})), "Photo by an Unsplash photographer on Unsplash (https://unsplash.com/?utm_source=screenforge&utm_medium=referral)");
    }

    #[test]
    fn ids_and_links_are_sanitised() {
        assert_eq!(super::safe_id("a/../b_C-9"), "ab_C-9");
        assert_eq!(super::referral("https://unsplash.com/@x"), "https://unsplash.com/@x?utm_source=screenforge&utm_medium=referral");
        assert_eq!(super::referral("https://unsplash.com/?a=1"), "https://unsplash.com/?a=1&utm_source=screenforge&utm_medium=referral");
        assert!(super::fetch("https://evil.example/x.jpg", std::path::Path::new("/tmp/never"), 10, None).is_err());
    }

    #[test]
    fn search_reply_parses() {
        let v = serde_json::json!({ "results": [{ "id": "abc", "thumb": "https://images.unsplash.com/t", "full": "https://images.unsplash.com/f", "author": "Ann", "downloadLocation": "https://api.unsplash.com/photos/abc/download" }], "totalPages": 3 });
        let r: super::SearchReply = serde_json::from_value(v).unwrap();
        assert_eq!(r.results[0].author.as_deref(), Some("Ann"));
        assert_eq!(r.total_pages, 3);
    }
}
