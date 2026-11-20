//! Sprache der Oberfläche (Issue #99).
//!
//! Die Texte stehen auf Deutsch im Code und sind zugleich der Schlüssel: `tr("Speichern")`.
//! Für Englisch liefert der Katalog `i18n/en.tsv` die Übersetzung (eine Zeile je Text:
//! deutsch, Tabulator, englisch; `#` leitet Kommentare ein). Fehlt eine Zeile, bleibt der
//! deutsche Text stehen; ein Test prüft, dass jeder `tr`-Text im Katalog steht. Weitere
//! Sprachen sind weitere Kataloge.
//!
//! Texte mit Platzhaltern schreiben die Werte als `{name}` und füllen sie mit [`trf`] ein.
//! Die Sprache gilt je Thread (die Oberfläche läuft in einem; Tests laufen so unabhängig).

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::LazyLock;

/// Gewählte Sprache (Einstellung).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    /// Die Sprache des Systems, wenn es sie gibt, sonst Englisch.
    #[default]
    System,
    German,
    English,
}

impl Language {
    pub const ALL: [Language; 3] = [Language::System, Language::German, Language::English];

    /// Kennung in settings.json.
    pub fn code(self) -> &'static str {
        match self {
            Language::System => "system",
            Language::German => "de",
            Language::English => "en",
        }
    }

    pub fn from_code(code: &str) -> Self {
        match code {
            "de" => Language::German,
            "en" => Language::English,
            _ => Language::System,
        }
    }

    /// Name im Sprachmenü: jede Sprache in ihrer eigenen Schreibweise.
    pub fn label(self) -> &'static str {
        match self {
            Language::System => tr("Wie das System"),
            Language::German => "Deutsch",
            Language::English => "English",
        }
    }
}

/// Sprache, in der gerade angezeigt wird.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lang {
    De,
    En,
}

thread_local! {
    static PREFERENCE: Cell<Language> = const { Cell::new(Language::System) };
    static CURRENT: Cell<Lang> = const { Cell::new(Lang::De) };
}

/// Stellt die Sprache ein (gilt sofort für alles, was danach gezeichnet wird).
pub fn set_language(language: Language) {
    PREFERENCE.with(|p| p.set(language));
    let lang = match language {
        Language::German => Lang::De,
        Language::English => Lang::En,
        Language::System => system(),
    };
    CURRENT.with(|c| c.set(lang));
}

/// Die eingestellte Sprache.
pub fn language() -> Language {
    PREFERENCE.with(Cell::get)
}

/// Sprache des Systems; Tests laufen immer auf Deutsch (ihre Texte sind deutsch).
fn system() -> Lang {
    if cfg!(test) {
        return Lang::De;
    }
    match sys_locale::get_locale() {
        Some(l) if l.to_ascii_lowercase().starts_with("de") => Lang::De,
        _ => Lang::En,
    }
}

static EN: LazyLock<HashMap<&'static str, &'static str>> =
    LazyLock::new(|| parse(include_str!("../i18n/en.tsv")));

/// Liest einen Katalog: `deutsch<TAB>übersetzung` je Zeile. Leere Zeilen, Kommentare und
/// Zeilen ohne Übersetzung zählen nicht.
fn parse(text: &'static str) -> HashMap<&'static str, &'static str> {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
        .filter(|(de, en)| !de.is_empty() && !en.is_empty())
        .collect()
}

