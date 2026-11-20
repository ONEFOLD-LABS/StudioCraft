//! Abspielliste für den Audio-Thread: alle hörbaren Audio-Events, in Frames
//! umgerechnet. Wird bei jeder Änderung neu gebaut und als Ganzes ausgetauscht.

use std::{collections::HashMap, sync::Arc};

use studiocraft_project::{ClipId, FadeCurve, OutTarget, Project, TempoMap, TrackKind, db_to_gain};

/// Ein Event, wie der Audio-Thread es braucht.
#[derive(Debug, Clone)]
pub struct PlayEvent {
    pub start: u64,
    pub length: u64,
    /// Startpunkt im Clip in Frames.
    pub offset: u64,
    /// Verschachtelte Stereo-Samples in Engine-Abtastrate.
    pub samples: Arc<[f32]>,
    /// Fade-In und Fade-Out in Frames, bezogen auf das ganze Event, mit ihrer Kurvenform.
    pub fade_in: u64,
    pub fade_out: u64,
    pub fade_in_curve: FadeCurve,
    pub fade_out_curve: FadeCurve,
    /// Lautstärke des Events als Faktor (Griff oben in der Mitte).
    pub gain: f32,
    /// Ist dies nur ein sichtbarer Ausschnitt eines Events (weil ein darüber
    /// liegendes Event den Rest verdeckt): Abstand zum Anfang des ganzen Events
    /// und dessen Länge. Damit verlaufen die Fades wie beim ganzen Event.
    pub env_offset: u64,
    pub env_length: u64,
    /// Überblendung an verdeckten Kanten dieses Ausschnitts (Comp-Grenzen), in Frames: am
    /// Anfang einblenden, am Ende ausblenden, während das obere Event aus- bzw. einblendet.
    pub edge_in: u64,
    pub edge_out: u64,
}

impl PlayEvent {
    pub fn new(start: u64, length: u64, offset: u64, samples: Arc<[f32]>) -> Self {
        Self {
            start,
            length,
            offset,
            samples,
            fade_in: 0,
            fade_out: 0,
            fade_in_curve: FadeCurve::Linear,
            fade_out_curve: FadeCurve::Linear,
            gain: 1.0,
            env_offset: 0,
            env_length: length,
            edge_in: 0,
            edge_out: 0,
        }
    }

    /// Lautstärkefaktor aus Fades und Event-Lautstärke an Frame `k` des Events.
    #[inline]
    fn fade_gain(&self, k: u64) -> f32 {
        let mut g = self.gain;
        // Comp-Grenzen: linear gegen das Fade des oberen Events.
        if k < self.edge_in {
            g *= k as f32 / self.edge_in as f32;
        }
        let left = self.length.saturating_sub(k);
        if left < self.edge_out {
            g *= left as f32 / self.edge_out as f32;
        }
        let k = k.saturating_add(self.env_offset);
        if k < self.fade_in {
            g *= self.fade_in_curve.shape(k as f32 / self.fade_in as f32);
        }
        let rest = self.env_length.saturating_sub(k);
        if rest < self.fade_out {
            g *= self
                .fade_out_curve
                .shape(rest as f32 / self.fade_out as f32);
        }
        g
    }
}

#[derive(Debug, Clone, Default)]
pub struct PlayTrack {
    /// Faktor links und rechts (Fader und Panorama, Stereo-Balance).
    pub gain_l: f32,
    pub gain_r: f32,
    /// Pan-Modus „Stereo kombiniert“: beide Kanäle mischen über die Matrix
    /// `[ll, lr, rl, rr]` (samt Lautstärke) statt über `gain_l`/`gain_r`.
    pub pan_mix: Option<[f32; 4]>,
    /// Zusätzlicher Faktor beim Summieren in das Ausgangsziel (nach Fader, Pan, Anzeige und
    /// Sends). `None` = 1. Das Offline-Rendern legt so den Pegel eines Ausgangsbusses auf
    /// Spuren, die es auf die Summe umleitet.
    pub out_gain: Option<f32>,
    /// Index der Projektspur, für die Pegelanzeige.
    pub slot: usize,
    /// Nach `start` sortiert.
    pub events: Vec<PlayEvent>,
    /// Eingangsgain als Faktor, negativ bei umgekehrter Phase; `None`: unverändert.
    pub input_gain: Option<f32>,
    /// Pre-Filter (Hochpass Platz 0, Tiefpass Platz 1) mit Platz im Filterzustand.
    pub pre_filters: Vec<(usize, crate::eq::Biquad)>,
    /// Insert-Slots im Projekt ab hier liegen hinter dem Fader; `None`: alle davor.
    pub post_from: Option<usize>,
    /// Instrument- und MIDI-Spuren: Noten in Frames, nach `start` sortiert.
    pub notes: Vec<PlayNote>,
    /// MIDI-Spuren: Platz des Ausgangsports in [`Song::midi_ports`] und Kanal 0–15.
    pub midi_out: Option<(u8, u8)>,
    /// Instrument-Spuren: Controller-Werte in Frames, nach `at` sortiert.
    pub ctrls: Vec<PlayCtrl>,
    /// Gruppen und FX-Kanäle: Nummer des Bus-Puffers, aus dem diese Spur liest.
    pub bus: Option<usize>,
    /// Wohin das Signal nach dem Fader geht.
    pub output: Output,
    /// Weitere Ziele desselben Signals (Spec „Routing und Busse“: mehrere Ziele); ohne
    /// `output`, ohne Doppel und ohne verworfene Ziele.
    pub extra: Vec<Output>,
    pub sends: Vec<PlaySend>,
    /// Audio-Spuren mit Monitor: Eingangskanal und ob Stereo (`first`, `first + 1`).
    pub monitor: Option<(u16, bool)>,
    /// Automation (nur mit R und wirksamen Kurven).
    pub auto: Option<Box<crate::automation::TrackAuto>>,
    /// Kanal-EQ: wirksame Bänder (Platz im EQ, Filter); leer, wenn er nichts ändert.
    pub eq: Vec<(usize, crate::eq::Biquad)>,
}

/// Ziel des Ausgangs einer Spur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Output {
    /// Ausgangsbus (Summe).
    #[default]
    Master,
    /// Bus-Puffer einer Gruppe.
    Bus(usize),
    /// Weiterer Ausgangsbus der Audio-Anschlüsse (Platz in [`Song::outputs`], ab 1).
    Hw(usize),
    /// Ziel ist stummgeschaltet: Signal geht nirgendwohin.
    Nowhere,
}

