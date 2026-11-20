//! Sample-Editor in der unteren Zone (Spec „Editoren“; Issue #18): die Wellenform des ganzen
//! Clips eines Audio-Events, der vom Event genutzte Teil hervorgehoben. Ziehen wählt einen
//! Bereich; den kann man vorhören, zum Event machen oder offline bearbeiten (Normalisieren,
//! Gain, Umkehren, Fade-In, Fade-Out).
//!
//! Bearbeiten ist nicht-destruktiv: Das Ergebnis wird ein neuer Clip im Pool, auf den nur
//! dieses Event zeigt; das Original bleibt, Rückgängig stellt den alten Zustand her. Ohne
//! Auswahl wirkt eine Bearbeitung auf den Teil des Clips, den das Event nutzt.
//!
//! Hitpoints und Tempo (Issue #41, `studiocraft_engine::onsets`): „Hitpoints erkennen“ findet
//! die Einsätze im Clip (Empfindlichkeit einstellbar) und zeigt sie als Linien; „In Slices
//! teilen“ teilt das Event an jedem Hitpoint in seinem Bereich. „Tempo erkennen“ schätzt das
//! Tempo der Auswahl bzw. des Events; „Als Projekttempo“ übernimmt es.

use crate::i18n::tr;
use egui::{Rect, Sense, Stroke, StrokeKind, Ui, vec2};
use studiocraft_engine::{Command, offline};
use studiocraft_project::{EventId, PoolClip};

use crate::waveform::Peaks;
use crate::{LoadedClip, LowerZone, ProjectWindow};

/// Zustand des Sample-Editors.
#[derive(Debug, Clone)]
pub(crate) struct SampleEditor {
    pub event: Option<EventId>,
    /// Sichtbarer Ausschnitt in Frames des Clips: Anfang und Frames pro Pixel (0 = einpassen).
    pub scroll: f64,
    pub per_px: f64,
    /// Auswahl in Frames des Clips.
    pub selection: Option<(u64, u64)>,
    drag_from: Option<u64>,
    /// Gain für „Gain anwenden“ in dB.
    pub gain_db: f32,
    /// Erkannte Hitpoints in Frames des Clips und der Clip, für den sie gelten.
    pub hitpoints: Vec<u64>,
    hit_clip: Option<u32>,
    /// Empfindlichkeit der Erkennung, 0–1.
    pub sensitivity: f32,
    /// Erkanntes Tempo in BPM (`Some(None)`: kein Puls gefunden).
    pub tempo: Option<Option<f64>>,
}

impl Default for SampleEditor {
    fn default() -> Self {
        Self {
            event: None,
            scroll: 0.0,
            per_px: 0.0,
            selection: None,
            drag_from: None,
            gain_db: 3.0,
            hitpoints: Vec::new(),
            hit_clip: None,
            sensitivity: 0.5,
            tempo: None,
        }
    }
}

/// Eine Offline-Bearbeitung.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Process {
    Normalize,
    Gain(f32),
    Reverse,
    FadeIn,
    FadeOut,
}

impl Process {
    fn label(self) -> &'static str {
        match self {
            Process::Normalize => tr("normalisiert"),
            Process::Gain(_) => tr("Gain"),
            Process::Reverse => tr("umgekehrt"),
            Process::FadeIn => tr("Fade-In"),
            Process::FadeOut => tr("Fade-Out"),
        }
    }
}

impl ProjectWindow {
    /// Öffnet den Sample-Editor für ein Audio-Event in der unteren Zone.
    pub(crate) fn open_sample_editor(&mut self, id: EventId) {
        if self.project.event(id).and_then(|e| e.clip).is_none() {
            return;
        }
        self.sample_editor = SampleEditor {
            event: Some(id),
            gain_db: self.sample_editor.gain_db,
            sensitivity: self.sample_editor.sensitivity,
            ..SampleEditor::default()
        };
        self.show_lower = true;
        self.lower = LowerZone::SampleEditor;
    }

    /// Frames, die das Event aus seinem Clip nutzt.
    fn event_frames(&self, id: EventId) -> Option<(u64, u64)> {
        let ev = self.project.event(id)?;
        let rate = self.project.pool.get(ev.clip? as usize)?.sample_rate;
        let len = self
            .project
            .tempo
            .duration_frames(ev.start.0, ev.length, rate);
        Some((ev.offset, ev.offset.saturating_add(len)))
    }

