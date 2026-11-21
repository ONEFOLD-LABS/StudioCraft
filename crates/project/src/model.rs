//! Spuren und Events (Spec-Abschnitte „Begriffe und Datenmodell“ und „Spurtypen“).

use crate::{Position, TICKS_PER_QUARTER, TempoMap};

pub type EventId = u64;
/// Index eines Clips im Pool.
pub type ClipId = u32;

/// Eintrag im Pool: eine Audio-Datei (Spec: „Pool“, „Audio-Clip“).
/// Die Samples selbst hält die Engine; das Modell kennt nur die Eckdaten.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PoolClip {
    pub name: String,
    pub path: Option<String>,
    /// Abtastrate der geladenen Samples.
    pub sample_rate: u32,
    pub frames: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TrackKind {
    Audio,
    Instrument,
    Midi,
    Group,
    Fx,
    Folder,
    /// Marker-Spur: zeigt die Marker des Projekts (eine je Projekt).
    Marker,
    /// Tempospur: zeigt und bearbeitet die Tempowechsel der Tempo-Map (eine je Projekt).
    Tempo,
    /// Lineal-Spur: ein weiteres Zeitlineal mit eigenem Anzeigeformat (beliebig viele).
    Ruler,
    /// Videospur (Issue #174): ein Video als Event mit Thumbnails; das Event legt fest, wo das
    /// Video beginnt, und trägt seinen Ton (stumm = Ton aus).
    Video,
}

/// Anzeigeformat einer Lineal-Spur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RulerFormat {
    /// Takte und Zählzeiten.
    #[default]
    Bars,
    /// Minuten:Sekunden.Millisekunden.
    Seconds,
    /// Timecode Stunden:Minuten:Sekunden:Frames mit 25 Bildern je Sekunde.
    Timecode25,
    /// Timecode mit 30 Bildern je Sekunde (ohne Drop-Frame).
    Timecode30,
    /// Samples bei der Abtastrate des Projekts.
    Samples,
}

impl RulerFormat {
    pub const ALL: [RulerFormat; 5] = [
        Self::Bars,
        Self::Seconds,
        Self::Timecode25,
        Self::Timecode30,
        Self::Samples,
    ];

    pub fn is_bars(&self) -> bool {
        *self == Self::Bars
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Bars => "Takte",
            Self::Seconds => "Sekunden",
            Self::Timecode25 => "Timecode 25 fps",
            Self::Timecode30 => "Timecode 30 fps",
            Self::Samples => "Samples",
        }
    }

    /// Bilder je Sekunde der Timecode-Formate.
    pub fn fps(self) -> Option<u32> {
        match self {
            Self::Timecode25 => Some(25),
            Self::Timecode30 => Some(30),
            _ => None,
        }
    }

    /// Beschriftung einer Zeit in Sekunden (nicht für Takte).
    pub fn format_seconds(self, secs: f64, rate: u32) -> String {
        let secs = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
        match self {
            Self::Bars | Self::Seconds => {
                let ms = (secs * 1000.0).round() as u64;
                format!("{}:{:02}.{:03}", ms / 60_000, ms / 1000 % 60, ms % 1000)
            }
            Self::Timecode25 | Self::Timecode30 => {
                let fps = u64::from(self.fps().unwrap_or(25));
                let frames = (secs * fps as f64).floor() as u64;
                let s = frames / fps;
                format!(
                    "{:02}:{:02}:{:02}:{:02}",
                    s / 3600,
                    s / 60 % 60,
                    s % 60,
                    frames % fps
                )
            }
            Self::Samples => format!("{}", (secs * f64::from(rate)).round() as u64),
        }
    }
}

impl TrackKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Audio => "Audio",
            Self::Instrument => "Instrument",
            Self::Midi => "MIDI",
            Self::Group => "Gruppe",
            Self::Fx => "FX-Kanal",
            Self::Folder => "Ordner",
            Self::Marker => "Marker",
            Self::Tempo => "Tempo",
            Self::Ruler => "Lineal",
            Self::Video => "Video",
        }
    }

    /// Ob auf dieser Spur Events liegen können.
    pub fn holds_events(self) -> bool {
        matches!(self, Self::Audio | Self::Instrument | Self::Midi)
    }
}

/// Ein Event auf einer Spur. Positionen und Längen in Ticks.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Event {
    pub id: EventId,
    pub start: Position,
    pub length: u64,
    pub name: String,
    pub muted: bool,
    /// Audio-Events: Clip im Pool. `None` bei MIDI-Parts und leeren Events.
    pub clip: Option<ClipId>,
    /// Audio-Events: Startpunkt im Clip in Frames des Clips.
    pub offset: u64,
    /// Fade-In und Fade-Out in Ticks (nicht-destruktiv).
    pub fade_in: u64,
    pub fade_out: u64,
    /// Kurvenform der Fades (Kurveneditor per Doppelklick auf den Fade).
    #[serde(default, skip_serializing_if = "FadeCurve::is_linear")]
    pub fade_in_curve: FadeCurve,
    #[serde(default, skip_serializing_if = "FadeCurve::is_linear")]
    pub fade_out_curve: FadeCurve,
    /// Lautstärke des Events in dB (Griff oben in der Mitte), nicht-destruktiv.
    #[serde(default, skip_serializing_if = "is_zero_db")]
    pub gain_db: f32,
    /// MIDI-Parts: Noten, Position relativ zum Part-Anfang.
    #[serde(default)]
    pub notes: Vec<Note>,
    /// MIDI-Parts: Controller-Werte (Pitchbend, CC, Aftertouch), relativ zum Part-Anfang.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controllers: Vec<crate::CtrlPoint>,
    /// Feste Lane (Takes); `None`: wird aus der Reihenfolge verteilt ([`Track::lanes`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<u32>,
    /// Comping-Rang: Bei Überlappung klingt das Event mit dem höchsten Rang, bei Gleichstand
    /// das spätere in der Liste.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub comp: u32,
    /// Eigene Farbe; `None`: die Farbe der Spur (Spec „Farbe“: neue Events erben sie).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
    /// Geteilte Kopien: Events mit derselben Gruppe teilen ihren Inhalt (Noten, Clip).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared: Option<u32>,
    /// Audio-Events, die dem Projekttempo folgen (Time-Stretch, Issue #55): das Tempo der
    /// Datei in BPM. Solche Events hängen an Takten wie MIDI; ihr Inhalt wird so gedehnt,
    /// dass eine Viertel der Datei eine Viertel im Projekt dauert. `None`: Wiedergabe 1:1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub musical: Option<f64>,
    /// Art des Time-Stretch für solche Events (Korngröße: Klangcharakter gegen Rechenzeit).
    #[serde(default, skip_serializing_if = "StretchMode::is_standard")]
    pub stretch_mode: StretchMode,
    /// Warp-Marker solcher Events (siehe `warp.rs`): Stellen im Clip, die auf eine
    /// musikalische Stelle gehören.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warp: Vec<crate::WarpPoint>,
    /// MIDI-Parts: Anzeige-Quantisierung im Noten-Editor in Ticks (nur Darstellung);
    /// `None`: die Vorgabe ([`crate::DEFAULT_SCORE_QUANTIZE`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_quantize: Option<u64>,
    /// Audio-Part (Issue #184): die Audio-Events darin, Anfang relativ zum Part-Anfang. Ein
    /// Part hat keinen eigenen Clip; Wiedergabe und Export lösen ihn in diese Events auf.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Event>,
}

/// Art des Time-Stretch eines Audio-Events, das dem Tempo folgt.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum StretchMode {
    /// Mittlere Körner (etwa 43 ms): für gemischtes Material.
    #[default]
    Standard,
    /// Kurze Körner (etwa 21 ms): Anschläge bleiben knackig (Schlagzeug, Perkussion).
    Rhythmic,
    /// Lange Körner (etwa 85 ms): weiche, stehende Klänge (Flächen, Gesang), mehr Rechenzeit.
    Pad,
}

impl StretchMode {
    pub const ALL: [StretchMode; 3] = [Self::Standard, Self::Rhythmic, Self::Pad];

    pub fn is_standard(&self) -> bool {
        *self == Self::Standard
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Rhythmic => "Rhythmisch",
            Self::Pad => "Flächig",
        }
    }

    /// Korngröße in Frames bei 48 kHz.
    pub fn grain_48k(self) -> usize {
        match self {
            Self::Standard => 2048,
            Self::Rhythmic => 1024,
            Self::Pad => 4096,
        }
    }
}

/// Frames des Clip-Inhalts für `ticks`: bei Events, die dem Tempo folgen (`musical`), im
/// Tempo der Datei, sonst über die Tempo-Map.
pub fn content_frames(tempo: &TempoMap, musical: Option<f64>, ticks: u64, rate: u32) -> u64 {
    match musical {
        Some(bpm) if bpm.is_finite() && bpm > 0.0 => {
            let f = ticks as f64 / TICKS_PER_QUARTER as f64 * 60.0 / bpm * f64::from(rate);
            if f.is_finite() {
                f.round().max(0.0) as u64
            } else {
                0
            }
        }
        _ => tempo.frames(ticks, rate),
    }
}

/// Tempo aus einem Dateinamen wie „Beat 92bpm“ oder „loop_120_BPM“: die Zahl direkt vor
/// „bpm“ (Groß-/Kleinschreibung egal, Trenner erlaubt), 40 bis 300.
pub fn bpm_in_name(name: &str) -> Option<f64> {
    let lower = name.to_lowercase();
    let bytes = lower.as_bytes();
    let mut from = 0;
    while let Some(i) = lower[from..].find("bpm").map(|i| i + from) {
        from = i + 3;
        // Trenner zwischen Zahl und „bpm“ überspringen.
        let mut end = i;
        while end > 0 && matches!(bytes[end - 1], b' ' | b'_' | b'-') {
            end -= 1;
        }
        let mut start = end;
        while start > 0 && (bytes[start - 1].is_ascii_digit() || bytes[start - 1] == b'.') {
            start -= 1;
        }
        if let Ok(v) = lower[start..end].parse::<f64>()
            && (40.0..=300.0).contains(&v)
        {
            return Some(v);
        }
    }
    None
}

/// Umkehrung von [`content_frames`]: Ticks für `frames` Frames des Clip-Inhalts.
pub fn content_ticks(tempo: &TempoMap, musical: Option<f64>, frames: u64, rate: u32) -> u64 {
    match musical {
        Some(bpm) if bpm.is_finite() && bpm > 0.0 && rate > 0 => {
            let t = frames as f64 / f64::from(rate) * bpm / 60.0 * TICKS_PER_QUARTER as f64;
            if t.is_finite() {
                t.round().max(0.0) as u64
            } else {
                0
            }
        }
        _ => tempo.ticks(frames, rate),
    }
}

fn default_channel() -> u8 {
    1
}

fn is_channel_one(v: &u8) -> bool {
    *v == 1
}

/// Standard: Inserts 1–6 vor dem Fader, 7–8 danach.
pub const DEFAULT_PRE_INSERTS: u8 = 6;

fn default_pre_inserts() -> u8 {
    DEFAULT_PRE_INSERTS
}

fn is_default_pre_inserts(v: &u8) -> bool {
    *v == DEFAULT_PRE_INSERTS
}

/// Bereich des Eingangsgains in dB.
pub const INPUT_GAIN_DB: (f32, f32) = (-24.0, 24.0);
/// Bereich der Pre-Filter in Hz.
pub const PRE_FILTER_HZ: (f32, f32) = (20.0, 20_000.0);

fn is_zero(v: &u32) -> bool {
    *v == 0
}

fn is_zero_db(v: &f32) -> bool {
    *v == 0.0
}

/// Höchste Lautstärke eines Events in dB (darunter bis [`MIN_DB`], dort stumm).
pub const EVENT_MAX_DB: f32 = 6.0;

/// Kurvenform eines Fades.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FadeCurve {
    #[default]
    Linear,
    /// Langsamer Anfang, schneller Schluss (quadratisch).
    Exponential,
    /// Weicher Anfang und Schluss.
    SCurve,
    /// Gleiche Leistung (Sinus/Kosinus): im Crossfade bleibt die Lautheit gleich.
    EqualPower,
}

impl FadeCurve {
    pub const ALL: [FadeCurve; 4] = [
        Self::Linear,
        Self::Exponential,
        Self::SCurve,
        Self::EqualPower,
    ];

    pub fn is_linear(&self) -> bool {
        *self == Self::Linear
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Exponential => "Exponentiell",
            Self::SCurve => "S-Kurve",
            Self::EqualPower => "Gleiche Leistung",
        }
    }

    /// Verstärkung bei `x` (0 = stumm, 1 = voll) für den Fortschritt `x` in 0..=1.
    pub fn shape(self, x: f32) -> f32 {
        let x = if x.is_finite() {
            x.clamp(0.0, 1.0)
        } else {
            0.0
        };
        match self {
            Self::Linear => x,
            Self::Exponential => x * x,
            Self::SCurve => x * x * (3.0 - 2.0 * x),
            Self::EqualPower => (x * std::f32::consts::FRAC_PI_2).sin(),
        }
    }
}

/// Eine MIDI-Note in einem Part (Spec „MIDI-Part“: Noten relativ zum Part-Start).
/// `start` darf negativ sein oder hinter dem Part-Ende liegen: Beim Kürzen eines
/// Parts bleiben Noten erhalten und sind nur ausgeblendet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Note {
    pub pitch: u8,
    pub velocity: u8,
    pub start: i64,
    pub length: u64,
    /// Schreibweise im Noten-Editor (nur Darstellung): 1 mit Kreuz, −1 mit Be, 0 nach der
    /// Tonart.
    #[serde(default, skip_serializing_if = "is_zero_i8")]
    pub spelling: i8,
}

fn is_zero_i8(v: &i8) -> bool {
    *v == 0
}

impl Note {
    pub fn end(&self) -> i64 {
        self.start + self.length as i64
    }
}

impl Event {
    /// Farbe des Events auf `track`: die eigene oder die der Spur.
    pub fn color_on(&self, track: &Track) -> [u8; 3] {
        self.color.unwrap_or(track.color)
    }

    pub fn end(&self) -> u64 {
        self.start.0 + self.length
    }

