//! Noten-Editor (Spec „Editoren“, Issue #40): die Noten eines MIDI-Parts als Partitur.

mod edit;
pub(crate) mod notation;
mod view;

pub(crate) use view::{ScoreView, show};