    /// Auswahl oder, ohne Auswahl, der vom Event genutzte Teil.
    fn target_range(&self, id: EventId) -> Option<(u64, u64)> {
        self.sample_editor
            .selection
            .filter(|(a, b)| b > a)
            .or_else(|| self.event_frames(id))
    }

    /// Wendet `p` auf den Clip des Events an: neuer Clip im Pool, das Event zeigt darauf.
    pub(crate) fn process_clip(&mut self, id: EventId, p: Process) -> Result<(), String> {
        let ev = self.project.event(id).ok_or(tr("Event fehlt"))?;
        let clip_id = ev.clip.ok_or(tr("kein Audio-Event"))?;
        let (a, b) = self.target_range(id).ok_or(tr("kein Bereich"))?;
        let pool = self
            .project
            .pool
            .get(clip_id as usize)
            .cloned()
            .ok_or(tr("Clip fehlt im Pool"))?;
        let loaded = self.clip(clip_id).ok_or(tr("Clip ist nicht geladen"))?;
        let mut samples = loaded.samples.to_vec();
        match p {
            Process::Normalize => {
                offline::normalize(&mut samples, a..b, 0.0);
            }
            Process::Gain(db) => offline::gain(&mut samples, a..b, db),
            Process::Reverse => offline::reverse(&mut samples, a..b),
            Process::FadeIn => offline::fade_in(&mut samples, a..b),
            Process::FadeOut => offline::fade_out(&mut samples, a..b),
        }
        let before = self.project.clone();
        let name = format!("{} ({})", pool.name, p.label());
        let new_id = self.project.add_clip(PoolClip {
            name,
            path: None,
            sample_rate: pool.sample_rate,
            frames: (samples.len() / 2) as u64,
        });
        let samples: std::sync::Arc<[f32]> = samples.into();
        self.clips.insert(
            new_id,
            LoadedClip {
                peaks: Peaks::new(&samples),
                samples,
            },
        );
        if let Some(e) = self.project.event_mut(id) {
            e.clip = Some(new_id);
        }
        self.checkpoint(before);
        Ok(())
    }

    /// Macht die Auswahl zum Event: Anfang und Ende folgen ihr, das Audio bleibt an seinem Platz.
    pub(crate) fn event_to_selection(&mut self, id: EventId) {
        let Some((a, b)) = self.sample_editor.selection.filter(|(a, b)| b > a) else {
            return;
        };
        let (Some(tb), Some(ta)) = (self.clip_frame_tick(id, b), self.clip_frame_tick(id, a))
        else {
            return;
        };
        let before = self.project.clone();
        self.project.trim_end(id, tb);
        self.project.trim_start(id, ta);
        self.checkpoint(before);
    }

    /// Tick auf der Zeitachse, an dem Frame `f` des Clips von Event `id` klingt.
    fn clip_frame_tick(&self, id: EventId, f: u64) -> Option<u64> {
        let ev = self.project.event(id)?;
        let rate = self.project.pool.get(ev.clip? as usize)?.sample_rate;
        let tempo = &self.project.tempo;
        // Über die Zeitachse in Frames, damit Tempowechsel stimmen.
        let start_f = tempo.frames(ev.start.0, rate);
        Some(if f >= ev.offset {
            tempo.ticks(start_f.saturating_add(f - ev.offset), rate)
        } else {
            tempo.ticks(start_f.saturating_sub(ev.offset - f), rate)
        })
    }

    /// Sucht Hitpoints im ganzen Clip des Events.
    pub(crate) fn detect_hitpoints(&mut self, id: EventId) {
        let Some(clip) = self.project.event(id).and_then(|e| e.clip) else {
            return;
        };
        let Some(rate) = self.project.pool.get(clip as usize).map(|c| c.sample_rate) else {
            return;
        };
        let Some(samples) = self.clip(clip).map(|c| c.samples.clone()) else {
            return;
        };
        let ed = &mut self.sample_editor;
        ed.hitpoints = studiocraft_engine::onsets::hitpoints(&samples, rate, ed.sensitivity);
        ed.hit_clip = Some(clip);
    }

