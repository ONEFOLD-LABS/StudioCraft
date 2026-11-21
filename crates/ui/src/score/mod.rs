//! Noten-Editor (Spec „Editoren“, Issue #40): die Noten eines MIDI-Parts als Partitur.

pub(crate) mod notation;
mod view;

pub(crate) use view::{ScoreView, show};