/// Ein Send in der Abspielliste.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaySend {
    pub bus: usize,
    pub gain: f32,
    pub pre_fader: bool,
    /// Platz des Sends in der Projektspur (für seine Automation).
    pub model: usize,
}

/// Höchstzahl Gruppen und FX-Kanäle (Bus-Puffer werden vorab angelegt).
pub const MAX_BUSES: usize = 64;

/// Ein Controller-Wert (Pitchbend, CC, Aftertouch), wie der Audio-Thread ihn braucht:
/// Projektposition in Frames und die fertige MIDI-Nachricht.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayCtrl {
    pub at: u64,
    pub data: [u8; 3],
}

/// Eine Note, wie der Audio-Thread sie braucht (Projektpositionen in Frames).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayNote {
    pub start: u64,
    pub end: u64,
    pub key: u8,
    pub velocity: u8,
}

#[derive(Debug, Clone)]
pub struct Song {
    pub tracks: Vec<PlayTrack>,
    /// Faktor des Ausgangsbusses.
    pub master: f32,
    /// Anzahl benutzter Bus-Puffer.
    pub buses: usize,
    /// Tempo-Map für das Metronom im Audio-Thread (eine Kopie: sie wird mit dem Song
    /// ausgetauscht und im Hauptthread freigegeben).
    pub tempo: TempoMap,
    /// MIDI-Ausgänge der MIDI-Spuren: Platz `i` in [`PlayTrack::midi_out`] ist dieser Port.
    pub midi_ports: Vec<String>,
    /// Metronom- und Vorzähler-Einstellungen des Projekts.
    pub click: studiocraft_project::ClickSettings,
    /// Ausgangsbusse: (erster Hardware-Kanal, Stereo); Platz 0 ist der Hauptmix.
    pub outputs: Vec<(u16, bool)>,
    /// Pegel (linear) der Ausgangsbusse wie [`Self::outputs`]; Platz 0 (Hauptmix) nutzt
    /// stattdessen [`Self::master`]. Fehlende Plätze gelten als 1.
    pub output_gains: Vec<f32>,
}

impl Default for Song {
    fn default() -> Self {
        Self {
            tracks: Vec::new(),
            master: 1.0,
            buses: 0,
            tempo: TempoMap::default(),
            midi_ports: Vec::new(),
            click: Default::default(),
            outputs: Vec::new(),
            output_gains: Vec::new(),
        }
    }
}

impl Song {
    /// Ein einzelner Clip ab Frame 0 (für die Kommandozeile).
    pub fn single(samples: Arc<[f32]>) -> Self {
        let length = (samples.len() / 2) as u64;
        Self {
            tracks: vec![PlayTrack {
                gain_l: 1.0,
                gain_r: 1.0,
                slot: 0,
                events: vec![PlayEvent::new(0, length, 0, samples)],
                notes: Vec::new(),
                ..Default::default()
            }],
            master: 1.0,
            buses: 0,
            tempo: TempoMap::default(),
            midi_ports: Vec::new(),
            click: Default::default(),
            outputs: Vec::new(),
            output_gains: Vec::new(),
        }
    }

    /// Baut die Abspielliste aus dem Projekt. `clips` liefert die Samples je
    /// Clip, bereits in `rate`. Stumme Spuren und Events fallen weg; Solo wirkt.
    pub fn build(project: &Project, clips: &HashMap<ClipId, Arc<[f32]>>, rate: u32) -> Self {
        Self::build_with(project, clips, rate, None)
    }

    /// Wie [`Self::build`], aber gedehntes Audio (Issue #55) entsteht im Hintergrund: Events,
    /// deren Ergebnis noch fehlt, bleiben still, bis [`crate::stretch::take_ready`] meldet,
    /// dass neu gebaut werden soll. Für die Wiedergabe im Projekt; Export wartet ([`Self::build`]).
    pub fn build_live(project: &Project, clips: &HashMap<ClipId, Arc<[f32]>>, rate: u32) -> Self {
        let generation = crate::stretch::new_generation();
        Self::build_with(project, clips, rate, Some(generation))
    }

