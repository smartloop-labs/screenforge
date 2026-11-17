//! Loom share links (`https://www.loom.com/share/<id>`) downloaded as a video file, which
//! [`crate::import`] then turns into a recording like any other video.
//!
//! Loom keeps no project on disk, so like other tools that import from Loom this asks Loom's
//! web endpoints for the video behind a public link: `transcoded-url` / `raw-url` (an MP4 when
//! Loom has one), else its GraphQL API for a signed HLS playlist (DASH is skipped: not every
//! ffmpeg reads it), and the same API for the title and the owner's download setting. A
//! playlist's segments are fetched with its CloudFront signature sent as cookies, the way
//! Loom's player does. These are the endpoints Loom's own player uses and are not a documented API: when
//! Loom changes them this stops working and says so. Only links set to "anyone with the link"
//! work, and a video whose owner turned downloads off is refused.
//!
//! The camera bubble is part of Loom's picture, so it comes in with the screen; there is no
//! cursor or click log (no automatic zooms).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{json, Value};

use crate::{Error, Result};

const API: &str = "https://www.loom.com/api/campaigns/sessions";
const GRAPHQL: &str = "https://www.loom.com/graphql";

/// The video id in a Loom link (`loom.com/share/<id>`, `/embed/<id>`, `/v/<id>`, with or without
/// `www.`, a title slug before the id, or a query), or `None` for anything else.
pub fn video_id(link: &str) -> Option<String> {
    let link = link.trim();
    let rest = link.strip_prefix("https://").or_else(|| link.strip_prefix("http://")).unwrap_or(link);
    let (host, path) = rest.split_once('/')?;
    let host = host.to_ascii_lowercase();
    if host != "loom.com" && !host.ends_with(".loom.com") {
        return None;
    }
    let path = path.split(['?', '#']).next().unwrap_or("");
    let mut parts = path.split('/').filter(|p| !p.is_empty());
    if !matches!(parts.next(), Some("share" | "embed" | "v")) {
        return None;
    }
    // `share/<id>` or `share/<title-slug>-<id>`
    let last = parts.next()?;
    let id = last.rsplit('-').next().unwrap_or(last);
    (id.len() >= 16 && id.len() <= 64 && id.chars().all(|c| c.is_ascii_hexdigit())).then(|| id.to_ascii_lowercase())
}

/// True for text that is a Loom video link.
pub fn is_link(text: &str) -> bool {
    video_id(text).is_some()
}

/// What Loom says about a video before it is downloaded.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Info {
    pub title: Option<String>,
    /// The owner allows downloads (`None`: Loom did not say).
    pub downloads_allowed: Option<bool>,
    pub created_at: Option<String>,
}

/// Where the video can be fetched from.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// A whole MP4 (or WebM) file.
    File(String),
    /// An HLS playlist (signed: the signature also goes on its segments as cookies).
    Hls(String),
    /// A DASH playlist (not downloaded: ffmpeg builds often cannot read it).
    Dash(String),
}

impl Source {
    fn from_url(url: String) -> Source {
        let path = url.split('?').next().unwrap_or("").to_ascii_lowercase();
        if path.ends_with(".m3u8") {
            Source::Hls(url)
        } else if path.ends_with(".mpd") {
            Source::Dash(url)
        } else {
            Source::File(url)
        }
    }
}

/// The value of query parameter `key` in `url`.
fn query_param<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    url.split_once('?')?.1.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v))
}

/// A signed URL's CloudFront signature as a `Cookie` header value, for the files a playlist
/// names without one.
pub fn cloudfront_cookie(url: &str) -> Option<String> {
    let (sig, key) = (query_param(url, "Signature")?, query_param(url, "Key-Pair-Id")?);
    let first = match (query_param(url, "Policy"), query_param(url, "Expires")) {
        (Some(p), _) => format!("CloudFront-Policy={p}"),
        (None, Some(e)) => format!("CloudFront-Expires={e}"),
        _ => return None,
    };
    Some(format!("{first}; CloudFront-Signature={sig}; CloudFront-Key-Pair-Id={key}"))
}

