//! Notensatz ohne Zeichnen (Issue #40): aus den Noten eines MIDI-Parts werden Takte mit
//! Noten, Pausen, Haltebögen, Balken und Vorzeichen. Reine Logik, damit sie gründlich getestet
//! und später für MusicXML (#183) wiederverwendet werden kann.
//!
//! Vereinfachungen dieses Teils:
//! - Je System eine Stimme pro Notensystem: Klingen Noten über den nächsten Einsatz hinaus,
//!   endet ihre Anzeige dort (wie die Anzeige-Quantisierung ändert das nichts am Part).
//! - Keine Triolen; Dauern werden auf die Anzeige-Quantisierung gerundet (Viertel bis 32tel).
//! - Notenwerte zeigen den Schlag: Ein Wert beginnt nur auf einem Vielfachen seines
//!   unpunktierten Werts, sonst wird er geteilt und übergebunden.

use studiocraft_project::{KeySignature, Note, TICKS_PER_QUARTER, TempoMap, TimeSignature};

/// Kleinster Notenwert in Ticks (Zweiunddreißigstel).
pub(crate) const MIN_TICKS: u64 = TICKS_PER_QUARTER / 8;
/// Höchstens so viele Takte werden gesetzt (Schutz bei sehr langen Parts).
pub(crate) const MAX_BARS: usize = 2000;

/// Notenschlüssel eines Systems.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Clef {
    Treble,
    Bass,
}

impl Clef {
    /// Diatonische Stufe der Mittellinie (Oktave × 7 + Stufe, C4 = 28): h' bzw. d.
    fn middle(self) -> i32 {
        match self {
            Self::Treble => 4 * 7 + 6,
            Self::Bass => 3 * 7 + 1,
        }
    }

    /// Lage der Vorzeichen der Tonart (Halbe Zwischenräume über der Mittellinie).
    pub(crate) fn key_positions(self, fifths: i8) -> Vec<i32> {
        const SHARPS: [i32; 7] = [4, 1, 5, 2, -1, 3, 0];
        const FLATS: [i32; 7] = [0, 3, -1, 2, -3, 1, -5];
        let shift = match self {
            Self::Treble => 0,
            Self::Bass => -2,
        };
        let n = usize::from(fifths.unsigned_abs()).min(7);
        let table = if fifths >= 0 { &SHARPS } else { &FLATS };
        table[..n].iter().map(|p| p + shift).collect()
    }
}

/// Grundwert einer Note oder Pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Base {
    ThirtySecond,
    Sixteenth,
    Eighth,
    Quarter,
    Half,
    Whole,
}

impl Base {
    pub(crate) fn ticks(self) -> u64 {
        match self {
            Self::Whole => TICKS_PER_QUARTER * 4,
            Self::Half => TICKS_PER_QUARTER * 2,
            Self::Quarter => TICKS_PER_QUARTER,
            Self::Eighth => TICKS_PER_QUARTER / 2,
            Self::Sixteenth => TICKS_PER_QUARTER / 4,
            Self::ThirtySecond => TICKS_PER_QUARTER / 8,
        }
    }

    /// Anzahl der Balken bzw. Fähnchen (Achtel 1, Sechzehntel 2, 32tel 3).
    pub(crate) fn beams(self) -> u8 {
        match self {
            Self::Eighth => 1,
            Self::Sixteenth => 2,
            Self::ThirtySecond => 3,
            _ => 0,
        }
    }

    const ALL: [Self; 6] = [
        Self::Whole,
        Self::Half,
        Self::Quarter,
        Self::Eighth,
        Self::Sixteenth,
        Self::ThirtySecond,
    ];
}

/// Notenwert: Grundwert und Punktierung (0 oder 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Value {
    pub base: Base,
    pub dotted: bool,
}

impl Value {
    pub(crate) fn ticks(self) -> u64 {
        let b = self.base.ticks();
        if self.dotted { b + b / 2 } else { b }
    }
}

/// Vorzeichen vor einer Note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Accidental {
    Sharp,
    Flat,
    Natural,
}

/// Ein Notenkopf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Head {
    pub pitch: u8,
    /// Lage in halben Zwischenräumen über der Mittellinie des Systems (0 = Mittellinie).
    pub pos: i32,
    pub accidental: Option<Accidental>,
    /// Übergebunden zur gleichen Note im nächsten Element.
    pub tie: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Kind {
    Rest,
    /// Ein ganzer Takt Pause (zentrierte Ganze Pause, unabhängig von der Taktart).
    BarRest,
    Chord(Vec<Head>),
}

