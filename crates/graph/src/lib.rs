//! Echtzeit-Audiograph: Kanäle, Inserts, Sends und Busse als Knoten.
//!
//! Die Verarbeitungsreihenfolge wird außerhalb des Audio-Threads berechnet
//! und als fertiger Plan an die Engine übergeben.
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

/// Ein Verarbeitungsknoten im Graphen.
pub trait AudioNode: Send {
    /// Verarbeitet einen Block. `inputs` und `outputs` sind kanalweise Puffer.
    fn process(&mut self, inputs: &[&[f32]], outputs: &mut [&mut [f32]]);
}
