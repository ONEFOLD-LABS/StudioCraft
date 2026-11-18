//! Werkzeuge und Raster (Spec-Abschnitte „Werkzeuge“ und „Raster, Snap und Bearbeitung“).

use egui::Key;
use studiocraft_project::Project;

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

    /// Ob das Werkzeug im Prototyp schon etwas tut.
    pub fn implemented(self) -> bool {
        !matches!(self, Tool::Range | Tool::Glue | Tool::TimeWarp)
    }
}

/// Raster-Typ für Snap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grid {
    Bar,
    Beat,
    Sixteenth,
}

impl Grid {
    pub const ALL: [Grid; 3] = [Grid::Bar, Grid::Beat, Grid::Sixteenth];

    pub fn label(self) -> &'static str {
        match self {
            Grid::Bar => "Takt",
            Grid::Beat => "Zählzeit",
            Grid::Sixteenth => "1/16",
        }
    }

    pub fn ticks(self, project: &Project) -> u64 {
        let sig = project.tempo.signature;
        match self {
            Grid::Bar => sig.bar_ticks(),
            Grid::Beat => sig.beat_ticks(),
            Grid::Sixteenth => studiocraft_project::TICKS_PER_QUARTER / 4,
        }
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