    pub fn contains(&self, ticks: u64) -> bool {
        (self.start.0..self.end()).contains(&ticks)
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Track {
    /// Feste Kennung (für Routing-Ziele); bleibt beim Umsortieren gleich.
    #[serde(default)]
    pub id: TrackId,
    pub name: String,
    pub kind: TrackKind,
    pub color: [u8; 3],
    pub mute: bool,
    pub solo: bool,
    pub record: bool,
    /// Eingang mithören (Audio-Spuren): das Eingangssignal läuft durch den Kanal der Spur.
    #[serde(default)]
    pub monitor: bool,
    /// Automation lesen (R) und schreiben (W); wirken, sobald es Automationsspuren gibt.
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub write: bool,
    /// Zeitbasis; `None` = Standard des Spurtyps ([`Track::time_base`]).
    #[serde(default)]
    pub time_base: Option<TimeBase>,
    /// Lanes aufgeklappt: überlappende Events (Takes) liegen untereinander.
    #[serde(default)]
    pub show_lanes: bool,
    /// Automationsspuren (Spec „Automation“).
    #[serde(default)]
    pub automation: Vec<crate::AutomationLane>,
    /// Ordnerspur, zu der diese Spur gehört (siehe `folders.rs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<TrackId>,
    /// Eingangsgain in dB (Spec „Signalfluss“, Punkt 1), −24 bis +24.
    #[serde(default, skip_serializing_if = "is_zero_db")]
    pub input_gain_db: f32,
    /// Phase des Eingangs umkehren.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub phase_invert: bool,
    /// Pre-Sektion: Hochpass bzw. Tiefpass mit Eckfrequenz in Hz; `None` aus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_hpf: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_lpf: Option<f32>,
    /// Im Mixer ausgeblendet (Kanalliste, Spec „Sichtbarkeit und Ansicht“).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mixer_hidden: bool,
    /// Im Mixer als schmaler Kanalzug.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mixer_narrow: bool,
    /// Pan-Modus: Stereo-Balance (Standard) oder Stereo kombiniert.
    #[serde(default, skip_serializing_if = "PanMode::is_balance")]
    pub pan_mode: PanMode,
    /// Solo-Defeat (Strg+Klick auf Solo): bleibt hörbar, wenn andere Spuren solo sind.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub solo_defeat: bool,
    /// Drum-Map (`drummap.rs`): Parts öffnen im Drum-Editor, Mute/Solo je Zeile wirken auf
    /// die Wiedergabe. `None`: Key-Editor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drum_map: Option<crate::DrumMap>,
    /// Link-Gruppe im Mixer (1 bis [`crate::MAX_LINK_GROUPS`]): Fader, Mute und Solo der
    /// Gruppe bewegen sich gemeinsam (`links.rs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_group: Option<u8>,
    /// MIDI-Spuren: Ausgang zu einem externen Gerät (Name des MIDI-Ports).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub midi_port: Option<String>,
    /// Lineal-Spuren: Anzeigeformat.
    #[serde(default, skip_serializing_if = "RulerFormat::is_bars")]
    pub ruler: RulerFormat,
    /// Videospuren: Pfad der Videodatei.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<String>,
    /// MIDI-Spuren: Kanal 1–16.
    #[serde(default = "default_channel", skip_serializing_if = "is_channel_one")]
    pub midi_channel: u8,
    /// MIDI-Spuren: Programm 0–127, das beim Wählen und beim Start gesendet wird.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<u8>,
    /// Ordnerspuren: zugeklappt, die enthaltenen Spuren sind ausgeblendet.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub folded: bool,
    /// Platz in der Spurliste, nachdem Spuren umsortiert wurden (siehe `folders.rs`); ohne
    /// Wert gilt der Index. Die Reihenfolge in `tracks` selbst ändert sich nie.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<u32>,
    /// Kanal-EQ (4 Bänder); Standard: neutral, wird dann nicht gespeichert.
    #[serde(default, skip_serializing_if = "crate::ChannelEq::is_default")]
    pub eq: crate::ChannelEq,
    /// Fader in dB; [`MIN_DB`] und darunter gilt als −∞.
    pub volume_db: f32,
    /// Panorama: −1 links, 0 Mitte, +1 rechts (Stereo-Balance).
    pub pan: f32,
    /// Aufnahme-Eingang (Audio-Spuren); gilt, wenn kein Eingangsbus gewählt ist.
    #[serde(default)]
    pub input: TrackInput,
    /// Eingangsbus (Audio-Anschlüsse); `None`: `input`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_bus: Option<u32>,
    /// Insert-Effekte in Signalfluss-Reihenfolge.
    #[serde(default)]
    pub inserts: Vec<InsertSlot>,
    /// Instrument-Spuren: das Instrument-Plugin.
    #[serde(default)]
    pub instrument: Option<InsertSlot>,
    /// Ausgang: `None` = Summe (Ausgangsbus), sonst eine Gruppen-Spur.
    #[serde(default)]
    pub output: Option<TrackId>,
    /// Ausgangsbus, wenn `output` die Summe ist: `None` = Hauptmix, sonst ein weiterer
    /// Ausgangsbus der Audio-Anschlüsse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_bus: Option<u32>,
    /// Weitere Ausgänge zusätzlich zu `output`/`out_bus` (Spec „Routing und Busse“: direkt
    /// auf mehrere Ziele). Dasselbe Signal nach dem Fader, ohne eigenen Pegel; höchstens
    /// [`MAX_EXTRA_OUTPUTS`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_outputs: Vec<OutTarget>,
    /// Sends an FX-Kanäle oder Gruppen (Spec: Sends 1–8).
    #[serde(default)]
    pub sends: Vec<SendSlot>,
    /// Eingefroren: Instrument und Inserts klingen als gerenderter Clip, die Plugins sind
    /// entladen und die Events gesperrt (Spec „Einfrieren“).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frozen: Option<Frozen>,
    pub events: Vec<Event>,
}

/// Höchstzahl weiterer Ausgänge einer Spur (mit dem Hauptausgang acht Ziele).
pub const MAX_EXTRA_OUTPUTS: usize = 7;

/// Ein weiteres Ausgangsziel einer Spur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutTarget {
    /// Eine Gruppen-Spur.
    Group(TrackId),
    /// Ein Ausgangsbus der Audio-Anschlüsse (auch der Hauptmix, [`crate::MAIN_OUT`]).
    Bus(u32),
}

impl Track {
    /// Ob das Signal dieser Spur über einen Ausgang (Haupt- oder weiteren) in `id` geht.
    pub fn routes_to(&self, id: TrackId) -> bool {
        self.output == Some(id) || self.extra_outputs.contains(&OutTarget::Group(id))
    }

    /// Ob ein weiteres Ziel schon der Hauptausgang ist (dann wäre es doppelt).
    pub fn is_main_output(&self, target: OutTarget) -> bool {
        match target {
            OutTarget::Group(id) => self.output == Some(id),
            OutTarget::Bus(id) => {
                self.output.is_none() && self.out_bus.unwrap_or(crate::MAIN_OUT) == id
            }
        }
    }
}

/// Gerendertes Signal einer eingefrorenen Spur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Frozen {
    /// Clip im Pool (Stereo, Projekt-Abtastrate).
    pub clip: ClipId,
    /// Anfang des Clips in Ticks.
    pub start: u64,
}

/// Zeitbasis einer Spur (Spec „Zeitbasis“): Musikalische Events hängen an Takten und wandern
/// bei Tempoänderungen mit, lineare hängen an Sekunden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TimeBase {
    Musical,
    Linear,
}

impl Track {
    /// Wirksame Zeitbasis: Standard ist linear für Audio, sonst musikalisch.
    pub fn time_base(&self) -> TimeBase {
        self.time_base.unwrap_or(if self.kind == TrackKind::Audio {
            TimeBase::Linear
        } else {
            TimeBase::Musical
        })
    }

    /// Lane je Event (gleiche Reihenfolge wie `events`) und Zahl der Lanes, mindestens 1.
    /// Jedes Event kommt in die oberste Lane, in der es nichts Früheres überlappt; spätere
    /// Takes landen so darunter.
    pub fn lanes(&self) -> (Vec<usize>, usize) {
        let mut ends: Vec<Vec<(u64, u64)>> = Vec::new();
        let mut out = vec![0; self.events.len()];
        // Feste Lanes zuerst (Comping), dann der Rest in der Reihenfolge der Liste.
        for (i, e) in self.events.iter().enumerate() {
            if let Some(l) = e.lane {
                let l = l as usize;
                if ends.len() <= l {
                    ends.resize_with(l + 1, Vec::new);
                }
                ends[l].push((e.start.0, e.end()));
                out[i] = l;
            }
        }
        for (i, e) in self.events.iter().enumerate() {
            if e.lane.is_some() {
                continue;
            }
            let span = (e.start.0, e.end());
            let lane = ends
                .iter()
                .position(|l| l.iter().all(|&(a, b)| span.1 <= a || span.0 >= b))
                .unwrap_or(ends.len());
            if lane == ends.len() {
                ends.push(Vec::new());
            }
            ends[lane].push(span);
            out[i] = lane;
        }
        (out, ends.len().max(1))
    }

    /// Schreibt die aktuelle Lane-Verteilung fest, damit Teilen und Comping nichts verschieben.
    pub fn freeze_lanes(&mut self) {
        let (lanes, _) = self.lanes();
        for (e, l) in self.events.iter_mut().zip(lanes) {
            e.lane = u32::try_from(l).ok();
        }
    }

    /// Hörbare Abschnitte (Ticks) je Event, in der Reihenfolge von `events`: Ein Event wird
    /// von nicht stummen Events mit höherem (Comp-Rang, Listenplatz) verdeckt, aber nur von
    /// deren Teil ohne Fades: Im Fade-In oder Fade-Out des oberen klingt das untere durch
    /// (Crossfade).
    pub fn audible_parts(&self) -> Vec<Vec<(u64, u64)>> {
        self.audible_spans()
            .into_iter()
            .map(|v| v.into_iter().map(|p| (p.from, p.to)).collect())
            .collect()
    }

    /// Wie [`Self::audible_parts`], dazu je Abschnitt die Überblendung an verdeckten Kanten:
    /// Endet ein Abschnitt, weil ein oberes Event einblendet, blendet er im selben Fenster aus
    /// (`ramp_out` = Fade-In des oberen); beginnt er, weil das obere ausblendet, blendet er
    /// ein (`ramp_in` = dessen Fade-Out). So entsteht an Comp-Grenzen ein Crossfade statt
    /// eines harten Schnitts. Kanten am eigenen Anfang oder Ende des Events: 0.
    pub fn audible_spans(&self) -> Vec<Vec<AudiblePart>> {
        self.events
            .iter()
            .enumerate()
            .map(|(i, e)| {
                if e.muted {
                    return Vec::new();
                }
                let mut parts = vec![AudiblePart {
                    from: e.start.0,
                    to: e.end(),
                    ramp_in: 0,
                    ramp_out: 0,
                }];
                for (j, c) in self.events.iter().enumerate() {
                    if j == i || c.muted || (c.comp, j) < (e.comp, i) {
                        continue;
                    }
                    let ca = c.start.0.saturating_add(c.fade_in);
                    let cb = c.end().saturating_sub(c.fade_out);
                    if cb <= ca {
                        continue;
                    }
                    parts = parts
                        .into_iter()
                        .flat_map(|p| {
                            let (a, b) = (p.from, p.to);
                            if cb <= a || ca >= b {
                                return vec![p];
                            }
                            let mut out = Vec::with_capacity(2);
                            if ca > a {
                                let len = ca - a;
                                out.push(AudiblePart {
                                    from: a,
                                    to: ca,
                                    ramp_in: p.ramp_in.min(len),
                                    ramp_out: c.fade_in.min(len),
                                });
                            }
                            if cb < b {
                                let len = b - cb;
                                out.push(AudiblePart {
                                    from: cb,
                                    to: b,
                                    ramp_in: c.fade_out.min(len),
                                    ramp_out: p.ramp_out.min(len),
                                });
                            }
                            out
                        })
                        .collect();
                }
                parts
            })
            .collect()
    }
}

/// Ein hörbarer Abschnitt eines Events (Ticks) mit Überblendung an verdeckten Kanten
/// ([`Track::audible_spans`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudiblePart {
    pub from: u64,
    pub to: u64,
    /// Einblenden am Anfang des Abschnitts (Ticks), weil ein oberes Event hier ausblendet.
    pub ramp_in: u64,
    /// Ausblenden am Ende des Abschnitts (Ticks), weil ein oberes Event hier einblendet.
    pub ramp_out: u64,
}

/// Länge der Überblendung an den Grenzen eines Comp-Abschnitts: eine 64tel.
pub const COMP_FADE: u64 = TICKS_PER_QUARTER / 16;

/// Kennung einer Spur.
pub type TrackId = u32;

/// Ein Send: ein Teil des Signals an einen FX-Kanal oder eine Gruppe.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SendSlot {
    pub target: TrackId,
    pub level_db: f32,
    /// Vor dem Fader abgreifen (sonst danach).
    pub pre_fader: bool,
    pub on: bool,
}

/// Ein Insert-Effekt: welches Plugin, ob umgangen, gesetzte Parameter.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InsertSlot {
    /// CLAP-Plugin-Id, z. B. `in.lsp-plug.para_equalizer_x8_stereo`.
    pub plugin_id: String,
    /// Anzeigename (auch wenn das Plugin fehlt).
    pub name: String,
    pub bypass: bool,
    /// Vom Standard abweichende Parameter: (Id, Wert).
    pub params: Vec<(u32, f64)>,
    /// Kompletter Plugin-Zustand (`clap.state`), Base64. Wird beim Speichern erfasst.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// In einem eigenen Prozess rechnen (Plugin-Bridge, Issue #146): stürzt das Plugin ab,
    /// verstummt nur dieser Insert. Fehlt in älteren Dateien.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub isolated: bool,
}

impl InsertSlot {
    pub fn set_param(&mut self, id: u32, value: f64) {
        match self.params.iter_mut().find(|(i, _)| *i == id) {
            Some(p) => p.1 = value,
            None => self.params.push((id, value)),
        }
    }

    /// Gleiches Plugin an gleicher Stelle, im selben Prozess (Parameter und Bypass dürfen
    /// abweichen).
    pub fn same_plugin(&self, other: &Self) -> bool {
        self.plugin_id == other.plugin_id && self.isolated == other.isolated
    }
}

/// Welche Eingangskanäle eine Audio-Spur aufnimmt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TrackInput {
    /// Erster Kanal, ab 0.
    pub first: u16,
    /// `true`: `first` und `first + 1` als Stereo, sonst Mono.
    pub stereo: bool,
}

impl Default for TrackInput {
    fn default() -> Self {
        Self {
            first: 0,
            stereo: true,
        }
    }
}

impl TrackInput {
    /// Anzeige wie „Eingang 1/2“ oder „Eingang 2“ (Kanäle ab 1 gezählt).
    pub fn label(self) -> String {
        if self.stereo {
            format!("Eingang {}/{}", self.first + 1, self.first + 2)
        } else {
            format!("Eingang {}", self.first + 1)
        }
    }

    /// Alle sinnvollen Eingänge bei `channels` Gerätekanälen: Stereopaare, dann Mono.
    pub fn options(channels: u16) -> Vec<Self> {
        let pairs = (0..channels / 2).map(|i| Self {
            first: 2 * i,
            stereo: true,
        });
        let monos = (0..channels).map(|c| Self {
            first: c,
            stereo: false,
        });
        pairs.chain(monos).collect()
    }
}

/// Untere Grenze des Faders; darunter ist der Kanal stumm (−∞).
pub const MIN_DB: f32 = -60.0;
/// Obere Grenze des Faders.
pub const MAX_DB: f32 = 6.0;

