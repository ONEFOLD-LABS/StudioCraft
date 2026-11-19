//! Snap (Spec „Raster, Snap und Bearbeitung“): Snap-Modus, Raster-Typ und
//! „An Zoom anpassen“. Events und Cursor wirken magnetisch nur in der Nähe (wenige Pixel),
//! das Raster überall. Alt beim Ziehen hebt Snap auf (in den Zieh-Funktionen).

use egui::Ui;
use studiocraft_project::{EventId, TICKS_PER_QUARTER};

use crate::ProjectWindow;
use crate::tools::{Grid, SnapMode};

/// Fangbereich für Events und Cursor in Pixeln.
const MAGNET_PX: f32 = 8.0;
/// „An Zoom anpassen“ teilt das Raster, solange ein Schritt breiter als so viele Pixel ist.
const ZOOM_STEP_PX: f32 = 48.0;
/// Feine Rasterlinien erst ab diesem Abstand in Pixeln.
pub(crate) const FINE_LINE_PX: f32 = 6.0;
/// Höchstens so viele feine Linien pro Bild (Schutz bei extremem Zoom).
const MAX_FINE_LINES: usize = 4000;
/// Feinstes Raster beim Anpassen an den Zoom: 1/64.
const ZOOM_MIN_TICKS: f64 = TICKS_PER_QUARTER as f64 / 16.0;

/// Snap-Modus und Raster-Typ in der Werkzeugzeile.
pub(crate) fn toolbar(w: &mut ProjectWindow, ui: &mut Ui) {
    egui::ComboBox::from_id_salt("snap-modus")
        .selected_text(w.snap_mode.label())
        .width(110.0)
        .show_ui(ui, |ui| {
            for m in SnapMode::ALL {
                ui.selectable_value(&mut w.snap_mode, m, m.label());
            }
        })
        .response
        .on_hover_text("Snap-Modus");
    egui::ComboBox::from_id_salt("raster")
        .selected_text(w.grid.label())
        .width(90.0)
        .show_ui(ui, |ui| {
            for g in Grid::ALL {
                ui.selectable_value(&mut w.grid, g, g.label());
            }
            ui.separator();
            ui.checkbox(&mut w.grid_zoom, "An Zoom anpassen");
        })
        .response
        .on_hover_text("Raster");
}

impl ProjectWindow {
    fn px_to_ticks(&self, px: f32) -> f64 {
        f64::from(px / self.view.px_per_quarter.max(0.01)) * TICKS_PER_QUARTER as f64
    }

    /// Rasterabstand in Ticks, mit „An Zoom anpassen“ beim Einzoomen verfeinert.
    pub(crate) fn grid_step(&self) -> f64 {
        let mut step = self
            .grid
            .step(&self.project, self.quantize.value, self.sample_rate);
        if self.grid_zoom && self.grid.is_musical() {
            let wide = self.px_to_ticks(ZOOM_STEP_PX);
            while step >= wide && step / 2.0 >= ZOOM_MIN_TICKS {
                step /= 2.0;
            }
        }
        step
    }

    /// Rasterabstand auf ganze Ticks gerundet, mindestens 1 (z. B. Länge neuer Events).
    pub(crate) fn grid_ticks(&self) -> u64 {
        self.grid_step().round().max(1.0) as u64
    }

    /// Feine Rasterlinien des Snap-Rasters in `t0..=t1` (Ticks) bei `px_per_quarter`.
    /// Leer, wenn die Linien enger als [`FINE_LINE_PX`] lägen. Mit `skip_beats` fallen Linien
    /// weg, die auf einer Zählzeit liegen (die zeichnet der Aufrufer kräftiger).
    pub(crate) fn fine_grid_lines(
        &self,
        t0: f64,
        t1: f64,
        px_per_quarter: f32,
        skip_beats: bool,
    ) -> Vec<f64> {
        let step = self.grid_step();
        let step_px = step / TICKS_PER_QUARTER as f64 * f64::from(px_per_quarter);
        if !step_px.is_finite() || step_px < f64::from(FINE_LINE_PX) || t1 < t0 {
            return Vec::new();
        }
        let beat = self.beat_ticks() as f64;
        let first = (t0.max(0.0) / step).ceil() as u64;
        let mut out = Vec::new();
        for k in first.. {
            let t = k as f64 * step;
            if t > t1 || out.len() >= MAX_FINE_LINES {
                break;
            }
            if skip_beats && beat > 0.0 && (t - (t / beat).round() * beat).abs() < 0.5 {
                continue;
            }
            out.push(t);
        }
        out
    }

