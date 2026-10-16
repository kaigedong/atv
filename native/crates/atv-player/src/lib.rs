//! Shared native live-player engine. No JNI, window pointers or OS codec APIs.
pub mod engine;
pub mod identity;
pub mod playback;
pub mod quality;
pub use atv_media::{MediaOutput, OutputError, OutputFactory, OutputStats};
pub use engine::{ControlError, LiveSource, Player, Session};
pub use quality::Quality;
