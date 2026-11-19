//! Projektmodell von StudioCraft.
//!
//! Siehe `docs/spec.md`, Abschnitt „Begriffe und Datenmodell“. Zeitangaben
//! werden tempo-unabhängig in Ticks gespeichert und erst über die
//! [`TempoMap`] in Sekunden umgerechnet.

pub mod automation;
pub mod copies;
pub mod eq;
pub mod file;
pub mod folders;
pub mod markers;
pub mod model;
pub mod quantize;
pub mod time;

pub use automation::{AutoParam, AutoPoint, AutomationLane};
pub use eq::{BandKind, ChannelEq, EQ_FREQ, EQ_GAIN, EQ_Q, EqBand};
pub use markers::{FIRST_MARKER_ID, Marker};
pub use model::{
    ClipId, Event, EventId, InsertSlot, MAX_DB, MIN_DB, Note, PoolClip, Project, SendSlot,
    TRACK_COLORS, TimeBase, Track, TrackId, TrackInput, TrackKind, db_to_gain,
};
pub use quantize::{Feel, NoteValue, Quantize, QuantizeValue};
pub use time::{Position, TICKS_PER_QUARTER, TempoMap, TimeSignature};
