//! Dialog „Über StudioCraft“ (Menü Hilfe): Version, Lizenz und die Lizenzen der eingebauten
//! Open-Source-Komponenten. Deren Texte stehen in `THIRD-PARTY-LICENSES.txt` (erzeugt mit
//! `cargo xtask licenses`) und werden hier eingebettet, damit jede Kopie der App sie enthält.

use std::sync::LazyLock;

use egui::{RichText, Ui};

use crate::ProjectWindow;
use crate::i18n::{tr, trf};

/// Hinweise zu StudioCraft selbst.
const NOTICE: &str = include_str!("../../../NOTICE");
/// Lizenztexte der Komponenten.
const THIRD_PARTY: &str = include_str!("../../../THIRD-PARTY-LICENSES.txt");
static LINES: LazyLock<Vec<&'static str>> = LazyLock::new(|| THIRD_PARTY.lines().collect());

/// Zustand des Dialogs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum About {
    #[default]
    Closed,
    Open,
    /// Mit aufgeklappten Lizenzen der Komponenten.
    Licenses,
}

impl ProjectWindow {
    pub(crate) fn about_dialog(&mut self, ui: &mut Ui) {
        if self.about == About::Closed {
            return;
        }
        let mut open = true;
        self.dialogs.escape_closes(
            ui.ctx(),
            crate::dialog::window_id(tr("Über StudioCraft")),
            &mut open,
        );
        egui::Window::new(tr("Über StudioCraft"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_size([620.0, 480.0])
            .show(ui.ctx(), |ui| {
                ui.heading("StudioCraft");
                ui.label(trf("Version {v}", &[("v", &env!("CARGO_PKG_VERSION"))]));
                ui.label(tr(
                    "Freie Software unter der MIT- oder der Apache-2.0-Lizenz, nach Wahl.",
                ));
                ui.add_space(6.0);
                egui::CollapsingHeader::new(tr("Hinweise"))
                    .default_open(false)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("hinweise")
                            .max_height(160.0)
                            .show(ui, |ui| ui.label(RichText::new(NOTICE).small()));
                    });
                let showing = self.about == About::Licenses;
                if ui
                    .selectable_label(showing, tr("Lizenzen der Open-Source-Komponenten"))
                    .clicked()
                {
                    self.about = if showing {
                        About::Open
                    } else {
                        About::Licenses
                    };
                }
                if self.about == About::Licenses {
                    let row = ui.text_style_height(&egui::TextStyle::Monospace);
                    egui::ScrollArea::both()
                        .id_salt("lizenzen")
                        .auto_shrink([false, false])
                        .show_rows(ui, row, LINES.len(), |ui, range| {
                            for line in &LINES[range] {
                                // Ohne Umbruch: jede Zeile gleich hoch (waagerecht blättern).
                                ui.add(egui::Label::new(RichText::new(*line).monospace()).extend());
                            }
                        });
                }
            });
        if !open {
            self.about = About::Closed;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_license_texts_are_built_in() {
        assert!(THIRD_PARTY.starts_with("StudioCraft – third-party licences"));
        assert!(THIRD_PARTY.contains("Permission is hereby granted"));
        assert!(THIRD_PARTY.contains("Mozilla Public License Version 2.0"));
        assert!(LINES.len() > 1000);
        assert!(NOTICE.contains("StudioCraft"));
    }
}
