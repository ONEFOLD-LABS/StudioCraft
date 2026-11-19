//! Werkzeuge und Raster (Spec-Abschnitte „Werkzeuge“ und „Raster, Snap und Bearbeitung“).

use egui::Key;
use studiocraft_project::{Project, QuantizeValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

impl Tool {
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
        }
    }

    /// Ob das Werkzeug im Prototyp schon etwas tut.
    pub fn implemented(self) -> bool {
        !matches!(self, Tool::TimeWarp)
    }
}

/// Raster-Typ für Snap. „Quantisierung verwenden“ folgt dem globalen Quantisierungswert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grid {
    Bar,
    Beat,
    UseQuantize,
}

impl Grid {
    pub const ALL: [Grid; 3] = [Grid::Bar, Grid::Beat, Grid::UseQuantize];

    pub fn label(self) -> &'static str {
        match self {
            Grid::Bar => "Takt",
            Grid::Beat => "Zählzeit",
            Grid::UseQuantize => "Quantisierung verwenden",
        }
    }

    /// Rasterabstand in Ticks, nie 0.
    pub fn ticks(self, project: &Project, quantize: QuantizeValue) -> u64 {
        let sig = project.tempo.signature;
        let t = match self {
            Grid::Bar => sig.bar_ticks(),
            Grid::Beat => sig.beat_ticks(),
            Grid::UseQuantize => quantize.ticks(),
        };
        t.max(1)
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
