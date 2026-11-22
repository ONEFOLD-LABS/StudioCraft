//! Fehler der Engine in der Sprache der Oberfläche (Issue #265). Die Engine liefert
//! [`EngineError`] und [`ClipError`] mit deutschem `Display` als Rückfall; hier entsteht der
//! angezeigte Text mit `tr`/`trf`. Meldungen des Systems oder eines Treibers bleiben, wie sie
//! sind.

use studiocraft_engine::{ClipError, EngineError, WasapiProblem};

use crate::i18n::{tr, trf};

/// Der Text eines Engine-Fehlers für die Oberfläche.
pub fn engine_error_text(e: &EngineError) -> String {
    use EngineError as E;
    match e {
        E::HostMissing { name, available } => trf(
            "Audio-Host „{name}“ gibt es in diesem Build nicht (verfügbar: {liste})",
            &[("name", name), ("liste", &available.join(", "))],
        ),
        E::Host { name, detail } => trf(
            "Audio-Host „{name}“: {grund}",
            &[("name", name), ("grund", detail)],
        ),
        E::NoDevice { input: true } => tr("Kein Audio-Eingang gefunden").to_owned(),
        E::NoDevice { input: false } => tr("Kein Audio-Ausgabegerät gefunden").to_owned(),
        E::DeviceList {
            input: true,
            detail,
        } => trf("Eingabegeräte: {grund}", &[("grund", detail)]),
        E::DeviceList {
            input: false,
            detail,
        } => trf("Ausgabegeräte: {grund}", &[("grund", detail)]),
        E::DeviceNotFound { input: true, name } => {
            trf("Eingabegerät „{name}“ nicht gefunden", &[("name", name)])
        }
        E::DeviceNotFound { input: false, name } => {
            trf("Ausgabegerät „{name}“ nicht gefunden", &[("name", name)])
        }
        E::Device {
            input: true,
            name,
            detail,
        } => trf(
            "Eingang „{name}“: {grund}",
            &[("name", name), ("grund", detail)],
        ),
        E::Device {
            input: false,
            name,
            detail,
        } => trf(
            "Gerät „{name}“: {grund}",
            &[("name", name), ("grund", detail)],
        ),
        E::RateUnsupported { name, rate } => trf(
            "Eingang „{name}“ unterstützt {rate} Hz nicht",
            &[("name", name), ("rate", rate)],
        ),
        E::SampleFormat(format) => trf(
            "Sampleformat {format} wird nicht unterstützt",
            &[("format", format)],
        ),
        E::StreamBuild {
            input: true,
            detail,
        } => trf("Eingang öffnen: {grund}", &[("grund", detail)]),
        E::StreamBuild {
            input: false,
            detail,
        } => trf("Stream-Aufbau: {grund}", &[("grund", detail)]),
        E::StreamStart {
            input: true,
            detail,
        } => trf("Eingang starten: {grund}", &[("grund", detail)]),
        E::StreamStart {
            input: false,
            detail,
        } => trf("Stream-Start: {grund}", &[("grund", detail)]),
        E::NotResponding => {
            tr("Das Audiogerät antwortet nicht; der Wechsel gilt nach einem Neustart").to_owned()
        }
        E::Handover => tr("Audio-Stream: Übergabe des Kerns fehlgeschlagen").to_owned(),
        E::Wasapi(p) => trf("WASAPI: {grund}", &[("grund", &wasapi_text(p))]),
        E::RecorderNotRunning => tr("Recorder läuft nicht").to_owned(),
        E::RecorderNoAnswer => tr("Recorder antwortet nicht").to_owned(),
        E::NothingRecorded => tr("Es lief keine Aufnahme").to_owned(),
        // Pfad und Meldung des Systems: so, wie sie sind.
        E::File { path, detail } => format!("{}: {detail}", path.display()),
        E::Encode(detail) => detail.clone(),
        E::NoFile => tr("keine Datei").to_owned(),
        E::FileMismatch(path) => trf(
            "{datei}: passt nicht zur ersten Datei",
            &[("datei", &path.display())],
        ),
        E::RangeTooLong { max_secs, rate } => trf(
            "Der Bereich ist zu lang für eine Exportdatei: höchstens {h} h {min} min bei {rate} Hz. Locatoren enger setzen oder in Teilen exportieren.",
            &[
                ("h", &(max_secs / 3600)),
                ("min", &(max_secs % 3600 / 60)),
                ("rate", rate),
            ],
        ),
        E::TooLarge { mb } => trf(
            "Zu lang für eine Audiodatei: {mb} MB, höchstens 4096 MB",
            &[("mb", mb)],
        ),
        E::MidiUnsupported => tr("MIDI wird auf diesem System nicht unterstützt").to_owned(),
        E::Midi {
            output: true,
            detail,
        } => trf("MIDI-Ausgang: {grund}", &[("grund", detail)]),
        E::Midi {
            output: false,
            detail,
        } => trf("MIDI: {grund}", &[("grund", detail)]),
    }
}

