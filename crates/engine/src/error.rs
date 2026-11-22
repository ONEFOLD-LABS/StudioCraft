//! Fehler der Engine (Issue #265): was schiefging, mit den Werten dazu. Die Oberfläche
//! übersetzt sie (`crates/ui`, `engine_error_text`); [`std::fmt::Display`] gibt den deutschen
//! Text als Rückfall (Protokolle, Kommandozeile, MCP-Antworten). Meldungen des Systems oder
//! eines Treibers stehen unverändert in den `String`-Feldern.

use std::fmt;
use std::path::PathBuf;

/// Ein Fehler der Engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineError {
    /// Einen Audio-Host aus den Einstellungen gibt es in diesem Build nicht.
    HostMissing {
        name: String,
        available: Vec<String>,
    },
    /// Das Audiosystem lehnt den Host ab.
    Host { name: String, detail: String },
    /// Es gibt kein Standardgerät (Eingang oder Ausgang).
    NoDevice { input: bool },
    /// Die Geräteliste ließ sich nicht lesen.
    DeviceList { input: bool, detail: String },
    /// Ein Gerät aus den Einstellungen gibt es nicht (mehr).
    DeviceNotFound { input: bool, name: String },
    /// Das Gerät lehnt ab (Einstellungen lesen, öffnen).
    Device {
        input: bool,
        name: String,
        detail: String,
    },
    /// Der Eingang unterstützt die Abtastrate des Projekts nicht.
    RateUnsupported { name: String, rate: u32 },
    /// Ein Sampleformat, das StudioCraft nicht verarbeitet.
    SampleFormat(String),
    /// Der Audio-Stream ließ sich nicht aufbauen.
    StreamBuild { input: bool, detail: String },
    /// Der Audio-Stream ließ sich nicht starten.
    StreamStart { input: bool, detail: String },
    /// Das Audiogerät gab den Kern nicht rechtzeitig zurück (Gerätewechsel).
    NotResponding,
    /// Der Kern ließ sich nicht an den Audio-Stream übergeben.
    Handover,
    /// Ein Fehler der eigenen WASAPI-Hosts (Windows).
    Wasapi(WasapiProblem),
    /// Der Aufnahme-Thread läuft nicht.
    RecorderNotRunning,
    /// Der Aufnahme-Thread antwortet nicht.
    RecorderNoAnswer,
    /// Stopp ohne laufende Aufnahme.
    NothingRecorded,
    /// Eine Datei ließ sich nicht schreiben oder lesen; `detail` vom System oder Format.
    File { path: PathBuf, detail: String },
    /// Audio ließ sich nicht kodieren; `detail` vom Kodierer.
    Encode(String),
    /// Eine Aufnahme ohne Datei.
    NoFile,
    /// Eine Folgedatei einer Aufnahme passt nicht zur ersten (Kanäle, Rate, Format).
    FileMismatch(PathBuf),
    /// Der Exportbereich ist länger, als eine Datei fasst.
    RangeTooLong { max_secs: u64, rate: u32 },
    /// Die Audiodaten sind größer, als eine WAV- oder AIFF-Datei fasst.
    TooLarge { mb: u64 },
    /// MIDI gibt es auf diesem System nicht.
    MidiUnsupported,
    /// MIDI-Eingang oder -Ausgang ließ sich nicht öffnen.
    Midi { output: bool, detail: String },
}

