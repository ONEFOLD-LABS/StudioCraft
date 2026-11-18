//! Projektmodell von StudioCraft.
//!
//! Siehe `docs/spec.md`, Abschnitt „Begriffe und Datenmodell“. Zeitangaben
//! werden tempo-unabhängig in Ticks gespeichert und erst über die
//! [`TempoMap`] in Sekunden umgerechnet.

pub mod file;
pub mod model;
pub mod time;

pub use model::{
    ClipId, Event, EventId, MAX_DB, MIN_DB, PoolClip, Project, Track, TrackInput, TrackKind,
    db_to_gain,
};
pub use time::{Position, TICKS_PER_QUARTER, TempoMap, TimeSignature};
