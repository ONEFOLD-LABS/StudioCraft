//! Bearbeiten im Notenbild (Issue #182): Tonhöhen aus Notenlinien, Verschieben um Stufen der
//! Tonart, Vorzeichen und enharmonische Verwechslung. Reine Funktionen auf Noten; die
//! Oberfläche (`view.rs`) ruft sie auf und macht daraus Rückgängig-Schritte.

use studiocraft_project::{KeySignature, Note};

use super::notation::{Clef, key_alter, spell};

/// Halbtöne der Stammtöne C D E F G A H über C.
const NATURAL: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];

/// Tonhöhe der diatonischen Stufe `step` (Oktave × 7 + Stufe, C4 = 28) in der Tonart:
/// Stammton mit dem Vorzeichen der Tonart. Außerhalb von MIDI: `None`.
pub(crate) fn pitch_of_step(step: i32, key: KeySignature) -> Option<u8> {
    let octave = step.div_euclid(7);
    let letter = step.rem_euclid(7);
    let p = (octave + 1) * 12 + NATURAL[letter as usize] + i32::from(key_alter(key, step));
    u8::try_from(p).ok().filter(|p| *p <= 127)
}

/// Tonhöhe an einer Lage im Notensystem (halbe Zwischenräume über der Mittellinie).
pub(crate) fn pitch_at(clef: Clef, pos: i32, key: KeySignature) -> Option<u8> {
    pitch_of_step(clef.middle() + pos, key)
}

/// Die Note um `steps` Stufen der Tonart verschieben (auf den Linien nach oben/unten); ihre
/// Schreibweise folgt dann wieder der Tonart.
pub(crate) fn move_steps(n: &mut Note, steps: i32, key: KeySignature) {
    if steps == 0 {
        return;
    }
    let (step, _) = spell(n.pitch, key, n.spelling);
    if let Some(p) = pitch_of_step(step + steps, key) {
        n.pitch = p;
        n.spelling = 0;
    }
}

/// Einen Halbton höher, mit Kreuz geschrieben.
pub(crate) fn sharpen(n: &mut Note) {
    if n.pitch < 127 {
        n.pitch += 1;
        n.spelling = 1;
    }
}

/// Einen Halbton tiefer, mit Be geschrieben.
pub(crate) fn flatten(n: &mut Note) {
    if n.pitch > 0 {
        n.pitch -= 1;
        n.spelling = -1;
    }
}

/// Enharmonisch verwechseln: Kreuz wird Be und umgekehrt (nur schwarze Tasten).
pub(crate) fn respell(n: &mut Note, key: KeySignature) {
    let (_, alter) = spell(n.pitch, key, n.spelling);
    n.spelling = match alter {
        1 => -1,
        -1 => 1,
        _ => n.spelling,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    const C: KeySignature = KeySignature::C_MAJOR;
    const G: KeySignature = KeySignature {
        fifths: 1,
        minor: false,
    };

    fn note(pitch: u8) -> Note {
        Note {
            pitch,
            velocity: 100,
            start: 0,
            length: 960,
            spelling: 0,
        }
    }

    #[test]
    fn lines_give_pitches_in_the_key() {
        // Treble middle line is B4 (71); the top line F5 is F♯ in G major.
        assert_eq!(pitch_at(Clef::Treble, 0, C), Some(71));
        assert_eq!(pitch_at(Clef::Treble, 4, C), Some(77));
        assert_eq!(pitch_at(Clef::Treble, 4, G), Some(78));
        assert_eq!(pitch_at(Clef::Treble, -6, C), Some(60), "middle C");
        assert_eq!(pitch_at(Clef::Bass, 0, C), Some(50), "D3");
        assert_eq!(pitch_at(Clef::Treble, 200, C), None, "above MIDI");
    }

    #[test]
    fn moving_by_steps_stays_in_the_key() {
        let mut n = note(76); // E5
        move_steps(&mut n, 1, G);
        assert_eq!(n.pitch, 78, "E to F♯ in G major");
        move_steps(&mut n, -2, G);
        assert_eq!(n.pitch, 74, "down to D");
        let mut s = note(61);
        s.spelling = -1; // D♭4
        move_steps(&mut s, 1, C);
        assert_eq!((s.pitch, s.spelling), (64, 0), "D♭ up a step is E");
    }

    #[test]
    fn accidentals_and_respelling() {
        let mut n = note(60);
        sharpen(&mut n);
        assert_eq!((n.pitch, n.spelling), (61, 1));
        respell(&mut n, C);
        assert_eq!(n.spelling, -1, "C♯ becomes D♭");
        assert_eq!(spell(n.pitch, C, n.spelling), (4 * 7 + 1, -1));
        respell(&mut n, C);
        assert_eq!(n.spelling, 1);
        flatten(&mut n);
        assert_eq!((n.pitch, n.spelling), (60, -1));
        let mut white = note(64);
        respell(&mut white, C);
        assert_eq!(white.spelling, 0, "white keys stay");
        let mut top = note(127);
        sharpen(&mut top);
        assert_eq!(top.pitch, 127);
        let mut bottom = note(0);
        flatten(&mut bottom);
        assert_eq!(bottom.pitch, 0);
    }
}
