//! Ordnerspuren (Spec „Spurtypen und Inspector“: Ordner; Issue #10).
//!
//! Eine Spur gehört über `Track::parent` zu einer Ordnerspur. Die Reihenfolge der Spuren im
//! Projekt bleibt dabei unverändert (Engine und Plugins hängen an ihr); Spurliste und
//! Event-Anzeige zeigen die Spuren in Baumreihenfolge: jeder Ordner, darunter seine Spuren
//! eingerückt. Zugeklappte Ordner blenden ihre Spuren aus. Mute und Solo eines Ordners wirken
//! auf alle enthaltenen Spuren.

use crate::{Project, TrackId, TrackKind};

impl Project {
    /// Index der Ordnerspur, zu der `track` gehört (nur gültige Ordner).
    pub fn parent_of(&self, track: usize) -> Option<usize> {
        let id = self.tracks.get(track)?.parent?;
        let p = self.track_index(id)?;
        (p != track && self.tracks[p].kind == TrackKind::Folder).then_some(p)
    }

    /// Alle Ordner über `track`, vom nächsten nach außen. Bricht bei Zyklen ab.
    pub fn ancestors(&self, track: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut cur = track;
        while let Some(p) = self.parent_of(cur) {
            if p == track || out.contains(&p) {
                break;
            }
            out.push(p);
            cur = p;
        }
        out
    }

    /// Ob `track` in `folder` liegt (direkt oder tiefer).
    pub fn is_inside(&self, track: usize, folder: usize) -> bool {
        self.ancestors(track).contains(&folder)
    }

    /// Legt `track` in den Ordner `folder` (`None`: heraus). Abgelehnt, wenn `folder` keine
    /// Ordnerspur ist oder ein Ordner in sich selbst landen würde.
    pub fn set_parent(&mut self, track: usize, folder: Option<usize>) -> bool {
        let Some(t) = self.tracks.get(track) else {
            return false;
        };
        let id: Option<TrackId> = match folder {
            None => None,
            Some(f) => {
                let ok = self
                    .tracks
                    .get(f)
                    .is_some_and(|ft| ft.kind == TrackKind::Folder)
                    && f != track
                    && !self.is_inside(f, track);
                if !ok {
                    return false;
                }
                Some(self.tracks[f].id)
            }
        };
        if t.parent == id {
            return false;
        }
        self.tracks[track].parent = id;
        true
    }

    /// Platz einer Spur in der Baumreihenfolge: ihr eigener Index, bei Ordnern der kleinste
    /// Index darin. So erscheint ein neuer Ordner dort, wo seine erste Spur stand.
    fn sort_key(&self, i: usize) -> usize {
        if self.tracks[i].kind != TrackKind::Folder {
            return i;
        }
        (0..self.tracks.len())
            .filter(|&c| self.is_inside(c, i))
            .fold(i, usize::min)
    }

    /// Spuren in Baumreihenfolge mit Tiefe: jeder Ordner gefolgt von seinen Spuren; Geschwister
    /// nach [`Self::sort_key`]. Spuren in zugeklappten Ordnern fehlen.
    pub fn display_order(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::with_capacity(self.tracks.len());
        let mut roots: Vec<usize> = (0..self.tracks.len())
            .filter(|&i| self.parent_of(i).is_none())
            .collect();
        roots.sort_by_key(|&i| (self.sort_key(i), i));
        let mut seen = vec![false; self.tracks.len()];
        for r in roots {
            self.push_subtree(r, 0, &mut out, &mut seen);
        }
        // Spuren in einem Zyklus (sollte `set_parent` verhindern) trotzdem zeigen.
        for (i, s) in seen.iter().enumerate() {
            if !s && !self.is_hidden(i) && !out.iter().any(|(t, _)| *t == i) {
                out.push((i, 0));
            }
        }
        out
    }