    fn on_grid(&self, t: f64) -> f64 {
        let g = self.grid_step();
        // Takte und Zählzeiten folgen den Taktwechseln (nicht gleichmäßig ab 0).
        let tempo = &self.project.tempo;
        if !tempo.signatures.is_empty() && !self.grid_zoom {
            let at = t.max(0.0) as u64;
            let (_, start, sig) = tempo.bar_at(at);
            let unit = match self.grid {
                crate::tools::Grid::Bar => Some(sig.bar_ticks()),
                crate::tools::Grid::Beat => Some(sig.beat_ticks()),
                _ => None,
            };
            if let Some(unit) = unit {
                let unit = unit.max(1) as f64;
                let k = ((t - start as f64) / unit).round().max(0.0);
                // Nicht über den Anfang des nächsten Takts hinaus (dort beginnt die neue Taktart).
                let next = tempo.bar_start(tempo.bar_at(at).0 + 1) as f64;
                return (start as f64 + k * unit).min(next);
            }
        }
        ((t / g).round() * g).max(0.0)
    }

    /// Takt- und Zählzeitlinien in `t0..=t1` entlang der Taktwechsel: (Tick, Taktnummer bei
    /// Taktlinien, sonst `None`). Zählzeiten nur mit `beats`.
    pub(crate) fn meter_lines(&self, t0: u64, t1: u64, beats: bool) -> Vec<(u64, Option<u64>)> {
        let tempo = &self.project.tempo;
        let mut out = Vec::new();
        for (at, bar, sig) in tempo.bar_lines(t0, t1) {
            out.push((at, Some(bar)));
            if beats {
                let beat = sig.beat_ticks().max(1);
                for k in 1..u64::from(sig.numerator) {
                    let t = at + k * beat;
                    if t > t1 {
                        break;
                    }
                    out.push((t, None));
                }
            }
        }
        out
    }