/// Übersetzt den deutschen Text `de` in die eingestellte Sprache.
pub fn tr(de: &'static str) -> &'static str {
    match CURRENT.with(Cell::get) {
        Lang::De => de,
        Lang::En => EN.get(de).copied().unwrap_or(de),
    }
}

/// Übersetzt einen zur Laufzeit gebildeten Text (etwa „CC 7 Lautstärke“, „1/16 triolisch“):
/// erst als Ganzes, sonst Wort für Wort; unbekannte Wörter bleiben stehen.
pub fn tr_dyn(de: &str) -> String {
    if CURRENT.with(Cell::get) == Lang::De {
        return de.to_owned();
    }
    if let Some(en) = EN.get(de) {
        return (*en).to_owned();
    }
    de.split(' ')
        .map(|w| EN.get(w).copied().unwrap_or(w))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Ob es für `de` eine englische Zeile gibt (Tests für Texte, die nicht als `tr("…")` im Code
/// stehen, etwa Befehlsnamen).
#[cfg(test)]
pub(crate) fn has_english(de: &str) -> bool {
    EN.contains_key(de)
}

/// Wie [`tr`], füllt danach `{name}`-Platzhalter mit den Werten.
pub fn trf(de: &'static str, values: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut text = tr(de).to_owned();
    for (name, value) in values {
        text = text.replace(&format!("{{{name}}}"), &value.to_string());
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Alle `tr("…")`- und `trf("…"`-Texte aus dem Quelltext der Oberfläche.
    fn source_keys() -> Vec<String> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = vec![];
        let mut stack = vec![dir];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "rs")
                    && p.file_name().is_some_and(|n| n != "i18n.rs")
                {
                    files.push(p);
                }
            }
        }
        let mut keys = vec![];
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap();
            for start in ["tr(", "trf("] {
                let mut rest = text.as_str();
                while let Some(i) = rest.find(start) {
                    // Nicht mitten in einem Namen wie `attr(`.
                    let before = rest[..i].chars().last();
                    rest = &rest[i + start.len()..];
                    if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        continue;
                    }
                    // Der Text darf nach einem Zeilenumbruch stehen (rustfmt).
                    let Some(lit) = rest.trim_start().strip_prefix('"') else {
                        continue;
                    };
                    let Some(end) = lit.find('"') else { break };
                    keys.push(lit[..end].replace("\\\"", "\""));
                }
            }
        }
        keys.sort();
        keys.dedup();
        keys
    }

    #[test]
    fn every_translated_text_has_an_english_entry() {
        let missing: Vec<String> = source_keys()
            .into_iter()
            .filter(|k| !EN.contains_key(k.as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "fehlt in i18n/en.tsv:\n{}",
            missing.join("\n")
        );
    }

    /// Namen, die aus dem Projektmodell kommen und über `tr(x.label())` laufen (der Scan oben
    /// sieht sie nicht).
    #[test]
    fn model_labels_have_english_entries() {
        use studiocraft_project::ProcessKind as K;
        let processes = [
            K::Normalize { target_db: 0.0 },
            K::Gain { db: 0.0 },
            K::Silence,
            K::Reverse,
            K::InvertPhase,
            K::FadeIn,
            K::FadeOut,
            K::RemoveDc,
        ]
        .map(|k| k.label());
        let labels = studiocraft_project::DrawShape::ALL.map(|s| s.label());
        let missing: Vec<&str> = labels
            .into_iter()
            .chain(processes)
            .filter(|k| !EN.contains_key(k))
            .collect();
        assert!(missing.is_empty(), "fehlt in i18n/en.tsv: {missing:?}");
    }

    #[test]
    fn the_catalog_has_no_duplicate_keys_and_keeps_placeholders() {
        let mut seen = std::collections::HashSet::new();
        for line in include_str!("../i18n/en.tsv").lines() {
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let (de, en) = line.split_once('\t').expect("Tabulator");
            assert!(seen.insert(de), "doppelt: {de}");
            let holes = |s: &str| {
                let mut v: Vec<String> = s
                    .split('{')
                    .skip(1)
                    .filter_map(|p| p.split_once('}').map(|(n, _)| n.to_owned()))
                    .collect();
                v.sort();
                v
            };
            assert_eq!(holes(de), holes(en), "Platzhalter: {de}");
        }
    }

    #[test]
    fn switching_the_language_changes_texts_and_falls_back_to_german() {
        set_language(Language::English);
        assert_eq!(tr("Speichern"), "Save");
        assert_eq!(tr("gibt es nicht im Katalog"), "gibt es nicht im Katalog");
        assert_eq!(language(), Language::English);
        assert_eq!(tr_dyn("CC 7 Lautstärke"), "CC 7 Volume");
        assert_eq!(tr_dyn("1/16 triolisch"), "1/16 triplet");
        set_language(Language::German);
        assert_eq!(tr("Speichern"), "Speichern");
        assert_eq!(tr_dyn("CC 7 Lautstärke"), "CC 7 Lautstärke");
        set_language(Language::System);
        assert_eq!(tr("Speichern"), "Speichern", "Tests: System = Deutsch");
    }
}