/// Eine Note (Akkord) oder Pause in einem Takt.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Item {
    /// Anfang in Ticks ab Projektanfang.
    pub start: u64,
    pub value: Value,
    pub kind: Kind,
    /// Notenhals nach oben.
    pub stem_up: bool,
    /// Nummer der Balkengruppe in diesem Takt und System.
    pub beam: Option<u32>,
}

/// Ein Takt mit je einer Elementliste pro System.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Bar {
    pub number: u64,
    pub start: u64,
    pub sig: TimeSignature,
    /// Taktart neu (erster Takt oder Taktwechsel): wird angezeigt.
    pub show_sig: bool,
    pub staves: Vec<Vec<Item>>,
}

impl Bar {
    pub(crate) fn end(&self) -> u64 {
        self.start + self.sig.bar_ticks()
    }
}

/// Der gesetzte Part.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Score {
    pub clefs: Vec<Clef>,
    pub key: KeySignature,
    pub bars: Vec<Bar>,
}

/// Was gesetzt wird: Noten relativ zum Part, wo der Part liegt, Tempo-Map (Taktarten),
/// Tonart und Anzeige-Quantisierung.
pub(crate) struct Input<'a> {
    pub notes: &'a [Note],
    pub part_start: u64,
    pub part_len: u64,
    pub tempo: &'a TempoMap,
    pub key: KeySignature,
    pub quantize: u64,
}

/// Schlüssel aus dem Tonumfang: nur Violin- oder nur Bassschlüssel, sonst Klaviersystem
/// (Trennung beim eingestrichenen c).
pub(crate) fn clefs_for(notes: &[Note]) -> Vec<Clef> {
    let (lo, hi) = notes.iter().fold((u8::MAX, 0u8), |(lo, hi), n| {
        (lo.min(n.pitch), hi.max(n.pitch))
    });
    if notes.is_empty() || lo >= 55 {
        vec![Clef::Treble]
    } else if hi <= 64 {
        vec![Clef::Bass]
    } else {
        vec![Clef::Treble, Clef::Bass]
    }
}

/// Das System einer Tonhöhe.
fn staff_of(clefs: &[Clef], pitch: u8) -> usize {
    if clefs.len() == 2 && pitch < 60 { 1 } else { 0 }
}

/// Schreibweise einer Tonhöhe in der Tonart: (diatonische Stufe absolut, Alteration −1/0/1).
fn spell(pitch: u8, key: KeySignature) -> (i32, i8) {
    // Kreuz-Tonarten (und C) schreiben schwarze Tasten mit Kreuz, Be-Tonarten mit Be.
    const SHARP: [(i32, i8); 12] = [
        (0, 0),
        (0, 1),
        (1, 0),
        (1, 1),
        (2, 0),
        (3, 0),
        (3, 1),
        (4, 0),
        (4, 1),
        (5, 0),
        (5, 1),
        (6, 0),
    ];
    const FLAT: [(i32, i8); 12] = [
        (0, 0),
        (1, -1),
        (1, 0),
        (2, -1),
        (2, 0),
        (3, 0),
        (4, -1),
        (4, 0),
        (5, -1),
        (5, 0),
        (6, -1),
        (6, 0),
    ];
    let table = if key.fifths >= 0 { &SHARP } else { &FLAT };
    let (step, alter) = table[usize::from(pitch % 12)];
    let octave = i32::from(pitch / 12) - 1;
    (octave * 7 + step, alter)
}

/// Alteration, die die Tonart einer Stufe (0 = C … 6 = H) gibt.
fn key_alter(key: KeySignature, step: i32) -> i8 {
    // Reihenfolge der Kreuze F C G D A E H und der Be H E A D G C F.
    const SHARP_ORDER: [i32; 7] = [3, 0, 4, 1, 5, 2, 6];
    const FLAT_ORDER: [i32; 7] = [6, 2, 5, 1, 4, 0, 3];
    let n = usize::from(key.fifths.unsigned_abs()).min(7);
    let s = step.rem_euclid(7);
    if key.fifths > 0 && SHARP_ORDER[..n].contains(&s) {
        1
    } else if key.fifths < 0 && FLAT_ORDER[..n].contains(&s) {
        -1
    } else {
        0
    }
}

