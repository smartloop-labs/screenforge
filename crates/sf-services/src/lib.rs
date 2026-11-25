//! What ScreenForge does beyond the project file without a window: music (Jamendo, favourites),
//! Unsplash photos, AI voices (on this computer and ElevenLabs through the backend), the user's
//! own voices, share links, and images and videos over the recording. The app's panels and agent and
//! `sf mcp` share it, through [`host::LocalHost`] for the agent.

pub mod host;
pub mod media;
pub mod music;
pub mod my_voice;
pub mod narration;
pub mod share;
pub mod unsplash;
