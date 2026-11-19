//! Werkzeuge und Raster (Spec-Abschnitte „Werkzeuge“ und „Raster, Snap und Bearbeitung“).

use egui::Key;
use studiocraft_project::{Project, QuantizeValue, TICKS_PER_QUARTER};

/// Was Ziehen am Rand eines Events mit der Objektauswahl tut (Spec „Varianten pro Werkzeug“).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResizeMode {
    /// Länge ändern, der Inhalt bleibt an seiner Stelle.
    #[default]
    Normal,
    /// Der Inhalt wandert mit dem gezogenen Rand.
    Slide,
    /// Der Inhalt wird gestreckt bzw. gestaucht (bisher nur MIDI-Parts).
    Stretch,
}

impl ResizeMode {
    pub const ALL: [ResizeMode; 3] = [ResizeMode::Normal, ResizeMode::Slide, ResizeMode::Stretch];

    pub fn label(self) -> &'static str {
        match self {
            ResizeMode::Normal => "Größe ändern",
            ResizeMode::Slide => "Größe ändern, Inhalt mitschieben",
            ResizeMode::Stretch => "Größe ändern, Inhalt strecken",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    Select,
    Range,
    Split,
    Glue,
    Erase,
    Zoom,
    Mute,
    Draw,
    Play,
    TimeWarp,
    /// Comping in den Lanes: Klick oder Ziehen über ein Take macht den Abschnitt hörbar.
    Comp,
}

impl Tool {
    /// Reihenfolge in der Werkzeugzeile: wie [`Tool::ALL`], Comping hinter dem Bereich.
    pub const TOOLBAR: [Tool; 11] = [
        Tool::Select,
        Tool::Range,
        Tool::Comp,
        Tool::Split,
        Tool::Glue,
        Tool::Erase,
        Tool::Zoom,
        Tool::Mute,
        Tool::Draw,
        Tool::Play,
        Tool::TimeWarp,
    ];

    /// Die Werkzeuge auf den Tasten 1 bis 0.
    pub const ALL: [Tool; 10] = [
        Tool::Select,
        Tool::Range,
        Tool::Split,
        Tool::Glue,
        Tool::Erase,
        Tool::Zoom,
        Tool::Mute,
        Tool::Draw,
        Tool::Play,
        Tool::TimeWarp,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Auswahl",
            Tool::Range => "Bereich",
            Tool::Split => "Trennen",
            Tool::Glue => "Kleben",
            Tool::Erase => "Löschen",
            Tool::Zoom => "Zoom",
            Tool::Mute => "Stumm",
            Tool::Draw => "Zeichnen",
            Tool::Play => "Wiedergabe",
            Tool::TimeWarp => "Zeitkorrektur",
            Tool::Comp => "Comping",
        }
    }

    pub fn key_label(self) -> &'static str {
        match self {
            Tool::Select => "1",
            Tool::Range => "2",
            Tool::Split => "3",
            Tool::Glue => "4",
            Tool::Erase => "5",
            Tool::Zoom => "6",
            Tool::Mute => "7",
            Tool::Draw => "8",
            Tool::Play => "9",
            Tool::TimeWarp => "0",
            Tool::Comp => "",
        }
    }

    pub fn from_key(key: Key) -> Option<Tool> {
        Some(match key {
            Key::Num1 => Tool::Select,
            Key::Num2 => Tool::Range,
            Key::Num3 => Tool::Split,
            Key::Num4 => Tool::Glue,
            Key::Num5 => Tool::Erase,
            Key::Num6 => Tool::Zoom,
            Key::Num7 => Tool::Mute,
            Key::Num8 => Tool::Draw,
            Key::Num9 => Tool::Play,
            Key::Num0 => Tool::TimeWarp,
            _ => return None,
        })
    }

    /// Symbol in der Werkzeugzeile.
    pub fn icon(self) -> crate::icons::Icon {
        use crate::icons::Icon;
        match self {
            Tool::Select => Icon::Select,
            Tool::Range => Icon::Range,
            Tool::Split => Icon::Split,
            Tool::Glue => Icon::Glue,
            Tool::Erase => Icon::Erase,
            Tool::Zoom => Icon::Zoom,
            Tool::Mute => Icon::Mute,
            Tool::Draw => Icon::Draw,
            Tool::Play => Icon::Play,
            Tool::TimeWarp => Icon::TimeWarp,
            Tool::Comp => Icon::Comp,
        }
    }

    /// Ob das Werkzeug im Prototyp schon etwas tut.
    pub fn implemented(self) -> bool {
        !matches!(self, Tool::TimeWarp)
    }
}

/// Wohin eingerastet wird (Spec „Raster, Snap und Bearbeitung“ → Snap-Modi).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapMode {
    Grid,
    GridRelative,
    Events,
    Cursor,
    GridCursor,
    EventsCursor,
    EventsGridCursor,
    Shuffle,
}

