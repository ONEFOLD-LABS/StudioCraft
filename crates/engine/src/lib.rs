//! Audio-Engine von StudioCraft.
//!
//! Regel für alles, was im Audio-Thread läuft: keine Allokationen, keine
//! Locks, keine Systemaufrufe. GUI und Engine sprechen nur über
//! Message-Queues ([`Command`]) und atomare Statuswerte ([`EngineStatus`]).

pub mod clip;
pub mod core;
pub mod transport;

pub mod output;
pub mod song;

pub use clip::{AudioClip, ClipError};
pub use core::{Command, EngineControl, EngineCore, EngineStatus, engine};
pub use output::{AudioOutput, OutputInfo, PreparedOutput};
pub use song::{PlayEvent, PlayTrack, Song};
pub use transport::{Transport, TransportState};
