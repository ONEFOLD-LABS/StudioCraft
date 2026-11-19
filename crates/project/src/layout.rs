//! Fensterlayout (Spec „Fenster-Architektur“): welche Zonen sichtbar sind und wie groß sie
//! sind. Wird mit dem Projekt gespeichert; dazu benannte Layouts, die sich per Taste
//! abrufen lassen.

use serde::{Deserialize, Serialize};

/// Größte gespeicherte Zonengröße in Punkten (schützt vor unsinnigen Dateiwerten).
pub const MAX_ZONE_SIZE: f32 = 4000.0;
/// Höchstzahl benannter Layouts (Tasten Strg+Alt+1 bis 9).
pub const MAX_LAYOUTS: usize = 9;
/// Längster Name eines Layouts in Zeichen.
pub const MAX_LAYOUT_NAME: usize = 64;

/// Sichtbarkeit und Größe der Zonen des Projektfensters. Größe 0 heißt „Standard der
/// Oberfläche“.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowLayout {
    /// Linke Zone (Kanalzug und Inspector).
    pub left: bool,
    /// Inspector-Spalte innerhalb der linken Zone.
    pub inspector: bool,
    pub lower: bool,
    pub right: bool,
    pub transport: bool,
    pub left_width: f32,
    pub right_width: f32,
    pub lower_height: f32,
}

impl Default for WindowLayout {
    fn default() -> Self {
        Self {
            left: true,
            inspector: true,
            lower: true,
            right: false,
            transport: true,
            left_width: 0.0,
            right_width: 0.0,
            lower_height: 0.0,
        }
    }
}

impl WindowLayout {
    /// Größen auf `0..=MAX_ZONE_SIZE` ziehen (NaN wird 0). Gibt `true` zurück, wenn sich
    /// etwas geändert hat.
    pub fn sanitize(&mut self) -> bool {
        let mut changed = false;
        for v in [
            &mut self.left_width,
            &mut self.right_width,
            &mut self.lower_height,
        ] {
            let new = if v.is_finite() {
                v.clamp(0.0, MAX_ZONE_SIZE)
            } else {
                0.0
            };
            changed |= new != *v;
            *v = new;
        }
        changed
    }
}

/// Ein benanntes Fensterlayout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamedLayout {
    pub name: String,
    pub layout: WindowLayout,
}

/// Kürzt `name` auf [`MAX_LAYOUT_NAME`] Zeichen (an Zeichengrenzen).
pub fn clip_name(name: &str) -> String {
    name.chars().take(MAX_LAYOUT_NAME).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_fixes_hostile_sizes() {
        let mut l = WindowLayout {
            left_width: f32::NAN,
            right_width: -5.0,
            lower_height: 1e9,
            ..WindowLayout::default()
        };
        assert!(l.sanitize());
        assert_eq!(
            (l.left_width, l.right_width, l.lower_height),
            (0.0, 0.0, MAX_ZONE_SIZE)
        );
        assert!(!l.sanitize());
    }

    #[test]
    fn missing_fields_take_defaults() {
        let l: WindowLayout = serde_json::from_str(r#"{"right": true}"#).unwrap();
        assert!(l.right && l.left && l.inspector && l.transport);
    }

    #[test]
    fn clip_name_counts_characters() {
        let long = "ä".repeat(100);
        assert_eq!(clip_name(&long).chars().count(), MAX_LAYOUT_NAME);
    }
}
