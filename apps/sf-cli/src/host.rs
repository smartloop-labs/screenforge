//! The editing tools' host outside the app: what `sf mcp` gives [`sf_agent::Workspace`] in
//! place of the editor. It is the app's own host ([`sf_services::host::LocalHost`]), so the
//! tools here do what they do in the agent panel: Unsplash photos, Jamendo music and the
//! user's favourites, ElevenLabs voices and the user's own voice included.

use std::sync::Arc;

use sf_agent::{Host, Workspace};
use sf_services::host::LocalHost;
use sf_session::Session;

pub use sf_services::host::audio_tracks;

/// The host for `session`'s project (nothing to repaint here).
pub fn new_host(session: &Session) -> LocalHost {
    LocalHost::new(session, Arc::new(|| {}))
}

/// The agent's view of an open project, as the editor builds it when a chat turn starts.
pub fn workspace(session: &Session, host: Arc<dyn Host>) -> Workspace {
    let t = &session.cursor_track;
    let clicks: Vec<f64> = t.clicks.iter().filter(|c| c.down).map(|c| c.time_ms).collect();
    let moves: Vec<f64> = t.samples.iter().map(|s| s.time_ms).collect();
    let activity = (!t.key_downs.is_empty() || !clicks.is_empty() || !moves.is_empty()).then(|| sf_agent::activity::Activity::from_events(&t.key_downs, &clicks, &moves, session.source.duration_ms));
    let (w, h) = session.content_size_at(0.0);
    let mut ws = Workspace::new(session.pkg.project.clone());
    ws.words = session.captions.lines.iter().flat_map(|l| l.words.iter().cloned()).collect();
    ws.auto_zooms = session.auto_zooms();
    ws.activity = activity;
    ws.content_px = (w as f64, h as f64);
    ws.audio_tracks = audio_tracks(session);
    ws.host = Some(host);
    ws
}
