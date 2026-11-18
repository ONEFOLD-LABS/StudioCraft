use studiocraft_engine::Transport;
use studiocraft_project::TempoMap;

fn main() {
    let tempo = TempoMap::default();
    let transport = Transport::default();
    println!(
        "StudioCraft {} – {} BPM, {}/{}, Transport: {:?}",
        env!("CARGO_PKG_VERSION"),
        tempo.bpm,
        tempo.signature.numerator,
        tempo.signature.denominator,
        transport.state,
    );
}
