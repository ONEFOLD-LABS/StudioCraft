//! Link-Gruppen im Mixer (Spec „Mixer“, Issue #43): verknüpfte Kanäle reagieren gemeinsam auf
//! Fader, Mute und Solo.
//!
//! Die Bedienelemente ändern Spuren an vielen Stellen (Mixer, Spurliste, Inspector, eigene
//! Fenster). Statt jede Stelle anzupassen, vergleicht die Oberfläche einmal je Bild den Stand
//! der verknüpften Spuren mit dem vorigen ([`Project::link_snapshot`]) und gleicht die Gruppe
//! an ([`Project::propagate_links`]):
//!
//! - Hat sich in einer Gruppe **genau eine** Spur bewegt, folgen die anderen: der Fader um
//!   dieselbe Differenz in dB (Abstände bleiben, Grenzen [`MIN_DB`]..[`MAX_DB`]), Mute und
//!   Solo auf denselben Wert.
//! - Haben sich mehrere zugleich geändert (Rückgängig, Laden, exklusives Solo), bleibt alles,
//!   wie es ist: das war kein Handgriff an einem Kanal.

use crate::{MAX_DB, MIN_DB, Project, TrackId};

/// Höchste Nummer einer Link-Gruppe (Gruppen 1 bis 8).
pub const MAX_LINK_GROUPS: u8 = 8;

/// Stand einer verknüpften Spur zu Beginn eines Bildes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkState {
    pub track: TrackId,
    pub group: u8,
    pub volume_db: f32,
    pub mute: bool,
    pub solo: bool,
}

impl Project {
    /// Stand aller verknüpften Spuren (für [`Self::propagate_links`] im nächsten Bild).
    pub fn link_snapshot(&self) -> Vec<LinkState> {
        self.tracks
            .iter()
            .filter_map(|t| {
                t.link_group.map(|group| LinkState {
                    track: t.id,
                    group,
                    volume_db: t.volume_db,
                    mute: t.mute,
                    solo: t.solo,
                })
            })
            .collect()
    }

    /// Überträgt die Bewegung einer verknüpften Spur auf ihre Gruppe (siehe Moduldoku).
    /// Gibt zurück, ob sich dabei etwas geändert hat.
    pub fn propagate_links(&mut self, before: &[LinkState]) -> bool {
        let was = |id: TrackId| before.iter().find(|s| s.track == id);
        let mut changed = false;
        for group in 1..=MAX_LINK_GROUPS {
            let members: Vec<usize> = (0..self.tracks.len())
                .filter(|&i| self.tracks[i].link_group == Some(group))
                .collect();
            if members.len() < 2 {
                continue;
            }
            // Nur Spuren, die schon im letzten Bild in dieser Gruppe waren, zählen als bewegt.
            let moved = |pick: &dyn Fn(&LinkState, &crate::Track) -> bool| -> Option<usize> {
                let mut found = None;
                for &i in &members {
                    let t = &self.tracks[i];
                    if let Some(s) = was(t.id)
                        && s.group == group
                        && pick(s, t)
                    {
                        if found.is_some() {
                            return None;
                        }
                        found = Some(i);
                    }
                }
                found
            };
            let volume = moved(&|s, t| s.volume_db != t.volume_db);
            let mute = moved(&|s, t| s.mute != t.mute);
            let solo = moved(&|s, t| s.solo != t.solo);
            if let Some(lead) = volume
                && let Some(s) = was(self.tracks[lead].id)
            {
                let delta = self.tracks[lead].volume_db - s.volume_db;
                if delta.is_finite() {
                    for &i in &members {
                        if i != lead {
                            let t = &mut self.tracks[i];
                            let v = (t.volume_db + delta).clamp(MIN_DB, MAX_DB);
                            changed |= v != t.volume_db;
                            t.volume_db = v;
                        }
                    }
                }
            }
            if let Some(lead) = mute {
                let on = self.tracks[lead].mute;
                for &i in &members {
                    changed |= self.tracks[i].mute != on;
                    self.tracks[i].mute = on;
                }
            }
            if let Some(lead) = solo {
                let on = self.tracks[lead].solo;
                for &i in &members {
                    changed |= self.tracks[i].solo != on;
                    self.tracks[i].solo = on;
                }
            }
        }
        changed
    }