impl SnapMode {
    pub const ALL: [SnapMode; 8] = [
        SnapMode::Grid,
        SnapMode::GridRelative,
        SnapMode::Events,
        SnapMode::Cursor,
        SnapMode::GridCursor,
        SnapMode::EventsCursor,
        SnapMode::EventsGridCursor,
        SnapMode::Shuffle,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SnapMode::Grid => "Raster",
            SnapMode::GridRelative => "Raster relativ",
            SnapMode::Events => "Events",
            SnapMode::Cursor => "Cursor",
            SnapMode::GridCursor => "Raster + Cursor",
            SnapMode::EventsCursor => "Events + Cursor",
            SnapMode::EventsGridCursor => "Events + Raster + Cursor",
            SnapMode::Shuffle => "Schieben",
        }
    }

    /// Rastet auf Rasterlinien ein.
    pub fn uses_grid(self) -> bool {
        matches!(
            self,
            SnapMode::Grid
                | SnapMode::GridRelative
                | SnapMode::GridCursor
                | SnapMode::EventsGridCursor
                | SnapMode::Shuffle
        )
    }

    /// Rastet auf Anfang und Ende anderer Events ein.
    pub fn uses_events(self) -> bool {
        matches!(
            self,
            SnapMode::Events | SnapMode::EventsCursor | SnapMode::EventsGridCursor
        )
    }

    /// Rastet auf den Projekt-Cursor ein.
    pub fn uses_cursor(self) -> bool {
        matches!(
            self,
            SnapMode::Cursor
                | SnapMode::GridCursor
                | SnapMode::EventsCursor
                | SnapMode::EventsGridCursor
        )
    }
}

/// Bildrate des Raster-Typs „Frames“, bis das Projekt eine eigene Bildrate hat.
pub const FRAMES_PER_SECOND: f64 = 25.0;

/// Raster-Typ für Snap. „Quantisierung verwenden“ folgt dem globalen Quantisierungswert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grid {
    Bar,
    Beat,
    UseQuantize,
    Seconds,
    Frames,
    Samples,
}

impl Grid {
    pub const ALL: [Grid; 6] = [
        Grid::Bar,
        Grid::Beat,
        Grid::UseQuantize,
        Grid::Seconds,
        Grid::Frames,
        Grid::Samples,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Grid::Bar => "Takt",
            Grid::Beat => "Zählzeit",
            Grid::UseQuantize => "Quantisierung verwenden",
            Grid::Seconds => "Sekunden",
            Grid::Frames => "Frames (25 fps)",
            Grid::Samples => "Samples",
        }
    }

    /// Musikalisches Raster (Takt, Zählzeit, Quantisierung), das „An Zoom anpassen“ teilt.
    pub fn is_musical(self) -> bool {
        matches!(self, Grid::Bar | Grid::Beat | Grid::UseQuantize)
    }

    /// Rasterabstand in Ticks (nicht ganzzahlig bei Sekunden, Frames und Samples), stets > 0.
    pub fn step(self, project: &Project, quantize: QuantizeValue, sample_rate: u32) -> f64 {
        let sig = project.tempo.signature;
        let bpm = if project.tempo.bpm.is_finite() && project.tempo.bpm > 0.0 {
            project.tempo.bpm
        } else {
            120.0
        };
        let per_second = bpm / 60.0 * TICKS_PER_QUARTER as f64;
        let t = match self {
            Grid::Bar => sig.bar_ticks() as f64,
            Grid::Beat => sig.beat_ticks() as f64,
            Grid::UseQuantize => quantize.ticks() as f64,
            Grid::Seconds => per_second,
            Grid::Frames => per_second / FRAMES_PER_SECOND,
            Grid::Samples => per_second / f64::from(sample_rate.max(1)),
        };
        if t.is_finite() && t > 0.0 { t } else { 1.0 }
    }
}

impl crate::ProjectWindow {
    /// Öffnet die Werkzeugpalette an `pos` (Rechtsklick in die Event-Anzeige, Spec „Werkzeuge“).
    pub(crate) fn open_tool_palette(&mut self, pos: egui::Pos2) {
        self.tool_palette = Some(pos);
        self.tool_palette_age = 0;
    }

    /// Zeichnet die offene Werkzeugpalette: alle Werkzeuge als Symbolknöpfe in zwei Reihen.
    /// Ein Klick auf ein Werkzeug wählt es und schließt die Palette, ebenso ein Klick daneben
    /// oder Esc.
    pub(crate) fn tool_palette(&mut self, ctx: &egui::Context) {
        let Some(pos) = self.tool_palette else {
            return;
        };
        let current = self.tool;
        let resp = egui::Area::new(egui::Id::unique("werkzeugpalette"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .show(ui, |ui| {
                        let mut picked = None;
                        for row in Tool::TOOLBAR.chunks(6) {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 2.0;
                                for &tool in row {
                                    let name = format!("{} (Palette)", tool.label());
                                    if crate::icons::icon_button(
                                        ui,
                                        tool.icon(),
                                        tool == current,
                                        None,
                                        &name,
                                    )
                                    .clicked()
                                    {
                                        picked = Some(tool);
                                    }
                                }
                            });
                        }
                        picked
                    })
                    .inner
            });
        if let Some(tool) = resp.inner {
            self.tool = tool;
            self.tool_palette = None;
            return;
        }
        let clicked_outside = ctx.input(|i| i.pointer.any_click())
            && ctx
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|p| !resp.response.rect.contains(p));
        if (clicked_outside && self.tool_palette_age > 0)
            || ctx.input(|i| i.key_pressed(Key::Escape))
        {
            self.tool_palette = None;
        }
        self.tool_palette_age = self.tool_palette_age.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_keys_map_to_tools_in_toolbar_order() {
        let keys = [
            Key::Num1,
            Key::Num2,
            Key::Num3,
            Key::Num4,
            Key::Num5,
            Key::Num6,
            Key::Num7,
            Key::Num8,
            Key::Num9,
            Key::Num0,
        ];
        for (k, t) in keys.into_iter().zip(Tool::ALL) {
            assert_eq!(Tool::from_key(k), Some(t));
        }
    }
}