    fn build_with(
        project: &Project,
        clips: &HashMap<ClipId, Arc<[f32]>>,
        rate: u32,
        live: Option<u64>,
    ) -> Self {
        let tempo = &project.tempo;
        // Benutzte MIDI-Ausgänge, je Name ein Platz (höchstens 64).
        let mut ports: Vec<String> = Vec::new();
        for t in project.tracks.iter().filter(|t| t.kind == TrackKind::Midi) {
            if let Some(p) = &t.midi_port
                && !ports.contains(p)
                && ports.len() < 64
            {
                ports.push(p.clone());
            }
        }
        let tracks = project
            .tracks
            .iter()
            .enumerate()
            .filter(|(i, t)| {
                matches!(
                    t.kind,
                    TrackKind::Audio
                        | TrackKind::Instrument
                        | TrackKind::Midi
                        | TrackKind::Group
                        | TrackKind::Fx
                ) && project.audible(*i)
            })
            .map(|(slot, t)| {
                // Auf einer Audiospur klingt bei Überlappung nur das oberste Event
                // (höchster Comp-Rang, sonst das spätere in der Liste). Verdeckte Teile fallen weg.
                let audible = t.audible_spans();
                let mut events = Vec::new();
                for (e, parts) in t.events.iter().zip(&audible) {
                    let Some(samples) = e.clip.and_then(|c| clips.get(&c)) else {
                        continue;
                    };
                    let ev_start = tempo.frames(e.start.0, rate);
                    let env_length = tempo.frames(e.end(), rate) - ev_start;
                    // Folgt das Event dem Tempo, klingt ein gedehnter Ausschnitt ab Frame 0.
                    let (samples, base) = match musical_render(tempo, e, samples, rate, live) {
                        Ok(Some(s)) => (s, 0),
                        Ok(None) => (samples.clone(), e.offset),
                        // Wird noch gedehnt: bis dahin still.
                        Err(()) => continue,
                    };
                    for part in parts {
                        let (a, b) = (part.from, part.to);
                        let (fa, fb) = (tempo.frames(a, rate), tempo.frames(b, rate));
                        if fb <= fa {
                            continue;
                        }
                        let env_offset = fa - ev_start;
                        let edge_in = tempo.duration_frames(a, part.ramp_in, rate);
                        let edge_out = tempo.duration_frames(
                            b.saturating_sub(part.ramp_out),
                            part.ramp_out,
                            rate,
                        );
                        events.push(PlayEvent {
                            // Fades als Dauer an ihrer Stelle (Tempowechsel möglich).
                            fade_in: tempo.duration_frames(e.start.0, e.fade_in, rate),
                            fade_out: tempo.duration_frames(
                                e.end().saturating_sub(e.fade_out),
                                e.fade_out,
                                rate,
                            ),
                            fade_in_curve: e.fade_in_curve,
                            fade_out_curve: e.fade_out_curve,
                            gain: db_to_gain(e.gain_db),
                            env_offset,
                            env_length,
                            edge_in: edge_in.min(fb - fa),
                            edge_out: edge_out.min(fb - fa),
                            ..PlayEvent::new(fa, fb - fa, base + env_offset, samples.clone())
                        });
                    }
                }
                events.sort_by_key(|e| e.start);
                // Eingefroren: nur der gerenderte Clip klingt (fehlt er, bleibt die Spur still).
                if let Some(f) = &t.frozen {
                    events.clear();
                    if let Some(samples) = clips.get(&f.clip) {
                        let frames = (samples.len() / 2) as u64;
                        let at = tempo.frames(f.start, rate);
                        events.push(PlayEvent::new(at, frames, 0, samples.clone()));
                    }
                }
                let frozen = t.frozen.is_some();
                // Noten aller nicht stummen Parts; nur der sichtbare Teil klingt.
                let mut notes = Vec::new();
                if !frozen && matches!(t.kind, TrackKind::Instrument | TrackKind::Midi) {
                    for e in t.events.iter().filter(|e| !e.muted) {
                        let len = e.length as i64;
                        // Drum-Map: Noten stummer bzw. nicht soloierter Zeilen klingen nicht.
                        let drums = t.drum_map.as_ref().filter(|m| m.filters());
                        for n in e.notes.iter().filter(|n| {
                            n.start >= 0
                                && n.start < len
                                && drums.is_none_or(|m| m.audible(n.pitch))
                        }) {
                            let a = e.start.0 + n.start as u64;
                            let b = e.start.0 + n.end().min(len) as u64;
                            notes.push(PlayNote {
                                start: tempo.frames(a, rate),
                                end: tempo.frames(b, rate).max(tempo.frames(a, rate) + 1),
                                key: n.pitch,
                                velocity: n.velocity,
                            });
                        }
                    }
                    notes.sort_by_key(|n| n.start);
                }
                // Controller der nicht stummen Parts, nur im sichtbaren Teil.
                let mut ctrls = Vec::new();
                if !frozen && matches!(t.kind, TrackKind::Instrument | TrackKind::Midi) {
                    for e in t.events.iter().filter(|e| !e.muted) {
                        let len = e.length as i64;
                        for c in e
                            .controllers
                            .iter()
                            .filter(|c| c.start >= 0 && c.start < len)
                        {
                            ctrls.push(PlayCtrl {
                                at: tempo.frames(e.start.0 + c.start as u64, rate),
                                data: c.kind.midi(c.value),
                            });
                        }
                    }
                    ctrls.sort_by_key(|c| c.at);
                }
                let (gain_l, gain_r) = t.gains();
                let pan_mix =
                    (t.pan_mode == studiocraft_project::PanMode::Combined).then(|| t.mix_matrix());
                let monitor = (t.monitor && t.kind == TrackKind::Audio).then(|| {
                    let input = project.input_of(slot);
                    (input.first, input.stereo)
                });
                let mut auto = crate::automation::TrackAuto::build(t, tempo, rate);
                // Eingefroren: Plugin-Automation steckt im Clip.
                if frozen && let Some(a) = auto.as_mut() {
                    a.plugins.clear();
                }
                // Eingang und Pre-Sektion (Spec „Signalfluss“, Punkte 1 und 2).
                let gain = studiocraft_project::db_to_gain(t.input_gain_db)
                    * if t.phase_invert { -1.0 } else { 1.0 };
                // Eingefroren: Eingang und Pre-Sektion stecken schon im Clip.
                let input_gain = (gain != 1.0 && !frozen).then_some(gain);
                let filter = |kind, freq: Option<f32>| {
                    freq.map(|f| {
                        crate::eq::Biquad::design(
                            &studiocraft_project::EqBand {
                                on: true,
                                kind,
                                freq: f,
                                gain: 0.0,
                                q: std::f32::consts::FRAC_1_SQRT_2,
                            },
                            rate,
                        )
                    })
                };
                let pre_filters: Vec<(usize, crate::eq::Biquad)> = [
                    filter(studiocraft_project::BandKind::LowCut, t.pre_hpf),
                    filter(studiocraft_project::BandKind::HighCut, t.pre_lpf),
                ]
                .into_iter()
                .enumerate()
                .filter_map(|(i, f)| f.map(|f| (i, f)))
                .filter(|_| !frozen)
                .collect();
                let post_from = Some(usize::from(project.pre_fader_inserts));
                // MIDI-Spuren mit Ausgang: Port-Platz und Kanal.
                let midi_out = (t.kind == TrackKind::Midi)
                    .then_some(t.midi_port.as_ref())
                    .flatten()
                    .and_then(|p| ports.iter().position(|q| q == p))
                    .and_then(|i| u8::try_from(i).ok())
                    .map(|i| (i, t.midi_channel.clamp(1, 16) - 1));
                let keep = midi_out.is_some()
                    || monitor.is_some()
                    || auto.is_some()
                    || !events.is_empty()
                    || !t.inserts.is_empty()
                    || t.instrument.is_some()
                    || Project::is_bus(t.kind);
                (
                    keep,
                    PlayTrack {
                        gain_l,
                        gain_r,
                        pan_mix,
                        slot,
                        events,
                        notes,
                        input_gain,
                        pre_filters,
                        post_from,
                        midi_out,
                        ctrls,
                        monitor,
                        auto,
                        eq: crate::eq::design(&t.eq, rate),
                        ..Default::default()
                    },
                )
            })
            // Spuren mit Inserts bleiben, damit Effekte (Hall, Delay) ausklingen.
            .filter(|(keep, _)| *keep)
            .map(|(_, t)| t)
            .collect();
        let mut song = Self {
            tracks,
            master: db_to_gain(project.master_db),
            buses: 0,
            tempo: project.tempo.clone(),
            midi_ports: ports,
            click: project.click,
            outputs: project
                .io
                .outputs
                .iter()
                .map(|b| (b.first, b.stereo))
                .collect(),
            output_gains: project
                .io
                .outputs
                .iter()
                .map(|b| if b.mute { 0.0 } else { db_to_gain(b.gain_db) })
                .collect(),
        };
        song.route(project);
        song
    }