/// Zerlegt eine Dauer ab `offset` (im Takt) in darstellbare Werte, die den Schlag zeigen:
/// Ein Wert kürzer als ein Schlag (`beat`) überschreitet keine Schlaggrenze, ein längerer
/// beginnt auf einem Schlag.
pub(crate) fn split(offset: u64, len: u64, beat: u64) -> Vec<Value> {
    let beat = beat.max(MIN_TICKS);
    // Innerhalb eines Schlags genügt ein Anfang auf dem halben Grundwert (Sechzehntel und
    // punktierte Achtel, Viertel auf der zweiten Achtel im 6/8); über Schläge hinweg beginnt
    // ein Wert auf einem Schlag und auf einem Vielfachen seines Grundwerts.
    let fits = |off: u64, v: Value| {
        let base = v.base.ticks();
        if v.ticks() < beat {
            off % (base / 2).max(MIN_TICKS) == 0 && off / beat == (off + v.ticks() - 1) / beat
        } else {
            off % beat == 0 && off % base.min(beat) == 0
        }
    };
    let mut out = Vec::new();
    let (mut off, mut left) = (offset, len);
    while left >= MIN_TICKS {
        let pick = Base::ALL.iter().find_map(|&base| {
            let b = base.ticks();
            let dotted = Value { base, dotted: true };
            if base != Base::ThirtySecond && dotted.ticks() <= left && fits(off, dotted) {
                return Some(dotted);
            }
            let plain = Value {
                base,
                dotted: false,
            };
            (b <= left && fits(off, plain)).then_some(plain)
        });
        let Some(v) = pick else { break };
        out.push(v);
        off += v.ticks();
        left -= v.ticks();
    }
    out
}

/// Ein Einsatz: ein Akkord (oder eine Pause) von `start` bis `end`, Ticks ab Projektanfang.
#[derive(Debug, Clone)]
struct Span {
    start: u64,
    end: u64,
    pitches: Vec<u8>,
}

/// Die Einsätze eines Systems: quantisiert, eine Stimme, Lücken als Pausen. `notes` ist nach
/// Anfang sortiert.
fn spans(notes: &[(u64, u64, u8)], from: u64, to: u64) -> Vec<Span> {
    // Gruppen gleicher Anfänge: (Anfang, spätestes Ende, Tonhöhen).
    let mut groups: Vec<(u64, u64, Vec<u8>)> = Vec::new();
    for &(start, end, pitch) in notes {
        match groups.last_mut() {
            Some(g) if g.0 == start => {
                g.1 = g.1.max(end);
                if !g.2.contains(&pitch) {
                    g.2.push(pitch);
                }
            }
            _ => groups.push((start, end, vec![pitch])),
        }
    }
    let mut out = Vec::new();
    let mut t = from;
    for (i, (on, end, mut pitches)) in groups.iter().cloned().enumerate() {
        if on > t {
            out.push(Span {
                start: t,
                end: on,
                pitches: Vec::new(),
            });
        }
        let next = groups.get(i + 1).map_or(to, |g| g.0);
        pitches.sort_unstable();
        let end = end.min(next).min(to);
        out.push(Span {
            start: on,
            end,
            pitches,
        });
        t = end.max(t);
    }
    if t < to {
        out.push(Span {
            start: t,
            end: to,
            pitches: Vec::new(),
        });
    }
    out.retain(|s| s.end > s.start);
    out
}

/// Takt-Raster des Parts: (Nummer, Anfang, Taktart), vom Takt des Part-Anfangs bis zum Ende.
fn bar_grid(tempo: &TempoMap, from: u64, to: u64) -> Vec<(u64, u64, TimeSignature)> {
    let mut out = Vec::new();
    let (mut number, mut start, _) = tempo.bar_at(from);
    while start < to.max(from + 1) && out.len() < MAX_BARS {
        let sig = tempo.signature_at(start);
        out.push((number, start, sig));
        number += 1;
        let next = tempo.bar_start(number);
        if next <= start {
            break;
        }
        start = next;
    }
    out
}