    /// Hitpoints, die im vom Event genutzten Teil liegen (ohne dessen Anfang).
    fn hitpoints_in_event(&self, id: EventId) -> Vec<u64> {
        let clip = self.project.event(id).and_then(|e| e.clip);
        if clip.is_none() || clip != self.sample_editor.hit_clip {
            return Vec::new();
        }
        let Some((a, b)) = self.event_frames(id) else {
            return Vec::new();
        };
        self.sample_editor
            .hitpoints
            .iter()
            .copied()
            .filter(|&h| h > a && h < b)
            .collect()
    }

    /// Teilt das Event an seinen Hitpoints (ein Rückgängig-Schritt). Gibt die Zahl der Slices
    /// zurück.
    pub(crate) fn slice_at_hitpoints(&mut self, id: EventId) -> usize {
        let ticks: Vec<u64> = self
            .hitpoints_in_event(id)
            .into_iter()
            .filter_map(|h| self.clip_frame_tick(id, h))
            .collect();
        if ticks.is_empty() {
            return 1;
        }
        let before = self.project.clone();
        let mut current = id;
        let mut slices = 1;
        for t in ticks {
            if let Some(right) = self.project.split(current, t) {
                current = right;
                slices += 1;
            }
        }
        self.checkpoint(before);
        slices
    }

    /// Schätzt das Tempo der Auswahl bzw. des vom Event genutzten Teils.
    pub(crate) fn detect_tempo(&mut self, id: EventId) {
        let range = self.target_range(id);
        let clip = self.project.event(id).and_then(|e| e.clip);
        let rate = clip
            .and_then(|c| self.project.pool.get(c as usize))
            .map(|c| c.sample_rate);
        let samples = clip.and_then(|c| self.clip(c)).map(|c| c.samples.clone());
        let found = match (range, rate, samples) {
            (Some((a, b)), Some(rate), Some(s)) => {
                let n = s.len() / 2;
                let (a, b) = ((a as usize).min(n), (b as usize).min(n));
                studiocraft_engine::onsets::tempo(&s[a * 2..b * 2], rate)
            }
            _ => None,
        };
        self.sample_editor.tempo = Some(found);
    }

    /// Hört die Auswahl (ohne Auswahl den Teil des Events) vor.
    fn preview_range(&mut self, id: EventId) {
        let Some((a, b)) = self.target_range(id) else {
            return;
        };
        let Some(clip) = self.project.event(id).and_then(|e| e.clip) else {
            return;
        };
        let Some(loaded) = self.clip(clip) else {
            return;
        };
        let n = loaded.samples.len() / 2;
        let (a, b) = ((a as usize).min(n), (b as usize).min(n));
        let part: std::sync::Arc<[f32]> = loaded.samples[a * 2..b * 2].to_vec().into();
        self.send(Command::Preview(Some(part)));
    }
}