/// `uri` from a playlist at `base`, as a full address.
fn resolve(base: &str, uri: &str) -> String {
    if uri.starts_with("https://") || uri.starts_with("http://") {
        return uri.to_string();
    }
    let path = base.split('?').next().unwrap_or(base);
    let dir = &path[..path.rfind('/').map_or(path.len(), |i| i + 1)];
    format!("{dir}{uri}")
}

/// The video and audio playlists to download from an HLS master playlist: the variant with the
/// most bandwidth, and its audio group's (default) rendition. `None` for a media playlist.
pub fn pick_variants(master: &str, base: &str) -> Option<(String, Option<String>)> {
    let attr = |line: &str, key: &str| -> Option<String> {
        let at = line.find(&format!("{key}="))? + key.len() + 1;
        let rest = &line[at..];
        Some(if let Some(q) = rest.strip_prefix('"') { q.split('"').next().unwrap_or("").to_string() } else { rest.split(',').next().unwrap_or("").to_string() })
    };
    let lines: Vec<&str> = master.lines().map(str::trim).collect();
    let mut best: Option<(u64, String, Option<String>)> = None;
    for (i, l) in lines.iter().enumerate() {
        if let Some(info) = l.strip_prefix("#EXT-X-STREAM-INF:") {
            let bw = attr(info, "BANDWIDTH").and_then(|b| b.parse().ok()).unwrap_or(0);
            let Some(uri) = lines[i + 1..].iter().find(|u| !u.is_empty() && !u.starts_with('#')) else { continue };
            if best.as_ref().is_none_or(|b| bw > b.0) {
                best = Some((bw, uri.to_string(), attr(info, "AUDIO")));
            }
        }
    }
    let (_, video, group) = best?;
    let audio = group.and_then(|g| {
        let media: Vec<&&str> = lines.iter().filter(|l| l.starts_with("#EXT-X-MEDIA:") && l.contains("TYPE=AUDIO") && attr(l, "GROUP-ID").as_deref() == Some(g.as_str()) && attr(l, "URI").is_some()).collect();
        media.iter().find(|l| l.contains("DEFAULT=YES")).or(media.first()).and_then(|l| attr(l, "URI"))
    });
    Some((resolve(base, &video), audio.map(|a| resolve(base, &a))))
}

/// Length of an HLS media playlist (its `#EXTINF` durations added up), in ms.
pub fn playlist_duration_ms(media: &str) -> Option<f64> {
    let ms: f64 = media.lines().filter_map(|l| l.trim().strip_prefix("#EXTINF:")).filter_map(|d| d.split(',').next()?.trim().parse::<f64>().ok()).sum::<f64>() * 1000.0;
    (ms > 0.0).then_some(ms)
}

/// Read a `transcoded-url` / `raw-url` answer: `{ url, part_credentials? }`. A playlist's
/// CloudFront signature comes apart from its address and goes on as its query.
pub fn parse_session_url(v: &Value) -> Option<Source> {
    let url = v["url"].as_str().filter(|u| u.starts_with("https://"))?;
    let mut url = url.to_string();
    let c = &v["part_credentials"];
    if c.is_object() {
        let (sig, key) = (c["Signature"].as_str().unwrap_or(""), c["Key-Pair-Id"].as_str().unwrap_or(""));
        if !sig.is_empty() && !key.is_empty() {
            let mut q = vec![];
            if let Some(p) = c["Policy"].as_str().filter(|p| !p.is_empty()) {
                q.push(format!("Policy={p}"));
            } else if let Some(e) = c["Expires"].as_u64().or_else(|| c["Expires"].as_str().and_then(|s| s.parse().ok())) {
                q.push(format!("Expires={e}"));
            }
            q.push(format!("Signature={sig}"));
            q.push(format!("Key-Pair-Id={key}"));
            let base = url.split('?').next().unwrap_or(&url).to_string();
            url = format!("{base}?{}", q.join("&"));
        }
    }
    Some(Source::from_url(url))
}

