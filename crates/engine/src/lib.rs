//! Audio-Engine von StudioCraft.
//!
//! Regel für alles, was im Audio-Thread läuft: keine Allokationen, keine
//! Locks, keine Systemaufrufe. GUI und Engine sprechen nur über
//! Message-Queues ([`Command`]) und atomare Statuswerte ([`EngineStatus`]).
// Kein unwrap/expect/panic außerhalb von Tests (AGENTS.md „Robustness“, Issue #49).
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::todo,
        clippy::unimplemented,
        clippy::unreachable
    )
)]

pub mod automation;
pub mod clip;
pub mod clock;
pub mod core;
pub mod delay;
pub mod eq;
pub mod error;
pub mod export;
pub mod input;
pub mod inserts;
pub mod meters;
pub mod midi;
mod midi_backend;
pub mod midi_out;
pub mod monitor;
pub mod offline;
pub mod onsets;
pub mod transport;

pub mod output;
pub mod song;
pub mod stretch;
pub mod wasapi;

pub use clip::{AudioClip, ClipError};
pub use core::{Command, EngineControl, EngineCore, EngineStatus, engine};
pub use error::{EngineError, WasapiProblem};
pub use input::{AudioInput, InputInfo, PreparedInput, Recorder, Recording};
pub use inserts::{Insert, InsertChain};
pub use meters::{InputLevels, MAX_INPUT_CHANNELS, MAX_METERED_TRACKS, Meters};
pub use monitor::MonitorInput;
pub use output::{AudioOutput, DeviceChoice, OutputInfo, PreparedOutput, device_names};
pub use song::{PlayEvent, PlayNote, PlayTrack, Song};
pub use transport::{Transport, TransportState};
