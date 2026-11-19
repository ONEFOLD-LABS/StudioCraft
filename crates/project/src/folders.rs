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

    /// Platz einer Spur in der Baumreihenfolge: der beim Umsortieren vergebene Rang, sonst
    /// ihr Index; bei Ordnern ohne Rang der kleinste Platz darin. So erscheint ein neuer Ordner
    /// dort, wo seine erste Spur stand.
    fn sort_key(&self, i: usize) -> usize {
        if let Some(o) = self.tracks[i].order {
            return o as usize;
        }
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

    /// Alle Spuren in Baumreihenfolge, auch die in zugeklappten Ordnern.
    fn tree_order(&self) -> Vec<usize> {
        let mut out = Vec::with_capacity(self.tracks.len());
        let mut roots: Vec<usize> = (0..self.tracks.len())
            .filter(|&i| self.parent_of(i).is_none())
            .collect();
        roots.sort_by_key(|&i| (self.sort_key(i), i));
        let mut seen = vec![false; self.tracks.len()];
        let mut stack: Vec<usize> = roots.into_iter().rev().collect();
        while let Some(i) = stack.pop() {
            if std::mem::replace(&mut seen[i], true) {
                continue;
            }
            out.push(i);
            if self.tracks[i].kind == TrackKind::Folder {
                let mut children: Vec<usize> = (0..self.tracks.len())
                    .filter(|&c| self.parent_of(c) == Some(i))
                    .collect();
                children.sort_by_key(|&c| (self.sort_key(c), c));
                stack.extend(children.into_iter().rev());
            }
        }
        out.extend((0..self.tracks.len()).filter(|&i| !seen[i]));
        out
    }

    /// Spur `track` in der Spurliste vor `before` einordnen (`None`: ans Ende) und in den
    /// Ordner `folder` legen (`None`: oberste Ebene). Ein Ordner nimmt seine Spuren mit. Die
    /// Reihenfolge in `tracks` bleibt; nur die Ränge der Anzeige (`Track::order`) ändern sich.
    /// Abgelehnt, wenn ein Ordner in sich selbst landen würde.
    pub fn move_track(
        &mut self,
        track: usize,
        before: Option<usize>,
        folder: Option<usize>,
    ) -> bool {
        let n = self.tracks.len();
        if track >= n || before.is_some_and(|b| b >= n || b == track) {
            return false;
        }
        if let Some(f) = folder
            && (f == track
                || f >= n
                || self.tracks[f].kind != TrackKind::Folder
                || self.is_inside(f, track))
        {
            return false;
        }
        let mut order = self.tree_order();
        order.retain(|&i| i != track);
        let at = before
            .and_then(|b| order.iter().position(|&i| i == b))
            .unwrap_or(order.len());
        order.insert(at, track);
        self.tracks[track].parent = folder.map(|f| self.tracks[f].id);
        for (rank, &i) in order.iter().enumerate() {
            self.tracks[i].order = u32::try_from(rank).ok();
        }
        true
    }

    /// Verschiebt alle Events der Spuren `tracks` um `delta` Ticks (Ordnerbereich ziehen).
    /// Nach links höchstens bis 0; gibt die tatsächlich angewandte Verschiebung zurück.
    pub fn shift_tracks(&mut self, tracks: &[usize], delta: i64) -> i64 {
        let earliest = tracks
            .iter()
            .filter_map(|&t| self.tracks.get(t))
            .flat_map(|t| t.events.iter().map(|e| e.start.0))
            .min();
        let Some(earliest) = earliest else {
            return 0;
        };
        let delta = delta.max(-(earliest.min(i64::MAX as u64) as i64));
        for &t in tracks {
            if let Some(track) = self.tracks.get_mut(t) {
                for e in &mut track.events {
                    e.start.0 = e.start.0.saturating_add_signed(delta);
                }
            }
        }
        delta
    }

    /// Alle Spuren in `folder`, auch in Unterordnern.
    pub fn tracks_inside(&self, folder: usize) -> Vec<usize> {
        (0..self.tracks.len())
            .filter(|&t| self.is_inside(t, folder))
            .collect()
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

    fn shown(p: &Project) -> Vec<usize> {
        p.display_order().iter().map(|(t, _)| *t).collect()
    }

    #[test]
    fn move_track_reorders_and_changes_folder() {
        let (mut p, a, f, k, s) = project();
        assert!(p.set_parent(k, Some(f)));
        assert_eq!(shown(&p), [a, f, k, s]);
        // Snare into the folder, before Kick.
        assert!(p.move_track(s, Some(k), Some(f)));
        assert_eq!(shown(&p), [a, f, s, k]);
        assert_eq!(p.parent_of(s), Some(f));
        // Gesang to the end, top level.
        assert!(p.move_track(a, None, None));
        assert_eq!(shown(&p), [f, s, k, a]);
        // Kick out of the folder, before the folder: storage order never changes.
        assert!(p.move_track(k, Some(f), None));
        assert_eq!(shown(&p), [k, f, s, a]);
        assert_eq!(
            p.tracks.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["Gesang", "Drums", "Kick", "Snare"]
        );
    }

    #[test]
    fn a_folder_cannot_move_into_itself() {
        let (mut p, _a, f, k, _s) = project();
        let inner = p.add_track(TrackKind::Folder, "Toms");
        assert!(p.set_parent(inner, Some(f)));
        assert!(!p.move_track(f, None, Some(f)));
        assert!(!p.move_track(f, None, Some(inner)));
        assert!(!p.move_track(k, Some(k), None));
        // Moving a folder takes its tracks along.
        assert!(p.set_parent(k, Some(f)));
        assert!(p.move_track(f, None, None));
        let order = shown(&p);
        let pos = |t| order.iter().position(|&x| x == t).unwrap();
        assert!(pos(f) < pos(inner) && pos(f) < pos(k));
    }

    #[test]
    fn shift_tracks_moves_events_and_stops_at_zero() {
        let (mut p, _a, f, k, s) = project();
        p.set_parent(k, Some(f));
        p.set_parent(s, Some(f));
        let e1 = p.create_event(k, 960, 960).unwrap();
        let e2 = p.create_event(s, 1920, 960).unwrap();
        let inside = p.tracks_inside(f);
        assert_eq!(inside, [k, s]);
        assert_eq!(p.shift_tracks(&inside, 480), 480);
        assert_eq!(p.event(e1).unwrap().start.0, 1440);
        // Left only as far as the earliest event can go.
        assert_eq!(p.shift_tracks(&inside, -5000), -1440);
        assert_eq!(p.event(e1).unwrap().start.0, 0);
        assert_eq!(p.event(e2).unwrap().start.0, 960);
    }
}