/// Read the GraphQL `getVideo` answer.
pub fn parse_info(v: &Value) -> Info {
    let video = &v["data"]["getVideo"];
    Info {
        title: video["name"].as_str().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        downloads_allowed: match (video["downloadable"].as_bool(), video["download_enabled"].as_bool()) {
            (Some(a), Some(b)) => Some(a && b),
            (Some(a), None) | (None, Some(a)) => Some(a),
            _ => None,
        },
        created_at: video["createdAt"].as_str().map(String::from),
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(15)).timeout_read(Duration::from_secs(60)).user_agent("ScreenForge").build()
}

/// A random id in UUID form for Loom's anonymous viewer field.
fn anon_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut bytes = [0u8; 16];
    for chunk in bytes.chunks_mut(8) {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        chunk.copy_from_slice(&h.finish().to_le_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

fn temporary(e: impl std::fmt::Display) -> Error {
    Error::Capture(format!("Could not reach Loom ({e}). Check your connection and try again."))
}

/// The title and download setting (best effort: `Info::default()` when Loom does not answer).
pub fn info(id: &str) -> Info {
    let query = "query GetVideo($videoId: ID!, $password: String) { getVideo(id: $videoId, password: $password) { ... on RegularUserVideo { id name createdAt downloadable download_enabled } __typename } }";
    let body = json!({ "operationName": "GetVideo", "variables": { "videoId": id, "password": null }, "query": query });
    agent().post(GRAPHQL).set("Accept", "application/json").send_json(body).ok().and_then(|r| r.into_json::<Value>().ok()).map(|v| parse_info(&v)).unwrap_or_default()
}

/// A signed playlist or file from the GraphQL API, for `mimes` (`MP4`, `M3U8`, `DASH`).
fn graphql_source(id: &str, mimes: &[&str]) -> Option<Source> {
    let query = "query GetVideoSource($videoId: ID!, $password: String, $acceptableMimes: [CloudfrontVideoAcceptableMime]) { getVideo(id: $videoId, password: $password) { ... on RegularUserVideo { id nullableRawCdnUrl(acceptableMimes: $acceptableMimes, password: $password) { url credentials { Policy Signature KeyPairId } } } __typename } }";
    let body = json!({ "operationName": "GetVideoSource", "variables": { "videoId": id, "password": null, "acceptableMimes": mimes }, "query": query });
    let v: Value = agent().post(GRAPHQL).set("Accept", "application/json").send_json(body).ok()?.into_json().ok()?;
    parse_graphql_source(&v)
}

/// Read a GraphQL `nullableRawCdnUrl` answer (credentials apart, as for the session endpoints).
pub fn parse_graphql_source(v: &Value) -> Option<Source> {
    let src = &v["data"]["getVideo"]["nullableRawCdnUrl"];
    let c = &src["credentials"];
    parse_session_url(&json!({ "url": src["url"], "part_credentials": if c.is_object() { json!({ "Policy": c["Policy"], "Signature": c["Signature"], "Key-Pair-Id": c["KeyPairId"] }) } else { Value::Null } }))
}

/// Where the video behind `id` can be fetched: an MP4 when Loom has one, else its HLS playlist.
pub fn source(id: &str) -> Result<Source> {
    let mut stream = None;
    for (endpoint, with_body) in [("transcoded-url", true), ("raw-url", true), ("transcoded-url", false), ("raw-url", false)] {
        let req = agent().post(&format!("{API}/{id}/{endpoint}")).set("Accept", "application/json");
        let resp = if with_body { req.send_json(json!({ "anonID": anon_id(), "deviceID": null, "force_original": false, "password": null })) } else { req.call() };
        let v: Value = match resp {
            Ok(r) if r.status() == 204 => continue,
            Ok(r) => match r.into_json() {
                Ok(v) => v,
                Err(_) => continue,
            },
            Err(ureq::Error::Status(429, _)) => return Err(Error::Capture("Loom is busy right now. Wait a minute and try again.".into())),
            Err(ureq::Error::Status(..)) => continue,
            Err(ureq::Error::Transport(t)) => {
                // a playlist already found is good enough
                if stream.is_some() {
                    break;
                }
                return Err(temporary(t.kind()));
            }
        };
        match parse_session_url(&v) {
            Some(Source::File(u)) => return Ok(Source::File(u)),
            Some(s @ Source::Hls(_)) => {
                stream.get_or_insert(s);
            }
            _ => {}
        }
    }
    if let Some(s @ Source::File(_)) = graphql_source(id, &["MP4"]) {
        return Ok(s);
    }
    if stream.is_none() {
        stream = graphql_source(id, &["M3U8"]).filter(|s| matches!(s, Source::Hls(_)));
    }
    stream.ok_or_else(|| Error::Capture("Loom has no public video at that link. In Loom, set the video's sharing to \u{201c}Anyone with the link\u{201d} and try again. A video that is still uploading can be imported once it is done.".into()))
}

/// A file name from a Loom title (`Loom video` when there is none).
pub fn file_name(title: Option<&str>) -> String {
    let t: String = title.unwrap_or("").chars().filter(|c| !"?/\\*\"'`<>|:".contains(*c) && !c.is_control()).collect();
    let t = t.trim().trim_end_matches('.').trim();
    let t: String = t.chars().take(100).collect();
    if t.is_empty() {
        "Loom video".into()
    } else {
        t
    }
}

/// Download the Loom video at `link` into `dir` as `<title>.mp4` and return its path.
/// `progress` gets 0..1 (a playlist without a known length reports nothing until the end).
pub fn download(link: &str, dir: &Path, progress: impl Fn(f64), cancel: &AtomicBool) -> Result<PathBuf> {
    let id = video_id(link).ok_or_else(|| Error::InvalidConfig("that is not a Loom video link (loom.com/share/…)".into()))?;
    let info = info(&id);
    if info.downloads_allowed == Some(false) {
        return Err(Error::Capture("The owner of this Loom video turned downloads off. In Loom, turn on \u{201c}Allow downloads\u{201d} for it (or ask its owner to), then import it again.".into()));
    }
    let src = source(&id)?;
    std::fs::create_dir_all(dir).map_err(|e| Error::Io { path: dir.to_path_buf(), source: e })?;
    let out = dir.join(format!("{}.mp4", file_name(info.title.as_deref())));
    let part = out.with_extension("mp4.part");
    let done = match &src {
        Source::File(url) => fetch_file(url, &part, &progress, cancel),
        Source::Hls(url) => fetch_hls(url, &part, &progress, cancel),
        Source::Dash(_) => Err(Error::Capture("Loom offers this video only in a format ScreenForge cannot download. Download it from Loom and drop the file here instead.".into())),
    };
    if let Err(e) = done {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    std::fs::rename(&part, &out).map_err(|e| Error::Io { path: out.clone(), source: e })?;
    progress(1.0);
    Ok(out)
}

fn cancelled() -> Error {
    Error::Capture("cancelled".into())
}

fn fetch_file(url: &str, out: &Path, progress: &dyn Fn(f64), cancel: &AtomicBool) -> Result<()> {
    let resp = agent().get(url).call().map_err(|e| match e {
        ureq::Error::Status(code, _) => Error::Capture(format!("Loom refused the download ({code}). Try again in a moment.")),
        ureq::Error::Transport(t) => temporary(t.kind()),
    })?;
    let total = resp.header("Content-Length").and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
    let mut body = resp.into_reader();
    let mut file = std::fs::File::create(out).map_err(|e| Error::Io { path: out.to_path_buf(), source: e })?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut got = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(cancelled());
        }
        let n = body.read(&mut buf).map_err(temporary)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| Error::Io { path: out.to_path_buf(), source: e })?;
        got += n as u64;
        if total > 0 {
            progress(got as f64 / total as f64);
        }
    }
    if total > 0 && got < total {
        return Err(Error::Capture("The download from Loom stopped before the end. Try again.".into()));
    }
    Ok(())
}

