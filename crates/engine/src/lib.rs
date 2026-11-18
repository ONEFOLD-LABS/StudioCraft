//! Audio-Engine von StudioCraft.
//!
//! Regel für alles, was im Audio-Thread läuft: keine Allokationen, keine
//! Locks, keine Systemaufrufe. GUI und Engine sprechen nur über
//! Message-Queues ([`Command`]) und atomare Statuswerte ([`EngineStatus`]).

pub mod clip;
pub mod clock;
pub mod core;
pub mod export;
pub mod input;
pub mod inserts;
pub mod meters;
pub mod midi;
pub mod transport;

pub mod output;
pub mod song;

pub use clip::{AudioClip, ClipError};
pub use core::{Command, EngineControl, EngineCore, EngineStatus, engine};
pub use input::{AudioInput, InputInfo, PreparedInput, Recorder, Recording};
pub use inserts::{Insert, InsertChain};
pub use meters::{MAX_METERED_TRACKS, Meters};
pub use output::{AudioOutput, OutputInfo, PreparedOutput};
pub use song::{PlayEvent, PlayNote, PlayTrack, Song};
pub use transport::{Transport, TransportState};
