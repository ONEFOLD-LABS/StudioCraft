//! Echtzeit-Audiograph: Kanäle, Inserts, Sends und Busse als Knoten.
//!
//! Die Verarbeitungsreihenfolge wird außerhalb des Audio-Threads berechnet
//! und als fertiger Plan an die Engine übergeben.

/// Ein Verarbeitungsknoten im Graphen.
pub trait AudioNode: Send {
    /// Verarbeitet einen Block. `inputs` und `outputs` sind kanalweise Puffer.
    fn process(&mut self, inputs: &[&[f32]], outputs: &mut [&mut [f32]]);
}