fn get_text(url: &str, cookie: Option<&str>) -> Result<String> {
    let mut req = agent().get(url);
    if let Some(c) = cookie {
        req = req.set("Cookie", c);
    }
    req.call().map_err(|e| match e {
        ureq::Error::Status(code, _) => Error::Capture(format!("Loom refused the download ({code}). Try again in a moment.")),
        ureq::Error::Transport(t) => temporary(t.kind()),
    })?
    .into_string()
    .map_err(temporary)
}

/// A media playlist rewritten to name local files (`<prefix><n>.<ext>`), and the files to fetch
/// for it: `(url, local name)`, its init section and keys included.
pub fn localize(media: &str, base: &str, prefix: &str) -> (String, Vec<(String, String)>) {
    let mut files: Vec<(String, String)> = vec![];
    let local = |uri: &str, files: &mut Vec<(String, String)>| -> String {
        let url = resolve(base, uri);
        if let Some((_, name)) = files.iter().find(|(u, _)| *u == url) {
            return name.clone();
        }
        let path = uri.split(['?', '#']).next().unwrap_or("");
        let ext = path.rsplit_once('.').map(|(_, e)| e).filter(|e| !e.is_empty() && e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric())).unwrap_or("bin");
        let name = format!("{prefix}{}.{ext}", files.len());
        files.push((url, name.clone()));
        name
    };
    let mut out = String::new();
    for line in media.lines() {
        let l = line.trim();
        let rewritten = if l.is_empty() {
            String::new()
        } else if l.starts_with("#EXT-X-MAP:") || l.starts_with("#EXT-X-KEY:") {
            match l.find("URI=\"") {
                Some(at) => {
                    let start = at + 5;
                    let end = l[start..].find('"').map_or(l.len(), |e| start + e);
                    let name = local(&l[start..end], &mut files);
                    format!("{}{}{}", &l[..start], name, &l[end..])
                }
                None => l.to_string(),
            }
        } else if l.starts_with('#') {
            l.to_string()
        } else {
            local(l, &mut files)
        };
        out.push_str(&rewritten);
        out.push('\n');
    }
    (out, files)
}

