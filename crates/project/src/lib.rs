//! Projektmodell von StudioCraft.
//!
//! Siehe `docs/spec.md`, Abschnitt „Begriffe und Datenmodell“. Zeitangaben
//! werden tempo-unabhängig in Ticks gespeichert und erst über die
//! [`TempoMap`] in Sekunden umgerechnet.
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

pub mod audio_parts;
pub mod automation;
pub mod click;
pub mod controllers;
pub mod copies;
pub mod drummap;
pub mod eq;
pub mod file;
pub mod folders;
pub mod io;
pub mod layout;
pub mod links;
pub mod markers;
pub mod model;
pub mod processing;
pub mod quantize;
pub mod recording;
pub mod sanitize;
pub mod score;
pub mod shapes;
pub mod smf;
pub mod time;
pub mod warp;

pub use automation::{AutoParam, AutoPoint, AutomationLane};
pub use click::{CLICK_DB, ClickSettings, ClickSound, MAX_PRECOUNT_BARS};
pub use controllers::{CtrlKind, CtrlPoint};
pub use drummap::{DRUM_MAP_EXTENSION, DrumMap, DrumRow};
pub use eq::{BandKind, ChannelEq, EQ_FREQ, EQ_GAIN, EQ_Q, EqBand};
pub use io::{AudioIo, IoBus, MAIN_OUT, MAX_IO_BUSES};
pub use layout::{NamedLayout, WindowLayout};
pub use links::{LinkState, MAX_LINK_GROUPS};
pub use markers::{FIRST_MARKER_ID, Marker};
pub use model::{
    AudiblePart, COMP_FADE, ClipId, EVENT_MAX_DB, Event, EventId, FadeCurve, Frozen, InsertSlot,
    MAX_DB, MAX_EXTRA_OUTPUTS, MAX_MIXER_VIEWS, MIN_DB, MixerView, Note, OutTarget, PanMode,
    PoolClip, Project, RulerFormat, SendSlot, StretchMode, TRACK_COLORS, TimeBase, Track, TrackId,
    TrackInput, TrackKind, bpm_in_name, content_frames, content_ticks, db_to_gain, pan_matrix,
};
pub use processing::{ClipProcess, Derived, ProcessKind};
pub use quantize::{Feel, NoteValue, Quantize, QuantizeValue};
pub use recording::{CycleRecordMode, MidiRecordMode, RecordSettings, TakeNote};
pub use score::{DEFAULT_SCORE_QUANTIZE, KeySignature, SCORE_QUANTIZE, score_quantize};
pub use shapes::DrawShape;
pub use time::{
    BPM_RANGE, Position, SignatureEvent, TICKS_PER_QUARTER, TempoEvent, TempoMap, TimeSignature,
};
pub use warp::{ContentMap, WarpPoint};