    /// Busse vergeben, Ausgänge und Sends auflösen und die Reihenfolge so
    /// festlegen, dass jede Spur nach allen Spuren läuft, die in sie speisen.
    fn route(&mut self, project: &Project) {
        let is_bus = |slot: usize| {
            project
                .tracks
                .get(slot)
                .is_some_and(|t| Project::is_bus(t.kind))
        };
        // Bus-Nummern für die Busse in der Liste (stumme Busse fehlen hier).
        let mut bus_of: HashMap<u32, usize> = HashMap::new();
        for t in &mut self.tracks {
            if is_bus(t.slot) && bus_of.len() < MAX_BUSES {
                let id = project.tracks[t.slot].id;
                t.bus = Some(bus_of.len());
                bus_of.insert(id, bus_of.len());
            }
        }
        self.buses = bus_of.len();
        let target = |id: u32| -> Output {
            match bus_of.get(&id) {
                Some(&b) => Output::Bus(b),
                // Ziel stumm oder (noch) nicht hörbar: Signal verwerfen.
                None if project.track_index(id).is_some() => Output::Nowhere,
                None => Output::Master,
            }
        };
        for t in &mut self.tracks {
            let pt = &project.tracks[t.slot];
            t.output = pt.output.map_or(Output::Master, target);
            if t.output == Output::Bus(t.bus.unwrap_or(usize::MAX)) {
                t.output = Output::Master; // Spur auf sich selbst: zur Summe
            }
            // Summe: auf einen weiteren Ausgangsbus statt auf den Hauptmix.
            if t.output == Output::Master
                && let Some(k) = pt
                    .out_bus
                    .and_then(|id| project.io.outputs.iter().position(|b| b.id == id))
                    .filter(|&k| k > 0)
            {
                t.output = Output::Hw(k);
            }
            t.sends = pt
                .sends
                .iter()
                .enumerate()
                .filter(|(_, s)| s.on)
                .filter_map(|(model, s)| match target(s.target) {
                    Output::Bus(b) if Some(b) != t.bus => Some(PlaySend {
                        bus: b,
                        gain: db_to_gain(s.level_db),
                        pre_fader: s.pre_fader,
                        model,
                    }),
                    _ => None,
                })
                .collect();
            let mut extra: Vec<Output> = Vec::new();
            for o in &pt.extra_outputs {
                let out = match *o {
                    OutTarget::Group(id) => target(id),
                    OutTarget::Bus(id) => {
                        match project.io.outputs.iter().position(|b| b.id == id) {
                            Some(0) => Output::Master,
                            Some(k) => Output::Hw(k),
                            None => continue,
                        }
                    }
                };
                let own = matches!(out, Output::Bus(b) if Some(b) == t.bus);
                if !own && out != Output::Nowhere && out != t.output && !extra.contains(&out) {
                    extra.push(out);
                }
            }
            t.extra = extra;
        }
        // Reihenfolge: Quellen zuerst, dann Busse so, dass ein Bus nach allen
        // Bussen läuft, die in ihn speisen. Schleifen (sollte die Oberfläche
        // verhindern) werden aufgelöst, indem die schließende Verbindung entfällt.
        let (mut order, mut buses): (Vec<PlayTrack>, Vec<PlayTrack>) =
            std::mem::take(&mut self.tracks)
                .into_iter()
                .partition(|t| t.bus.is_none());
        let feeds = |t: &PlayTrack| -> Vec<usize> {
            let mut v: Vec<usize> = t.sends.iter().map(|s| s.bus).collect();
            if let Output::Bus(b) = t.output {
                v.push(b);
            }
            v.extend(t.extra.iter().filter_map(|o| match *o {
                Output::Bus(b) => Some(b),
                _ => None,
            }));
            v
        };
        while !buses.is_empty() {
            // Ein Bus ist bereit, wenn kein anderer verbleibender Bus in ihn speist.
            let ready = (0..buses.len()).find(|&i| {
                let me = buses[i].bus;
                !buses
                    .iter()
                    .enumerate()
                    .any(|(j, o)| j != i && feeds(o).iter().any(|b| Some(*b) == me))
            });
            match ready {
                Some(i) => order.push(buses.remove(i)),
                None => {
                    // Schleife: Verbindungen zwischen den restlichen Bussen kappen.
                    let rest: Vec<Option<usize>> = buses.iter().map(|b| b.bus).collect();
                    for b in &mut buses {
                        b.sends.retain(|s| !rest.contains(&Some(s.bus)));
                        if let Output::Bus(x) = b.output
                            && rest.contains(&Some(x))
                        {
                            b.output = Output::Master;
                        }
                        b.extra
                            .retain(|o| !matches!(o, Output::Bus(x) if rest.contains(&Some(*x))));
                    }
                }
            }
        }
        self.tracks = order;
    }

    /// Mischt alle Spuren mit ihren Faktoren und dem Ausgangsfader in `out`
    /// (wird addiert). Für Tests; der Kern mischt Spur für Spur, um Pegel zu messen.
    pub fn mix(&self, out: &mut [f32], pos: u64, frames: usize) {
        let mut tmp = vec![0.0; frames * 2];
        for track in &self.tracks {
            tmp.fill(0.0);
            track.render(&mut tmp, pos, frames);
            for (o, t) in out.chunks_exact_mut(2).zip(tmp.chunks_exact(2)) {
                o[0] += t[0] * track.gain_l * self.master;
                o[1] += t[1] * track.gain_r * self.master;
            }
        }
    }
}

/// Kennzahl der Tempo-Map für den Zwischenspeicher des Time-Stretch.
fn tempo_key(tempo: &TempoMap) -> (u64, bool, Vec<(u64, u64, bool)>) {
    (
        tempo.bpm.to_bits(),
        tempo.use_track,
        tempo
            .events
            .iter()
            .map(|e| (e.tick, e.bpm.to_bits(), e.ramp))
            .collect(),
    )
}