/// Download a signed HLS playlist (its best variant and audio) into one MP4: the segments are
/// fetched here (the bundled ffmpeg has no certificate store for HTTPS), then ffmpeg joins them
/// without re-encoding.
fn fetch_hls(url: &str, out: &Path, progress: &dyn Fn(f64), cancel: &AtomicBool) -> Result<()> {
    let cookie = cloudfront_cookie(url);
    let master = get_text(url, cookie.as_deref())?;
    let (video, audio) = pick_variants(&master, url).unwrap_or_else(|| (url.to_string(), None));
    let work = out.with_extension("parts");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| Error::Io { path: work.clone(), source: e })?;
    let result = (|| {
        let mut playlists = vec![];
        let mut files = vec![];
        for (prefix, u) in std::iter::once(("v", &video)).chain(audio.iter().map(|a| ("a", a))) {
            let text = if *u == url { master.clone() } else { get_text(u, cookie.as_deref())? };
            let (local, mut f) = localize(&text, u, prefix);
            let name = format!("{prefix}.m3u8");
            std::fs::write(work.join(&name), local).map_err(|e| Error::Io { path: work.join(&name), source: e })?;
            playlists.push(name);
            files.append(&mut f);
        }
        if files.is_empty() {
            return Err(Error::Capture("Loom's playlist for this video is empty.".into()));
        }
        // the download is nearly all of the time; joining is quick
        let n = files.len() as f64;
        for (i, (u, name)) in files.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err(cancelled());
            }
            let mut req = agent().get(u);
            if let Some(c) = &cookie {
                req = req.set("Cookie", c);
            }
            let resp = req.call().map_err(|e| match e {
                ureq::Error::Status(code, _) => Error::Capture(format!("Loom refused part of the video ({code}). Try again in a moment.")),
                ureq::Error::Transport(t) => temporary(t.kind()),
            })?;
            let path = work.join(name);
            let mut file = std::fs::File::create(&path).map_err(|e| Error::Io { path: path.clone(), source: e })?;
            std::io::copy(&mut resp.into_reader(), &mut file).map_err(temporary)?;
            progress(0.95 * (i + 1) as f64 / n);
        }
        // run in the parts folder, so a relative ffmpeg path (SF_FFMPEG) is made whole first
        let ffmpeg = crate::ffmpeg::ffmpeg_path()?;
        let mut cmd = std::process::Command::new(std::path::absolute(ffmpeg).unwrap_or_else(|_| ffmpeg.clone()));
        crate::ffmpeg::hide_console(&mut cmd);
        cmd.current_dir(&work).args(["-y", "-nostdin", "-hide_banner", "-loglevel", "error"]);
        for p in &playlists {
            cmd.args(["-allowed_extensions", "ALL", "-i", p]);
        }
        cmd.args(["-map", "0:v:0", "-map", if playlists.len() > 1 { "1:a:0" } else { "0:a:0?" }]);
        // ffmpeg runs in the parts folder: the output by its full path
        let out = std::path::absolute(out).map_err(|e| Error::Io { path: out.to_path_buf(), source: e })?;
        cmd.args(["-c", "copy", "-bsf:a", "aac_adtstoasc", "-f", "mp4", "-movflags", "+faststart"]).arg(&out);
        let done = cmd.output().map_err(|e| Error::Capture(format!("ffmpeg: {e}")))?;
        if !done.status.success() {
            let err = String::from_utf8_lossy(&done.stderr);
            return Err(Error::Capture(format!("Could not put the Loom video together: {}", err.lines().last().unwrap_or("ffmpeg failed"))));
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&work);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn video_ids_from_loom_links() {
        assert_eq!(video_id(&format!("https://www.loom.com/share/{ID}")).as_deref(), Some(ID));
        assert_eq!(video_id(&format!("https://loom.com/share/{ID}?sid=abc#t=10")).as_deref(), Some(ID));
        assert_eq!(video_id(&format!("loom.com/embed/{ID}")).as_deref(), Some(ID));
        assert_eq!(video_id(&format!(" https://www.loom.com/share/My-Demo-Video-{} ", ID.to_uppercase())).as_deref(), Some(ID));
        assert_eq!(video_id(&format!("https://notloom.com/share/{ID}")), None);
        assert_eq!(video_id(&format!("https://www.loom.com/looms/videos/{ID}")), None);
        assert_eq!(video_id("https://www.loom.com/share/short"), None);
        assert_eq!(video_id("/Users/me/Movies/demo.mp4"), None);
    }

    #[test]
    fn mp4_and_signed_playlists() {
        let mp4 = parse_session_url(&json!({ "url": "https://cdn.loom.com/sessions/transcoded/x.mp4?Policy=p&Signature=s" }));
        assert_eq!(mp4, Some(Source::File("https://cdn.loom.com/sessions/transcoded/x.mp4?Policy=p&Signature=s".into())));
        let hls = parse_session_url(&json!({ "url": "https://luna.loom.com/id/x/playlist.m3u8?old=1", "part_credentials": { "Policy": "P", "Signature": "S", "Key-Pair-Id": "K" } }));
        assert_eq!(hls, Some(Source::Hls("https://luna.loom.com/id/x/playlist.m3u8?Policy=P&Signature=S&Key-Pair-Id=K".into())));
        let expires = parse_session_url(&json!({ "url": "https://luna.loom.com/x.mpd", "part_credentials": { "Expires": 99, "Signature": "S", "Key-Pair-Id": "K" } }));
        assert_eq!(expires, Some(Source::Dash("https://luna.loom.com/x.mpd?Expires=99&Signature=S&Key-Pair-Id=K".into())));
        let gql = json!({ "data": { "getVideo": { "nullableRawCdnUrl": { "url": "https://luna.loom.com/a/playlist.m3u8", "credentials": { "Policy": "P", "Signature": "S", "KeyPairId": "K" } } } } });
        assert_eq!(parse_graphql_source(&gql), Some(Source::Hls("https://luna.loom.com/a/playlist.m3u8?Policy=P&Signature=S&Key-Pair-Id=K".into())));
        assert_eq!(parse_graphql_source(&json!({ "data": { "getVideo": { "nullableRawCdnUrl": null } } })), None);
        assert_eq!(parse_session_url(&json!({ "url": "http://insecure/x.mp4" })), None);
        assert_eq!(parse_session_url(&json!({})), None);
    }

    #[test]
    fn hls_master_picks_the_best_variant_and_its_audio() {
        let master = "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"audio\",NAME=\"audio\",DEFAULT=YES,URI=\"mediaplaylist-audio.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=1500000,RESOLUTION=1272x720,AUDIO=\"audio\"\nmediaplaylist-video-bitrate1500.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=3200000,RESOLUTION=1908x1080,AUDIO=\"audio\"\nmediaplaylist-video-bitrate3200.m3u8\n";
        let base = "https://luna.loom.com/id/x/resource/hls/playlist.m3u8?Policy=P&Signature=S&Key-Pair-Id=K";
        let (v, a) = pick_variants(master, base).unwrap();
        assert_eq!(v, "https://luna.loom.com/id/x/resource/hls/mediaplaylist-video-bitrate3200.m3u8");
        assert_eq!(a.as_deref(), Some("https://luna.loom.com/id/x/resource/hls/mediaplaylist-audio.m3u8"));
        assert_eq!(pick_variants("#EXTM3U\n#EXTINF:4.0,\nseg0.ts\n", base), None);
        assert_eq!(playlist_duration_ms("#EXTINF:4.5,\na.ts\n#EXTINF:2.0,\nb.ts\n"), Some(6500.0));
        assert_eq!(cloudfront_cookie(base).as_deref(), Some("CloudFront-Policy=P; CloudFront-Signature=S; CloudFront-Key-Pair-Id=K"));
        assert_eq!(cloudfront_cookie("https://x/a.m3u8"), None);
    }

    #[test]
    fn media_playlists_name_local_files() {
        let media = "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:5.0,\nseg-0.m4s?x=1\n#EXTINF:5.0,\nhttps://cdn.example/seg-1.m4s\n#EXT-X-ENDLIST\n";
        let (local, files) = localize(media, "https://luna.loom.com/id/x/hls/v.m3u8?Policy=P", "v");
        assert_eq!(local, "#EXTM3U\n#EXT-X-MAP:URI=\"v0.mp4\"\n#EXTINF:5.0,\nv1.m4s\n#EXTINF:5.0,\nv2.m4s\n#EXT-X-ENDLIST\n");
        assert_eq!(files, vec![("https://luna.loom.com/id/x/hls/init.mp4".to_string(), "v0.mp4".to_string()), ("https://luna.loom.com/id/x/hls/seg-0.m4s?x=1".into(), "v1.m4s".into()), ("https://cdn.example/seg-1.m4s".into(), "v2.m4s".into())]);
    }

    #[test]
    fn info_reads_title_and_download_setting() {
        let v = json!({ "data": { "getVideo": { "id": ID, "name": " Weekly update ", "createdAt": "2026-11-01T10:00:00Z", "downloadable": true, "download_enabled": false } } });
        let i = parse_info(&v);
        assert_eq!(i.title.as_deref(), Some("Weekly update"));
        assert_eq!(i.downloads_allowed, Some(false));
        assert_eq!(parse_info(&json!({ "errors": [] })), Info::default());
    }

    #[test]
    fn titles_become_file_names() {
        assert_eq!(file_name(Some("Q3: plan / review?")), "Q3 plan  review");
        assert_eq!(file_name(Some("  ")), "Loom video");
        assert_eq!(file_name(None), "Loom video");
        assert_eq!(anon_id().len(), 36);
    }
}