    /// Setzt die Link-Gruppe einer Spur (`None`: nicht verknüpft). Nummern außerhalb
    /// 1..=[`MAX_LINK_GROUPS`] heben die Verknüpfung auf.
    pub fn set_link_group(&mut self, track: usize, group: Option<u8>) {
        if let Some(t) = self.tracks.get_mut(track) {
            t.link_group = group.filter(|g| (1..=MAX_LINK_GROUPS).contains(g));
        }
    }

    /// Wie viele Spuren in Gruppe `group` sind.
    pub fn link_group_size(&self, group: u8) -> usize {
        self.tracks
            .iter()
            .filter(|t| t.link_group == Some(group))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> Project {
        let mut p = Project::demo(4);
        for t in &mut p.tracks {
            t.volume_db = 0.0;
            t.mute = false;
            t.solo = false;
        }
        p.set_link_group(0, Some(1));
        p.set_link_group(1, Some(1));
        p.set_link_group(2, Some(2));
        p
    }

    #[test]
    fn a_fader_move_carries_over_with_the_same_offset() {
        let mut p = project();
        p.tracks[1].volume_db = -6.0;
        let before = p.link_snapshot();
        p.tracks[0].volume_db = -3.0;
        assert!(p.propagate_links(&before));
        assert_eq!(p.tracks[1].volume_db, -9.0);
        // Track 2 is in another group, track 3 is not linked.
        assert_eq!(p.tracks[2].volume_db, 0.0);
        assert_eq!(p.tracks[3].volume_db, 0.0);
        // Next frame: nothing moved, nothing changes.
        let before = p.link_snapshot();
        assert!(!p.propagate_links(&before));
    }

    #[test]
    fn fader_moves_stop_at_the_range_limits() {
        let mut p = project();
        p.tracks[1].volume_db = MAX_DB - 1.0;
        let before = p.link_snapshot();
        p.tracks[0].volume_db = 4.0;
        p.propagate_links(&before);
        assert_eq!(p.tracks[1].volume_db, MAX_DB);
        let before = p.link_snapshot();
        p.tracks[0].volume_db = MIN_DB;
        p.propagate_links(&before);
        assert!(p.tracks[1].volume_db >= MIN_DB);
    }

    #[test]
    fn mute_and_solo_follow_the_moved_channel() {
        let mut p = project();
        let before = p.link_snapshot();
        p.tracks[1].mute = true;
        p.tracks[0].solo = true;
        assert!(p.propagate_links(&before));
        assert!(p.tracks[0].mute && p.tracks[1].mute);
        assert!(p.tracks[0].solo && p.tracks[1].solo);
        assert!(!p.tracks[2].mute && !p.tracks[3].solo);
    }

    #[test]
    fn several_changes_at_once_are_left_alone() {
        // Undo or loading changes several channels of a group in one frame: no propagation.
        let mut p = project();
        let before = p.link_snapshot();
        p.tracks[0].volume_db = -2.0;
        p.tracks[1].volume_db = -5.0;
        p.tracks[0].mute = true;
        p.tracks[1].mute = true;
        assert!(!p.propagate_links(&before));
        assert_eq!(p.tracks[0].volume_db, -2.0);
        assert_eq!(p.tracks[1].volume_db, -5.0);
    }

    #[test]
    fn exclusive_solo_on_a_linked_channel_solos_its_group() {
        let mut p = project();
        p.tracks[3].solo = true;
        let before = p.link_snapshot();
        p.solo_exclusive(0);
        p.propagate_links(&before);
        assert!(p.tracks[0].solo && p.tracks[1].solo);
        assert!(!p.tracks[3].solo);
    }

    #[test]
    fn joining_a_group_does_not_move_its_members() {
        let mut p = project();
        p.tracks[3].volume_db = -12.0;
        let before = p.link_snapshot();
        p.set_link_group(3, Some(1));
        assert!(!p.propagate_links(&before));
        assert_eq!(p.tracks[0].volume_db, 0.0);
        assert_eq!(p.tracks[3].volume_db, -12.0);
        assert_eq!(p.link_group_size(1), 3);
    }

    #[test]
    fn group_numbers_outside_the_range_unlink() {
        let mut p = project();
        p.set_link_group(0, Some(0));
        assert_eq!(p.tracks[0].link_group, None);
        p.set_link_group(0, Some(MAX_LINK_GROUPS + 1));
        assert_eq!(p.tracks[0].link_group, None);
        p.set_link_group(99, Some(1));
    }
}