/// Der Grund eines WASAPI-Fehlers (auch für den Rückfall-Hinweis der Statuszeile).
pub fn wasapi_text(p: &WasapiProblem) -> String {
    use WasapiProblem as W;
    match p {
        W::InUse => tr("das Gerät wird von einer anderen Anwendung exklusiv benutzt").to_owned(),
        W::ExclusiveNotAllowed => {
            tr("der exklusive Modus ist in den Windows-Einstellungen des Geräts nicht erlaubt")
                .to_owned()
        }
        W::FormatUnsupported => tr("das Gerät unterstützt dieses Format nicht").to_owned(),
        W::Gone => tr("das Gerät ist nicht mehr verfügbar").to_owned(),
        W::NoFormat => tr("Gerät nennt kein Format").to_owned(),
        W::NoExclusiveFormat { channels, rate } => trf(
            "kein passendes Format für den exklusiven Modus ({kanaele} Kanäle, {rate} Hz)",
            &[("kanaele", channels), ("rate", rate)],
        ),
        W::MixFormatUnsupported => {
            tr("das Mix-Format des Geräts wird nicht unterstützt").to_owned()
        }
        W::NoLowLatency(detail) => trf(
            "IAudioClient3 (ab Windows 10): {grund}",
            &[("grund", detail)],
        ),
        W::WrongDirection => tr("Stream und Richtung passen nicht zusammen").to_owned(),
        W::AlreadyRunning => tr("Stream läuft schon").to_owned(),
        W::ThreadGone => tr("Audio-Thread ist beendet").to_owned(),
        W::NoStartAnswer => tr("Gerät antwortet nicht beim Start").to_owned(),
        W::NoAnswer => tr("Gerät antwortet nicht").to_owned(),
        W::NoRenderService => tr("kein Ausgabedienst").to_owned(),
        W::Thread(detail) => trf("Audio-Thread: {grund}", &[("grund", detail)]),
        W::Com(detail) => format!("COM: {detail}"),
        W::DeviceList(detail) => trf("Geräteliste: {grund}", &[("grund", detail)]),
        W::Device(detail) => trf("Gerät: {grund}", &[("grund", detail)]),
        W::Os(detail) => detail.clone(),
    }
}

/// Der Text eines Fehlers beim Laden einer Audiodatei.
pub fn clip_error_text(e: &ClipError) -> String {
    match e {
        ClipError::Wav(e) => trf("WAV-Datei nicht lesbar: {grund}", &[("grund", e)]),
        ClipError::UnsupportedChannels(n) => {
            trf("{n} Kanäle werden nicht unterstützt", &[("n", n)])
        }
        ClipError::Io(e) => trf("Datei nicht lesbar: {grund}", &[("grund", e)]),
        ClipError::Decode(e) => trf("Audioformat nicht lesbar: {grund}", &[("grund", e)]),
        ClipError::UnsupportedRate(r) => {
            trf("Abtastrate {r} Hz wird nicht unterstützt", &[("r", r)])
        }
        ClipError::UnsupportedBits(b) => trf("{b} Bit werden nicht unterstützt", &[("b", b)]),
    }
}

/// `?` mit übersetztem Text: `engine_call().tr_err()?` in Funktionen, die `String` melden.
pub(crate) trait TrErr<T> {
    fn tr_err(self) -> Result<T, String>;
}

impl<T> TrErr<T> for Result<T, EngineError> {
    fn tr_err(self) -> Result<T, String> {
        self.map_err(|e| engine_error_text(&e))
    }
}

impl<T> TrErr<T> for Result<T, ClipError> {
    fn tr_err(self) -> Result<T, String> {
        self.map_err(|e| clip_error_text(&e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Language, set_language};

    /// English chosen: an engine error from a real call shows in English; German again in
    /// German. System and driver messages inside stay as they are.
    #[test]
    fn engine_errors_follow_the_interface_language() {
        let dir = std::env::temp_dir().join(format!("sc-engine-err-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let missing = dir.join("fehlt").join("lang.wav");
        // A real engine call: the export refuses a range no file can hold.
        let song = studiocraft_engine::Song::default();
        let err = studiocraft_engine::export::render_file(
            &song,
            48_000,
            0,
            studiocraft_engine::export::MAX_EXPORT_FRAMES + 1,
            &missing,
        )
        .unwrap_err();
        set_language(Language::English);
        let en = engine_error_text(&err);
        set_language(Language::German);
        let de = engine_error_text(&err);
        assert!(en.starts_with("The range is too long"), "{en}");
        assert_eq!(de, err.to_string(), "German is the Display text");

        set_language(Language::English);
        let device = engine_error_text(&EngineError::DeviceNotFound {
            input: true,
            name: "USB".into(),
        });
        let wasapi = engine_error_text(&EngineError::Wasapi(WasapiProblem::InUse));
        let os = engine_error_text(&EngineError::File {
            path: "/x.wav".into(),
            detail: "Keine Berechtigung (os error 13)".into(),
        });
        let clip = clip_error_text(&ClipError::UnsupportedRate(7));
        set_language(Language::German);
        assert_eq!(device, "Input device “USB” not found");
        assert!(
            wasapi.starts_with("WASAPI: another application"),
            "{wasapi}"
        );
        assert_eq!(
            os, "/x.wav: Keine Berechtigung (os error 13)",
            "system text stays"
        );
        assert_eq!(clip, "Sample rate 7 Hz is not supported");
    }
}