/// Setzt den Part.
pub(crate) fn layout(input: &Input) -> Score {
    let q = input.quantize.max(MIN_TICKS);
    let round = |t: u64| t.saturating_add(q / 2) / q * q;
    let part_end = input.part_start.saturating_add(input.part_len);
    // Quantisierte Noten in Ticks ab Projektanfang, im Part.
    let mut notes: Vec<(u64, u64, u8)> = input
        .notes
        .iter()
        .filter_map(|n| {
            let s = u64::try_from(n.start).ok()?;
            let qs = round(s);
            let qe = round(s.saturating_add(n.length)).max(qs + q);
            let a = input.part_start.saturating_add(qs);
            let b = input.part_start.saturating_add(qe).min(part_end);
            (a < b).then_some((a, b, n.pitch.min(127)))
        })
        .collect();
    notes.sort_unstable();
    let visible: Vec<Note> = notes
        .iter()
        .map(|&(s, e, p)| Note {
            start: 0,
            length: e - s,
            pitch: p,
            velocity: 100,
        })
        .collect();
    let clefs = clefs_for(&visible);
    let grid = bar_grid(input.tempo, input.part_start, part_end);
    let (Some(first), Some(last)) = (grid.first(), grid.last()) else {
        return Score {
            clefs,
            key: input.key,
            bars: Vec::new(),
        };
    };
    let from = first.1;
    let to = last.1 + last.2.bar_ticks();

    let mut bars: Vec<Bar> = grid
        .iter()
        .enumerate()
        .map(|(i, &(number, start, sig))| Bar {
            number,
            start,
            sig,
            show_sig: i == 0 || grid[i - 1].2 != sig,
            staves: vec![Vec::new(); clefs.len()],
        })
        .collect();

    for (staff, &clef) in clefs.iter().enumerate() {
        let own: Vec<(u64, u64, u8)> = notes
            .iter()
            .copied()
            .filter(|n| staff_of(&clefs, n.2) == staff)
            .collect();
        for span in spans(&own, from, to) {
            place(&mut bars, staff, &span);
        }
        for bar in &mut bars {
            finish_bar(bar, staff, clef, input.key);
        }
    }
    Score {
        clefs,
        key: input.key,
        bars,
    }
}

/// Verteilt einen Einsatz auf die Takte, zerlegt ihn in Werte und bindet über.
fn place(bars: &mut [Bar], staff: usize, span: &Span) {
    let mut pieces: Vec<(usize, u64, Value)> = Vec::new();
    for (i, bar) in bars.iter().enumerate() {
        let (a, b) = (span.start.max(bar.start), span.end.min(bar.end()));
        if a >= b {
            continue;
        }
        let mut t = a;
        for v in split(a - bar.start, b - a, beat_group(bar.sig)) {
            pieces.push((i, t, v));
            t += v.ticks();
        }
    }
    let n = pieces.len();
    for (k, (i, t, value)) in pieces.into_iter().enumerate() {
        let kind = if span.pitches.is_empty() {
            Kind::Rest
        } else {
            Kind::Chord(
                span.pitches
                    .iter()
                    .map(|&p| Head {
                        pitch: p,
                        pos: 0,
                        accidental: None,
                        tie: k + 1 < n,
                    })
                    .collect(),
            )
        };
        bars[i].staves[staff].push(Item {
            start: t,
            value,
            kind,
            stem_up: true,
            beam: None,
        });
    }
}