/// Gründe der eigenen WASAPI-Hosts (Windows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WasapiProblem {
    /// Ein anderes Programm nutzt das Gerät exklusiv.
    InUse,
    /// Der exklusive Modus ist in den Windows-Einstellungen des Geräts nicht erlaubt.
    ExclusiveNotAllowed,
    /// Das Gerät unterstützt das Format nicht.
    FormatUnsupported,
    /// Das Gerät ist nicht mehr verfügbar.
    Gone,
    /// Das Gerät nennt kein Format.
    NoFormat,
    /// Kein Format für den exklusiven Modus.
    NoExclusiveFormat { channels: u16, rate: u32 },
    /// Das Mix-Format des Geräts wird nicht unterstützt.
    MixFormatUnsupported,
    /// IAudioClient3 (geringe Latenz) gibt es erst ab Windows 10.
    NoLowLatency(String),
    /// Stream und Richtung passen nicht zusammen.
    WrongDirection,
    /// Der Stream läuft schon.
    AlreadyRunning,
    /// Der Audio-Thread ist beendet.
    ThreadGone,
    /// Das Gerät antwortet nicht beim Start.
    NoStartAnswer,
    /// Das Gerät antwortet nicht.
    NoAnswer,
    /// Kein Ausgabedienst.
    NoRenderService,
    /// Der Audio-Thread ließ sich nicht starten.
    Thread(String),
    /// COM ließ sich nicht einrichten.
    Com(String),
    /// Die Geräteliste ließ sich nicht lesen.
    DeviceList(String),
    /// Das Gerät ließ sich nicht öffnen.
    Device(String),
    /// Eine andere Meldung von Windows.
    Os(String),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let io = |input: bool| {
            if input {
                "Eingabegerät"
            } else {
                "Ausgabegerät"
            }
        };
        match self {
            Self::HostMissing { name, available } => write!(
                f,
                "Audio-Host „{name}“ gibt es in diesem Build nicht (verfügbar: {})",
                available.join(", ")
            ),
            Self::Host { name, detail } => write!(f, "Audio-Host „{name}“: {detail}"),
            Self::NoDevice { input: true } => f.write_str("Kein Audio-Eingang gefunden"),
            Self::NoDevice { input: false } => f.write_str("Kein Audio-Ausgabegerät gefunden"),
            Self::DeviceList { input, detail } => write!(f, "{}e: {detail}", io(*input)),
            Self::DeviceNotFound { input, name } => {
                write!(f, "{} „{name}“ nicht gefunden", io(*input))
            }
            Self::Device {
                input: true,
                name,
                detail,
            } => write!(f, "Eingang „{name}“: {detail}"),
            Self::Device {
                input: false,
                name,
                detail,
            } => write!(f, "Gerät „{name}“: {detail}"),
            Self::RateUnsupported { name, rate } => {
                write!(f, "Eingang „{name}“ unterstützt {rate} Hz nicht")
            }
            Self::SampleFormat(format) => write!(f, "Sampleformat {format} wird nicht unterstützt"),
            Self::StreamBuild {
                input: true,
                detail,
            } => write!(f, "Eingang öffnen: {detail}"),
            Self::StreamBuild {
                input: false,
                detail,
            } => write!(f, "Stream-Aufbau: {detail}"),
            Self::StreamStart {
                input: true,
                detail,
            } => write!(f, "Eingang starten: {detail}"),
            Self::StreamStart {
                input: false,
                detail,
            } => write!(f, "Stream-Start: {detail}"),
            Self::NotResponding => {
                f.write_str("Das Audiogerät antwortet nicht; der Wechsel gilt nach einem Neustart")
            }
            Self::Handover => f.write_str("Audio-Stream: Übergabe des Kerns fehlgeschlagen"),
            Self::Wasapi(p) => write!(f, "WASAPI: {p}"),
            Self::RecorderNotRunning => f.write_str("Recorder läuft nicht"),
            Self::RecorderNoAnswer => f.write_str("Recorder antwortet nicht"),
            Self::NothingRecorded => f.write_str("Es lief keine Aufnahme"),
            Self::File { path, detail } => write!(f, "{}: {detail}", path.display()),
            Self::Encode(detail) => f.write_str(detail),
            Self::NoFile => f.write_str("keine Datei"),
            Self::FileMismatch(path) => {
                write!(f, "{}: passt nicht zur ersten Datei", path.display())
            }
            Self::RangeTooLong { max_secs, rate } => write!(
                f,
                "Der Bereich ist zu lang für eine Exportdatei: höchstens {} h {} min bei {rate} Hz. Locatoren enger setzen oder in Teilen exportieren.",
                max_secs / 3600,
                max_secs % 3600 / 60
            ),
            Self::TooLarge { mb } => {
                write!(f, "Zu lang für eine Audiodatei: {mb} MB, höchstens 4096 MB")
            }
            Self::MidiUnsupported => f.write_str("MIDI wird auf diesem System nicht unterstützt"),
            Self::Midi {
                output: true,
                detail,
            } => write!(f, "MIDI-Ausgang: {detail}"),
            Self::Midi {
                output: false,
                detail,
            } => write!(f, "MIDI: {detail}"),
        }
    }
}

