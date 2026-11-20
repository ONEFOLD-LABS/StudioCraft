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
use studiocraft_engine::Command;
use studiocraft_project::{EventId, ProcessKind};

use crate::{LowerZone, ProjectWindow};

/// Breite der Liste der Bearbeitungen rechts.
const PROCESS_LIST_W: f32 = 240.0;

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
    /// Gezogener Warp-Marker (Index im Event).
    warp_drag: Option<usize>,
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
            warp_drag: None,
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
        // Folgt das Event dem Tempo, zählt seine Abbildung (Tempo der Datei, Warp-Marker).
        if ev.musical.is_some() {
            let len = i64::try_from(ev.length).unwrap_or(i64::MAX);
            return Some((ev.offset, ev.shifted_offset(&self.project.tempo, rate, len)));
        }
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

    /// Hängt `kind` für die Auswahl (ohne Auswahl den Teil des Events) an die Bearbeitungen
    /// des Clips an: neuer Clip im Pool, das Event zeigt darauf (`processes_ui.rs`).
    pub(crate) fn process_clip(&mut self, id: EventId, kind: ProcessKind) -> Result<(), String> {
        let range = self.target_range(id).ok_or(tr("kein Bereich"))?;
        self.add_process(id, kind, range)
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
        // Folgt das Event dem Tempo, gilt seine Abbildung ab dem Event-Anfang.
        if let Some(m) = ev.content_map(rate) {
            let t = ev.start.0 as f64 + m.tick_at(f as f64) - m.tick_at(ev.offset as f64);
            return Some(if t.is_finite() && t > 0.0 {
                t.round() as u64
            } else {
                0
            });
        }
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

    /// Setzt an jedem Hitpoint im Event einen Warp-Marker (ein Rückgängig-Schritt): jeder
    /// Anschlag rückt auf die nächste Sechzehntel. Gibt die Zahl der neuen Marker zurück.
    pub(crate) fn warp_at_hitpoints(&mut self, id: EventId) -> usize {
        let hits = self.hitpoints_in_event(id);
        let before = self.project.clone();
        let added = hits
            .into_iter()
            .filter(|&h| self.project.add_warp(id, h).is_some())
            .count();
        self.checkpoint(before);
        added
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
    let mut action: Option<ProcessKind> = None;
    let mut to_event = false;
    let mut detect = false;
    let mut slice = false;
    let mut find_tempo = false;
    let mut use_tempo: Option<f64> = None;
    let (mut warp_hits, mut warp_clear) = (false, false);
    let mut warp_add: Option<u64> = None;
    let mut warp_move: Option<(usize, u64)> = None;
    let mut warp_remove: Option<usize> = None;
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
        for p in [
            ProcessKind::Normalize { target_db: 0.0 },
            ProcessKind::Reverse,
            ProcessKind::FadeIn,
            ProcessKind::FadeOut,
            ProcessKind::Silence,
            ProcessKind::InvertPhase,
            ProcessKind::RemoveDc,
        ] {
            if ui.button(tr(p.label())).clicked() {
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
            action = Some(ProcessKind::Gain {
                db: w.sample_editor.gain_db,
            });
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
        // Warp-Marker (nur für Audio, das dem Tempo folgt; Issue #55).
        let warps = w.project.event(id).map_or(0, |e| e.warp.len());
        if w.project.event(id).is_some_and(|e| e.musical.is_some()) {
            ui.separator();
            if ui
                .add_enabled(in_event > 0, egui::Button::new(tr("Warp an Hitpoints")))
                .on_hover_text(tr("Jeder Anschlag rückt auf die nächste Sechzehntel"))
                .clicked()
            {
                warp_hits = true;
            }
            if ui
                .add_enabled(warps > 0, egui::Button::new(tr("Warp löschen")))
                .clicked()
            {
                warp_clear = true;
            }
            ui.weak(crate::i18n::trf("{n} Warp-Marker", &[("n", &warps)]))
                .on_hover_text(tr(
                    "Doppelklick in die Markerleiste setzt einen Marker, Ziehen verschiebt ihn, Rechtsklick löscht ihn",
                ));
        }
    });
    // Rechts die Liste der Bearbeitungen, links die Wellenform.
    let full = ui.available_rect_before_wrap();
    let list_rect = Rect::from_min_max(
        egui::pos2(full.right() - PROCESS_LIST_W, full.top()),
        full.max,
    );
    let (rect, resp) = ui.allocate_exact_size(
        vec2(
            (ui.available_width() - PROCESS_LIST_W - 8.0).max(80.0),
            ui.available_height().max(80.0),
        ),
        Sense::click_and_drag(),
    );
    ui.scope_builder(egui::UiBuilder::new().max_rect(list_rect), |ui| {
        egui::ScrollArea::vertical()
            .id_salt("bearbeitungen")
            .show(ui, |ui| w.processes_panel(ui, id));
    });
    // Markerleiste oben, nur für Audio, das dem Tempo folgt.
    let warps: Vec<u64> = w
        .project
        .event(id)
        .filter(|e| e.musical.is_some())
        .map(|e| e.warp.iter().map(|p| p.frame).collect())
        .unwrap_or_default();
    let musical = w.project.event(id).is_some_and(|e| e.musical.is_some());
    let strip = musical
        .then(|| Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.top() + WARP_H)));
    let in_strip = |p: egui::Pos2| strip.is_some_and(|s| s.contains(p));
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
    // Marker unter der Maus (in der Leiste, höchstens 6 Pixel daneben).
    let marker_at = |p: egui::Pos2, ed: &SampleEditor| -> Option<usize> {
        if !in_strip(p) {
            return None;
        }
        warps
            .iter()
            .enumerate()
            .map(|(i, &f)| {
                (
                    i,
                    (rect.left() + ((f as f64 - ed.scroll) / ed.per_px) as f32 - p.x).abs(),
                )
            })
            .filter(|(_, d)| *d <= 6.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    };
    if resp.drag_started()
        && let Some(p) = resp.interact_pointer_pos()
    {
        ed.warp_drag = marker_at(p, ed);
        if ed.warp_drag.is_none() {
            ed.drag_from = Some(frame_at(p.x, ed));
        }
    }
    if resp.dragged()
        && let Some(p) = resp.interact_pointer_pos()
    {
        if let Some(i) = ed.warp_drag {
            warp_move = Some((i, frame_at(p.x, ed)));
        } else if let Some(from) = ed.drag_from {
            let to = frame_at(p.x, ed);
            ed.selection = Some((from.min(to), from.max(to)));
        }
    }
    if resp.drag_stopped() {
        ed.drag_from = None;
        ed.warp_drag = None;
    }
    let pointer = resp.interact_pointer_pos();
    if resp.double_clicked()
        && let Some(p) = pointer.filter(|p| in_strip(*p))
    {
        warp_add = Some(frame_at(p.x, ed));
    } else if resp.clicked() && !pointer.is_some_and(in_strip) {
        ed.selection = None;
    }
    if resp.secondary_clicked()
        && let Some(p) = pointer
    {
        warp_remove = marker_at(p, ed);
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
    if warp_hits {
        w.warp_at_hitpoints(id);
    }
    if warp_clear || warp_add.is_some() || warp_remove.is_some() {
        let before = w.project.clone();
        if warp_clear && let Some(ev) = w.project.event_mut(id) {
            ev.warp.clear();
        }
        if let Some(f) = warp_add {
            w.project.add_warp(id, f);
        }
        if let Some(i) = warp_remove {
            w.project.remove_warp(id, i);
        }
        w.checkpoint(before);
    }
    // Ziehen ist ein Schritt über das Panel-Undo der unteren Zone.
    if let Some((i, f)) = warp_move
        && w.project.move_warp(id, i, f)
    {
        w.dirty = true;
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
    paint_warp(w, &p, rect, id, &x_of);
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

/// Höhe der Leiste mit den Warp-Markern.
const WARP_H: f32 = 16.0;

/// Warp-Marker: Leiste oben, je Marker ein Dreieck mit Linie und der Taktposition.
fn paint_warp(
    w: &ProjectWindow,
    p: &egui::Painter,
    rect: Rect,
    id: EventId,
    x_of: &dyn Fn(u64) -> f32,
) {
    let Some(ev) = w.project.event(id).filter(|e| e.musical.is_some()) else {
        return;
    };
    let t = crate::theme::tokens(p.ctx());
    let strip = Rect::from_min_max(rect.min, egui::pos2(rect.right(), rect.top() + WARP_H));
    p.rect_filled(strip, 0.0, t.lane_alt);
    let rate = ev
        .clip
        .and_then(|c| w.project.pool.get(c as usize))
        .map_or(48_000, |c| c.sample_rate);
    let Some(m) = ev.content_map(rate) else {
        return;
    };
    let origin = m.tick_at(ev.offset as f64) - ev.start.0 as f64;
    let font = egui::FontId::proportional(10.0);
    for wp in &ev.warp {
        let x = x_of(wp.frame);
        if !rect.x_range().contains(x) {
            continue;
        }
        p.vline(x, rect.y_range(), Stroke::new(1.0, t.accent));
        let tip = egui::pos2(x, strip.bottom());
        p.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(x - 5.0, strip.top() + 2.0),
                egui::pos2(x + 5.0, strip.top() + 2.0),
                tip,
            ],
            t.accent,
            Stroke::NONE,
        ));
        let at = (wp.tick as f64 - origin).max(0.0) as u64;
        let label = w
            .project
            .tempo
            .format_position(studiocraft_project::Position(at));
        p.text(
            egui::pos2(x + 7.0, strip.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            font.clone(),
            t.text,
        );
    }
}