/// dB in linearen Faktor; [`MIN_DB`] und darunter ergibt 0.
pub fn db_to_gain(db: f32) -> f32 {
    if db <= MIN_DB {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

impl Track {
    /// Faktoren für links und rechts aus Fader und Stereo-Balance.
    /// Faktoren des Kanals als Matrix `[ll, lr, rl, rr]` (links in → links/rechts aus,
    /// rechts in → links/rechts aus): Lautstärke mal Pan im eingestellten Modus.
    pub fn mix_matrix(&self) -> [f32; 4] {
        let g = db_to_gain(self.volume_db);
        pan_matrix(self.pan_mode, self.pan).map(|v| v * g)
    }

    pub fn gains(&self) -> (f32, f32) {
        let g = db_to_gain(self.volume_db);
        let p = self.pan.clamp(-1.0, 1.0);
        let l = if p > 0.0 { 1.0 - p } else { 1.0 };
        let r = if p < 0.0 { 1.0 + p } else { 1.0 };
        (g * l, g * r)
    }
}

/// Eine gespeicherte Mixer-Ansicht (Spec „Sichtbarkeit und Ansicht“).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MixerView {
    pub name: String,
    /// Ausgeblendete und schmale Kanäle (nach Spur-Kennung).
    #[serde(default)]
    pub hidden: Vec<TrackId>,
    #[serde(default)]
    pub narrow: Vec<TrackId>,
}

/// Höchstzahl gespeicherter Mixer-Ansichten.
pub const MAX_MIXER_VIEWS: usize = 9;

impl Project {
    /// Speichert Sichtbarkeit und Breite aller Kanäle als Ansicht `name` (gleicher Name
    /// ersetzt). `false`, wenn schon [`MAX_MIXER_VIEWS`] andere Ansichten da sind.
    pub fn save_mixer_view(&mut self, name: &str) -> bool {
        let name: String = name.trim().chars().take(64).collect();
        if name.is_empty() {
            return false;
        }
        let pick = |f: fn(&Track) -> bool| -> Vec<TrackId> {
            self.tracks.iter().filter(|t| f(t)).map(|t| t.id).collect()
        };
        let view = MixerView {
            name: name.clone(),
            hidden: pick(|t| t.mixer_hidden),
            narrow: pick(|t| t.mixer_narrow),
        };
        if let Some(v) = self.mixer_views.iter_mut().find(|v| v.name == name) {
            *v = view;
        } else if self.mixer_views.len() < MAX_MIXER_VIEWS {
            self.mixer_views.push(view);
        } else {
            return false;
        }
        true
    }

    /// Ruft Ansicht `i` ab; Kanäle, die es damals nicht gab, werden sichtbar und breit.
    pub fn apply_mixer_view(&mut self, i: usize) -> bool {
        let Some(v) = self.mixer_views.get(i).cloned() else {
            return false;
        };
        for t in &mut self.tracks {
            t.mixer_hidden = v.hidden.contains(&t.id);
            t.mixer_narrow = v.narrow.contains(&t.id);
        }
        true
    }
}

/// Pan-Modus eines Kanals (Spec „Kanalzug-Elemente“).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum PanMode {
    /// Stereo-Balance: dämpft die Gegenseite, die Kanäle bleiben, wo sie sind.
    #[default]
    Balance,
    /// Stereo kombiniert: beide Kanäle wandern gemeinsam (gleiche Leistung); ganz rechts
    /// liegen beide rechts.
    Combined,
}

impl PanMode {
    pub fn is_balance(&self) -> bool {
        *self == PanMode::Balance
    }

    pub fn label(self) -> &'static str {
        match self {
            PanMode::Balance => "Stereo-Balance",
            PanMode::Combined => "Stereo kombiniert",
        }
    }
}