/// Gedehnte Samples eines Audio-Events, das dem Projekttempo folgt (Issue #55), für die
/// ganze Länge des Events ab dessen Anfang. `Ok(None)`: nicht nötig (folgt nicht dem Tempo,
/// oder das Tempo ist fest und gleich dem der Datei) – dann klingt der Clip 1:1. `Err(())`:
/// wird noch im Hintergrund gerechnet (nur mit `live`, der Generation aus
/// [`crate::stretch::new_generation`]); das Event bleibt bis dahin still.
fn musical_render(
    tempo: &TempoMap,
    e: &studiocraft_project::Event,
    samples: &Arc<[f32]>,
    rate: u32,
    live: Option<u64>,
) -> Result<Option<Arc<[f32]>>, ()> {
    let Some(bpm) = e.musical.filter(|b| b.is_finite() && *b > 0.0) else {
        return Ok(None);
    };
    if !tempo.varies() && (tempo.bpm - bpm).abs() < 1e-9 {
        return Ok(None);
    }
    let start = tempo.frames(e.start.0, rate);
    let frames = tempo
        .frames(e.end(), rate)
        .saturating_sub(start)
        .min(crate::stretch::MAX_RENDER_FRAMES);
    let grain = e.stretch_mode.grain_48k();
    let key = crate::stretch::Key::new(
        samples,
        (
            e.offset,
            e.start.0,
            e.length,
            bpm.to_bits(),
            rate,
            grain,
            tempo_key(tempo),
        ),
    );
    let (offset, ev_tick) = (e.offset as f64, e.start.0 as f64);
    let r = f64::from(rate);
    let per_tick = 60.0 / bpm / studiocraft_project::TICKS_PER_QUARTER as f64 * r;
    let (src, tempo) = (samples.clone(), tempo.clone());
    let make = move || {
        crate::stretch::render(&src, rate, grain, frames as usize, |j| {
            let tick = tempo.ticks_at_seconds((start as f64 + j) / r);
            offset + (tick - ev_tick) * per_tick
        })
    };
    match live {
        Some(generation) => crate::stretch::request(key, generation, make)
            .map(Some)
            .ok_or(()),
        None => Ok(Some(crate::stretch::cached(key, make))),
    }
}

