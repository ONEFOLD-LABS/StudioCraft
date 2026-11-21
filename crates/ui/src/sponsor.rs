//! Spendenlink im Menü Hilfe: öffnet die Ko-fi-Seite von ONEFOLD LABS im Browser.

use egui::Ui;

use crate::i18n::tr;

/// Die Spendenseite von ONEFOLD LABS.
pub(crate) const URL: &str = "https://ko-fi.com/onefoldlabs";

/// Der Menüeintrag für das Menü Hilfe.
pub(crate) fn menu_item(ui: &mut Ui) {
    if ui.button(tr("StudioCraft unterstützen …")).clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(URL));
        ui.close();
    }
}
