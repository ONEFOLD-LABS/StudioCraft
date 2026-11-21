//! Noten-Editor (Spec „Editoren“, Issue #40): die Noten eines MIDI-Parts als Partitur.

mod canvas;
mod edit;
mod export;
mod musicxml;
pub(crate) mod notation;
mod pdf;
mod view;

pub(crate) use view::{ScoreView, show};