impl PlayTrack {
    /// Schreibt die Events dieser Spur ohne Fader und Panorama in `out`
    /// (wird addiert). Echtzeitsicher: keine Allokation.
    pub fn render(&self, out: &mut [f32], pos: u64, frames: usize) {
        let end = pos + frames as u64;
        // Events sind nach Start sortiert; alles ab `end` kann übersprungen werden.
        let last = self.events.partition_point(|e| e.start < end);
        for ev in &self.events[..last] {
            let ev_end = ev.start + ev.length;
            if ev_end <= pos {
                continue;
            }
            let from = pos.max(ev.start);
            let to = end.min(ev_end);
            let src_from = (ev.offset + (from - ev.start)) as usize * 2;
            let n = (to - from) as usize * 2;
            let Some(src) = ev.samples.get(src_from..) else {
                continue;
            };
            let n = n.min(src.len());
            let dst_from = (from - pos) as usize * 2;
            let dst = &mut out[dst_from..dst_from + n];
            if ev.fade_in == 0 && ev.fade_out == 0 && ev.edge_in == 0 && ev.edge_out == 0 {
                let g = ev.gain;
                for (d, s) in dst.iter_mut().zip(&src[..n]) {
                    *d += s * g;
                }
            } else {
                let k0 = from - ev.start;
                for (i, (d, s)) in dst
                    .chunks_exact_mut(2)
                    .zip(src[..n].chunks_exact(2))
                    .enumerate()
                {
                    let g = ev.fade_gain(k0 + i as u64);
                    d[0] += s[0] * g;
                    d[1] += s[1] * g;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studiocraft_project::{PoolClip, TICKS_PER_QUARTER};

    #[test]
    fn monitor_keeps_empty_audio_tracks_and_carries_their_input() {
        let mut p = Project::default();
        let a = p.add_track(TrackKind::Audio, "Gesang");
        p.tracks[a].monitor = true;
        p.tracks[a].input = studiocraft_project::TrackInput {
            first: 2,
            stereo: false,
        };
        let m = p.add_track(TrackKind::Midi, "MIDI");
        p.tracks[m].monitor = true;
        let quiet = p.add_track(TrackKind::Audio, "Leer");
        let song = Song::build(&p, &HashMap::new(), 48_000);
        let monitored: Vec<(usize, Option<(u16, bool)>)> =
            song.tracks.iter().map(|t| (t.slot, t.monitor)).collect();
        assert_eq!(monitored, [(a, Some((2, false)))]);
        assert!(!song.tracks.iter().any(|t| t.slot == quiet));
    }

    #[test]
    fn controllers_play_on_instrument_tracks_in_frames() {
        use studiocraft_project::{CtrlKind, CtrlPoint, TICKS_PER_QUARTER};
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Instrument, "Synth");
        let id = p
            .create_event(t, TICKS_PER_QUARTER, 4 * TICKS_PER_QUARTER)
            .unwrap();
        p.event_mut(id).unwrap().controllers = vec![
            CtrlPoint {
                kind: CtrlKind::PitchBend,
                start: 0,
                value: 16_383,
            },
            CtrlPoint {
                kind: CtrlKind::Cc(1),
                start: TICKS_PER_QUARTER as i64,
                value: 64,
            },
            // Outside the part: silent.
            CtrlPoint {
                kind: CtrlKind::Cc(1),
                start: 10 * TICKS_PER_QUARTER as i64,
                value: 1,
            },
        ];
        // Instrument tracks without a plugin are dropped, so give it one.
        p.tracks[t].instrument = Some(studiocraft_project::InsertSlot {
            plugin_id: "x".into(),
            name: "x".into(),
            bypass: false,
            params: Vec::new(),
            state: None,
        });
        let song = Song::build(&p, &HashMap::new(), 48_000);
        let track = song.tracks.iter().find(|tr| tr.slot == t).unwrap();
        // One quarter at 120 BPM and 48 kHz = 24 000 frames.
        assert_eq!(
            track.ctrls,
            vec![
                PlayCtrl {
                    at: 24_000,
                    data: [0xe0, 0x7f, 0x7f]
                },
                PlayCtrl {
                    at: 48_000,
                    data: [0xb0, 1, 64]
                },
            ]
        );
    }

    /// Projekt mit einer Audio-Spur und einem Event über einen Sinus-Clip (1 s, 48 kHz).
    fn musical_project(bpm_file: f64) -> (Project, HashMap<ClipId, Arc<[f32]>>, u64) {
        let mut p = Project::default();
        p.add_track(TrackKind::Audio, "A");
        let c = p.add_clip(PoolClip {
            name: "loop".into(),
            path: None,
            sample_rate: 48_000,
            frames: 48_000,
        });
        let id = p.add_audio_event(0, 0, c).unwrap();
        assert!(p.set_musical(id, Some(bpm_file)));
        let samples: Vec<f32> = (0..48_000)
            .flat_map(|i| {
                let v = (std::f64::consts::TAU * 440.0 * f64::from(i) / 48_000.0).sin() as f32;
                [v, v]
            })
            .collect();
        (p, HashMap::from([(c, samples.into())]), id)
    }

    #[test]
    fn musical_audio_plays_one_to_one_at_the_file_tempo() {
        let (p, clips, _) = musical_project(120.0);
        let song = Song::build(&p, &clips, 48_000);
        let ev = &song.tracks[0].events[0];
        assert!(Arc::ptr_eq(&ev.samples, &clips[&0]), "kein Rendern nötig");
        assert_eq!((ev.length, ev.offset), (48_000, 0));
    }

    #[test]
    fn musical_audio_follows_a_faster_project_tempo() {
        // Datei in 120 BPM, Projekt in 240 BPM: die Sekunde Audio dauert eine halbe Sekunde.
        let (mut p, clips, id) = musical_project(120.0);
        let mut tempo = p.tempo.clone();
        tempo.bpm = 240.0;
        p.set_tempo_map(tempo);
        assert_eq!(
            p.event(id).unwrap().length,
            2 * TICKS_PER_QUARTER,
            "Takte bleiben"
        );
        let song = Song::build(&p, &clips, 48_000);
        let ev = &song.tracks[0].events[0];
        assert_eq!(ev.length, 24_000);
        assert_eq!(ev.samples.len(), 2 * 24_000);
        // Die Tonhöhe bleibt: etwa 220 Schwingungen in der halben Sekunde.
        let up = (1..24_000)
            .filter(|&i| ev.samples[(i - 1) * 2] < 0.0 && ev.samples[i * 2] >= 0.0)
            .count();
        assert!((214..=226).contains(&up), "{up}");
        // Gleiches Projekt: dasselbe Ergebnis aus dem Zwischenspeicher.
        let again = Song::build(&p, &clips, 48_000);
        assert!(Arc::ptr_eq(&again.tracks[0].events[0].samples, &ev.samples));
    }

    #[test]
    fn live_builds_stretch_in_the_background() {
        let (mut p, clips, id) = musical_project(100.0);
        p.event_mut(id).unwrap().stretch_mode = studiocraft_project::StretchMode::Rhythmic;
        let first = Song::build_live(&p, &clips, 48_000);
        // Solange gerechnet wird, ist die Spur still (und fällt ohne Events ganz weg).
        assert!(first.tracks.iter().all(|t| t.events.is_empty()));
        // Warten, bis der Arbeits-Thread fertig ist (höchstens 20 s).
        let mut song = first;
        for _ in 0..400 {
            song = Song::build_live(&p, &clips, 48_000);
            if song.tracks.first().is_some_and(|t| !t.events.is_empty()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let ev = &song.tracks[0].events[0];
        // 1 s bei 100 BPM sind 1600 Ticks, bei 120 BPM also 5/6 s.
        assert_eq!(ev.length, 40_000);
        // Export wartet nicht und nimmt dasselbe Ergebnis.
        let export = Song::build(&p, &clips, 48_000);
        assert!(Arc::ptr_eq(
            &export.tracks[0].events[0].samples,
            &ev.samples
        ));
        // Eine andere Stretch-Art ist ein anderes Ergebnis.
        p.event_mut(id).unwrap().stretch_mode = studiocraft_project::StretchMode::Pad;
        let pad = Song::build(&p, &clips, 48_000);
        assert!(!Arc::ptr_eq(&pad.tracks[0].events[0].samples, &ev.samples));
    }

    fn ones(frames: usize, v: f32) -> Arc<[f32]> {
        vec![v; frames * 2].into()
    }

    fn track(gain: f32, slot: usize, events: Vec<PlayEvent>) -> PlayTrack {
        PlayTrack {
            gain_l: gain,
            gain_r: gain,
            slot,
            events,
            notes: Vec::new(),
            ..Default::default()
        }
    }

    #[test]
    fn overlapping_tracks_add_up() {
        let song = Song {
            tracks: vec![
                track(1.0, 0, vec![PlayEvent::new(2, 4, 0, ones(10, 0.25))]),
                track(0.5, 1, vec![PlayEvent::new(0, 3, 0, ones(10, 1.0))]),
            ],
            master: 1.0,
            buses: 0,
            tempo: TempoMap::default(),
            midi_ports: Vec::new(),
            click: Default::default(),
            outputs: Vec::new(),
            output_gains: Vec::new(),
        };
        let mut out = vec![0.0; 16];
        song.mix(&mut out, 0, 8);
        let left: Vec<f32> = out.chunks(2).map(|c| c[0]).collect();
        assert_eq!(left, [0.5, 0.5, 0.75, 0.25, 0.25, 0.25, 0.0, 0.0]);
    }

    #[test]
    fn offset_and_block_start_inside_event() {
        let samples: Arc<[f32]> = (0..20)
            .flat_map(|i| [i as f32, 0.0])
            .collect::<Vec<_>>()
            .into();
        let song = Song {
            tracks: vec![track(1.0, 0, vec![PlayEvent::new(10, 5, 3, samples)])],
            master: 1.0,
            buses: 0,
            tempo: TempoMap::default(),
            midi_ports: Vec::new(),
            click: Default::default(),
            outputs: Vec::new(),
            output_gains: Vec::new(),
        };
        let mut out = vec![0.0; 8];
        song.mix(&mut out, 12, 4);
        let left: Vec<f32> = out.chunks(2).map(|c| c[0]).collect();
        // Frame 12 ist Event-Frame 2, also Clip-Frame 5; Event endet bei 15.
        assert_eq!(left, [5.0, 6.0, 7.0, 0.0]);
    }

    #[test]
    fn fades_ramp_linearly() {
        let ev = PlayEvent {
            fade_in: 4,
            fade_out: 2,
            ..PlayEvent::new(0, 8, 0, ones(8, 1.0))
        };
        let song = Song {
            tracks: vec![track(1.0, 0, vec![ev])],
            master: 1.0,
            buses: 0,
            tempo: TempoMap::default(),
            midi_ports: Vec::new(),
            click: Default::default(),
            outputs: Vec::new(),
            output_gains: Vec::new(),
        };
        let mut out = vec![0.0; 16];
        song.mix(&mut out, 0, 8);
        let left: Vec<f32> = out.chunks(2).map(|c| c[0]).collect();
        assert_eq!(left, [0.0, 0.25, 0.5, 0.75, 1.0, 1.0, 1.0, 0.5]);
    }

    #[test]
    fn fade_curves_and_event_gain_shape_the_mix() {
        let ev = PlayEvent {
            fade_in: 4,
            fade_out: 4,
            fade_in_curve: FadeCurve::Exponential,
            fade_out_curve: FadeCurve::SCurve,
            gain: 0.5,
            ..PlayEvent::new(0, 8, 0, ones(8, 1.0))
        };
        let song = Song {
            tracks: vec![track(1.0, 0, vec![ev])],
            master: 1.0,
            buses: 0,
            tempo: TempoMap::default(),
            midi_ports: Vec::new(),
            click: Default::default(),
            outputs: Vec::new(),
            output_gains: Vec::new(),
        };
        let mut out = vec![0.0; 16];
        song.mix(&mut out, 0, 8);
        let left: Vec<f32> = out.chunks(2).map(|c| c[0]).collect();
        // In: 0.5 * x², out: 0.5 * smoothstep(rest / 4).
        let s = |x: f32| x * x * (3.0 - 2.0 * x);
        let want = [
            0.0,
            0.5 * 0.0625,
            0.5 * 0.25,
            0.5 * 0.5625,
            0.5 * s(1.0),
            0.5 * s(0.75),
            0.5 * s(0.5),
            0.5 * s(0.25),
        ];
        for (a, b) in left.iter().zip(want) {
            assert!((a - b).abs() < 1e-6, "{left:?}");
        }
        // Without fades only the gain applies.
        let plain = PlayEvent {
            gain: 0.25,
            ..PlayEvent::new(0, 4, 0, ones(4, 1.0))
        };
        let song = Song {
            tracks: vec![track(1.0, 0, vec![plain])],
            master: 1.0,
            buses: 0,
            tempo: TempoMap::default(),
            midi_ports: Vec::new(),
            click: Default::default(),
            outputs: Vec::new(),
            output_gains: Vec::new(),
        };
        let mut out = vec![0.0; 8];
        song.mix(&mut out, 0, 4);
        assert!(out.iter().all(|&v| (v - 0.25).abs() < 1e-6), "{out:?}");
    }

    #[test]
    fn an_equal_power_crossfade_plays_both_events_through_the_overlap() {
        // Frames 0..8 and 4..12, overlap 4..8 with equal-power fades on both sides.
        let a = PlayEvent {
            fade_out: 4,
            fade_out_curve: FadeCurve::EqualPower,
            ..PlayEvent::new(0, 8, 0, ones(8, 1.0))
        };
        let b = PlayEvent {
            fade_in: 4,
            fade_in_curve: FadeCurve::EqualPower,
            ..PlayEvent::new(4, 8, 0, ones(8, 1.0))
        };
        let song = Song {
            tracks: vec![track(1.0, 0, vec![a, b])],
            master: 1.0,
            buses: 0,
            tempo: TempoMap::default(),
            midi_ports: Vec::new(),
            click: Default::default(),
            outputs: Vec::new(),
            output_gains: Vec::new(),
        };
        let mut out = vec![0.0; 24];
        song.mix(&mut out, 0, 12);
        let left: Vec<f32> = out.chunks(2).map(|c| c[0]).collect();
        let half_pi = std::f32::consts::FRAC_PI_2;
        for k in 0..4 {
            let x = k as f32 / 4.0;
            let want = (x * half_pi).sin() + ((1.0 - x) * half_pi).sin();
            assert!((left[4 + k] - want).abs() < 1e-5, "{left:?}");
        }
        assert_eq!(left[2], 1.0);
        assert_eq!(left[10], 1.0);
    }

    #[test]
    fn topmost_event_wins_on_overlap() {
        use studiocraft_project::Position;
        let mut p = Project::default();
        p.add_track(TrackKind::Audio, "a");
        let c = p.add_clip(PoolClip {
            name: "c".into(),
            path: None,
            sample_rate: 48_000,
            frames: 192_000,
        });
        let q = TICKS_PER_QUARTER;
        let old = p.add_audio_event(0, 0, c).unwrap(); // 4 s = 8 Viertel
        let new = p.create_event(0, 0, 1).unwrap();
        // Neuer Take obenauf (spätere Position in der Liste) von Viertel 2 bis 4.
        let ev = p.event_mut(new).unwrap();
        ev.clip = Some(c);
        ev.start = Position(2 * q);
        ev.length = 2 * q;
        ev.offset = 0;
        let i = p.find(new).unwrap().1;
        let taken = p.tracks[0].events.remove(i);
        p.tracks[0].events.push(taken);
        p.set_fades(old, 8 * q, 0);
        let clips = HashMap::from([(c, ones(192_000, 1.0))]);
        let song = Song::build(&p, &clips, 48_000);
        let spans: Vec<(u64, u64, u64)> = song.tracks[0]
            .events
            .iter()
            .map(|e| (e.start, e.length, e.offset))
            .collect();
        // Alter Take sichtbar 0–1 s und 2–4 s, neuer Take 1–2 s.
        assert_eq!(
            spans,
            [
                (0, 48_000, 0),
                (48_000, 48_000, 0),
                (96_000, 96_000, 96_000)
            ]
        );
        // Der zweite Teil des alten Takes setzt den Fade-In des ganzen Events fort.
        let tail = &song.tracks[0].events[2];
        assert_eq!(
            (tail.env_offset, tail.env_length, tail.fade_in),
            (96_000, 192_000, 192_000)
        );
        let mut out = vec![0.0; 2];
        song.mix(&mut out, 96_000, 1);
        assert!(
            (out[0] - 0.5).abs() < 1e-4,
            "Fade-In bei halber Länge: {}",
            out[0]
        );
    }

    #[test]
    fn drum_map_mute_and_solo_filter_the_played_notes() {
        use studiocraft_project::{DrumMap, Note};
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Instrument, "Drums");
        p.tracks[t].instrument = Some(studiocraft_project::InsertSlot {
            plugin_id: "x".into(),
            name: "X".into(),
            bypass: false,
            params: Vec::new(),
            state: None,
        });
        let id = p.create_event(t, 0, 4 * TICKS_PER_QUARTER).unwrap();
        p.event_mut(id).unwrap().notes = [36u8, 38, 42]
            .into_iter()
            .map(|pitch| Note {
                pitch,
                velocity: 100,
                start: 0,
                length: 10,
            })
            .collect();
        let keys = |p: &Project| -> Vec<u8> {
            let song = Song::build(p, &HashMap::new(), 48_000);
            song.tracks[0].notes.iter().map(|n| n.key).collect()
        };
        let mut map = DrumMap::general_midi();
        p.tracks[t].drum_map = Some(map.clone());
        assert_eq!(keys(&p), [36, 38, 42], "no mute/solo: everything");
        map.rows.iter_mut().find(|r| r.pitch == 42).unwrap().mute = true;
        p.tracks[t].drum_map = Some(map.clone());
        assert_eq!(keys(&p), [36, 38]);
        map.rows.iter_mut().find(|r| r.pitch == 38).unwrap().solo = true;
        p.tracks[t].drum_map = Some(map);
        assert_eq!(keys(&p), [38]);
    }

    #[test]
    fn comp_boundaries_crossfade_without_jumps() {
        use studiocraft_project::PoolClip;
        let rate = 48_000u32;
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "Gesang");
        let c = p.add_clip(PoolClip {
            name: "c".into(),
            path: None,
            sample_rate: rate,
            frames: u64::from(rate),
        });
        let take1 = p.add_audio_event(t, 0, c).unwrap();
        p.add_audio_event(t, 0, c).unwrap();
        // Comp the middle of take 1 (ticks at 120 BPM: 960 per quarter = 0.5 s).
        p.comp(take1, 240, 720).unwrap();
        let clips = HashMap::from([(c, vec![1.0f32; rate as usize * 2].into())]);
        let song = Song::build(&p, &clips, rate);
        let n = rate as usize;
        let mut out = vec![0.0f32; n * 2];
        song.mix(&mut out, 0, n);
        // Both takes are 1.0: the sum stays 1 everywhere, also inside the crossfades.
        let left: Vec<f32> = out.chunks(2).map(|f| f[0]).collect();
        let worst = left[..n - 1]
            .iter()
            .map(|v| (v - 1.0).abs())
            .fold(0.0f32, f32::max);
        let bad: Vec<(usize, f32)> = left[..n - 1]
            .iter()
            .enumerate()
            .filter(|(_, v)| (*v - 1.0).abs() >= 0.01)
            .map(|(i, v)| (i, *v))
            .take(5)
            .collect();
        assert!(
            worst < 0.01,
            "level stays 1: off by {worst} at {bad:?} of {n}"
        );
        let jump = left[..n - 1]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max);
        assert!(jump < 0.01, "no hard step: {jump}");
        // Inside a crossfade both takes play.
        let fades: usize = song.tracks[0]
            .events
            .iter()
            .filter(|e| e.edge_in > 0 || e.edge_out > 0)
            .count();
        assert_eq!(fades, 2);
    }

    #[test]
    fn instrument_notes_become_frames_and_are_clipped_to_part() {
        use studiocraft_project::Note;
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Instrument, "Synth");
        p.tracks[t].instrument = Some(studiocraft_project::InsertSlot {
            plugin_id: "x".into(),
            name: "X".into(),
            bypass: false,
            params: Vec::new(),
            state: None,
        });
        let q = TICKS_PER_QUARTER;
        let id = p.create_event(t, 0, 2 * q).unwrap();
        let ev = p.event_mut(id).unwrap();
        ev.notes = vec![
            Note {
                pitch: 60,
                velocity: 100,
                start: 0,
                length: q,
            },
            Note {
                pitch: 64,
                velocity: 90,
                start: q as i64,
                length: 4 * q,
            }, // ragt über das Part-Ende
            Note {
                pitch: 67,
                velocity: 80,
                start: -(q as i64),
                length: q,
            }, // ausgeblendet
        ];
        let song = Song::build(&p, &HashMap::new(), 48_000);
        assert_eq!(song.tracks.len(), 1);
        let n = &song.tracks[0].notes;
        assert_eq!(n.len(), 2);
        assert_eq!((n[0].start, n[0].end, n[0].key), (0, 24_000, 60));
        assert_eq!(
            (n[1].start, n[1].end, n[1].key),
            (24_000, 48_000, 64),
            "am Part-Ende abgeschnitten"
        );
    }

    #[test]
    fn build_respects_mute_solo_fader_and_converts_positions() {
        let mut p = Project::default();
        for n in ["a", "b"] {
            p.add_track(TrackKind::Audio, n);
        }
        let c = p.add_clip(PoolClip {
            name: "c".into(),
            path: None,
            sample_rate: 48_000,
            frames: 100,
        });
        p.add_audio_event(0, TICKS_PER_QUARTER, c).unwrap();
        p.add_audio_event(1, 0, c).unwrap();
        p.tracks[1].pan = 1.0;
        p.master_db = -6.0;
        let clips = HashMap::from([(c, ones(100, 1.0))]);

        let song = Song::build(&p, &clips, 48_000);
        assert_eq!(song.tracks.len(), 2);
        assert_eq!(
            song.tracks[0].events[0].start, 24_000,
            "Viertel bei 120 BPM"
        );
        assert_eq!(
            (
                song.tracks[1].gain_l,
                song.tracks[1].gain_r,
                song.tracks[1].slot
            ),
            (0.0, 1.0, 1)
        );
        assert!((song.master - 0.501).abs() < 0.001);

        p.tracks[1].solo = true;
        assert_eq!(Song::build(&p, &clips, 48_000).tracks.len(), 1);
        p.tracks[1].mute = true;
        assert_eq!(Song::build(&p, &clips, 48_000).tracks.len(), 0);
    }
    #[test]
    fn a_frozen_track_plays_its_clip_instead_of_notes_and_pre_section() {
        use studiocraft_project::{Frozen, Note};
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Instrument, "Synth");
        let q = TICKS_PER_QUARTER;
        let id = p.create_event(t, 0, 2 * q).unwrap();
        p.event_mut(id).unwrap().notes = vec![Note {
            pitch: 60,
            velocity: 100,
            start: 0,
            length: q,
        }];
        p.tracks[t].instrument = Some(studiocraft_project::InsertSlot {
            plugin_id: "x".into(),
            name: "X".into(),
            bypass: false,
            params: Vec::new(),
            state: None,
        });
        p.tracks[t].input_gain_db = 6.0;
        p.tracks[t].pre_hpf = Some(100.0);
        let c = p.add_clip(PoolClip {
            name: "Synth (eingefroren)".into(),
            path: None,
            sample_rate: 48_000,
            frames: 300,
        });
        let clips = HashMap::from([(c, ones(300, 0.5))]);
        let live = Song::build(&p, &clips, 48_000);
        assert_eq!(live.tracks[0].notes.len(), 1);
        assert!(live.tracks[0].input_gain.is_some());

        p.tracks[t].frozen = Some(Frozen { clip: c, start: q });
        let song = Song::build(&p, &clips, 48_000);
        let pt = &song.tracks[0];
        assert!(pt.notes.is_empty(), "Noten stecken im Clip");
        assert_eq!(pt.input_gain, None);
        assert!(pt.pre_filters.is_empty());
        assert_eq!(pt.events.len(), 1);
        assert_eq!((pt.events[0].start, pt.events[0].length), (24_000, 300));

        // Fehlt der Clip, bleibt die Spur still statt ohne Plugins zu spielen.
        let song = Song::build(&p, &HashMap::new(), 48_000);
        assert!(
            song.tracks
                .iter()
                .all(|t| t.events.is_empty() && t.notes.is_empty())
        );
    }
}
