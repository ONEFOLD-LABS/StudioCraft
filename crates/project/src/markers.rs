//! Marker (Spec „Begriffe und Datenmodell“: Marker; „Spurtypen“: Marker-Spur; Issue #11).
//!
//! Positionsmarker haben die Nummern ab 3 (1 und 2 stehen für die Locatoren), Cycle-Marker
//! eigene Nummern ab 1. Ein Cycle-Marker hat ein Ende und setzt beim Anwählen die Locatoren.

use crate::Project;

/// Ein Marker: eine benannte Position oder (mit `end`) ein Bereich.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Marker {
    /// Nummer: Positionsmarker ab 3, Cycle-Marker ab 1 (je eigene Zählung).
    pub id: u32,
    pub name: String,
    /// Position in Ticks.
    pub start: u64,
    /// Cycle-Marker: Ende in Ticks (größer als `start`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<u64>,
}

impl Marker {
    pub fn is_cycle(&self) -> bool {
        self.end.is_some()
    }
}

/// Erste Nummer der Positionsmarker (1 und 2 sind die Locatoren).
pub const FIRST_MARKER_ID: u32 = 3;

impl Project {
    fn free_id(&self, cycle: bool) -> u32 {
        let first = if cycle { 1 } else { FIRST_MARKER_ID };
        (first..)
            .find(|id| {
                !self
                    .markers
                    .iter()
                    .any(|m| m.is_cycle() == cycle && m.id == *id)
            })
            .unwrap_or(first)
    }

    /// Neuer Positionsmarker bei `at`. Gibt seine Nummer zurück.
    pub fn add_marker(&mut self, at: u64) -> u32 {
        let id = self.free_id(false);
        self.markers.push(Marker {
            id,
            name: format!("Marker {id}"),
            start: at,
            end: None,
        });
        self.sort_markers();
        id
    }

    /// Neuer Cycle-Marker von `a` bis `b`; `None`, wenn der Bereich leer ist.
    pub fn add_cycle_marker(&mut self, a: u64, b: u64) -> Option<u32> {
        let (start, end) = (a.min(b), a.max(b));
        if end == start {
            return None;
        }
        let id = self.free_id(true);
        self.markers.push(Marker {
            id,
            name: format!("Cycle {id}"),
            start,
            end: Some(end),
        });
        self.sort_markers();
        Some(id)
    }

    fn sort_markers(&mut self) {
        self.markers.sort_by_key(|m| (m.start, m.is_cycle(), m.id));
    }

    /// Positionsmarker mit Nummer `id`.
    pub fn marker(&self, id: u32) -> Option<&Marker> {
        self.markers.iter().find(|m| !m.is_cycle() && m.id == id)
    }

    /// Verschiebt Marker `i` (Index in `markers`) an `start`; ein Cycle-Marker behält seine
    /// Länge. Gibt den neuen Index zurück.
    pub fn move_marker(&mut self, i: usize, start: u64) -> Option<usize> {
        let m = self.markers.get_mut(i)?;
        if let Some(end) = m.end {
            m.end = Some(start.saturating_add(end - m.start));
        }
        m.start = start;
        let key = (m.id, m.is_cycle());
        self.sort_markers();
        self.markers
            .iter()
            .position(|m| (m.id, m.is_cycle()) == key)
    }

    pub fn remove_marker(&mut self, i: usize) -> bool {
        if i < self.markers.len() {
            self.markers.remove(i);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::Project;

    #[test]
    fn markers_get_free_numbers_and_stay_sorted() {
        let mut p = Project::default();
        assert_eq!(p.add_marker(1920), 3);
        assert_eq!(p.add_marker(960), 4);
        assert_eq!(p.add_cycle_marker(3840, 0), Some(1));
        assert_eq!(p.add_cycle_marker(10, 10), None, "empty range");
        let order: Vec<(u32, u64)> = p.markers.iter().map(|m| (m.id, m.start)).collect();
        assert_eq!(order, [(1, 0), (4, 960), (3, 1920)]);
        // A freed number is reused.
        let i = p
            .markers
            .iter()
            .position(|m| m.id == 3 && !m.is_cycle())
            .unwrap();
        assert!(p.remove_marker(i));
        assert_eq!(p.add_marker(5000), 3);
        assert_eq!(p.marker(4).map(|m| m.start), Some(960));
    }

    #[test]
    fn moving_a_cycle_marker_keeps_its_length_and_markers_are_saved() {
        let mut p = Project::default();
        p.add_cycle_marker(960, 1920);
        let i = p.move_marker(0, 3840).unwrap();
        assert_eq!((p.markers[i].start, p.markers[i].end), (3840, Some(4800)));
        let json = crate::file::to_json(&p);
        let back = crate::file::from_json(&json).unwrap();
        assert_eq!(back.markers, p.markers);
    }
}