/// Zeichnet den Sample-Editor.
pub(crate) fn show(w: &mut ProjectWindow, ui: &mut Ui) {
    let Some(id) = w
        .sample_editor
        .event
        .filter(|&id| w.project.event(id).is_some())
    else {
        ui.weak(tr("Doppelklick auf ein Audio-Event öffnet es hier."));
        return;
    };
    let Some(clip_id) = w.project.event(id).and_then(|e| e.clip) else {
        return;
    };
    let rate = w
        .project
        .pool
        .get(clip_id as usize)
        .map_or(48_000, |c| c.sample_rate.max(1));
    let total = w.clip(clip_id).map_or(0, |c| c.samples.len() / 2) as f64;
    let mut action: Option<Process> = None;
    let mut to_event = false;
    let mut detect = false;
    let mut slice = false;
    let mut find_tempo = false;
    let mut use_tempo: Option<f64> = None;
    let mut play = false;
    let mut stop = false;
    ui.horizontal_wrapped(|ui| {
        if ui
            .button(tr("Abspielen"))
            .on_hover_text(tr("Auswahl vorhören"))
            .clicked()
        {
            play = true;
        }
        if ui.button(tr("Stopp")).clicked() {
            stop = true;
        }
        ui.separator();
        if ui
            .add_enabled(
                w.sample_editor.selection.is_some_and(|(a, b)| b > a),
                egui::Button::new(tr("Event auf Auswahl")),
            )
            .clicked()
        {
            to_event = true;
        }
        ui.separator();
        for (label, p) in [
            (tr("Normalisieren"), Process::Normalize),
            (tr("Umkehren"), Process::Reverse),
            (tr("Fade-In"), Process::FadeIn),
            (tr("Fade-Out"), Process::FadeOut),
        ] {
            if ui.button(label).clicked() {
                action = Some(p);
            }
        }
        let l = ui.weak(tr("dB"));
        ui.add(
            egui::DragValue::new(&mut w.sample_editor.gain_db)
                .range(-24.0..=24.0)
                .speed(0.1),
        )
        .labelled_by(l.id);
        if ui.button(tr("Gain")).clicked() {
            action = Some(Process::Gain(w.sample_editor.gain_db));
        }
        ui.separator();
        if ui.button(tr("Einpassen")).clicked() {
            w.sample_editor.per_px = 0.0;
            w.sample_editor.scroll = 0.0;
        }
        if let Some((a, b)) = w.sample_editor.selection {
            ui.weak(crate::i18n::trf(
                "Auswahl {a}–{b} s",
                &[
                    ("a", &format!("{:.3}", a as f64 / f64::from(rate))),
                    ("b", &format!("{:.3}", b as f64 / f64::from(rate))),
                ],
            ));
        }
    });
    // Zweite Zeile: Hitpoints und Tempo.
    let in_event = w.hitpoints_in_event(id).len();
    ui.horizontal_wrapped(|ui| {
        let l = ui.weak(tr("Empfindlichkeit"));
        let r = ui
            .add(egui::Slider::new(&mut w.sample_editor.sensitivity, 0.0..=1.0).show_value(false))
            .labelled_by(l.id);
        // Gibt es schon Hitpoints, folgen sie dem Regler.
        if r.changed() && w.sample_editor.hit_clip.is_some() {
            detect = true;
        }
        if ui.button(tr("Hitpoints erkennen")).clicked() {
            detect = true;
        }
        if ui
            .add_enabled(in_event > 0, egui::Button::new(tr("In Slices teilen")))
            .on_hover_text(crate::i18n::trf(
                "Teilt das Event an {n} Hitpoints",
                &[("n", &in_event)],
            ))
            .clicked()
        {
            slice = true;
        }
        if w.sample_editor.hit_clip.is_some() {
            ui.weak(crate::i18n::trf("{n} Hitpoints", &[("n", &in_event)]));
        }
        ui.separator();
        if ui
            .button(tr("Tempo erkennen"))
            .on_hover_text(tr("Schätzt das Tempo der Auswahl bzw. des Events"))
            .clicked()
        {
            find_tempo = true;
        }
        match w.sample_editor.tempo {
            Some(Some(bpm)) => {
                ui.label(crate::i18n::trf(
                    "≈ {bpm} BPM",
                    &[("bpm", &format!("{bpm:.1}"))],
                ));
                if ui.button(tr("Als Projekttempo")).clicked() {
                    use_tempo = Some(bpm);
                }
            }
            Some(None) => {
                ui.weak(tr("kein Tempo gefunden"));
            }
            None => {}
        }
    });
    let (rect, resp) = ui.allocate_exact_size(
        vec2(ui.available_width(), ui.available_height().max(80.0)),
        Sense::click_and_drag(),
    );
    let ed = &mut w.sample_editor;
    if ed.per_px <= 0.0 {
        ed.per_px = (total / f64::from(rect.width().max(1.0))).max(0.01);
        ed.scroll = 0.0;
    }
    // Mausrad: zoomen um die Maus; mit Umschalt waagerecht blättern.
    if resp.hovered() {
        let (scroll, shift, hover) = ui.input(|i| {
            (
                i.smooth_scroll_delta,
                i.modifiers.shift,
                i.pointer.hover_pos(),
            )
        });
        if shift || scroll.x != 0.0 {
            ed.scroll -= f64::from(scroll.x + if shift { scroll.y } else { 0.0 }) * ed.per_px;
        } else if scroll.y != 0.0
            && let Some(h) = hover
        {
            let anchor = ed.scroll + f64::from(h.x - rect.left()) * ed.per_px;
            ed.per_px =
                (ed.per_px * (1.0 - f64::from(scroll.y) * 0.002)).clamp(0.01, total.max(1.0));
            ed.scroll = anchor - f64::from(h.x - rect.left()) * ed.per_px;
        }
        ed.scroll = ed
            .scroll
            .clamp(0.0, (total - f64::from(rect.width()) * ed.per_px).max(0.0));
    }
    let frame_at = |x: f32, ed: &SampleEditor| -> u64 {
        (ed.scroll + f64::from(x - rect.left()) * ed.per_px).clamp(0.0, total) as u64
    };
    if resp.drag_started()
        && let Some(p) = resp.interact_pointer_pos()
    {
        ed.drag_from = Some(frame_at(p.x, ed));
    }
    if resp.dragged()
        && let (Some(from), Some(p)) = (ed.drag_from, resp.interact_pointer_pos())
    {
        let to = frame_at(p.x, ed);
        ed.selection = Some((from.min(to), from.max(to)));
    }
    if resp.drag_stopped() {
        ed.drag_from = None;
    }
    if resp.clicked() {
        ed.selection = None;
    }
    paint(w, ui, rect, id, clip_id);
    if play {
        w.preview_range(id);
    }
    if stop {
        w.send(Command::Preview(None));
    }
    if to_event {
        w.event_to_selection(id);
    }
    if detect {
        w.detect_hitpoints(id);
    }
    if slice {
        w.slice_at_hitpoints(id);
    }
    if find_tempo {
        w.detect_tempo(id);
    }
    if let Some(bpm) = use_tempo {
        let before = w.project.clone();
        w.project.set_tempo((bpm * 100.0).round() / 100.0);
        w.checkpoint(before);
        w.dirty = true;
    }
    if let Some(p) = action
        && let Err(e) = w.process_clip(id, p)
    {
        w.message = Some(crate::i18n::trf("Sample-Editor: {e}", &[("e", &e)]));
    }
}