/// Lagen, Vorzeichen, Halsrichtung, Balken und ganze Taktpausen eines Takts.
fn finish_bar(bar: &mut Bar, staff: usize, clef: Clef, key: KeySignature) {
    let sig = bar.sig;
    let bar_start = bar.start;
    let items = &mut bar.staves[staff];
    // Leerer Takt: eine ganze Taktpause.
    if items.iter().all(|i| i.kind == Kind::Rest) {
        let value = items.first().map_or(
            Value {
                base: Base::Whole,
                dotted: false,
            },
            |i| i.value,
        );
        *items = vec![Item {
            start: bar_start,
            value,
            kind: Kind::BarRest,
            stem_up: true,
            beam: None,
        }];
        return;
    }
    // Vorzeichen: was im Takt gilt, je Stufe und Oktave; übergebundene Noten zeigen keins.
    let mut current: std::collections::HashMap<i32, i8> = std::collections::HashMap::new();
    let mut tied_in: Vec<u8> = Vec::new();
    for item in items.iter_mut() {
        let Kind::Chord(heads) = &mut item.kind else {
            tied_in.clear();
            continue;
        };
        let mut next_tied = Vec::new();
        for h in heads.iter_mut() {
            let (step, alter) = spell(h.pitch, key);
            h.pos = step - clef.middle();
            let now = *current.entry(step).or_insert_with(|| key_alter(key, step));
            if !tied_in.contains(&h.pitch) && now != alter {
                h.accidental = Some(match alter {
                    1 => Accidental::Sharp,
                    -1 => Accidental::Flat,
                    _ => Accidental::Natural,
                });
            }
            current.insert(step, alter);
            if h.tie {
                next_tied.push(h.pitch);
            }
        }
        tied_in = next_tied;
        // Unter der Mittellinie Hals nach oben, auf und über ihr nach unten.
        item.stem_up = heads.iter().map(|h| h.pos).sum::<i32>() < 0;
    }
    // Balken: Achtel und kürzer innerhalb einer Schlaggruppe, Pausen trennen.
    let group = beat_group(sig);
    let mut id = 0u32;
    let mut run: Vec<usize> = Vec::new();
    let flush = |run: &mut Vec<usize>, items: &mut Vec<Item>, id: &mut u32| {
        if run.len() >= 2 {
            let sum: i32 = run
                .iter()
                .filter_map(|&k| match &items[k].kind {
                    Kind::Chord(h) => Some(h.iter().map(|h| h.pos).sum::<i32>()),
                    _ => None,
                })
                .sum();
            for &k in run.iter() {
                items[k].beam = Some(*id);
                items[k].stem_up = sum < 0;
            }
            *id += 1;
        }
        run.clear();
    };
    for k in 0..items.len() {
        let it = &items[k];
        let beamable = matches!(it.kind, Kind::Chord(_)) && it.value.base.beams() > 0;
        let slot = (it.start - bar_start) / group;
        let same_group = run
            .last()
            .is_some_and(|&j| (items[j].start - bar_start) / group == slot);
        if !beamable || !same_group {
            flush(&mut run, items, &mut id);
        }
        if beamable {
            run.push(k);
        }
    }
    flush(&mut run, items, &mut id);
}

