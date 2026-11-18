//! Audio-Engine von StudioCraft.
//!
//! Regel für alles, was im Audio-Thread läuft: keine Allokationen, keine
//! Locks, keine Systemaufrufe. GUI und Engine sprechen nur über
//! Message-Queues miteinander.

pub mod transport;

pub use transport::{Transport, TransportState};