/// Pan als Matrix `[ll, lr, rl, rr]` für `pan` in −1..1 (NaN gilt als Mitte). In der Mitte ist
/// sie in beiden Modi die Einheitsmatrix.
pub fn pan_matrix(mode: PanMode, pan: f32) -> [f32; 4] {
    let p = if pan.is_finite() {
        pan.clamp(-1.0, 1.0)
    } else {
        0.0
    };
    match mode {
        PanMode::Balance => {
            let l = if p > 0.0 { 1.0 - p } else { 1.0 };
            let r = if p < 0.0 { 1.0 + p } else { 1.0 };
            [l, 0.0, 0.0, r]
        }
        PanMode::Combined => {
            // Lage der beiden Kanäle: links wandert ab der Mitte nach rechts mit, rechts nach
            // links; je Kanal gleiche Leistung.
            let pos_l = (-1.0 + 2.0 * p.max(0.0)).clamp(-1.0, 1.0);
            let pos_r = (1.0 + 2.0 * p.min(0.0)).clamp(-1.0, 1.0);
            let angle = |pos: f32| (pos + 1.0) * std::f32::consts::FRAC_PI_4;
            let (al, ar) = (angle(pos_l), angle(pos_r));
            [al.cos(), al.sin(), ar.cos(), ar.sin()]
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Project {
    pub tracks: Vec<Track>,
    pub tempo: TempoMap,
    pub pool: Vec<PoolClip>,
    /// Bearbeitete Clips: woraus und wie sie entstanden (`processing.rs`).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub derived: std::collections::BTreeMap<ClipId, crate::processing::Derived>,
    /// Fader des Ausgangsbusses in dB.
    pub master_db: f32,
    next_id: EventId,
    #[serde(default)]
    next_track_id: TrackId,
    /// Marker und Cycle-Marker, nach Position sortiert (siehe `markers.rs`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<crate::Marker>,
    /// MIDI-Aufnahmemodi (Projekteinstellung).
    #[serde(default, skip_serializing_if = "crate::RecordSettings::is_default")]
    pub record: crate::RecordSettings,
    /// Audio-Anschlüsse: Ein- und Ausgangsbusse.
    #[serde(default, skip_serializing_if = "crate::AudioIo::is_default")]
    pub io: crate::AudioIo,
    /// Metronom und Vorzähler (Projekteinstellung).
    #[serde(default, skip_serializing_if = "crate::ClickSettings::is_default")]
    pub click: crate::ClickSettings,
    /// Sichtbarkeit und Größe der Zonen (Spec „Fenster-Architektur“).
    #[serde(default)]
    pub layout: crate::WindowLayout,
    /// Benannte Fensterlayouts, abrufbar per Taste.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layouts: Vec<crate::NamedLayout>,
    /// So viele Insert-Slots liegen vor dem Fader (Projekt-Voreinstellung, Standard 6 von 8);
    /// die übrigen danach (Spec „Signalfluss“).
    #[serde(
        default = "default_pre_inserts",
        skip_serializing_if = "is_default_pre_inserts"
    )]
    pub pre_fader_inserts: u8,
    /// Gespeicherte Mixer-Ansichten: welche Kanäle sichtbar und welche schmal sind.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mixer_views: Vec<MixerView>,
    /// Tonart für den Noten-Editor (Vorzeichen); ändert keine Tonhöhen.
    #[serde(default, skip_serializing_if = "crate::KeySignature::is_c_major")]
    pub key: crate::KeySignature,
}

/// Farben neuer Spuren, reihum; auch die Palette der Farbauswahl.
pub const TRACK_COLORS: [[u8; 3]; 8] = [
    [214, 84, 84],
    [222, 150, 60],
    [204, 190, 70],
    [96, 180, 96],
    [70, 170, 170],
    [80, 130, 214],
    [150, 100, 214],
    [200, 90, 160],
];

impl Default for Project {
    fn default() -> Self {
        Self {
            tracks: Vec::new(),
            pool: Vec::new(),
            derived: std::collections::BTreeMap::new(),
            master_db: 0.0,
            tempo: TempoMap::default(),
            next_id: 1,
            next_track_id: 1,
            markers: Vec::new(),
            record: crate::RecordSettings::default(),
            click: crate::ClickSettings::default(),
            io: crate::AudioIo::default(),
            layout: crate::WindowLayout::default(),
            layouts: Vec::new(),
            mixer_views: Vec::new(),
            pre_fader_inserts: DEFAULT_PRE_INSERTS,
            key: crate::KeySignature::C_MAJOR,
        }
    }
}

impl Project {
    fn new_track_id(&mut self) -> TrackId {
        self.next_track_id = self.next_track_id.max(1);
        let id = self.next_track_id;
        self.next_track_id += 1;
        id
    }

    /// Fehlende oder doppelte Spur-Kennungen vergeben (ältere Dateien).
    pub fn ensure_track_ids(&mut self) {
        self.fix_ids();
    }

    /// Eindeutige Ids unterhalb der Grenzen und Zähler dahinter, auch für fremde Dateien
    /// (doppelte Ids, Ids nahe dem Maximum). Gibt zurück, wie viele Ids neu vergeben wurden.
    pub(crate) fn fix_ids(&mut self) -> usize {
        /// Grenzen, unter denen die Zähler nie überlaufen.
        const MAX_TRACK_ID: TrackId = 1 << 24;
        const MAX_EVENT_ID: EventId = 1 << 48;
        let mut fixed = 0;
        let mut seen = std::collections::HashSet::new();
        let ok = |id: TrackId| id != 0 && id < MAX_TRACK_ID;
        let top = self.tracks.iter().map(|t| t.id).filter(|&id| ok(id)).max();
        self.next_track_id = self
            .next_track_id
            .min(MAX_TRACK_ID)
            .max(top.map_or(1, |m| m + 1));
        for i in 0..self.tracks.len() {
            let id = self.tracks[i].id;
            if !ok(id) || !seen.insert(id) {
                self.tracks[i].id = self.new_track_id();
                seen.insert(self.tracks[i].id);
                fixed += 1;
            }
        }
        let mut seen = std::collections::HashSet::new();
        let top = self
            .tracks
            .iter()
            .flat_map(|t| &t.events)
            .map(|e| e.id)
            .filter(|&id| id != 0 && id < MAX_EVENT_ID)
            .max();
        self.next_id = self
            .next_id
            .clamp(1, MAX_EVENT_ID)
            .max(top.map_or(1, |m| m + 1));
        for t in 0..self.tracks.len() {
            for e in 0..self.tracks[t].events.len() {
                let id = self.tracks[t].events[e].id;
                if id == 0 || id >= MAX_EVENT_ID || !seen.insert(id) {
                    let new = self.next_event_id();
                    self.tracks[t].events[e].id = new;
                    seen.insert(new);
                    fixed += 1;
                }
            }
        }
        fixed
    }

    pub fn track_index(&self, id: TrackId) -> Option<usize> {
        self.tracks.iter().position(|t| t.id == id)
    }

    /// Ob eine Spur ein Bus ist (Gruppe oder FX-Kanal): Ziel für Sends bzw. Ausgänge.
    pub fn is_bus(kind: TrackKind) -> bool {
        matches!(kind, TrackKind::Group | TrackKind::Fx)
    }

    /// Ob ein Signal von `from` nach `to` geführt werden darf, ohne eine
    /// Rückkopplungsschleife zu bilden (über Ausgänge und Sends).
    pub fn can_route(&self, from: TrackId, to: TrackId) -> bool {
        if from == to {
            return false;
        }
        // Erreicht `to` (über seine Wege) wieder `from`? Dann Schleife.
        let mut stack = vec![to];
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = stack.pop() {
            if id == from {
                return false;
            }
            if !seen.insert(id) {
                continue;
            }
            if let Some(t) = self.track_index(id).map(|i| &self.tracks[i]) {
                stack.extend(t.output);
                stack.extend(t.extra_outputs.iter().filter_map(|o| match *o {
                    OutTarget::Group(g) => Some(g),
                    OutTarget::Bus(_) => None,
                }));
                stack.extend(t.sends.iter().filter(|s| s.on).map(|s| s.target));
            }
        }
        true
    }

    pub fn next_event_id(&mut self) -> EventId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Demo-Projekt mit `n` Spuren und verteilten Events, für Prototyp und Tests.
    pub fn demo(n: usize) -> Self {
        let mut p = Self::default();
        let bar = p.tempo.signature.bar_ticks();
        for i in 0..n {
            let kind = match i % 7 {
                0..=2 => TrackKind::Audio,
                3 | 4 => TrackKind::Instrument,
                5 => TrackKind::Midi,
                _ => TrackKind::Group,
            };
            let mut track = Track {
                id: 0,
                name: format!("{} {}", kind.label(), i + 1),
                kind,
                color: TRACK_COLORS[i % TRACK_COLORS.len()],
                mute: false,
                solo: false,
                record: false,
                monitor: false,
                read: false,
                write: false,
                time_base: None,
                show_lanes: false,
                automation: Vec::new(),
                eq: crate::ChannelEq::default(),
                parent: None,
                folded: false,
                order: None,
                midi_port: None,
                ruler: RulerFormat::Bars,
                video: None,
                midi_channel: 1,
                program: None,
                solo_defeat: false,
                link_group: None,
                drum_map: None,
                pan_mode: PanMode::Balance,
                mixer_hidden: false,
                mixer_narrow: false,
                input_gain_db: 0.0,
                phase_invert: false,
                pre_hpf: None,
                pre_lpf: None,
                volume_db: 0.0,
                pan: 0.0,
                input: TrackInput::default(),
                input_bus: None,
                out_bus: None,
                extra_outputs: Vec::new(),
                inserts: Vec::new(),
                instrument: None,
                output: None,
                sends: Vec::new(),
                frozen: None,
                events: Vec::new(),
            };
            if kind.holds_events() {
                // Deterministisch verteilte Events: 1–4 Takte lang, mit Lücken.
                let mut at = (i as u64 % 4) * bar;
                let mut k = 0u64;
                while at < 64 * bar {
                    let len = (1 + (i as u64 + k) % 4) * bar;
                    let id = p.next_event_id();
                    track.events.push(Event {
                        id,
                        start: Position(at),
                        length: len,
                        name: format!("{} #{}", kind.label(), k + 1),
                        muted: false,
                        clip: None,
                        offset: 0,
                        fade_in: 0,
                        fade_out: 0,
                        notes: Vec::new(),
                        controllers: Vec::new(),
                        lane: None,
                        comp: 0,
                        color: None,
                        shared: None,
                        musical: None,
                        stretch_mode: StretchMode::Standard,
                        warp: Vec::new(),
                        score_quantize: None,
                        children: Vec::new(),
                        gain_db: 0.0,
                        fade_in_curve: FadeCurve::Linear,
                        fade_out_curve: FadeCurve::Linear,
                    });
                    at += len + ((i as u64 + 2 * k) % 3) * bar;
                    k += 1;
                }
            }
            track.id = p.new_track_id();
            p.tracks.push(track);
        }
        p
    }

    pub fn event_at(&self, track: usize, ticks: u64) -> Option<EventId> {
        self.tracks
            .get(track)?
            .events
            .iter()
            .enumerate()
            .filter(|(_, e)| e.contains(ticks))
            .max_by_key(|(i, e)| (e.comp, *i))
            .map(|(_, e)| e.id)
    }

    pub fn find(&self, id: EventId) -> Option<(usize, usize)> {
        self.tracks
            .iter()
            .enumerate()
            .find_map(|(t, track)| track.events.iter().position(|e| e.id == id).map(|i| (t, i)))
    }

    pub fn event(&self, id: EventId) -> Option<&Event> {
        let (t, i) = self.find(id)?;
        Some(&self.tracks[t].events[i])
    }

    pub fn event_mut(&mut self, id: EventId) -> Option<&mut Event> {
        let (t, i) = self.find(id)?;
        Some(&mut self.tracks[t].events[i])
    }

    /// Fügt eine Kopie von `ev` mit neuer Id auf Spur `track` ein (Einfügen aus der
    /// Zwischenablage). `None`, wenn die Spur fehlt oder keine Events trägt.
    pub fn insert_event(&mut self, track: usize, mut ev: Event) -> Option<EventId> {
        if !self.tracks.get(track)?.kind.holds_events() {
            return None;
        }
        ev.id = self.next_event_id();
        let id = ev.id;
        let events = &mut self.tracks[track].events;
        let pos = events.partition_point(|e| e.start.0 <= ev.start.0);
        events.insert(pos, ev);
        Some(id)
    }

    /// Teilt ein Event an `at`. Gibt die Id des neuen rechten Teils zurück.
    pub fn split(&mut self, id: EventId, at: u64) -> Option<EventId> {
        let new_id = self.next_id;
        let (t, i) = self.find(id)?;
        let ev = &mut self.tracks[t].events[i];
        if at <= ev.start.0 || at >= ev.end() {
            return None;
        }
        let mut right = ev.clone();
        right.id = new_id;
        if let Some(c) = ev.clip.and_then(|c| self.pool.get(c as usize)) {
            right.offset = ev.shifted_offset(
                &self.tempo,
                c.sample_rate,
                i64::try_from(at - ev.start.0).unwrap_or(i64::MAX),
            );
        }
        right.start = Position(at);
        right.length = ev.end() - at;
        ev.length = at - ev.start.0;
        // Noten: was ab dem Schnitt beginnt, wandert in den rechten Part.
        let cut = (at - ev.start.0) as i64;
        right.notes = ev
            .notes
            .iter()
            .filter(|n| n.start >= cut)
            .map(|n| Note {
                start: n.start - cut,
                ..*n
            })
            .collect();
        ev.notes.retain(|n| n.start < cut);
        right.controllers = ev
            .controllers
            .iter()
            .filter(|c| c.start >= cut)
            .map(|c| crate::CtrlPoint {
                start: c.start - cut,
                ..*c
            })
            .collect();
        ev.controllers.retain(|c| c.start < cut);
        // Audio-Part: seine Events werden am Schnitt geteilt.
        if ev.is_audio_part() {
            let (l, r) = crate::audio_parts::split_children(
                &ev.children,
                cut.unsigned_abs(),
                &self.pool,
                &self.tempo,
            );
            ev.children = l;
            right.children = r;
        }
        // Fade-In bleibt links, Fade-Out wandert nach rechts.
        right.fade_in = 0;
        ev.fade_out = 0;
        ev.fade_in = ev.fade_in.min(ev.length);
        right.fade_out = right.fade_out.min(right.length);
        self.tracks[t].events.insert(i + 1, right);
        self.next_id += 1;
        Some(new_id)
    }

    /// Höchstzahl der Stücke bei [`Project::split_equal`]: schützt vor zehntausenden Events
    /// bei sehr kleinem Abstand.
    pub const MAX_EQUAL_PIECES: u64 = 1000;

    /// Teilt das Event in gleich lange Stücke; die Länge ist der Abstand von seinem Anfang
    /// bis `at` (Werkzeug Trennen mit Alt). Gibt die Ids aller Stücke von links nach rechts
    /// zurück, leer, wenn `at` nicht im Event liegt. Höchstens [`Self::MAX_EQUAL_PIECES`].
    pub fn split_equal(&mut self, id: EventId, at: u64) -> Vec<EventId> {
        let Some(ev) = self.event(id) else {
            return Vec::new();
        };
        let (start, end) = (ev.start.0, ev.end());
        if at <= start || at >= end {
            return Vec::new();
        }
        let piece = at - start;
        let mut ids = vec![id];
        let mut current = id;
        let mut cut = at;
        while cut < end && (ids.len() as u64) < Self::MAX_EQUAL_PIECES {
            match self.split(current, cut) {
                Some(next) => {
                    ids.push(next);
                    current = next;
                }
                None => break,
            }
            cut = match cut.checked_add(piece) {
                Some(c) => c,
                None => break,
            };
        }
        ids
    }

    pub fn delete(&mut self, id: EventId) -> bool {
        match self.find(id) {
            Some((t, i)) => {
                self.tracks[t].events.remove(i);
                true
            }
            None => false,
        }
    }

    /// Verschiebt (oder kopiert) ein Event auf Spur `track` an `start`.
    /// Gibt die Id des Events an der neuen Stelle zurück.
    pub fn move_event(
        &mut self,
        id: EventId,
        track: usize,
        start: u64,
        copy: bool,
    ) -> Option<EventId> {
        if !self.tracks.get(track)?.kind.holds_events() {
            return None;
        }
        let (t, i) = self.find(id)?;
        let mut ev = if copy {
            let mut c = self.tracks[t].events[i].clone();
            c.id = self.next_event_id();
            c
        } else {
            self.tracks[t].events.remove(i)
        };
        ev.start = Position(start);
        let new_id = ev.id;
        let events = &mut self.tracks[track].events;
        let pos = events.partition_point(|e| e.start.0 <= start);
        events.insert(pos, ev);
        Some(new_id)
    }

    /// Neues leeres Event (Zeichnen-Werkzeug).
    pub fn create_event(&mut self, track: usize, start: u64, length: u64) -> Option<EventId> {
        let kind = self.tracks.get(track)?.kind;
        if !kind.holds_events() || length == 0 {
            return None;
        }
        Some(self.insert_blank(track, start, length))
    }

    /// Legt ohne Prüfung ein leeres Event an (für [`Self::create_event`] und Videospuren).
    fn insert_blank(&mut self, track: usize, start: u64, length: u64) -> EventId {
        let kind = self.tracks[track].kind;
        let id = self.next_event_id();
        let name = if kind == TrackKind::Audio {
            "Audio"
        } else {
            "Part"
        }
        .to_owned();
        let events = &mut self.tracks[track].events;
        let pos = events.partition_point(|e| e.start.0 <= start);
        events.insert(
            pos,
            Event {
                id,
                start: Position(start),
                length,
                name,
                muted: false,
                clip: None,
                offset: 0,
                fade_in: 0,
                fade_out: 0,
                notes: Vec::new(),
                controllers: Vec::new(),
                lane: None,
                comp: 0,
                color: None,
                shared: None,
                musical: None,
                stretch_mode: StretchMode::Standard,
                warp: Vec::new(),
                score_quantize: None,
                children: Vec::new(),
                gain_db: 0.0,
                fade_in_curve: FadeCurve::Linear,
                fade_out_curve: FadeCurve::Linear,
            },
        );
        id
    }

    /// Legt eine Videospur für die Datei `path` an: ein Event von `length` Ticks ab
    /// Projektanfang, mit dem Ton des Videos (`clip`), falls es einen hat. Ohne Ton bleibt das
    /// Event stumm und ohne Clip.
    pub fn add_video_track(
        &mut self,
        name: impl Into<String>,
        path: impl Into<String>,
        length: u64,
        clip: Option<ClipId>,
    ) -> usize {
        let name = name.into();
        let t = self.add_track(TrackKind::Video, name.clone());
        self.tracks[t].video = Some(path.into());
        let length = length.max(1);
        let id = match clip.and_then(|c| self.add_audio_event(t, 0, c)) {
            Some(id) => id,
            None => {
                let id = self.insert_blank(t, 0, length);
                if let Some(ev) = self.event_mut(id) {
                    ev.muted = true;
                }
                id
            }
        };
        if let Some(ev) = self.event_mut(id) {
            ev.length = length;
            ev.name = name;
        }
        t
    }

    /// Wie weit ein Audio-Event höchstens nach links (Start früher) und
    /// insgesamt reichen darf, in Ticks: begrenzt durch den Clip.
    fn clip_bounds(&self, ev: &Event) -> Option<(u64, u64)> {
        let c = self.pool.get(ev.clip? as usize)?;
        let before = ev.content_span_ticks(&self.tempo, c.sample_rate, 0, ev.offset);
        let total = ev.content_span_ticks(&self.tempo, c.sample_rate, 0, c.frames);
        Some((before, total))
    }

    /// Crossfade (Taste X): Überlappen sich `a` und `b` auf einer Spur teilweise, bekommt das
    /// frühere ein Fade-Out und das spätere ein Fade-In über die ganze Überlappung. Liegt eins
    /// ganz im anderen oder überlappen sie nicht, passiert nichts.
    pub fn crossfade(&mut self, a: EventId, b: EventId) -> bool {
        let (Some((ta, _)), Some((tb, _))) = (self.find(a), self.find(b)) else {
            return false;
        };
        let (Some(ea), Some(eb)) = (self.event(a).cloned(), self.event(b).cloned()) else {
            return false;
        };
        if ta != tb || a == b {
            return false;
        }
        let (first, second) = if (ea.start.0, a) <= (eb.start.0, b) {
            (ea, eb)
        } else {
            (eb, ea)
        };
        if !(second.start.0 > first.start.0
            && second.start.0 < first.end()
            && second.end() > first.end())
        {
            return false;
        }
        let overlap = first.end() - second.start.0;
        let first_in = first.fade_in.min(first.length - overlap);
        let second_out = second.fade_out.min(second.length - overlap);
        let before = (
            first.fade_in,
            first.fade_out,
            second.fade_in,
            second.fade_out,
        );
        self.set_fades(first.id, first_in, overlap);
        self.set_fades(second.id, overlap, second_out);
        let fades = |id| self.event(id).map_or((0, 0), |e| (e.fade_in, e.fade_out));
        let (f, g) = (fades(first.id), fades(second.id));
        (f.0, f.1, g.0, g.1) != before
    }

    /// Crossfade-Partner des Fade-Ins (`fade_in`) bzw. Fade-Outs von `id`: das Event auf derselben
    /// Spur, dessen Fade-Out bzw. Fade-In dieselbe Überlappung abdeckt. Liefert (früheres,
    /// späteres) Event.
    pub fn crossfade_pair(&self, id: EventId, fade_in: bool) -> Option<(EventId, EventId)> {
        let (t, _) = self.find(id)?;
        let e = self.event(id)?;
        let partner = self.tracks[t].events.iter().find(|o| {
            if o.id == id {
                return false;
            }
            if fade_in {
                e.fade_in > 0
                    && o.fade_out > 0
                    && o.start.0 < e.start.0
                    && o.end() > e.start.0
                    && o.end() < e.end()
            } else {
                e.fade_out > 0
                    && o.fade_in > 0
                    && o.start.0 > e.start.0
                    && o.start.0 < e.end()
                    && o.end() > e.end()
            }
        })?;
        Some(if fade_in {
            (partner.id, id)
        } else {
            (id, partner.id)
        })
    }

    /// Crossfades für jedes überlappende Paar unter `ids` und ihren Nachbarn auf der Spur.
    /// Gibt zurück, ob sich etwas geändert hat.
    pub fn crossfade_selection(&mut self, ids: &[EventId]) -> bool {
        let mut pairs = Vec::new();
        for &id in ids {
            let Some((t, _)) = self.find(id) else {
                continue;
            };
            for other in self.tracks[t].events.iter().map(|e| e.id) {
                if other != id && !pairs.contains(&(other, id)) {
                    pairs.push((id, other));
                }
            }
        }
        let mut changed = false;
        for (a, b) in pairs {
            changed |= self.crossfade(a, b);
        }
        changed
    }

    /// Comping: Der Abschnitt `from..to` des Takes `id` klingt ab jetzt über allen anderen.
    /// Das Take wird an den Grenzen geteilt (die Lanes bleiben fest); der Abschnitt bekommt den
    /// höchsten Comp-Rang der Spur. Gibt die Id des Abschnitts zurück.
    pub fn comp(&mut self, id: EventId, from: u64, to: u64) -> Option<EventId> {
        let (t, _) = self.find(id)?;
        self.tracks[t].freeze_lanes();
        let ev = self.event(id)?;
        let (a, b) = (from.min(to).max(ev.start.0), from.max(to).min(ev.end()));
        if b <= a {
            return None;
        }
        let mut mid = id;
        let cut_a = match self.split(mid, a) {
            Some(right) => {
                mid = right;
                true
            }
            None => false,
        };
        let cut_b = self.split(mid, b).is_some();
        let top = self.tracks[t]
            .events
            .iter()
            .map(|e| e.comp)
            .max()
            .unwrap_or(0);
        let ev = self.event_mut(mid)?;
        ev.comp = top.saturating_add(1);
        // An den Schnitten kurz überblenden (der verdeckte Take blendet im selben Fenster
        // aus bzw. ein, siehe `audible_spans`); vorhandene Fades bleiben.
        let fade = COMP_FADE.min(ev.length / 4);
        if cut_a && ev.fade_in == 0 {
            ev.fade_in = fade;
        }
        if cut_b && ev.fade_out == 0 {
            ev.fade_out = fade;
        }
        Some(mid)
    }

    /// Snap-Modus „Schieben“: Das Event wandert innerhalb seiner Spur an die Stelle, an der
    /// seine Mitte bei `new_start` läge. Die Events zwischen alter und neuer Stelle rücken ohne
    /// Lücke zusammen, beginnend am Anfang des frühesten von ihnen. Andere Events bleiben.
    /// Gibt zurück, ob sich die Reihenfolge geändert hat.
    pub fn shuffle_event(&mut self, id: EventId, new_start: u64) -> bool {
        let Some((t, from)) = self.find(id) else {
            return false;
        };
        let events = &mut self.tracks[t].events;
        let ev = events.remove(from);
        let mid = new_start.saturating_add(ev.length / 2);
        let to = events.partition_point(|e| e.start.0.saturating_add(e.length / 2) < mid);
        events.insert(to, ev);
        if to == from {
            return false;
        }
        let (a, b) = (from.min(to), from.max(to));
        let mut pos = events[a..=b].iter().map(|e| e.start.0).min().unwrap_or(0);
        for e in &mut events[a..=b] {
            e.start = Position(pos);
            pos = pos.saturating_add(e.length);
        }
        true
    }

    /// Verschiebt den Anfang eines Events (Länge ändern am linken Rand).
    /// Bei Audio bleibt der Inhalt an seiner Stelle; der Offset wandert mit.
    pub fn trim_start(&mut self, id: EventId, new_start: u64) -> bool {
        let Some(ev) = self.event(id) else {
            return false;
        };
        let end = ev.end();
        let mut start = new_start.min(end.saturating_sub(1));
        let bounds = self.clip_bounds(ev);
        let rate = ev
            .clip
            .and_then(|c| self.pool.get(c as usize))
            .map(|c| c.sample_rate);
        if let Some((before, _)) = bounds {
            start = start.max(ev.start.0.saturating_sub(before));
        }
        let old_start = ev.start.0;
        let tempo = self.tempo.clone();
        let pool = std::mem::take(&mut self.pool);
        let Some(ev) = self.event_mut(id) else {
            self.pool = pool;
            return false;
        };
        if let Some(rate) = rate {
            let delta = start as i64 - old_start as i64;
            ev.offset = ev.shifted_offset(&tempo, rate, delta);
        }
        // Noten, Controller und die Events eines Audio-Parts bleiben an ihrer Stelle.
        let shift = start as i64 - ev.start.0 as i64;
        crate::audio_parts::shift_children(&mut ev.children, shift, &pool, &tempo);
        self.pool = pool;
        let Some(ev) = self.event_mut(id) else {
            return false;
        };
        for n in &mut ev.notes {
            n.start -= shift;
        }
        for c in &mut ev.controllers {
            c.start -= shift;
        }
        ev.start = Position(start);
        ev.length = end - start;
        ev.fade_in = ev.fade_in.min(ev.length);
        ev.fade_out = ev.fade_out.min(ev.length - ev.fade_in);
        true
    }

    /// Linken Rand verschieben, der Inhalt wandert mit (Objektauswahl, Modus „Inhalt
    /// mitschieben“): Audio beginnt weiter an derselben Clip-Stelle, Noten behalten ihren
    /// Abstand zum Part-Anfang. Das Ende bleibt; bei Audio reicht der Rand höchstens so weit,
    /// wie der Clip ab dieser Stelle Material hat.
    pub fn slide_start(&mut self, id: EventId, new_start: u64) -> bool {
        let Some(ev) = self.event(id) else {
            return false;
        };
        let end = ev.end();
        let mut start = new_start.min(end.saturating_sub(1));
        if let Some((before, total)) = self.clip_bounds(ev) {
            start = start.max(end.saturating_sub(total.saturating_sub(before).max(1)));
        }
        let Some(ev) = self.event_mut(id) else {
            return false;
        };
        ev.start = Position(start);
        ev.length = end - start;
        ev.fade_in = ev.fade_in.min(ev.length);
        ev.fade_out = ev.fade_out.min(ev.length - ev.fade_in);
        true
    }

    /// Rechten Rand verschieben, der Inhalt wandert mit (Modus „Inhalt mitschieben“): Der
    /// Anfang bleibt, Audio und Noten rücken um dieselbe Strecke wie das Ende. Nach rechts
    /// geht es bei Audio nur so weit, wie vor dem Event noch Clip-Material liegt.
    pub fn slide_end(&mut self, id: EventId, new_end: u64) -> bool {
        let Some(ev) = self.event(id) else {
            return false;
        };
        let (start, old_end) = (ev.start.0, ev.end());
        let mut end = new_end.max(start + 1);
        let bounds = self.clip_bounds(ev);
        if let Some((before, _)) = bounds {
            end = end.min(old_end.saturating_add(before));
        }
        let rate = ev
            .clip
            .and_then(|c| self.pool.get(c as usize))
            .map(|c| c.sample_rate);
        let tempo = self.tempo.clone();
        let pool = std::mem::take(&mut self.pool);
        let Some(ev) = self.event_mut(id) else {
            self.pool = pool;
            return false;
        };
        if let Some(rate) = rate {
            let delta = old_end as i64 - end as i64;
            ev.offset = ev.shifted_offset(&tempo, rate, delta);
        }
        let shift = end as i64 - old_end as i64;
        crate::audio_parts::shift_children(&mut ev.children, -shift, &pool, &tempo);
        self.pool = pool;
        let Some(ev) = self.event_mut(id) else {
            return false;
        };
        for n in &mut ev.notes {
            n.start += shift;
        }
        for c in &mut ev.controllers {
            c.start += shift;
        }
        ev.length = end - start;
        ev.fade_in = ev.fade_in.min(ev.length);
        ev.fade_out = ev.fade_out.min(ev.length - ev.fade_in);
        true
    }

    /// Part auf `new_start..new_end` strecken bzw. stauchen, die Noten werden mit skaliert
    /// (Modus „Inhalt strecken“). Nur MIDI-Parts: Für Audio fehlt noch das Time-Stretch,
    /// dann `false` und nichts ändert sich.
    pub fn stretch_to(&mut self, id: EventId, new_start: u64, new_end: u64) -> bool {
        let Some(ev) = self.event_mut(id) else {
            return false;
        };
        if ev.clip.is_some() || ev.is_audio_part() || ev.length == 0 {
            return false;
        }
        let new_end = new_end.max(new_start + 1);
        let new_len = new_end - new_start;
        let f = new_len as f64 / ev.length as f64;
        for n in &mut ev.notes {
            n.start = (n.start as f64 * f).round() as i64;
            n.length = ((n.length as f64 * f).round() as u64).max(1);
        }
        for c in &mut ev.controllers {
            c.start = (c.start as f64 * f).round() as i64;
        }
        ev.fade_in = (ev.fade_in as f64 * f).round() as u64;
        ev.fade_out = (ev.fade_out as f64 * f).round() as u64;
        ev.start = Position(new_start);
        ev.length = new_len;
        ev.fade_in = ev.fade_in.min(ev.length);
        ev.fade_out = ev.fade_out.min(ev.length - ev.fade_in);
        true
    }

    /// Verschiebt das Ende eines Events (Länge ändern am rechten Rand).
    pub fn trim_end(&mut self, id: EventId, new_end: u64) -> bool {
        let Some(ev) = self.event(id) else {
            return false;
        };
        let mut end = new_end.max(ev.start.0 + 1);
        if let Some((before, total)) = self.clip_bounds(ev) {
            // Rest des Clips ab dem Event-Anfang.
            end = end.min(ev.start.0 + total.saturating_sub(before));
        }
        let Some(ev) = self.event_mut(id) else {
            return false;
        };
        ev.length = end - ev.start.0;
        ev.fade_out = ev.fade_out.min(ev.length);
        ev.fade_in = ev.fade_in.min(ev.length - ev.fade_out);
        true
    }

    /// Setzt die Fades; zusammen höchstens so lang wie das Event.
    pub fn set_fades(&mut self, id: EventId, fade_in: u64, fade_out: u64) -> bool {
        let Some(ev) = self.event_mut(id) else {
            return false;
        };
        ev.fade_in = fade_in.min(ev.length);
        ev.fade_out = fade_out.min(ev.length - ev.fade_in);
        true
    }

    pub fn add_track(&mut self, kind: TrackKind, name: impl Into<String>) -> usize {
        let i = self.tracks.len();
        let id = self.new_track_id();
        self.tracks.push(Track {
            id,
            name: name.into(),
            kind,
            color: TRACK_COLORS[i % TRACK_COLORS.len()],
            mute: false,
            solo: false,
            record: false,
            monitor: false,
            read: false,
            write: false,
            time_base: None,
            show_lanes: false,
            automation: Vec::new(),
            eq: crate::ChannelEq::default(),
            parent: None,
            folded: false,
            order: None,
            midi_port: None,
            ruler: RulerFormat::Bars,
            video: None,
            midi_channel: 1,
            program: None,
            solo_defeat: false,
            link_group: None,
            drum_map: None,
            pan_mode: PanMode::Balance,
            mixer_hidden: false,
            mixer_narrow: false,
            input_gain_db: 0.0,
            phase_invert: false,
            pre_hpf: None,
            pre_lpf: None,
            volume_db: 0.0,
            pan: 0.0,
            input: TrackInput::default(),
            input_bus: None,
            out_bus: None,
            extra_outputs: Vec::new(),
            inserts: Vec::new(),
            instrument: None,
            output: None,
            sends: Vec::new(),
            frozen: None,
            events: Vec::new(),
        });
        i
    }

    pub fn add_clip(&mut self, clip: PoolClip) -> ClipId {
        self.pool.push(clip);
        (self.pool.len() - 1) as ClipId
    }

    /// Legt ein Audio-Event an, das den ganzen Clip abspielt.
    pub fn add_audio_event(&mut self, track: usize, start: u64, clip: ClipId) -> Option<EventId> {
        let c = self.pool.get(clip as usize)?;
        if !matches!(
            self.tracks.get(track)?.kind,
            TrackKind::Audio | TrackKind::Video
        ) {
            return None;
        }
        let length = self.tempo.ticks(c.frames, c.sample_rate).max(1);
        let name = c.name.clone();
        let id = self.next_event_id();
        let events = &mut self.tracks[track].events;
        let pos = events.partition_point(|e| e.start.0 <= start);
        events.insert(
            pos,
            Event {
                id,
                start: Position(start),
                length,
                name,
                muted: false,
                clip: Some(clip),
                offset: 0,
                fade_in: 0,
                fade_out: 0,
                notes: Vec::new(),
                controllers: Vec::new(),
                lane: None,
                comp: 0,
                color: None,
                shared: None,
                musical: None,
                stretch_mode: StretchMode::Standard,
                warp: Vec::new(),
                score_quantize: None,
                children: Vec::new(),
                gain_db: 0.0,
                fade_in_curve: FadeCurve::Linear,
                fade_out_curve: FadeCurve::Linear,
            },
        );
        Some(id)
    }

    /// Lässt ein Audio-Event dem Projekttempo folgen (`Some(Tempo der Datei)`) oder wieder 1:1
    /// spielen (`None`). Der hörbare Ausschnitt des Clips bleibt derselbe; die Länge in Ticks
    /// passt sich an. Gibt `false` zurück, wenn es kein Audio-Event ist.
    pub fn set_musical(&mut self, id: EventId, bpm: Option<f64>) -> bool {
        let bpm = match bpm {
            Some(b) if b.is_finite() => Some(b.clamp(crate::BPM_RANGE.0, crate::BPM_RANGE.1)),
            Some(_) => return false,
            None => None,
        };
        let Some(ev) = self.event(id) else {
            return false;
        };
        let Some(c) = ev.clip.and_then(|c| self.pool.get(c as usize)) else {
            return false;
        };
        let rate = c.sample_rate;
        // Hörbarer Ausschnitt des Clips bleibt: seine Länge in Ticks nach der neuen Abbildung.
        let end = ev.shifted_offset(
            &self.tempo,
            rate,
            i64::try_from(ev.length).unwrap_or(i64::MAX),
        );
        let mut changed = ev.clone();
        changed.musical = bpm;
        let length = changed
            .content_span_ticks(&self.tempo, rate, ev.offset, end)
            .max(1);
        let Some(ev) = self.event_mut(id) else {
            return false;
        };
        ev.musical = bpm;
        ev.length = length;
        ev.fade_in = ev.fade_in.min(length);
        ev.fade_out = ev.fade_out.min(length - ev.fade_in);
        true
    }

    /// Rechnet einen Clip auf eine andere Abtastrate um (Länge im Pool und
    /// Offsets der Events), z. B. wenn ein Projekt mit anderer Geräterate geladen wird.
    pub fn rescale_clip(&mut self, clip: ClipId, rate: u32) {
        let Some(c) = self.pool.get_mut(clip as usize) else {
            return;
        };
        if c.sample_rate == rate || c.sample_rate == 0 {
            return;
        }
        let f = f64::from(rate) / f64::from(c.sample_rate);
        c.frames = (c.frames as f64 * f).round() as u64;
        c.sample_rate = rate;
        // Auch die Events in Audio-Parts.
        for ev in self.tracks.iter_mut().flat_map(|t| t.events.iter_mut()) {
            for c in ev.children.iter_mut().filter(|c| c.clip == Some(clip)) {
                c.offset = (c.offset as f64 * f).round() as u64;
            }
            if ev.clip == Some(clip) {
                ev.offset = (ev.offset as f64 * f).round() as u64;
            }
        }
    }

    /// Legt einen Take obenauf (ans Ende der Liste, damit er bei Überlappung
    /// klingt): ein Ausschnitt eines Clips ab `offset` Frames, `length` in Ticks.
    pub fn place_take(
        &mut self,
        track: usize,
        start: u64,
        clip: ClipId,
        offset: u64,
        length: u64,
        name: &str,
    ) -> Option<EventId> {
        if self.tracks.get(track)?.kind != TrackKind::Audio
            || self.pool.get(clip as usize).is_none()
            || length == 0
        {
            return None;
        }
        let id = self.next_event_id();
        self.tracks[track].events.push(Event {
            id,
            start: Position(start),
            length,
            name: name.to_owned(),
            muted: false,
            clip: Some(clip),
            offset,
            fade_in: 0,
            fade_out: 0,
            notes: Vec::new(),
            controllers: Vec::new(),
            lane: None,
            comp: 0,
            color: None,
            shared: None,
            musical: None,
            stretch_mode: StretchMode::Standard,
            warp: Vec::new(),
            score_quantize: None,
            children: Vec::new(),
            gain_db: 0.0,
            fade_in_curve: FadeCurve::Linear,
            fade_out_curve: FadeCurve::Linear,
        });
        Some(id)
    }

    /// Das zeitlich nächste Event auf derselben Spur (beginnt am oder nach dem Start von `id`).
    fn next_on_track(&self, id: EventId) -> Option<EventId> {
        let (t, i) = self.find(id)?;
        let me = &self.tracks[t].events[i];
        self.tracks[t]
            .events
            .iter()
            .filter(|e| e.id != id && (e.start.0, e.id) > (me.start.0, me.id))
            .min_by_key(|e| (e.start.0, e.id))
            .map(|e| e.id)
    }

    /// Kleben (Werkzeug 4): verbindet ein Event mit dem nächsten auf der Spur.
    /// MIDI-Parts werden zu einem Part mit allen Noten (Lücken bleiben leer).
    /// Audio nur, wenn das Material direkt anschließt (gleicher Clip, nahtlos).
    /// Gibt das verbleibende Event zurück.
    pub fn glue(&mut self, id: EventId) -> Result<EventId, &'static str> {
        let next = self
            .next_on_track(id)
            .ok_or("kein folgendes Event auf der Spur")?;
        let (a, b) = (
            self.event(id).cloned().ok_or("Event fehlt")?,
            self.event(next).cloned().ok_or("Event fehlt")?,
        );
        if a.is_audio_part() || b.is_audio_part() {
            return Err("Audio-Parts lassen sich nicht kleben");
        }
        let midi = a.clip.is_none() && b.clip.is_none();
        if !midi {
            let rate = a
                .clip
                .and_then(|c| self.pool.get(c as usize))
                .map(|c| c.sample_rate)
                .ok_or("Clip fehlt")?;
            let seamless = a.clip == b.clip
                && b.start.0 == a.end()
                && a.musical == b.musical
                && a.warp == b.warp
                && b.offset
                    == a.shifted_offset(
                        &self.tempo,
                        rate,
                        i64::try_from(a.length).unwrap_or(i64::MAX),
                    );
            if !seamless {
                return Err("Audio lässt sich nur kleben, wenn das Material direkt anschließt");
            }
        }
        let shift = (b.start.0 - a.start.0) as i64;
        let end = a.end().max(b.end());
        let ev = self.event_mut(id).ok_or("Event fehlt")?;
        ev.length = end - ev.start.0;
        ev.fade_out = b.fade_out;
        ev.notes.extend(b.notes.iter().map(|n| Note {
            start: n.start + shift,
            ..*n
        }));
        ev.controllers
            .extend(b.controllers.iter().map(|c| crate::CtrlPoint {
                start: c.start + shift,
                ..*c
            }));
        ev.controllers.sort_by_key(|c| c.start);
        self.delete(next);
        Ok(id)
    }

    /// Löscht alles im Zeitbereich `t0..t1` auf den Spuren `rows` (Bereichsauswahl
    /// + Entf). Angeschnittene Events werden an den Rändern geteilt.
    pub fn delete_range(&mut self, rows: std::ops::RangeInclusive<usize>, t0: u64, t1: u64) {
        if t1 <= t0 {
            return;
        }
        for t in rows {
            let Some(track) = self.tracks.get(t) else {
                continue;
            };
            let hit: Vec<EventId> = track
                .events
                .iter()
                .filter(|e| e.start.0 < t1 && e.end() > t0)
                .map(|e| e.id)
                .collect();
            for id in hit {
                let mut cur = id;
                if self.event(cur).is_some_and(|e| e.start.0 < t0)
                    && let Some(right) = self.split(cur, t0)
                {
                    cur = right;
                }
                if self.event(cur).is_some_and(|e| e.end() > t1) {
                    self.split(cur, t1);
                }
                self.delete(cur);
            }
        }
    }

    /// Neues Tempo: Audio-Events behalten ihre Zeitposition
    /// (lineare Zeitbasis), MIDI-Parts ihre Taktposition (musikalisch).
    pub fn set_tempo(&mut self, bpm: f64) {
        let bpm = bpm.clamp(20.0, 300.0);
        if (bpm - self.tempo.bpm).abs() < 1e-12 {
            return;
        }
        let mut map = self.tempo.clone();
        map.bpm = bpm;
        self.set_tempo_map(map);
    }

    /// Neue Tempo-Map (Tempospur, festes Tempo, Taktart): Events auf Spuren mit linearer
    /// Zeitbasis behalten ihre Zeit in Sekunden, musikalische ihre Ticks.
    pub fn set_tempo_map(&mut self, new: crate::TempoMap) {
        if new == self.tempo {
            return;
        }
        let old = std::mem::replace(&mut self.tempo, new);
        let new = &self.tempo;
        let conv = |t: u64| {
            let x = new.ticks_at_seconds(old.seconds(Position(t))).round();
            if x.is_finite() && x > 0.0 {
                x as u64
            } else {
                0
            }
        };
        // Negative Notenstarts (ausgeblendete Noten vor dem Part) mit dem Anfangstempo.
        let ratio = new.bpm_at(0) / old.bpm_at(0);
        let conv_i = |t: i64| {
            if t >= 0 {
                i64::try_from(conv(t.unsigned_abs())).unwrap_or(i64::MAX)
            } else {
                (t as f64 * ratio).round() as i64
            }
        };
        for t in self
            .tracks
            .iter_mut()
            .filter(|t| t.time_base() == TimeBase::Linear)
        {
            // Events, die dem Tempo folgen, behalten ihre Takte.
            for e in t.events.iter_mut().filter(|e| e.musical.is_none()) {
                let (s0, e0) = (e.start.0, e.end());
                let (s1, e1) = (conv(s0), conv(e0));
                let fin_end = conv(s0.saturating_add(e.fade_in));
                let fout_start = conv(e0.saturating_sub(e.fade_out));
                e.start = Position(s1);
                e.length = e1.saturating_sub(s1).max(1);
                e.fade_in = fin_end.saturating_sub(s1).min(e.length);
                e.fade_out = e1.saturating_sub(fout_start).min(e.length - e.fade_in);
                // Noten linearer Parts behalten ihre Zeit in Sekunden.
                let base = i64::try_from(s0).unwrap_or(i64::MAX);
                let base1 = i64::try_from(s1).unwrap_or(i64::MAX);
                for n in &mut e.notes {
                    let a = base.saturating_add(n.start);
                    let b = a.saturating_add(i64::try_from(n.length).unwrap_or(i64::MAX));
                    let (a1, b1) = (conv_i(a), conv_i(b));
                    n.start = a1.saturating_sub(base1);
                    n.length = u64::try_from(b1.saturating_sub(a1)).unwrap_or(0).max(1);
                }
                // Events eines Audio-Parts ebenso (Anfang relativ zum Part).
                for c in &mut e.children {
                    let a = s0.saturating_add(c.start.0);
                    let b = a.saturating_add(c.length);
                    let (a1, b1) = (conv(a), conv(b));
                    c.start = Position(a1.saturating_sub(s1));
                    c.length = b1.saturating_sub(a1).max(1);
                    c.fade_in = c.fade_in.min(c.length);
                    c.fade_out = c.fade_out.min(c.length - c.fade_in);
                }
            }
        }
    }

    /// Ob eine Spur zu hören ist: nicht stumm, und bei aktivem Solo selbst Solo.
    /// Gruppen und FX-Kanäle sind solo-sicher (sonst fehlte bei Solo der Hall).
    /// Mute und Solo eines Ordners wirken auf alle Spuren darin.
    pub fn audible(&self, track: usize) -> bool {
        // Lineal-Spuren klingen nicht; ihr Solo (etwa aus einer Datei) zählt nicht.
        let any_solo = self
            .tracks
            .iter()
            .any(|t| t.solo && t.kind != TrackKind::Ruler);
        self.tracks.get(track).is_some_and(|t| {
            !self.effective_mute(track)
                && (!any_solo
                    || self.effective_solo(track)
                    || t.solo_defeat
                    || Self::is_bus(t.kind))
        })
    }

    /// Exklusives Solo (Alt+Klick auf Solo): nur `track` ist solo.
    pub fn solo_exclusive(&mut self, track: usize) {
        for (i, t) in self.tracks.iter_mut().enumerate() {
            t.solo = i == track;
        }
    }

    /// Ende des letzten Events in Ticks (mindestens ein Takt).
    pub fn length(&self) -> u64 {
        self.tracks
            .iter()
            .flat_map(|t| t.events.iter().map(Event::end))
            .max()
            .unwrap_or(0)
            .max(4 * TICKS_PER_QUARTER)
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn extra_outputs_block_loops_and_are_cleaned_on_load() {
        use crate::{MAIN_OUT, MAX_EXTRA_OUTPUTS, OutTarget, Project, TrackKind};
        let mut p = Project::default();
        let src = p.add_track(TrackKind::Audio, "Quelle");
        let a = p.add_track(TrackKind::Group, "A");
        let b = p.add_track(TrackKind::Group, "B");
        let (ia, ib) = (p.tracks[a].id, p.tracks[b].id);
        // A feeds B through an extra output: B may not route back into A.
        p.tracks[a].extra_outputs.push(OutTarget::Group(ib));
        assert!(p.tracks[a].routes_to(ib));
        assert!(!p.can_route(ib, ia));
        assert!(p.can_route(p.tracks[src].id, ia));
        // Loading: unknown busses and groups, doubles, the main output and too many go.
        let me = p.tracks[src].id;
        let t = &mut p.tracks[src];
        t.extra_outputs = vec![
            OutTarget::Bus(MAIN_OUT),
            OutTarget::Bus(999),
            OutTarget::Group(ia),
            OutTarget::Group(ia),
            OutTarget::Group(me),
            OutTarget::Group(4242),
        ];
        assert!(p.sanitize() > 0);
        assert_eq!(p.tracks[src].extra_outputs, vec![OutTarget::Group(ia)]);
        let mut many = Vec::new();
        for k in 0..10 {
            let g = p.add_track(TrackKind::Group, format!("G{k}"));
            many.push(OutTarget::Group(p.tracks[g].id));
        }
        p.tracks[src].extra_outputs = many;
        p.sanitize();
        assert_eq!(p.tracks[src].extra_outputs.len(), MAX_EXTRA_OUTPUTS);
        // Removing an output bus removes it from the extra destinations as well.
        let bus = p.io.add(true, true).unwrap();
        p.tracks[src].extra_outputs = vec![OutTarget::Bus(bus)];
        assert!(p.remove_io_bus(true, bus));
        assert!(p.tracks[src].extra_outputs.is_empty());
    }

    #[test]
    fn crossfade_fades_both_events_over_the_overlap_and_both_sound() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "A");
        let a = p.create_event(t, 0, 2000).expect("a");
        let b = p.create_event(t, 1500, 2000).expect("b");
        // Without a crossfade the later event covers the overlap.
        assert_eq!(
            p.tracks[t].audible_parts(),
            [vec![(0, 1500)], vec![(1500, 3500)]]
        );
        assert!(p.crossfade(b, a));
        let (ea, eb) = (p.event(a).expect("a"), p.event(b).expect("b"));
        assert_eq!((ea.fade_out, eb.fade_in), (500, 500));
        // Now the earlier event plays through its fade-out under the later one's fade-in.
        assert_eq!(
            p.tracks[t].audible_parts(),
            [vec![(0, 2000)], vec![(1500, 3500)]]
        );
        // Contained or separate events are left alone.
        let inner = p.create_event(t, 100, 200).expect("inner");
        assert!(!p.crossfade(a, inner));
        let far = p.create_event(t, 9000, 100).expect("far");
        assert!(!p.crossfade(a, far));
        assert!(!p.crossfade_selection(&[a]), "already crossfaded");
    }

    #[test]
    fn crossfade_pairs_are_found_from_both_sides_and_equal_power_keeps_power() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "A");
        let a = p.create_event(t, 0, 2000).expect("a");
        let b = p.create_event(t, 1500, 2000).expect("b");
        assert_eq!(p.crossfade_pair(b, true), None, "no fades yet");
        assert!(p.crossfade(a, b));
        assert_eq!(p.crossfade_pair(b, true), Some((a, b)));
        assert_eq!(p.crossfade_pair(a, false), Some((a, b)));
        assert_eq!(p.crossfade_pair(a, true), None);
        for i in 0..=10 {
            let x = i as f32 / 10.0;
            let (o, n) = (
                FadeCurve::EqualPower.shape(1.0 - x),
                FadeCurve::EqualPower.shape(x),
            );
            assert!((o * o + n * n - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn events_follow_the_track_colour_unless_they_have_their_own() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "A");
        let a = p.create_event(t, 0, 960).expect("a");
        let b = p.create_event(t, 960, 960).expect("b");
        p.tracks[t].color = [1, 2, 3];
        p.event_mut(b).expect("b").color = Some([9, 9, 9]);
        let track = p.tracks[t].clone();
        assert_eq!(p.event(a).expect("a").color_on(&track), [1, 2, 3]);
        assert_eq!(p.event(b).expect("b").color_on(&track), [9, 9, 9]);
        let json = serde_json::to_string(&p).expect("save");
        assert_eq!(
            json.matches("\"color\":[9,9,9]").count(),
            1,
            "only the own colour is saved"
        );
        let back: Project = serde_json::from_str(&json).expect("load");
        assert_eq!(back.tracks[t].events, p.tracks[t].events);
    }

    #[test]
    fn comping_a_section_makes_it_audible_and_keeps_lanes() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "Gesang");
        let take1 = p.create_event(t, 0, 4000).expect("take 1");
        let take2 = p.create_event(t, 0, 4000).expect("take 2");
        // create_event keeps start order: take2 comes after take1 and sounds on top.
        let parts = p.tracks[t].audible_parts();
        assert_eq!(parts, [vec![], vec![(0, 4000)]]);
        assert_eq!(p.event_at(t, 100), Some(take2));
        // Comp the middle of take 1.
        let mid = p.comp(take1, 1000, 2000).expect("comped");
        let track = &p.tracks[t];
        assert_eq!(track.events.len(), 4);
        let (lanes, n) = track.lanes();
        assert_eq!(n, 2);
        let lane_of = |id: EventId| {
            track
                .events
                .iter()
                .position(|e| e.id == id)
                .map(|i| lanes[i])
        };
        assert_eq!(lane_of(mid), lane_of(take1), "pieces stay in their lane");
        assert_eq!(p.event_at(t, 1500), Some(mid));
        assert_eq!(p.event_at(t, 2500), Some(take2));
        let audible: Vec<(u64, u64)> = track.audible_parts().into_iter().flatten().collect();
        let mut sorted = audible.clone();
        sorted.sort_unstable();
        // The comped section crossfades with take 2 at both edges (comp fades).
        let f = crate::COMP_FADE;
        assert_eq!(sorted, [(0, 1000 + f), (1000, 2000), (2000 - f, 4000)]);
        // Saved projects keep lanes and ranks; older files load with defaults.
        let json = serde_json::to_string(&p).expect("save");
        let back: Project = serde_json::from_str(&json).expect("load");
        assert_eq!(back.tracks[t].events, p.tracks[t].events);
    }

    #[test]
    fn comping_crossfades_at_the_section_edges() {
        use crate::{AudiblePart, COMP_FADE};
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "Gesang");
        let take1 = p.create_event(t, 0, 8000).expect("take 1");
        p.create_event(t, 0, 8000).expect("take 2");
        let mid = p.comp(take1, 2000, 4000).expect("comped");
        let ev = p.event(mid).unwrap();
        assert_eq!((ev.fade_in, ev.fade_out), (COMP_FADE, COMP_FADE));
        let spans = p.tracks[t].audible_spans();
        let all: Vec<AudiblePart> = spans.into_iter().flatten().collect();
        // The comped section sounds whole with its fades; take 2 fades out while it fades
        // in and fades in while it fades out.
        assert!(all.contains(&AudiblePart {
            from: 2000,
            to: 4000,
            ramp_in: 0,
            ramp_out: 0
        }));
        assert!(all.contains(&AudiblePart {
            from: 0,
            to: 2000 + COMP_FADE,
            ramp_in: 0,
            ramp_out: COMP_FADE
        }));
        assert!(all.contains(&AudiblePart {
            from: 4000 - COMP_FADE,
            to: 8000,
            ramp_in: COMP_FADE,
            ramp_out: 0
        }));
        // Comping a whole take (no cut) adds no fades.
        let take3 = p.create_event(t, 10_000, 1000).expect("take 3");
        let whole = p.comp(take3, 10_000, 11_000).expect("whole");
        let ev = p.event(whole).unwrap();
        assert_eq!((ev.fade_in, ev.fade_out), (0, 0));
    }

    #[test]
    fn time_base_decides_what_follows_a_tempo_change() {
        let mut p = Project::default();
        let a = p.add_track(TrackKind::Audio, "A");
        let m = p.add_track(TrackKind::Instrument, "I");
        assert_eq!(p.tracks[a].time_base(), TimeBase::Linear);
        assert_eq!(p.tracks[m].time_base(), TimeBase::Musical);
        let ea = p.create_event(a, 960, 960).expect("audio");
        let em = p.create_event(m, 960, 960).expect("part");
        p.event_mut(em).expect("part").notes.push(Note {
            pitch: 60,
            velocity: 100,
            start: 480,
            length: 240,
            spelling: 0,
        });
        // Swap: audio musical, instrument linear.
        p.tracks[a].time_base = Some(TimeBase::Musical);
        p.tracks[m].time_base = Some(TimeBase::Linear);
        p.set_tempo(240.0);
        assert_eq!(p.event(ea).expect("audio").start.0, 960);
        let part = p.event(em).expect("part");
        assert_eq!((part.start.0, part.length), (1920, 1920));
        assert_eq!((part.notes[0].start, part.notes[0].length), (960, 480));
    }

    #[test]
    fn overlapping_takes_get_their_own_lanes() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "A");
        p.create_event(t, 0, 960);
        p.create_event(t, 960, 960);
        p.create_event(t, 480, 960);
        p.create_event(t, 600, 100);
        let (lanes, n) = p.tracks[t].lanes();
        // create_event keeps the list sorted by start.
        let starts: Vec<u64> = p.tracks[t].events.iter().map(|e| e.start.0).collect();
        assert_eq!(starts, [0, 480, 600, 960]);
        assert_eq!(lanes, [0, 1, 2, 0]);
        assert_eq!(n, 3);
    }

    #[test]
    fn shuffle_swaps_neighbours_without_gaps() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "A");
        let a = p.create_event(t, 0, 960).expect("a");
        let b = p.create_event(t, 1920, 480).expect("b");
        let c = p.create_event(t, 2400, 960).expect("c");
        let far = p.create_event(t, 9600, 960).expect("far");
        // Drop A behind B: B moves to 0, A follows directly.
        assert!(p.shuffle_event(a, 2000));
        let order: Vec<(EventId, u64)> = p.tracks[t]
            .events
            .iter()
            .map(|e| (e.id, e.start.0))
            .collect();
        assert_eq!(order, [(b, 0), (a, 480), (c, 2400), (far, 9600)]);
        // Dropping in place changes nothing.
        assert!(!p.shuffle_event(c, 2400));
        assert!(!p.shuffle_event(12345, 0));
    }

    use super::*;

    fn one_event() -> (Project, EventId) {
        let mut p = Project::demo(1);
        p.tracks[0].events.clear();
        let id = p.create_event(0, 0, 4 * TICKS_PER_QUARTER).unwrap();
        (p, id)
    }

    #[test]
    fn demo_has_events_on_event_tracks_only() {
        let p = Project::demo(14);
        for t in &p.tracks {
            assert_eq!(t.kind.holds_events(), !t.events.is_empty(), "{}", t.name);
        }
    }

    fn part_with_notes() -> (Project, EventId) {
        let (mut p, id) = one_event();
        let ev = p.event_mut(id).unwrap();
        ev.notes = vec![
            Note {
                pitch: 60,
                velocity: 100,
                start: 0,
                length: TICKS_PER_QUARTER,
                spelling: 0,
            },
            Note {
                pitch: 62,
                velocity: 100,
                start: 2 * TICKS_PER_QUARTER as i64,
                length: TICKS_PER_QUARTER,
                spelling: 0,
            },
        ];
        (p, id)
    }

    #[test]
    fn slide_moves_notes_with_the_edge() {
        let q = TICKS_PER_QUARTER;
        let (mut p, id) = part_with_notes();
        p.event_mut(id).unwrap().start = Position(4 * q);
        // Left edge one quarter earlier: notes keep their distance to the new start.
        assert!(p.slide_start(id, 3 * q));
        let e = p.event(id).unwrap();
        assert_eq!((e.start.0, e.length), (3 * q, 5 * q));
        assert_eq!(e.notes[0].start, 0);
        // Right edge one quarter later: notes move right by a quarter.
        assert!(p.slide_end(id, 9 * q));
        let e = p.event(id).unwrap();
        assert_eq!((e.start.0, e.length), (3 * q, 6 * q));
        assert_eq!(e.notes[0].start, q as i64);
        assert_eq!(e.notes[1].start, 3 * q as i64);
    }

    #[test]
    fn stretch_scales_notes_and_refuses_audio() {
        let q = TICKS_PER_QUARTER;
        let (mut p, id) = part_with_notes();
        assert!(p.stretch_to(id, 0, 8 * q));
        let e = p.event(id).unwrap();
        assert_eq!(e.length, 8 * q);
        assert_eq!(e.notes[1].start, 4 * q as i64);
        assert_eq!(e.notes[1].length, 2 * q);
        assert!(p.stretch_to(id, 0, 2 * q));
        assert_eq!(p.event(id).unwrap().notes[0].length, q / 2);
    }

    #[test]
    fn slide_on_audio_keeps_clip_position_and_limits() {
        let q = TICKS_PER_QUARTER;
        let (mut p, c) = pool_project();
        let id = p.add_audio_event(0, 0, c).unwrap();
        let total = p.event(id).unwrap().length;
        // Start the event one quarter into the clip.
        p.trim_start(id, q);
        assert_eq!(p.event(id).unwrap().offset, 24_000);
        // Sliding the end right by more than the quarter of material before the event stops
        // at that quarter; the offset reaches zero.
        assert!(p.slide_end(id, total + 4 * q));
        let e = p.event(id).unwrap();
        assert_eq!(e.offset, 0);
        assert_eq!(e.end(), total + q);
        // Stretching audio is not possible yet.
        assert!(!p.stretch_to(id, 0, 2 * total));
    }

    #[test]
    fn controllers_follow_split_trim_stretch_and_glue() {
        use crate::{CtrlKind, CtrlPoint};
        let q = TICKS_PER_QUARTER;
        let (mut p, id) = one_event();
        let at = |start: u64, value| CtrlPoint {
            kind: CtrlKind::Cc(1),
            start: start as i64,
            value,
        };
        p.event_mut(id).unwrap().controllers = vec![at(0, 10), at(q, 20), at(3 * q, 30)];
        // Split at two quarters: the third point moves to the right part.
        let right = p.split(id, 2 * q).unwrap();
        assert_eq!(p.event(id).unwrap().controllers.len(), 2);
        assert_eq!(p.event(right).unwrap().controllers, vec![at(q, 30)]);
        // Glue brings it back to its old place.
        p.glue(id).unwrap();
        assert_eq!(p.event(id).unwrap().controllers[2], at(3 * q, 30));
        // Trimming the start keeps the points at their project position.
        p.trim_start(id, q);
        assert_eq!(p.event(id).unwrap().controllers[1].start, 0);
        // Stretching a part scales the points.
        let (mut p, id) = one_event();
        p.event_mut(id).unwrap().controllers = vec![at(q, 1)];
        p.stretch_to(id, 0, 8 * q);
        assert_eq!(p.event(id).unwrap().controllers[0].start, 2 * q as i64);
    }

    #[test]
    fn hostile_controller_values_are_clamped() {
        use crate::{CtrlKind, CtrlPoint};
        let (mut p, id) = one_event();
        p.event_mut(id).unwrap().controllers = vec![
            CtrlPoint {
                kind: CtrlKind::Cc(200),
                start: i64::MIN,
                value: 999,
            },
            CtrlPoint {
                kind: CtrlKind::PitchBend,
                start: 0,
                value: u16::MAX,
            },
        ];
        assert!(p.sanitize() > 0);
        let c = &p.event(id).unwrap().controllers;
        assert_eq!(c[0].kind, CtrlKind::Cc(127));
        assert_eq!(c[0].value, 127);
        assert_eq!(c[1].value, 16_383);
        // Unknown in older files: a part without controllers still loads.
        let json = crate::file::to_json(&p);
        assert!(crate::file::from_json(&json).is_ok());
    }

    #[test]
    fn channel_strip_input_values_load_and_are_sanitized() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "a");
        assert_eq!(p.pre_fader_inserts, 6);
        p.tracks[t].input_gain_db = f32::NAN;
        p.tracks[t].pre_hpf = Some(5.0);
        p.tracks[t].pre_lpf = Some(f32::INFINITY);
        p.pre_fader_inserts = 30;
        p.sanitize();
        let tr = &p.tracks[t];
        assert_eq!(
            (tr.input_gain_db, tr.pre_hpf, tr.pre_lpf),
            (0.0, Some(20.0), None)
        );
        assert_eq!(p.pre_fader_inserts, 8);
        // Defaults are not written, older files load with 6 pre-fader slots.
        let json = crate::file::to_json(&Project::default());
        assert!(!json.contains("pre_fader_inserts") && !json.contains("input_gain_db"));
        assert_eq!(crate::file::from_json(&json).unwrap().pre_fader_inserts, 6);
    }

    #[test]
    fn mixer_views_save_and_apply() {
        let mut p = Project::default();
        for n in ["a", "b", "c"] {
            p.add_track(TrackKind::Audio, n);
        }
        p.tracks[1].mixer_hidden = true;
        p.tracks[2].mixer_narrow = true;
        assert!(p.save_mixer_view("Mischen"));
        p.tracks.iter_mut().for_each(|t| {
            t.mixer_hidden = false;
            t.mixer_narrow = false;
        });
        assert!(p.apply_mixer_view(0));
        assert!(p.tracks[1].mixer_hidden && p.tracks[2].mixer_narrow && !p.tracks[0].mixer_hidden);
        assert!(!p.apply_mixer_view(5));
        assert!(!p.save_mixer_view("   "));
        for k in 0..20 {
            p.save_mixer_view(&format!("v{k}"));
        }
        assert_eq!(p.mixer_views.len(), MAX_MIXER_VIEWS);
    }

    #[test]
    fn pan_matrix_modes() {
        let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5);
        for mode in [PanMode::Balance, PanMode::Combined] {
            assert!(
                near(pan_matrix(mode, 0.0), [1.0, 0.0, 0.0, 1.0]),
                "{mode:?} centre"
            );
            assert!(near(pan_matrix(mode, f32::NAN), [1.0, 0.0, 0.0, 1.0]));
        }
        // Balance hard right: the left channel is gone, right stays.
        assert!(near(
            pan_matrix(PanMode::Balance, 1.0),
            [0.0, 0.0, 0.0, 1.0]
        ));
        // Combined hard right: both channels go fully right.
        assert!(near(
            pan_matrix(PanMode::Combined, 1.0),
            [0.0, 1.0, 0.0, 1.0]
        ));
        // Combined half right: left channel in the middle (equal power), right stays right.
        let h = std::f32::consts::FRAC_1_SQRT_2;
        assert!(near(pan_matrix(PanMode::Combined, 0.5), [h, h, 0.0, 1.0]));
    }

    #[test]
    fn solo_defeat_and_exclusive_solo() {
        let mut p = Project::default();
        for n in ["a", "b", "c"] {
            p.add_track(TrackKind::Audio, n);
        }
        p.tracks[0].solo = true;
        p.tracks[2].solo_defeat = true;
        assert!(
            p.audible(0) && !p.audible(1) && p.audible(2),
            "defeat stays audible"
        );
        p.tracks[2].mute = true;
        assert!(!p.audible(2), "mute still wins");
        p.tracks[1].solo = true;
        p.solo_exclusive(1);
        assert!(!p.tracks[0].solo && p.tracks[1].solo && !p.tracks[2].solo);
    }

    #[test]
    fn split_keeps_total_length() {
        let (mut p, id) = one_event();
        let right = p.split(id, TICKS_PER_QUARTER).unwrap();
        assert_eq!(p.event(id).unwrap().length, TICKS_PER_QUARTER);
        assert_eq!(p.event(right).unwrap().start, Position(TICKS_PER_QUARTER));
        assert_eq!(p.event(right).unwrap().length, 3 * TICKS_PER_QUARTER);
        assert_eq!(p.split(id, 0), None, "am Rand wird nicht geteilt");
    }

    #[test]
    fn split_equal_cuts_the_whole_event_into_pieces() {
        let (mut p, id) = one_event();
        // Four quarters, cut at one quarter: four pieces of one quarter each.
        let ids = p.split_equal(id, TICKS_PER_QUARTER);
        assert_eq!(ids.len(), 4);
        for (k, id) in ids.iter().enumerate() {
            let e = p.event(*id).unwrap();
            assert_eq!(e.start, Position(k as u64 * TICKS_PER_QUARTER));
            assert_eq!(e.length, TICKS_PER_QUARTER);
        }
        // A remainder stays as a shorter last piece.
        let (mut p, id) = one_event();
        let ids = p.split_equal(id, 3 * TICKS_PER_QUARTER / 2);
        assert_eq!(ids.len(), 3);
        assert_eq!(p.event(ids[2]).unwrap().length, TICKS_PER_QUARTER);
        // Outside the event nothing happens.
        let (mut p, id) = one_event();
        assert!(p.split_equal(id, 0).is_empty());
        assert!(p.split_equal(id, 10 * TICKS_PER_QUARTER).is_empty());
    }

    #[test]
    fn split_equal_is_capped() {
        let (mut p, id) = one_event();
        let ids = p.split_equal(id, 1);
        assert_eq!(ids.len() as u64, Project::MAX_EQUAL_PIECES);
        let total: u64 = ids.iter().map(|i| p.event(*i).unwrap().length).sum();
        assert_eq!(total, 4 * TICKS_PER_QUARTER, "no time is lost");
    }

    #[test]
    fn move_and_copy() {
        let mut p = Project::demo(2);
        p.tracks[1].kind = TrackKind::Audio;
        p.tracks.iter_mut().for_each(|t| t.events.clear());
        let id = p.create_event(0, 0, 960).unwrap();
        let copy = p.move_event(id, 1, 1920, true).unwrap();
        assert_ne!(copy, id);
        assert_eq!(p.tracks[0].events.len(), 1);
        assert_eq!(p.find(copy), Some((1, 0)));
        let moved = p.move_event(id, 1, 0, false).unwrap();
        assert_eq!(moved, id);
        assert!(p.tracks[0].events.is_empty());
        assert_eq!(p.tracks[1].events[0].id, id, "nach Start sortiert");
    }

    fn pool_project() -> (Project, ClipId) {
        let mut p = Project::default();
        p.add_track(TrackKind::Audio, "Gesang");
        // 2 s bei 48 kHz = 1 Takt bei 120 BPM
        let c = p.add_clip(PoolClip {
            name: "take".into(),
            path: None,
            sample_rate: 48_000,
            frames: 96_000,
        });
        (p, c)
    }

    #[test]
    fn audio_event_spans_clip() {
        let (mut p, c) = pool_project();
        let id = p.add_audio_event(0, 0, c).unwrap();
        assert_eq!(p.event(id).unwrap().length, p.tempo.signature.bar_ticks());
        assert_eq!(p.event(id).unwrap().clip, Some(c));
    }

    #[test]
    fn musical_audio_keeps_its_content_bars_and_split_offsets() {
        let mut p = Project::default();
        p.add_track(TrackKind::Audio, "A");
        let c = p.add_clip(PoolClip {
            name: "loop".into(),
            path: None,
            sample_rate: 48_000,
            frames: 48_000,
        });
        let id = p.add_audio_event(0, 0, c).unwrap();
        assert_eq!(p.event(id).unwrap().length, 1920, "1 s bei 120 BPM");
        // Datei in 90 BPM: die Sekunde sind anderthalb Viertel.
        assert!(p.set_musical(id, Some(90.0)));
        assert_eq!(p.event(id).unwrap().length, 1440);
        // Ungültige Werte und MIDI-Parts werden abgelehnt.
        assert!(!p.set_musical(id, Some(f64::NAN)));
        let midi = p.add_track(TrackKind::Midi, "M");
        let part = p.create_event(midi, 0, 960).unwrap();
        assert!(!p.set_musical(part, Some(100.0)));
        // Tempowechsel: das Event behält Anfang und Länge in Ticks.
        p.add_audio_event(0, 3840, c).unwrap();
        let mut tempo = p.tempo.clone();
        tempo.bpm = 60.0;
        p.set_tempo_map(tempo);
        let ev = p.event(id).unwrap();
        assert_eq!((ev.start.0, ev.length), (0, 1440));
        let plain = &p.tracks[0].events[1];
        assert_eq!(plain.start.0, 1920, "lineares Audio behält seine Sekunden");
        // Teilen nach einer Viertel: rechts beginnt der Clip 2/3 s später (Tempo der Datei).
        let right = p.split(id, 960).unwrap();
        assert_eq!(p.event(right).unwrap().offset, 32_000);
        // Zurück auf 1:1: der Ausschnitt (16 000 Frames) dauert jetzt bei 60 BPM 1/3 s.
        assert!(p.set_musical(right, None));
        assert_eq!(p.event(right).unwrap().length, 320);
    }

    #[test]
    fn tempo_from_file_names() {
        assert_eq!(bpm_in_name("Beat 92bpm"), Some(92.0));
        assert_eq!(bpm_in_name("loop_120_BPM.wav"), Some(120.0));
        assert_eq!(bpm_in_name("Pad 87.5 bpm"), Some(87.5));
        assert_eq!(bpm_in_name("bpm 120"), None);
        assert_eq!(bpm_in_name("Take 3bpm"), None, "zu langsam");
        assert_eq!(bpm_in_name("x9999bpm 100bpm"), Some(100.0));
        assert_eq!(bpm_in_name("Gesang"), None);
    }

    #[test]
    fn ruler_formats_label_times() {
        let t = 3723.5; // 1 h 2 min 3,5 s
        assert_eq!(RulerFormat::Seconds.format_seconds(t, 48_000), "62:03.500");
        assert_eq!(
            RulerFormat::Timecode25.format_seconds(t, 48_000),
            "01:02:03:12"
        );
        assert_eq!(
            RulerFormat::Timecode30.format_seconds(t, 48_000),
            "01:02:03:15"
        );
        assert_eq!(RulerFormat::Samples.format_seconds(1.5, 48_000), "72000");
        assert_eq!(
            RulerFormat::Seconds.format_seconds(f64::NAN, 48_000),
            "0:00.000"
        );
        // Älteres Projekt ohne Feld: Takte.
        let mut p = Project::default();
        let i = p.add_track(TrackKind::Ruler, "Lineal");
        p.tracks[i].ruler = RulerFormat::Timecode25;
        let json = serde_json::to_string(&p).unwrap();
        let back: Project = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tracks[i].ruler, RulerFormat::Timecode25);
        assert!(!TrackKind::Ruler.holds_events());
        // Ein Solo auf dem Lineal schaltet nichts stumm.
        let a = p.add_track(TrackKind::Audio, "A");
        p.tracks[i].solo = true;
        assert!(p.audible(a));
    }

    #[test]
    fn sanitize_drops_bad_file_tempos() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Midi, "M");
        let id = p.create_event(t, 0, 960).unwrap();
        p.event_mut(id).unwrap().musical = Some(120.0);
        let a = p.add_track(TrackKind::Audio, "A");
        let c = p.add_clip(PoolClip {
            name: "c".into(),
            path: None,
            sample_rate: 48_000,
            frames: 4800,
        });
        let au = p.add_audio_event(a, 0, c).unwrap();
        p.event_mut(au).unwrap().musical = Some(5000.0);
        p.sanitize();
        assert_eq!(p.event(id).unwrap().musical, None, "nur Audio-Events");
        assert_eq!(p.event(au).unwrap().musical, Some(999.0));
    }

    #[test]
    fn split_moves_offset_of_right_part() {
        let (mut p, c) = pool_project();
        let id = p.add_audio_event(0, 0, c).unwrap();
        let right = p.split(id, TICKS_PER_QUARTER).unwrap();
        // Eine Viertel bei 120 BPM = 0,5 s = 24 000 Frames.
        assert_eq!(p.event(right).unwrap().offset, 24_000);
        assert_eq!(p.event(id).unwrap().offset, 0);
    }

    #[test]
    fn solo_and_mute_decide_audibility() {
        let mut p = Project::default();
        for n in ["a", "b", "c"] {
            p.add_track(TrackKind::Audio, n);
        }
        assert!(p.audible(0) && p.audible(1));
        p.tracks[1].mute = true;
        assert!(!p.audible(1));
        p.tracks[2].solo = true;
        assert!(!p.audible(0) && !p.audible(1) && p.audible(2));
    }

    #[test]
    fn fader_and_balance_gains() {
        let mut p = Project::default();
        p.add_track(TrackKind::Audio, "a");
        let t = &mut p.tracks[0];
        assert_eq!(t.gains(), (1.0, 1.0));
        t.volume_db = -6.0;
        let (l, r) = t.gains();
        assert!((l - 0.501).abs() < 0.001 && l == r);
        t.volume_db = 0.0;
        t.pan = -0.5;
        assert_eq!(t.gains(), (1.0, 0.5), "links: rechter Kanal leiser");
        t.volume_db = MIN_DB;
        assert_eq!(t.gains(), (0.0, 0.0));
    }

    #[test]
    fn trimming_keeps_audio_in_place_and_respects_clip() {
        let (mut p, c) = pool_project(); // Clip = 1 Takt
        let bar = p.tempo.signature.bar_ticks();
        let q = TICKS_PER_QUARTER;
        let id = p.add_audio_event(0, bar, c).unwrap();
        // Anfang eine Viertel nach rechts: Offset wächst um 0,5 s.
        assert!(p.trim_start(id, bar + q));
        let ev = p.event(id).unwrap();
        assert_eq!(
            (ev.start.0, ev.length, ev.offset),
            (bar + q, bar - q, 24_000)
        );
        // Zurück nach links geht nur bis zum Clip-Anfang.
        p.trim_start(id, 0);
        let ev = p.event(id).unwrap();
        assert_eq!((ev.start.0, ev.offset), (bar, 0));
        // Ende nicht über das Clip-Ende hinaus.
        p.trim_end(id, 10 * bar);
        assert_eq!(p.event(id).unwrap().end(), 2 * bar);
        p.trim_end(id, bar + q);
        assert_eq!(p.event(id).unwrap().length, q);
    }

    #[test]
    fn fades_are_clamped_and_split_correctly() {
        let (mut p, c) = pool_project();
        let bar = p.tempo.signature.bar_ticks();
        let id = p.add_audio_event(0, 0, c).unwrap();
        p.set_fades(id, bar, bar);
        let ev = p.event(id).unwrap();
        assert_eq!(
            (ev.fade_in, ev.fade_out),
            (bar, 0),
            "zusammen höchstens Event-Länge"
        );
        p.set_fades(id, 960, 960);
        let right = p.split(id, bar / 2).unwrap();
        assert_eq!(
            (p.event(id).unwrap().fade_in, p.event(id).unwrap().fade_out),
            (960, 0)
        );
        assert_eq!(
            (
                p.event(right).unwrap().fade_in,
                p.event(right).unwrap().fade_out
            ),
            (0, 960)
        );
    }

    #[test]
    fn input_options_for_two_channels() {
        let o = TrackInput::options(2);
        let labels: Vec<String> = o.iter().map(|i| i.label()).collect();
        assert_eq!(labels, ["Eingang 1/2", "Eingang 1", "Eingang 2"]);
    }

    #[test]
    fn notes_follow_split_and_trim() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Instrument, "Synth");
        let q = TICKS_PER_QUARTER;
        let id = p.create_event(t, 4 * q, 4 * q).unwrap();
        let ev = p.event_mut(id).unwrap();
        for (i, pitch) in [60u8, 62, 64, 65].into_iter().enumerate() {
            ev.notes.push(Note {
                pitch,
                velocity: 100,
                start: (i as u64 * q) as i64,
                length: q,
                spelling: 0,
            });
        }
        // Anfang eine Viertel nach rechts: die Noten bleiben an ihrem Platz.
        p.trim_start(id, 5 * q);
        let ev = p.event(id).unwrap();
        assert_eq!(
            ev.notes[0].start,
            -(q as i64),
            "erste Note ist jetzt ausgeblendet"
        );
        assert_eq!(ev.notes[1].start, 0);
        // Teilen bei Viertel 7: E und F gehen in den rechten Part.
        let right = p.split(id, 7 * q).unwrap();
        let pitches = |id| {
            p.event(id)
                .unwrap()
                .notes
                .iter()
                .map(|n| n.pitch)
                .collect::<Vec<_>>()
        };
        assert_eq!(pitches(id), [60, 62, 64]);
        assert_eq!(pitches(right), [65]);
        assert_eq!(p.event(right).unwrap().notes[0].start, 0);
    }

    #[test]
    fn tempo_change_keeps_audio_time_and_midi_bars() {
        let (mut p, c) = pool_project(); // 2 s Audio = 1 Takt bei 120 BPM
        let bar = p.tempo.signature.bar_ticks();
        let a = p.add_audio_event(0, bar, c).unwrap(); // bei 2 s
        let m = p.add_track(TrackKind::Instrument, "Synth");
        let part = p.create_event(m, bar, bar).unwrap();
        let secs = |p: &Project, id| {
            let e = p.event(id).unwrap();
            (
                p.tempo.seconds(e.start),
                p.tempo.seconds(Position(e.length)),
            )
        };
        assert_eq!(secs(&p, a), (2.0, 2.0));
        p.set_tempo(60.0);
        assert_eq!(secs(&p, a), (2.0, 2.0), "Audio bleibt bei 2 s und 2 s lang");
        assert_eq!(
            p.event(part).unwrap().start.0,
            bar,
            "MIDI-Part bleibt bei Takt 2"
        );
        assert_eq!(secs(&p, part), (4.0, 4.0), "… und dauert nun 4 s");
    }

    #[test]
    fn a_tempo_track_keeps_audio_in_seconds_and_midi_in_bars() {
        let (mut p, c) = pool_project(); // 2 s audio = one bar at 120 BPM
        let bar = p.tempo.signature.bar_ticks();
        let a = p.add_audio_event(0, 2 * bar, c).unwrap(); // at 4 s
        p.set_fades(a, bar / 4, bar / 2);
        let m = p.add_track(TrackKind::Instrument, "Synth");
        let part = p.create_event(m, 3 * bar, bar).unwrap();
        let span = |p: &Project, id| {
            let e = p.event(id).unwrap();
            (p.tempo.seconds(e.start), p.tempo.seconds(Position(e.end())))
        };
        let fade_secs = |p: &Project| {
            let e = p.event(a).unwrap();
            (
                p.tempo.seconds(Position(e.start.0 + e.fade_in)) - p.tempo.seconds(e.start),
                p.tempo.seconds(Position(e.end()))
                    - p.tempo.seconds(Position(e.end() - e.fade_out)),
            )
        };
        let (audio, fades) = (span(&p, a), fade_secs(&p));
        // From bar 2 on, ramp from 120 up to 180 at bar 3, then jump to 90.
        let mut map = p.tempo.clone();
        map.use_track = true;
        map.set_event(bar, 120.0, false);
        map.set_event(2 * bar, 180.0, true);
        map.set_event(3 * bar, 90.0, false);
        p.set_tempo_map(map);
        let close =
            |x: (f64, f64), y: (f64, f64)| (x.0 - y.0).abs() < 1e-3 && (x.1 - y.1).abs() < 1e-3;
        assert!(close(span(&p, a), audio), "{:?} vs {audio:?}", span(&p, a));
        assert!(close(fade_secs(&p), fades), "fades keep their seconds");
        assert_eq!(
            p.event(part).unwrap().start.0,
            3 * bar,
            "MIDI stays in bar 4"
        );
        assert_eq!(p.event(part).unwrap().length, bar);
        // Switching the track off goes back to the fixed tempo, audio still in place.
        let mut off = p.tempo.clone();
        off.use_track = false;
        p.set_tempo_map(off);
        assert!(close(span(&p, a), audio));
        assert_eq!(p.tempo.events.len(), 3, "points are kept");
    }

    #[test]
    fn track_ids_and_loop_free_routing() {
        let mut p = Project::default();
        let a = p.add_track(TrackKind::Audio, "Gesang");
        let fx = p.add_track(TrackKind::Fx, "Hall");
        let g = p.add_track(TrackKind::Group, "Gruppe");
        let (ia, ifx, ig) = (p.tracks[a].id, p.tracks[fx].id, p.tracks[g].id);
        assert!(ia != ifx && ifx != ig && ia > 0);
        assert!(p.can_route(ia, ifx));
        p.tracks[fx].output = Some(ig);
        assert!(
            !p.can_route(ig, ifx),
            "Gruppe → Hall → Gruppe wäre eine Schleife"
        );
        assert!(!p.can_route(ig, ig));
        // Ältere Dateien ohne Kennung bekommen eine.
        p.tracks[a].id = 0;
        p.tracks[g].id = ifx;
        p.ensure_track_ids();
        let ids: std::collections::HashSet<_> = p.tracks.iter().map(|t| t.id).collect();
        assert_eq!(ids.len(), 3);
        assert!(!ids.contains(&0));
    }

    #[test]
    fn buses_stay_audible_under_solo() {
        let mut p = Project::default();
        p.add_track(TrackKind::Audio, "a");
        p.add_track(TrackKind::Audio, "b");
        p.add_track(TrackKind::Fx, "Hall");
        p.tracks[0].solo = true;
        assert!(p.audible(0) && !p.audible(1) && p.audible(2));
    }

    #[test]
    fn delete_range_cuts_out_the_middle() {
        let q = TICKS_PER_QUARTER;
        let (mut p, c) = pool_project(); // Clip = 1 Takt = 4 Viertel
        let id = p.add_audio_event(0, 0, c).unwrap();
        p.delete_range(0..=0, q, 2 * q);
        let spans: Vec<(u64, u64, u64)> = p.tracks[0]
            .events
            .iter()
            .map(|e| (e.start.0, e.length, e.offset))
            .collect();
        assert_eq!(
            spans,
            [(0, q, 0), (2 * q, 2 * q, 48_000)],
            "Mitte weg, rechter Teil passt zum Material"
        );
        assert_eq!(p.tracks[0].events[0].id, id);
        // Ganz umschlossen: verschwindet.
        p.delete_range(0..=0, 0, 10 * q);
        assert!(p.tracks[0].events.is_empty());
    }

    #[test]
    fn glue_parts_and_seamless_audio() {
        let q = TICKS_PER_QUARTER;
        // MIDI: zwei Parts mit Lücke werden einer, Noten an ihrer Stelle.
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Instrument, "Synth");
        let a = p.create_event(t, 0, 2 * q).unwrap();
        let b = p.create_event(t, 4 * q, 2 * q).unwrap();
        p.event_mut(a).unwrap().notes.push(Note {
            pitch: 60,
            velocity: 100,
            start: 0,
            length: q,
            spelling: 0,
        });
        p.event_mut(b).unwrap().notes.push(Note {
            pitch: 64,
            velocity: 100,
            start: 0,
            length: q,
            spelling: 0,
        });
        assert_eq!(p.glue(a), Ok(a));
        let ev = p.event(a).unwrap();
        assert_eq!(ev.length, 6 * q);
        assert_eq!(
            ev.notes
                .iter()
                .map(|n| (n.pitch, n.start))
                .collect::<Vec<_>>(),
            [(60, 0), (64, 4 * q as i64)]
        );
        assert!(p.event(b).is_none());
        assert!(p.glue(a).is_err(), "nichts mehr dahinter");
        // Audio: Teilen und wieder kleben ergibt das Original.
        let (mut p, c) = pool_project();
        let id = p.add_audio_event(0, 0, c).unwrap();
        let original = p.event(id).unwrap().clone();
        p.split(id, q);
        assert_eq!(p.glue(id), Ok(id));
        assert_eq!(p.event(id).unwrap(), &original);
        // Audio mit Lücke: nicht klebbar.
        let r = p.split(id, q).unwrap();
        p.move_event(r, 0, 3 * q, false);
        assert!(p.glue(id).is_err());
    }

    #[test]
    fn no_events_on_group_tracks() {
        let mut p = Project::demo(7);
        assert_eq!(p.tracks[6].kind, TrackKind::Group);
        assert_eq!(p.create_event(6, 0, 960), None);
    }
}
