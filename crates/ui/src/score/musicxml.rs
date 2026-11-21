//! MusicXML-Export (Issue #183): eigener Writer für das offene Format (MusicXML 4.0,
//! `score-partwise`), aus demselben Satz wie die Anzeige. Je MIDI-Part ein `<part>`, ein
//! Klaviersystem als zwei Notensysteme eines Parts.

use std::fmt::Write as _;

use super::notation::{Accidental, Base, Clef, Item, Kind, Score, spell};

/// Teilungen je Viertel: die Zweiunddreißigstel ist eine Einheit.
const DIVISIONS: u64 = 8;
const TICKS_PER_DIVISION: u64 = studiocraft_project::TICKS_PER_QUARTER / DIVISIONS;

/// Ein Part für den Export: Name und Satz.
pub(crate) struct PartScore {
    pub name: String,
    pub score: Score,
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Steuerzeichen sind in XML 1.0 nicht erlaubt.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

fn type_name(b: Base) -> &'static str {
    match b {
        Base::Whole => "whole",
        Base::Half => "half",
        Base::Quarter => "quarter",
        Base::Eighth => "eighth",
        Base::Sixteenth => "16th",
        Base::ThirtySecond => "32nd",
    }
}

fn duration(ticks: u64) -> u64 {
    (ticks / TICKS_PER_DIVISION).max(1)
}

/// Das ganze Dokument.
pub(crate) fn write(title: &str, parts: &[PartScore]) -> String {
    let mut x = String::new();
    x.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    x.push_str(
        "<!DOCTYPE score-partwise PUBLIC \"-//Recordare//DTD MusicXML 4.0 Partwise//EN\" \
         \"http://www.musicxml.org/dtds/partwise.dtd\">\n",
    );
    x.push_str("<score-partwise version=\"4.0\">\n");
    let _ = writeln!(x, "  <work><work-title>{}</work-title></work>", esc(title));
    x.push_str("  <identification><encoding><software>StudioCraft</software></encoding></identification>\n");
    x.push_str("  <part-list>\n");
    for (i, p) in parts.iter().enumerate() {
        let _ = writeln!(
            x,
            "    <score-part id=\"P{}\"><part-name>{}</part-name></score-part>",
            i + 1,
            esc(&p.name)
        );
    }
    x.push_str("  </part-list>\n");
    for (i, p) in parts.iter().enumerate() {
        let _ = writeln!(x, "  <part id=\"P{}\">", i + 1);
        write_part(&mut x, &p.score);
        x.push_str("  </part>\n");
    }
    x.push_str("</score-partwise>\n");
    x
}

fn write_part(x: &mut String, score: &Score) {
    let staves = score.clefs.len();
    // Je Notensystem die Tonhöhen, die aus dem vorigen Element übergebunden ankommen.
    let mut tied_in: Vec<Vec<u8>> = vec![Vec::new(); staves];
    for (b, bar) in score.bars.iter().enumerate() {
        let _ = writeln!(x, "    <measure number=\"{}\">", bar.number);
        if b == 0 || bar.show_sig {
            x.push_str("      <attributes>\n");
            if b == 0 {
                let _ = writeln!(x, "        <divisions>{DIVISIONS}</divisions>");
                let mode = if score.key.minor { "minor" } else { "major" };
                let _ = writeln!(
                    x,
                    "        <key><fifths>{}</fifths><mode>{mode}</mode></key>",
                    score.key.fifths
                );
            }
            if bar.show_sig {
                let _ = writeln!(
                    x,
                    "        <time><beats>{}</beats><beat-type>{}</beat-type></time>",
                    bar.sig.numerator, bar.sig.denominator
                );
            }
            if b == 0 {
                if staves > 1 {
                    let _ = writeln!(x, "        <staves>{staves}</staves>");
                }
                for (s, clef) in score.clefs.iter().enumerate() {
                    let (sign, line) = match clef {
                        Clef::Treble => ("G", 2),
                        Clef::Bass => ("F", 4),
                    };
                    let _ = writeln!(
                        x,
                        "        <clef number=\"{}\"><sign>{sign}</sign><line>{line}</line></clef>",
                        s + 1
                    );
                }
            }
            x.push_str("      </attributes>\n");
        }
        for (s, items) in bar.staves.iter().enumerate() {
            if s > 0 {
                let _ = writeln!(
                    x,
                    "      <backup><duration>{}</duration></backup>",
                    duration(bar.sig.bar_ticks())
                );
            }
            for (k, it) in items.iter().enumerate() {
                let beam = beam_role(items, k);
                write_item(
                    x,
                    score,
                    it,
                    s,
                    staves,
                    bar.sig.bar_ticks(),
                    &mut tied_in[s],
                    beam,
                );
            }
        }
        x.push_str("    </measure>\n");
    }
}