    /// Nächster magnetischer Punkt (Event-Ränder, Cursor) im Fangbereich um `t`, mit Abstand.
    /// Events in `exclude` zählen nicht (die gerade gezogenen).
    fn magnet(&self, t: f64, exclude: &[EventId]) -> Option<(f64, f64)> {
        let range = self.px_to_ticks(MAGNET_PX);
        let mut best: Option<(f64, f64)> = None;
        let mut consider = |p: f64| {
            let d = (p - t).abs();
            if d <= range && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((p, d));
            }
        };
        if self.snap_mode.uses_cursor() {
            consider(self.cursor as f64);
        }
        if self.snap_mode.uses_events() {
            for ev in self.project.tracks.iter().flat_map(|tr| tr.events.iter()) {
                if !exclude.contains(&ev.id) {
                    consider(ev.start.0 as f64);
                    consider(ev.end() as f64);
                }
            }
        }
        best
    }

    /// Eine Position nach Snap-Modus und Raster (Klicks, Ränder, Locatoren, Zeichnen).
    pub(crate) fn snap_ticks(&self, ticks: f64) -> u64 {
        let t = ticks.max(0.0);
        if !self.snap {
            return t.round() as u64;
        }
        if let Some((p, _)) = self.magnet(t, &self.selection) {
            return p.round() as u64;
        }
        if self.snap_mode.uses_grid() {
            self.on_grid(t).round() as u64
        } else {
            t.round() as u64
        }
    }

    /// Neuer Anfang eines gezogenen Events: Anfang oder Ende rasten an Events und Cursor ein;
    /// „Raster relativ“ behält den Abstand zur Rasterlinie.
    pub(crate) fn snap_move(&self, raw: f64, length: u64, original: u64) -> u64 {
        let raw = raw.max(0.0);
        if !self.snap {
            return raw.round() as u64;
        }
        let len = length as f64;
        let by_start = self.magnet(raw, &self.selection);
        let by_end = self
            .magnet(raw + len, &self.selection)
            .map(|(p, d)| (p - len, d));
        let best = match (by_start, by_end) {
            (Some(a), Some(b)) => Some(if b.1 < a.1 { b } else { a }),
            (a, b) => a.or(b),
        };
        if let Some((p, _)) = best {
            return p.max(0.0).round() as u64;
        }
        let t = match self.snap_mode {
            SnapMode::GridRelative => {
                let g = self.grid_step();
                let o = original as f64;
                (o + ((raw - o) / g).round() * g).max(0.0)
            }
            m if m.uses_grid() => self.on_grid(raw),
            _ => raw,
        };
        t.round() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studiocraft_project::{Project, TrackKind};

    fn window() -> ProjectWindow {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "A");
        p.create_event(t, 3840, 960).expect("event");
        let mut w = ProjectWindow::new(p, None, String::new());
        w.view.px_per_quarter = 24.0; // 8 px = 320 ticks
        w.selection.clear();
        w
    }

    #[test]
    fn grid_mode_rounds_to_the_grid() {
        let mut w = window();
        w.grid = Grid::Beat;
        w.snap_mode = SnapMode::Grid;
        assert_eq!(w.snap_ticks(1300.0), 960);
        assert_eq!(w.snap_ticks(1500.0), 1920);
        w.snap = false;
        assert_eq!(w.snap_ticks(1500.0), 1500);
    }

    #[test]
    fn events_mode_snaps_only_near_event_edges() {
        let mut w = window();
        w.snap_mode = SnapMode::Events;
        assert_eq!(w.snap_ticks(3700.0), 3840, "near the start");
        assert_eq!(w.snap_ticks(4900.0), 4800, "near the end");
        assert_eq!(w.snap_ticks(2000.0), 2000, "far away: free");
    }

    #[test]
    fn cursor_modes_combine_with_the_grid() {
        let mut w = window();
        w.cursor = 1000;
        w.grid = Grid::Bar;
        w.snap_mode = SnapMode::GridCursor;
        assert_eq!(w.snap_ticks(1100.0), 1000, "cursor wins nearby");
        assert_eq!(w.snap_ticks(2500.0), 3840, "grid elsewhere");
        w.snap_mode = SnapMode::Cursor;
        assert_eq!(w.snap_ticks(2500.0), 2500);
    }

    #[test]
    fn moving_snaps_the_end_to_other_events() {
        let mut w = window();
        w.snap_mode = SnapMode::Events;
        // A 480-tick event dropped so its end lands just before 3840.
        assert_eq!(w.snap_move(3300.0, 480, 0), 3360);
    }

    #[test]
    fn grid_relative_keeps_the_offset() {
        let mut w = window();
        w.grid = Grid::Beat;
        w.snap_mode = SnapMode::GridRelative;
        assert_eq!(w.snap_move(1100.0 + 900.0, 480, 1100), 2060);
        w.snap_mode = SnapMode::Grid;
        assert_eq!(w.snap_move(2000.0, 480, 1100), 1920);
    }

    #[test]
    fn time_based_grids_follow_tempo_and_rate() {
        let mut w = window();
        w.grid = Grid::Seconds;
        assert!(
            (w.grid_step() - 1920.0).abs() < 1e-9,
            "120 bpm: 1 s = 2 quarters"
        );
        w.grid = Grid::Frames;
        assert!((w.grid_step() - 76.8).abs() < 1e-9);
        w.grid = Grid::Samples;
        assert!(w.grid_step() > 0.0 && w.grid_step() < 1.0);
        assert_eq!(w.grid_ticks(), 1);
    }

    #[test]
    fn fine_lines_follow_the_snap_grid_and_skip_beats() {
        let mut w = window();
        w.grid = Grid::UseQuantize; // 1/16 = 240 ticks
        // 24 px per quarter: 6 px per sixteenth, just visible.
        let lines = w.fine_grid_lines(0.0, 960.0, 24.0, true);
        assert_eq!(lines, [240.0, 480.0, 720.0]);
        assert_eq!(w.fine_grid_lines(0.0, 960.0, 24.0, false).len(), 5);
        assert!(
            w.fine_grid_lines(0.0, 960.0, 20.0, true).is_empty(),
            "too dense"
        );
        w.grid = Grid::Frames; // 76.8 ticks
        let frames = w.fine_grid_lines(0.0, 1920.0, 200.0, false);
        assert_eq!(frames.len(), 26);
        assert!((frames[1] - 76.8).abs() < 1e-9);
        assert!(w.fine_grid_lines(0.0, 1e12, 2000.0, false).len() <= MAX_FINE_LINES);
    }

    #[test]
    fn adapt_to_zoom_refines_when_zoomed_in() {
        let mut w = window();
        w.grid = Grid::Bar;
        w.view.px_per_quarter = 200.0;
        assert_eq!(w.grid_ticks(), 3840);
        w.grid_zoom = true;
        // 48 px at 200 px per quarter = 230 ticks: halve 3840 down to 120.
        assert_eq!(w.grid_ticks(), 120);
        w.view.px_per_quarter = 2000.0;
        assert_eq!(w.grid_ticks(), 60, "never finer than 1/64");
    }
}
