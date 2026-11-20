//! MIDI-Dateien importieren und exportieren (Datei-Menü, Issue #47). Lesen und Schreiben
//! liegen in `studiocraft_project::smf`; hier sind Dialoge, Meldungen und die Auswahl danach.

use studiocraft_project::smf::{self, SmfError};

use crate::ProjectWindow;
use crate::i18n::{tr, trf};

/// Endungen von MIDI-Dateien.
pub(crate) const EXTENSIONS: [&str; 4] = ["mid", "midi", "smf", "kar"];

/// Meldung zu einem Lesefehler.
fn message(e: SmfError) -> &'static str {
    match e {
        SmfError::NotMidi => tr("keine MIDI-Datei"),
        SmfError::Corrupt => tr("MIDI-Datei beschädigt"),
        SmfError::Format2 => tr("MIDI-Format 2 wird nicht unterstützt"),
        SmfError::Smpte => tr("SMPTE-Zeitbasis wird nicht unterstützt"),
        SmfError::TooLarge => tr("MIDI-Datei zu groß"),
        SmfError::Empty => tr("keine Noten"),
    }
}

impl ProjectWindow {
    /// Importiert eine MIDI-Datei aus Bytes: je Spur und Kanal der Datei eine MIDI-Spur. In
    /// einem Projekt ohne Events gelten danach Tempo und Taktart der Datei und die Parts
    /// beginnen am Anfang; sonst bleibt das Tempo und die Parts beginnen am Takt des Cursors.
    pub fn import_midi_bytes(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let err = |e: SmfError| {
            trf(
                "{name}: {fehler}",
                &[("name", &name), ("fehler", &message(e))],
            )
        };
        let file = smf::read(bytes).map_err(err)?;
        if file.tracks.is_empty() {
            return Err(err(SmfError::Empty));
        }
        let fresh = self.project.tracks.iter().all(|t| t.events.is_empty());
        let start = if fresh {
            0
        } else {
            self.project.tempo.bar_at(self.cursor).1
        };
        let added = self.project.import_midi(&file, start, fresh);
        self.selected_track = added.first().copied();
        self.selection = added
            .iter()
            .filter_map(|&i| self.project.tracks.get(i))
            .flat_map(|t| t.events.iter().map(|e| e.id))
            .collect();
        self.dirty = true;
        Ok(())
    }

    /// Die MIDI- und Instrument-Spuren als MIDI-Datei: (Dateiname, Bytes).
    pub fn midi_file_bytes(&self) -> Result<(String, Vec<u8>), String> {
        let bytes = self.project.export_midi().map_err(|e| match e {
            SmfError::Empty => tr("Keine MIDI- oder Instrument-Spuren zum Exportieren").to_owned(),
            e => message(e).to_owned(),
        })?;
        Ok((
            format!("{}.mid", crate::session::sanitize(&self.project_stem())),
            bytes,
        ))
    }

    /// Dialog „MIDI-Datei importieren“.
    pub(crate) fn import_midi_dialog(&mut self) {
        let Some(files) = crate::menu::dialog()
            .add_filter(tr("MIDI-Dateien"), &EXTENSIONS)
            .pick_files()
        else {
            return;
        };
        let before = self.project.clone();
        let mut errors = Vec::new();
        for f in files {
            let name = f
                .file_stem()
                .map_or_else(|| "MIDI".into(), |s| s.to_string_lossy().into_owned());
            let read = match std::fs::metadata(&f) {
                Ok(m) if m.len() > smf::MAX_FILE_BYTES as u64 => {
                    Err(message(SmfError::TooLarge).to_owned())
                }
                _ => std::fs::read(&f).map_err(|e| e.to_string()),
            };
            let result = read
                .map_err(|e| trf("{name}: {fehler}", &[("name", &name), ("fehler", &e)]))
                .and_then(|bytes| self.import_midi_bytes(&name, &bytes));
            if let Err(e) = result {
                errors.push(e);
            }
        }
        self.message = (!errors.is_empty()).then(|| errors.join("; "));
        self.checkpoint(before);
    }

    /// Dialog „MIDI-Datei exportieren“.
    pub(crate) fn export_midi_dialog(&mut self) -> Result<(), String> {
        let (name, bytes) = self.midi_file_bytes()?;
        match crate::menu::dialog()
            .add_filter(tr("MIDI-Datei"), &["mid"])
            .set_file_name(name)
            .save_file()
        {
            Some(p) => std::fs::write(&p, bytes).map_err(|e| format!("{}: {e}", p.display())),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studiocraft_project::{Project, TempoMap, TrackKind};

    fn window(project: Project) -> ProjectWindow {
        ProjectWindow::new(project, None, "kein Audio (Test)".into())
    }

    /// Projekt mit einer MIDI-Spur, einem Part und 90 BPM als Datei.
    fn file() -> Vec<u8> {
        let mut p = Project::default();
        p.tempo = TempoMap::fixed(90.0);
        let t = p.add_track(TrackKind::Midi, "Bass");
        let ev = p.create_event(t, 0, 3840).unwrap();
        p.event_mut(ev).unwrap().notes = vec![studiocraft_project::Note {
            pitch: 40,
            velocity: 100,
            start: 0,
            length: 960,
        }];
        p.export_midi().unwrap()
    }

    #[test]
    fn import_into_an_empty_project_takes_the_tempo() {
        let mut w = window(Project::default());
        w.import_midi_bytes("Song", &file()).unwrap();
        assert!((w.project.tempo.bpm - 90.0).abs() < 0.01);
        let t = &w.project.tracks[0];
        assert_eq!((t.kind, t.name.as_str()), (TrackKind::Midi, "Bass"));
        assert_eq!(w.selection, vec![t.events[0].id]);
        assert_eq!(t.events[0].start.0, 0);
    }

    #[test]
    fn import_into_a_project_keeps_the_tempo_and_starts_at_the_cursor_bar() {
        let mut w = window(Project::demo(3));
        let tempo = w.project.tempo.clone();
        w.cursor = 3840 * 2 + 500;
        let before = w.project.tracks.len();
        w.import_midi_bytes("Song", &file()).unwrap();
        assert_eq!(w.project.tempo, tempo);
        let t = &w.project.tracks[before];
        assert_eq!(
            t.events[0].start.0,
            w.project.tempo.bar_at(3840 * 2 + 500).1
        );
    }

    #[test]
    fn bad_files_and_empty_exports_give_messages() {
        let mut w = window(Project::default());
        let e = w.import_midi_bytes("Kaputt", b"nonsense").unwrap_err();
        assert_eq!(e, "Kaputt: keine MIDI-Datei");
        assert!(w.project.tracks.is_empty());
        assert_eq!(
            w.midi_file_bytes().unwrap_err(),
            "Keine MIDI- oder Instrument-Spuren zum Exportieren"
        );
        w.import_midi_bytes("Song", &file()).unwrap();
        let (name, bytes) = w.midi_file_bytes().unwrap();
        assert_eq!(name, "Unbenannt.mid");
        assert!(bytes.starts_with(b"MThd"));
    }
}