    fn push_subtree(
        &self,
        i: usize,
        depth: usize,
        out: &mut Vec<(usize, usize)>,
        seen: &mut [bool],
    ) {
        if seen[i] {
            return;
        }
        seen[i] = true;
        out.push((i, depth));
        if self.tracks[i].kind != TrackKind::Folder {
            return;
        }
        let mut children: Vec<usize> = (0..self.tracks.len())
            .filter(|&c| self.parent_of(c) == Some(i))
            .collect();
        children.sort_by_key(|&c| (self.sort_key(c), c));
        for c in children {
            if self.tracks[i].folded {
                // ausgeblendet, aber als erledigt markieren (samt Unterordnern)
                self.mark_subtree(c, seen);
            } else {
                self.push_subtree(c, depth + 1, out, seen);
            }
        }
    }

    fn mark_subtree(&self, i: usize, seen: &mut [bool]) {
        if seen[i] {
            return;
        }
        seen[i] = true;
        for c in 0..self.tracks.len() {
            if self.parent_of(c) == Some(i) {
                self.mark_subtree(c, seen);
            }
        }
    }

    /// Ob `track` in einem zugeklappten Ordner liegt.
    pub fn is_hidden(&self, track: usize) -> bool {
        self.ancestors(track).iter().any(|&a| self.tracks[a].folded)
    }

    /// Stumm durch sich selbst oder einen Ordner darüber.
    pub fn effective_mute(&self, track: usize) -> bool {
        self.tracks.get(track).is_some_and(|t| t.mute)
            || self.ancestors(track).iter().any(|&a| self.tracks[a].mute)
    }

    /// Solo durch sich selbst oder einen Ordner darüber.
    pub fn effective_solo(&self, track: usize) -> bool {
        self.tracks.get(track).is_some_and(|t| t.solo)
            || self.ancestors(track).iter().any(|&a| self.tracks[a].solo)
    }
}

#[cfg(test)]
mod tests {
    use crate::{Project, TrackKind};

    fn project() -> (Project, usize, usize, usize, usize) {
        let mut p = Project::default();
        let a = p.add_track(TrackKind::Audio, "Gesang");
        let f = p.add_track(TrackKind::Folder, "Drums");
        let k = p.add_track(TrackKind::Audio, "Kick");
        let s = p.add_track(TrackKind::Audio, "Snare");
        (p, a, f, k, s)
    }

    #[test]
    fn children_show_under_their_folder_and_fold_away() {
        let (mut p, a, f, k, s) = project();
        assert!(p.set_parent(s, Some(f)));
        assert!(p.set_parent(a, Some(f)));
        // Storage order stays a, f, k, s; the folder shows where its first track (a) was.
        assert_eq!(p.display_order(), [(f, 0), (a, 1), (s, 1), (k, 0)]);
        assert!(p.set_parent(a, None));
        assert_eq!(p.display_order(), [(a, 0), (f, 0), (s, 1), (k, 0)]);
        assert!(p.set_parent(a, Some(f)));
        p.tracks[f].folded = true;
        assert_eq!(p.display_order(), [(f, 0), (k, 0)]);
        assert!(p.is_hidden(s) && !p.is_hidden(k));
        assert!(p.set_parent(s, None));
        p.tracks[f].folded = false;
        assert_eq!(p.display_order(), [(f, 0), (a, 1), (k, 0), (s, 0)]);
    }

    #[test]
    fn folders_cannot_end_up_inside_themselves() {
        let (mut p, a, f, ..) = project();
        let inner = p.add_track(TrackKind::Folder, "Innen");
        assert!(p.set_parent(inner, Some(f)));
        assert!(!p.set_parent(f, Some(inner)), "cycle");
        assert!(!p.set_parent(f, Some(f)), "self");
        assert!(!p.set_parent(a, Some(a)), "not a folder");
        assert!(p.set_parent(a, Some(inner)));
        assert_eq!(p.ancestors(a), [inner, f]);
        let order: Vec<(usize, usize)> = p.display_order();
        assert!(order.contains(&(a, 2)));
    }

    #[test]
    fn folder_mute_and_solo_reach_every_contained_track() {
        let (mut p, a, f, k, s) = project();
        p.set_parent(k, Some(f));
        p.set_parent(s, Some(f));
        p.tracks[f].mute = true;
        assert!(!p.audible(k) && !p.audible(s) && p.audible(a));
        p.tracks[f].mute = false;
        p.tracks[f].solo = true;
        assert!(p.audible(k) && p.audible(s) && !p.audible(a));
    }
}
