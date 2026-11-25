//! Images and videos shown over the recording: copying them into a project.

use sf_project::tracks::{self, MediaItem, MediaKind};

/// Copy an image or video into the project's media folder, a video with its length (the
/// tool rail and the agent).
pub fn import(package: &std::path::Path, path: &std::path::Path) -> Result<MediaItem, String> {
    match MediaItem::kind_of(path) {
        Some(MediaKind::Image) | Some(MediaKind::Video) => {}
        _ => return Err(format!("{} is not an image or a video", path.display())),
    }
    let probe = (MediaItem::kind_of(path) == Some(MediaKind::Video)).then(|| sf_media::probe::probe(path)).transpose().map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let duration = probe.as_ref().and_then(|p| p.duration_ms());
    let has_audio = probe.as_ref().is_some_and(|p| p.audio().is_some());
    tracks::import_media(package, path, duration, has_audio).map_err(|e| format!("could not add {}: {e}", path.display()))
}