fn paint(w: &ProjectWindow, ui: &Ui, rect: Rect, id: EventId, clip_id: u32) {
    let t = crate::theme::tokens(ui.ctx());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, t.lane_bg);
    let ed = &w.sample_editor;
    let x_of = |f: u64| rect.left() + ((f as f64 - ed.scroll) / ed.per_px) as f32;
    let color = w
        .project
        .find(id)
        .map(|(tr, _)| w.project.tracks[tr].color)
        .map_or(t.accent, |c| egui::Color32::from_rgb(c[0], c[1], c[2]));
    // Vom Event genutzter Teil heller hinterlegt.
    if let Some((a, b)) = w.event_frames(id) {
        let r = Rect::from_x_y_ranges(x_of(a)..=x_of(b), rect.y_range());
        p.rect_filled(r.intersect(rect), 0.0, t.lane_alt);
    }
    if let Some((a, b)) = ed.selection {
        let r = Rect::from_x_y_ranges(x_of(a)..=x_of(b).max(x_of(a) + 1.0), rect.y_range());
        p.rect_filled(r.intersect(rect), 0.0, t.selection);
        p.rect_stroke(
            r.intersect(rect),
            0.0,
            Stroke::new(1.0, t.accent),
            StrokeKind::Inside,
        );
    }
    // Hitpoints als feine Linien über die ganze Höhe.
    if ed.hit_clip == Some(clip_id) {
        for &h in &ed.hitpoints {
            let x = x_of(h);
            if rect.x_range().contains(x) {
                p.vline(
                    x,
                    rect.y_range(),
                    Stroke::new(1.0, t.playhead.gamma_multiply(0.7)),
                );
            }
        }
    }
    let mid = rect.center().y;
    let half = rect.height() * 0.45;
    p.hline(rect.x_range(), mid, Stroke::new(1.0, t.lane_alt));
    if let Some(c) = w.clip(clip_id) {
        let mut x = rect.left();
        while x < rect.right() {
            let f0 = ed.scroll + f64::from(x - rect.left()) * ed.per_px;
            let (lo, hi) = c.peaks.range(f0, f0 + ed.per_px.max(1.0));
            p.vline(
                x,
                mid - hi.clamp(-1.0, 1.0) * half..=mid - lo.clamp(-1.0, 1.0) * half,
                Stroke::new(1.0, color),
            );
            x += 1.0;
        }
    }
}