/// Länge einer Balkengruppe: der Schlag, bei zusammengesetzten Taktarten (6/8, 9/8, 12/8)
/// drei Schläge.
pub(crate) fn beat_group(sig: TimeSignature) -> u64 {
    let beat = sig.beat_ticks().max(MIN_TICKS);
    if sig.denominator >= 8 && sig.numerator % 3 == 0 && sig.numerator > 3 {
        beat * 3
    } else if sig.denominator >= 8 {
        beat * 2
    } else {
        beat
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start: i64, length: u64, pitch: u8) -> Note {
        Note {
            start,
            length,
            pitch,
            velocity: 100,
        }
    }

    const Q: u64 = TICKS_PER_QUARTER;

    fn score(notes: &[Note], len: u64, key: KeySignature) -> Score {
        let tempo = TempoMap::default();
        layout(&Input {
            notes,
            part_start: 0,
            part_len: len,
            tempo: &tempo,
            key,
            quantize: Q / 4,
        })
    }

    fn values(items: &[Item]) -> Vec<(u64, bool)> {
        items
            .iter()
            .map(|i| (i.value.base.ticks(), i.value.dotted))
            .collect()
    }

    #[test]
    fn clefs_follow_the_range() {
        assert_eq!(clefs_for(&[note(0, Q, 67)]), [Clef::Treble]);
        assert_eq!(clefs_for(&[note(0, Q, 40)]), [Clef::Bass]);
        assert_eq!(
            clefs_for(&[note(0, Q, 40), note(0, Q, 76)]),
            [Clef::Treble, Clef::Bass]
        );
        assert_eq!(clefs_for(&[]), [Clef::Treble]);
    }

    #[test]
    fn durations_show_the_beat() {
        // Dotted quarter on the beat; from an eighth offset a quarter is split.
        assert_eq!(
            split(0, Q + Q / 2, Q),
            [Value {
                base: Base::Quarter,
                dotted: true
            }]
        );
        let v = split(Q / 2, Q, Q);
        assert_eq!(
            v,
            [
                Value {
                    base: Base::Eighth,
                    dotted: false
                },
                Value {
                    base: Base::Eighth,
                    dotted: false
                }
            ]
        );
        assert_eq!(v.iter().map(|v| v.ticks()).sum::<u64>(), Q);
        // A whole bar of 4/4 is one whole note; 3/4 one dotted half.
        assert_eq!(split(0, 4 * Q, Q).len(), 1);
        // A quarter from the second eighth of 6/8 stays one value.
        assert_eq!(split(Q / 2, Q, 3 * Q / 2).len(), 1);
        assert_eq!(
            split(0, 3 * Q, Q),
            [Value {
                base: Base::Half,
                dotted: true
            }]
        );
        assert!(
            split(0, 10, Q).is_empty(),
            "shorter than the smallest value"
        );
    }

    #[test]
    fn a_note_over_the_bar_line_is_tied() {
        // Half note starting on beat 4: a quarter in bar 1 tied to a quarter in bar 2.
        let s = score(
            &[note(3 * Q as i64, 2 * Q, 72)],
            8 * Q,
            KeySignature::C_MAJOR,
        );
        assert_eq!(s.bars.len(), 2);
        let b1 = &s.bars[0].staves[0];
        let last = b1.last().unwrap();
        let Kind::Chord(h) = &last.kind else {
            panic!("chord")
        };
        assert!(h[0].tie, "tied over the bar line");
        assert_eq!(last.value.ticks(), Q);
        let first = &s.bars[1].staves[0][0];
        let Kind::Chord(h) = &first.kind else {
            panic!("chord")
        };
        assert!(!h[0].tie && h[0].accidental.is_none());
        assert_eq!(first.value.ticks(), Q);
        // The rest of bar 2 is rests; bar 1 starts with a dotted half rest.
        assert_eq!(values(&b1[..1]), [(2 * Q, true)]);
    }

    #[test]
    fn empty_bars_get_a_bar_rest_and_rests_fill_gaps() {
        let s = score(&[note(Q as i64, Q, 67)], 8 * Q, KeySignature::C_MAJOR);
        assert_eq!(s.bars[1].staves[0].len(), 1);
        assert_eq!(s.bars[1].staves[0][0].kind, Kind::BarRest);
        let kinds: Vec<bool> = s.bars[0].staves[0]
            .iter()
            .map(|i| i.kind == Kind::Rest)
            .collect();
        assert_eq!(kinds, [true, false, true]);
        let total: u64 = s.bars[0].staves[0].iter().map(|i| i.value.ticks()).sum();
        assert_eq!(total, 4 * Q, "the bar adds up");
    }

    #[test]
    fn quantize_rounds_for_display_only() {
        let notes = [note(10, Q - 30, 72)];
        let s = score(&notes, 4 * Q, KeySignature::C_MAJOR);
        assert_eq!(s.bars[0].staves[0][0].start, 0);
        assert_eq!(s.bars[0].staves[0][0].value.ticks(), Q);
        assert_eq!(notes[0].start, 10, "the part keeps its timing");
    }

    #[test]
    fn eighths_are_beamed_by_beat() {
        let notes: Vec<Note> = (0..8)
            .map(|k| note(k * (Q as i64) / 2, Q / 2, 72))
            .collect();
        let s = score(&notes, 4 * Q, KeySignature::C_MAJOR);
        let beams: Vec<Option<u32>> = s.bars[0].staves[0].iter().map(|i| i.beam).collect();
        assert_eq!(
            beams,
            [
                Some(0),
                Some(0),
                Some(1),
                Some(1),
                Some(2),
                Some(2),
                Some(3),
                Some(3)
            ]
        );
        // 6/8: three eighths per group.
        let tempo = TempoMap {
            signature: TimeSignature {
                numerator: 6,
                denominator: 8,
            },
            ..TempoMap::default()
        };
        let notes: Vec<Note> = (0..6)
            .map(|k| note(k * (Q as i64) / 2, Q / 2, 72))
            .collect();
        let s = layout(&Input {
            notes: &notes,
            part_start: 0,
            part_len: 3 * Q,
            tempo: &tempo,
            key: KeySignature::C_MAJOR,
            quantize: Q / 4,
        });
        let beams: Vec<Option<u32>> = s.bars[0].staves[0].iter().map(|i| i.beam).collect();
        assert_eq!(
            beams,
            [Some(0), Some(0), Some(0), Some(1), Some(1), Some(1)]
        );
    }

    #[test]
    fn accidentals_follow_the_key_and_last_for_the_bar() {
        // C major: F#, F#, F (natural), then next bar F# again.
        let notes = [
            note(0, Q, 66),
            note(Q as i64, Q, 66),
            note(2 * Q as i64, Q, 65),
            note(4 * Q as i64, Q, 66),
        ];
        let s = score(&notes, 8 * Q, KeySignature::C_MAJOR);
        let acc = |b: usize, i: usize| match &s.bars[b].staves[0][i].kind {
            Kind::Chord(h) => h[0].accidental,
            _ => None,
        };
        assert_eq!(acc(0, 0), Some(Accidental::Sharp));
        assert_eq!(acc(0, 1), None, "still valid in the bar");
        assert_eq!(acc(0, 2), Some(Accidental::Natural));
        assert_eq!(acc(1, 0), Some(Accidental::Sharp), "new bar");
        // G major: F# needs nothing, F needs a natural.
        let g = KeySignature {
            fifths: 1,
            minor: false,
        };
        let s = score(&[note(0, Q, 66), note(Q as i64, Q, 65)], 4 * Q, g);
        let accs: Vec<Option<Accidental>> = s.bars[0].staves[0][..2]
            .iter()
            .map(|i| match &i.kind {
                Kind::Chord(h) => h[0].accidental,
                _ => None,
            })
            .collect();
        assert_eq!(accs, [None, Some(Accidental::Natural)]);
        // Flat keys spell black keys with flats: B-flat in F major has no accidental.
        let f = KeySignature {
            fifths: -1,
            minor: false,
        };
        let s = score(&[note(0, Q, 70), note(Q as i64, Q, 63)], 4 * Q, f);
        let accs: Vec<Option<Accidental>> = s.bars[0].staves[0][..2]
            .iter()
            .map(|i| match &i.kind {
                Kind::Chord(h) => h[0].accidental,
                _ => None,
            })
            .collect();
        assert_eq!(accs, [None, Some(Accidental::Flat)], "B♭ in key, E♭ marked");
    }

    #[test]
    fn positions_and_stems() {
        // Treble: B4 on the middle line, C5 above (stem down), G4 below (stem up).
        let s = score(
            &[
                note(0, Q, 71),
                note(Q as i64, Q, 72),
                note(2 * Q as i64, Q, 67),
            ],
            4 * Q,
            KeySignature::C_MAJOR,
        );
        let items = &s.bars[0].staves[0];
        let pos = |i: usize| match &items[i].kind {
            Kind::Chord(h) => h[0].pos,
            _ => 99,
        };
        assert_eq!((pos(0), pos(1), pos(2)), (0, 1, -2));
        assert!(!items[1].stem_up && items[2].stem_up);
        // Grand staff: middle C and above on top, below on the bass staff.
        let s = score(
            &[note(0, Q, 60), note(0, Q, 48), note(Q as i64, Q, 72)],
            4 * Q,
            KeySignature::C_MAJOR,
        );
        assert_eq!(s.clefs, [Clef::Treble, Clef::Bass]);
        let top = match &s.bars[0].staves[0][0].kind {
            Kind::Chord(h) => h[0].pos,
            _ => 99,
        };
        let bottom = match &s.bars[0].staves[1][0].kind {
            Kind::Chord(h) => h[0].pos,
            _ => 99,
        };
        assert_eq!(top, -6, "middle C below the treble staff");
        assert_eq!(bottom, -1, "C3 in the bass staff");
    }

    #[test]
    fn key_signature_positions() {
        assert_eq!(Clef::Treble.key_positions(2), [4, 1]);
        assert_eq!(Clef::Bass.key_positions(-3), [-2, 1, -3]);
        assert!(Clef::Treble.key_positions(0).is_empty());
    }

    #[test]
    fn hostile_input_stays_bounded() {
        let notes = [note(-500, Q, 200), note(i64::MAX, u64::MAX, 60)];
        let s = score(&notes, u64::MAX / 4, KeySignature::C_MAJOR);
        assert!(s.bars.len() <= MAX_BARS);
        let s = score(&[], 0, KeySignature::C_MAJOR);
        assert!(s.bars.len() <= 1);
    }
}
