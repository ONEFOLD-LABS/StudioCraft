//! Snap (Spec „Raster, Snap und Bearbeitung“, Issue #12): Snap-Modus, Raster-Typ und
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

    fn on_grid(&self, t: f64) -> f64 {
        let g = self.grid_step();
        ((t / g).round() * g).max(0.0)
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