impl fmt::Display for WasapiProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InUse => {
                f.write_str("das Gerät wird von einer anderen Anwendung exklusiv benutzt")
            }
            Self::ExclusiveNotAllowed => f.write_str(
                "der exklusive Modus ist in den Windows-Einstellungen des Geräts nicht erlaubt",
            ),
            Self::FormatUnsupported => f.write_str("das Gerät unterstützt dieses Format nicht"),
            Self::Gone => f.write_str("das Gerät ist nicht mehr verfügbar"),
            Self::NoFormat => f.write_str("Gerät nennt kein Format"),
            Self::NoExclusiveFormat { channels, rate } => write!(
                f,
                "kein passendes Format für den exklusiven Modus ({channels} Kanäle, {rate} Hz)"
            ),
            Self::MixFormatUnsupported => {
                f.write_str("das Mix-Format des Geräts wird nicht unterstützt")
            }
            Self::NoLowLatency(detail) => write!(f, "IAudioClient3 (ab Windows 10): {detail}"),
            Self::WrongDirection => f.write_str("Stream und Richtung passen nicht zusammen"),
            Self::AlreadyRunning => f.write_str("Stream läuft schon"),
            Self::ThreadGone => f.write_str("Audio-Thread ist beendet"),
            Self::NoStartAnswer => f.write_str("Gerät antwortet nicht beim Start"),
            Self::NoAnswer => f.write_str("Gerät antwortet nicht"),
            Self::NoRenderService => f.write_str("kein Ausgabedienst"),
            Self::Thread(detail) => write!(f, "Audio-Thread: {detail}"),
            Self::Com(detail) => write!(f, "COM: {detail}"),
            Self::DeviceList(detail) => write!(f, "Geräteliste: {detail}"),
            Self::Device(detail) => write!(f, "Gerät: {detail}"),
            Self::Os(detail) => f.write_str(detail),
        }
    }
}

impl std::error::Error for EngineError {}
impl std::error::Error for WasapiProblem {}

impl From<WasapiProblem> for EngineError {
    fn from(p: WasapiProblem) -> Self {
        Self::Wasapi(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fallback text is the German one the UI showed before (MCP answers keep it).
    #[test]
    fn display_keeps_the_german_texts() {
        assert_eq!(
            EngineError::DeviceNotFound {
                input: true,
                name: "USB".into()
            }
            .to_string(),
            "Eingabegerät „USB“ nicht gefunden"
        );
        assert_eq!(
            EngineError::DeviceList {
                input: false,
                detail: "x".into()
            }
            .to_string(),
            "Ausgabegeräte: x"
        );
        assert_eq!(
            EngineError::Wasapi(WasapiProblem::InUse).to_string(),
            "WASAPI: das Gerät wird von einer anderen Anwendung exklusiv benutzt"
        );
        assert_eq!(
            EngineError::RangeTooLong {
                max_secs: 4 * 3600 + 8 * 60,
                rate: 48_000
            }
            .to_string(),
            "Der Bereich ist zu lang für eine Exportdatei: höchstens 4 h 8 min bei 48000 Hz. Locatoren enger setzen oder in Teilen exportieren."
        );
    }
}