/// Lage im Balken: Anfang, Mitte oder Ende einer Gruppe.
fn beam_role(items: &[Item], k: usize) -> Option<&'static str> {
    let id = items[k].beam?;
    let prev = k.checked_sub(1).and_then(|j| items[j].beam) == Some(id);
    let next = items.get(k + 1).and_then(|i| i.beam) == Some(id);
    Some(match (prev, next) {
        (false, _) => "begin",
        (true, true) => "continue",
        (true, false) => "end",
    })
}

#[allow(clippy::too_many_arguments)]
fn write_item(
    x: &mut String,
    score: &Score,
    it: &Item,
    staff: usize,
    staves: usize,
    bar_ticks: u64,
    tied_in: &mut Vec<u8>,
    beam: Option<&str>,
) {
    let staff_tag = if staves > 1 {
        format!("<staff>{}</staff>", staff + 1)
    } else {
        String::new()
    };
    let voice = staff + 1;
    match &it.kind {
        Kind::BarRest => {
            let _ = writeln!(
                x,
                "      <note><rest measure=\"yes\"/><duration>{}</duration><voice>{voice}</voice>{staff_tag}</note>",
                duration(bar_ticks)
            );
            tied_in.clear();
        }
        Kind::Rest => {
            let dot = if it.value.dotted { "<dot/>" } else { "" };
            let _ = writeln!(
                x,
                "      <note><rest/><duration>{}</duration><voice>{voice}</voice><type>{}</type>{dot}{staff_tag}</note>",
                duration(it.value.ticks()),
                type_name(it.value.base)
            );
            tied_in.clear();
        }
        Kind::Chord(heads) => {
            let mut next_in = Vec::new();
            for (j, h) in heads.iter().enumerate() {
                let (step, alter) = spell(h.pitch, score.key, h.spelling);
                let letter = ["C", "D", "E", "F", "G", "A", "B"][step.rem_euclid(7) as usize];
                let octave = step.div_euclid(7);
                let stop = tied_in.contains(&h.pitch);
                let mut s = String::from("      <note>");
                if j > 0 {
                    s.push_str("<chord/>");
                }
                let _ = write!(s, "<pitch><step>{letter}</step>");
                if alter != 0 {
                    let _ = write!(s, "<alter>{alter}</alter>");
                }
                let _ = write!(
                    s,
                    "<octave>{octave}</octave></pitch><duration>{}</duration>",
                    duration(it.value.ticks())
                );
                if stop {
                    s.push_str("<tie type=\"stop\"/>");
                }
                if h.tie {
                    s.push_str("<tie type=\"start\"/>");
                }
                let _ = write!(
                    s,
                    "<voice>{voice}</voice><type>{}</type>",
                    type_name(it.value.base)
                );
                if it.value.dotted {
                    s.push_str("<dot/>");
                }
                if let Some(a) = h.accidental {
                    let name = match a {
                        Accidental::Sharp => "sharp",
                        Accidental::Flat => "flat",
                        Accidental::Natural => "natural",
                    };
                    let _ = write!(s, "<accidental>{name}</accidental>");
                }
                if it.value.base != Base::Whole {
                    s.push_str(if it.stem_up {
                        "<stem>up</stem>"
                    } else {
                        "<stem>down</stem>"
                    });
                }
                s.push_str(&staff_tag);
                if j == 0
                    && let Some(b) = beam
                {
                    for level in 1..=it.value.base.beams() {
                        let _ = write!(s, "<beam number=\"{level}\">{b}</beam>");
                    }
                }
                if stop || h.tie {
                    s.push_str("<notations>");
                    if stop {
                        s.push_str("<tied type=\"stop\"/>");
                    }
                    if h.tie {
                        s.push_str("<tied type=\"start\"/>");
                    }
                    s.push_str("</notations>");
                }
                s.push_str("</note>\n");
                x.push_str(&s);
                if h.tie {
                    next_in.push(h.pitch);
                }
            }
            *tied_in = next_in;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::notation::{Input, layout};
    use super::*;
    use studiocraft_project::{KeySignature, Note, TempoMap};

    fn note(start: i64, length: u64, pitch: u8) -> Note {
        Note {
            start,
            length,
            pitch,
            velocity: 100,
            spelling: 0,
        }
    }

    fn part(notes: &[Note], len: u64, key: KeySignature) -> PartScore {
        let tempo = TempoMap::default();
        PartScore {
            name: "Flöte & <Bass>".into(),
            score: layout(&Input {
                notes,
                part_start: 0,
                part_len: len,
                tempo: &tempo,
                key,
                quantize: 240,
            }),
        }
    }

    #[test]
    fn a_tied_melody_becomes_partwise_musicxml() {
        let q = 960;
        // F♯ quarter, then a half over the bar line (tied), in G major.
        let g = KeySignature {
            fifths: 1,
            minor: false,
        };
        let p = part(&[note(0, q, 66), note(3 * q as i64, 2 * q, 72)], 8 * q, g);
        let xml = write("Lied", &[p]);
        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(xml.contains("<work-title>Lied</work-title>"));
        assert!(xml.contains("<part-name>Flöte &amp; &lt;Bass&gt;</part-name>"));
        assert!(xml.contains("<key><fifths>1</fifths><mode>major</mode></key>"));
        assert!(xml.contains("<time><beats>4</beats><beat-type>4</beat-type></time>"));
        assert!(xml.contains("<clef number=\"1\"><sign>G</sign><line>2</line></clef>"));
        // F♯ in G major: alter 1 but no accidental shown.
        assert!(xml.contains("<step>F</step><alter>1</alter><octave>4</octave>"));
        assert!(!xml.contains("<accidental>sharp</accidental>"));
        assert_eq!(xml.matches("<tie type=\"start\"/>").count(), 1);
        assert_eq!(xml.matches("<tie type=\"stop\"/>").count(), 1);
        assert_eq!(xml.matches("<measure ").count(), 2);
        assert!(xml.contains("<rest measure=\"yes\"/>") || xml.contains("<rest/>"));
        // Every tag that opens closes (simple well-formedness check).
        for tag in [
            "score-partwise",
            "part-list",
            "part",
            "measure",
            "note",
            "attributes",
        ] {
            let open =
                xml.matches(&format!("<{tag}>")).count() + xml.matches(&format!("<{tag} ")).count();
            assert_eq!(open, xml.matches(&format!("</{tag}>")).count(), "{tag}");
        }
    }

    #[test]
    fn grand_staff_and_beams() {
        let q = 960i64;
        let notes: Vec<Note> = (0..4)
            .map(|k| note(k * q / 2, 480, 72))
            .chain([note(0, 3840, 43)])
            .collect();
        let p = part(&notes, 3840, KeySignature::C_MAJOR);
        let xml = write("", &[p]);
        assert!(xml.contains("<staves>2</staves>"));
        assert!(xml.contains("<clef number=\"2\"><sign>F</sign><line>4</line></clef>"));
        assert!(xml.contains("<backup><duration>32</duration></backup>"));
        assert_eq!(xml.matches("<beam number=\"1\">begin</beam>").count(), 2);
        assert_eq!(xml.matches("<beam number=\"1\">end</beam>").count(), 2);
        assert!(xml.contains("<staff>2</staff>"));
    }
}
