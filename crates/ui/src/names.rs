//! Eingebaute Standardnamen (Busse, Zeilen der General-MIDI-Map) für die Anzeige übersetzen.
//!
//! Die Namen werden deutsch in der Projektdatei gespeichert. Übersetzt wird nur, was gezeigt
//! wird, und nur solange der Name noch der eingebaute ist; ein vom Nutzer umbenannter Bus oder
//! eine umbenannte Zeile bleibt, wie sie ist.

use crate::i18n::{tr_dyn, trf};

/// Name eines Ein- oder Ausgangsbusses, wie er angezeigt wird: „Eingang 1/2“ → „Input 1/2“.
pub(crate) fn bus(name: &str) -> String {
    for (prefix, pattern) in [
        ("Eingang ", "Eingang {ports}"),
        ("Ausgang ", "Ausgang {ports}"),
    ] {
        if let Some(ports) = name.strip_prefix(prefix)
            && is_ports(ports)
        {
            return trf(pattern, &[("ports", &ports)]);
        }
    }
    name.to_owned()
}

/// Kanalangabe wie „1/2“ oder „3“.
fn is_ports(s: &str) -> bool {
    let mut parts = s.split('/');
    let ok =
        |p: Option<&str>| p.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    match (parts.next(), parts.next(), parts.next()) {
        (a, None, None) => ok(a),
        (a, b @ Some(_), None) => ok(a) && ok(b),
        _ => false,
    }
}

/// Name einer Zeile der Drum-Map, wie er angezeigt wird: der eingebaute GM-Name übersetzt.
pub(crate) fn drum_row(pitch: u8, name: &str) -> String {
    if studiocraft_project::drummap::gm_name(pitch) == Some(name) {
        tr_dyn(name)
    } else {
        name.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Language, set_language};

    #[test]
    fn built_in_names_are_translated_for_display_only() {
        set_language(Language::English);
        let shown = [
            bus("Eingang 1/2"),
            bus("Ausgang 3"),
            bus("Ausgang 3/4 Kopfhörer"),
            bus("Stereo Out"),
            drum_row(80, "Triangel gedämpft"),
            drum_row(80, "Mein Triangel"),
            drum_row(36, "Klatschen"),
        ];
        set_language(Language::German);
        assert_eq!(
            shown,
            [
                "Input 1/2",
                "Output 3",
                "Ausgang 3/4 Kopfhörer",
                "Stereo Out",
                "Muted Triangle",
                "Mein Triangel",
                "Klatschen",
            ]
        );
        // German shows the stored names unchanged.
        assert_eq!(bus("Eingang 1/2"), "Eingang 1/2");
        assert_eq!(drum_row(80, "Triangel gedämpft"), "Triangel gedämpft");
    }
}
