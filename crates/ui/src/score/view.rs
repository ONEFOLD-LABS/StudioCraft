//! Zeichnen der Partitur (Issue #40) mit der Notenschrift Leland (SMuFL, SIL OFL 1.1, siehe
//! `assets/fonts/leland`). Maße in Zwischenräumen (`sp`): Die Schrift ist so gebaut, dass ein
//! Geviert vier Zwischenräume hoch ist und die Grundlinie eines Zeichens auf seiner Linie liegt.
//!
//! Nur lesend: Werkzeugzeile mit Anzeige-Quantisierung und Tonart, darunter die Systeme. Es
//! werden nur die sichtbaren Systeme gezeichnet; der Satz selbst wird nur neu berechnet, wenn
//! sich Part, Tonart, Taktarten oder Quantisierung ändern.

use egui::{Color32, FontFamily, FontId, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2, pos2};
use studiocraft_project::{
    EventId, KeySignature, Note, SCORE_QUANTIZE, TICKS_PER_QUARTER, TempoMap,
};

use super::canvas::Canvas;
use super::notation::{self, Accidental, Base, Clef, Item, Kind, Score};
use crate::ProjectWindow;
use crate::i18n::{tr, trf};

/// Name der Schriftfamilie in egui.
pub(crate) const FAMILY: &str = "Leland";
pub(crate) static FONT: &[u8] = include_bytes!("../../../../assets/fonts/leland/Leland.otf");

/// Größe eines Zwischenraums in Punkten.
pub(crate) const SP: f32 = 8.0;
/// Abstand der Notensysteme eines Klaviersystems (Oberkante zu Oberkante) in `sp`.
const STAFF_GAP: f32 = 11.0;
/// Rand über und unter einem System für Hilfslinien in `sp`.
pub(crate) const PAD: f32 = 7.0;

// SMuFL-Zeichen (Standard Music Font Layout).
const G_CLEF: char = '\u{E050}';
const F_CLEF: char = '\u{E062}';
const BRACE: char = '\u{E000}';
const HEAD_WHOLE: char = '\u{E0A2}';
const HEAD_HALF: char = '\u{E0A3}';
const HEAD_BLACK: char = '\u{E0A4}';
const SHARP: char = '\u{E262}';
const FLAT: char = '\u{E260}';
const NATURAL: char = '\u{E261}';
const DOT: char = '\u{E1E7}';
const TIME_DIGIT_0: u32 = 0xE080;

/// Breiten aus den Metadaten der Schrift (in `sp`).
const HEAD_W: f32 = 1.3;
const WHOLE_W: f32 = 1.49;
const STEM_LEN: f32 = 3.5;

/// Gemerkter Satz: berechnet aus diesen Eingaben.
pub(crate) struct ScoreView {
    font_ready: bool,
    cache: Option<Cached>,
    /// Notenwert zum Zeichnen in Ticks und punktiert.
    value: u64,
    dotted: bool,
    drag: Option<Drag>,
    /// Lage von Notenköpfen und Takten aus dem letzten Bild (Bedienung, Tests).
    pub(crate) last: Frame,
    /// Dialog „Noten exportieren“.
    pub(crate) export: super::export::ExportDialog,
}

impl Default for ScoreView {
    fn default() -> Self {
        Self {
            font_ready: false,
            cache: None,
            value: TICKS_PER_QUARTER,
            dotted: false,
            drag: None,
            last: Frame::default(),
            export: super::export::ExportDialog::default(),
        }
    }
}

/// Ziehen gewählter Noten: Projekt davor (ein Rückgängig-Schritt), Startpunkt in Ticks und
/// diatonischer Stufe.
struct Drag {
    before: studiocraft_project::Project,
    part: EventId,
    notes: Vec<usize>,
    tick: u64,
    step: i32,
}

/// Notenwerte zum Zeichnen: Ganze bis Zweiunddreißigstel.
const VALUES: [(u64, &str); 6] = [
    (4 * TICKS_PER_QUARTER, "Ganze"),
    (2 * TICKS_PER_QUARTER, "Halbe"),
    (TICKS_PER_QUARTER, "Viertel"),
    (TICKS_PER_QUARTER / 2, "Achtel"),
    (TICKS_PER_QUARTER / 4, "Sechzehntel"),
    (TICKS_PER_QUARTER / 8, "Zweiunddreißigstel"),
];

struct Cached {
    part: EventId,
    notes: Vec<Note>,
    start: u64,
    length: u64,
    tempo: TempoMap,
    key: KeySignature,
    quantize: u64,
    score: Score,
}

/// Die Notenschrift bei egui anmelden (einmal).
fn install_font(ctx: &egui::Context) {
    ctx.add_font(egui::epaint::text::FontInsert::new(
        FAMILY,
        egui::FontData::from_static(FONT),
        vec![egui::epaint::text::InsertFontFamily {
            family: FontFamily::Name(FAMILY.into()),
            priority: egui::epaint::text::FontPriority::Highest,
        }],
    ));
}

/// Zeichen der Notenschrift mit der Grundlinie bei `at` (links).
fn glyph(p: &dyn Canvas, ch: char, at: Pos2, size: f32, color: Color32) -> f32 {
    p.glyph(ch, at, size, color)
}

/// Platz eines Takts: Spalten je Einsatz (über alle Systeme), Breite in `sp`.
struct Columns {
    /// (Anfang in Ticks, x-Versatz im Takt in `sp`)
    cols: Vec<(u64, f32)>,
    width: f32,
}

fn columns(bar: &notation::Bar) -> Columns {
    let mut starts: Vec<u64> = bar
        .staves
        .iter()
        .flatten()
        .filter(|i| i.kind != Kind::BarRest)
        .map(|i| i.start)
        .collect();
    starts.sort_unstable();
    starts.dedup();
    if starts.is_empty() {
        return Columns {
            cols: vec![(bar.start, 3.0)],
            width: 8.0,
        };
    }
    let has_acc = |t: u64| {
        bar.staves.iter().flatten().any(|i| {
            i.start == t
                && matches!(&i.kind, Kind::Chord(h) if h.iter().any(|h| h.accidental.is_some()))
        })
    };
    let mut x = 1.2;
    let mut cols = Vec::new();
    for (k, &t) in starts.iter().enumerate() {
        if has_acc(t) {
            x += 1.2;
        }
        cols.push((t, x));
        let next = starts.get(k + 1).copied().unwrap_or(bar.end());
        let d = next.saturating_sub(t).max(1) as f32;
        x += 1.8 + 1.4 * (d / 120.0 + 1.0).log2();
    }
    Columns { cols, width: x }
}

/// Ein System (eine Zeile): welche Takte, mit welchem x-Anfang je Takt.
pub(crate) struct System {
    bars: Vec<(usize, f32, Columns)>,
    header: f32,
}

/// Kopfbreite eines Systems in `sp`: Schlüssel, Vorzeichen, ggf. Taktart.
fn header_width(key: KeySignature, time: bool, grand: bool) -> f32 {
    let brace = if grand { 1.5 } else { 0.0 };
    let key_w = f32::from(key.fifths.unsigned_abs().min(7)) * 1.1;
    brace + 4.0 + key_w + if time { 3.0 } else { 0.5 }
}

pub(crate) fn break_systems(score: &Score, width_sp: f32) -> Vec<System> {
    let grand = score.clefs.len() == 2;
    let mut systems: Vec<System> = Vec::new();
    let mut cur: Option<System> = None;
    for (i, bar) in score.bars.iter().enumerate() {
        let cols = columns(bar);
        let fits = cur.as_ref().is_some_and(|s| {
            let used = s.bars.last().map_or(s.header, |b| b.1 + b.2.width);
            used + cols.width + if bar.show_sig { 3.0 } else { 0.0 } <= width_sp
        });
        if !fits {
            if let Some(s) = cur.take() {
                systems.push(s);
            }
            cur = Some(System {
                bars: Vec::new(),
                header: header_width(score.key, bar.show_sig, grand),
            });
        }
        if let Some(s) = cur.as_mut() {
            let first = s.bars.is_empty();
            let x = s.bars.last().map_or(s.header, |b| b.1 + b.2.width);
            // Taktwechsel mitten in der Zeile: Platz für die Taktart.
            let x = if bar.show_sig && !first { x + 3.0 } else { x };
            s.bars.push((i, x, cols));
        }
    }
    if let Some(s) = cur {
        systems.push(s);
    }
    systems
}

/// Höhe eines Systems in `sp`.
pub(crate) fn system_height(staves: usize) -> f32 {
    PAD + (staves.saturating_sub(1)) as f32 * STAFF_GAP + 4.0 + PAD
}

impl ProjectWindow {
    /// Der Satz des Parts `part`, aus dem Zwischenspeicher, wenn sich nichts geändert hat.
    fn score_of(&mut self, part: EventId) -> Option<&Score> {
        let ev = self.project.event(part)?;
        let quantize = studiocraft_project::score_quantize(ev.score_quantize);
        let key = self.project.key.sane();
        let fresh = self.score_view.cache.as_ref().is_some_and(|c| {
            c.part == part
                && c.start == ev.start.0
                && c.length == ev.length
                && c.key == key
                && c.quantize == quantize
                && c.notes == ev.notes
                && c.tempo == self.project.tempo
        });
        if !fresh {
            let score = notation::layout(&notation::Input {
                notes: &ev.notes,
                part_start: ev.start.0,
                part_len: ev.length,
                tempo: &self.project.tempo,
                key,
                quantize,
            });
            self.score_view.cache = Some(Cached {
                part,
                notes: ev.notes.clone(),
                start: ev.start.0,
                length: ev.length,
                tempo: self.project.tempo.clone(),
                key,
                quantize,
                score,
            });
        }
        self.score_view.cache.as_ref().map(|c| &c.score)
    }

    fn set_score_quantize(&mut self, part: EventId, q: u64) {
        let before = self.project.clone();
        if let Some(e) = self.project.event_mut(part) {
            e.score_quantize = (q != studiocraft_project::DEFAULT_SCORE_QUANTIZE).then_some(q);
        }
        self.checkpoint(before);
    }

    fn set_key_fifths(&mut self, fifths: i8) {
        let before = self.project.clone();
        self.project.key.fifths = fifths.clamp(-7, 7);
        self.checkpoint(before);
    }

    /// Ändert die gewählten Noten des Parts mit `f` (ein Rückgängig-Schritt).
    fn edit_selected(&mut self, part: EventId, f: impl Fn(&mut Note, KeySignature)) {
        let key = self.project.key.sane();
        let sel = self.key_editor.selection.clone();
        let before = self.project.clone();
        if let Some(e) = self.project.event_mut(part) {
            for &i in &sel {
                if let Some(n) = e.notes.get_mut(i) {
                    f(n, key);
                }
            }
        }
        self.checkpoint(before);
    }

    fn delete_notes(&mut self, part: EventId, which: &[usize]) {
        let before = self.project.clone();
        if let Some(e) = self.project.event_mut(part) {
            let mut idx: Vec<usize> = which
                .iter()
                .copied()
                .filter(|&i| i < e.notes.len())
                .collect();
            idx.sort_unstable();
            idx.dedup();
            for i in idx.into_iter().rev() {
                e.notes.remove(i);
            }
        }
        self.key_editor.selection.clear();
        self.checkpoint(before);
    }

    /// Neue Note an der Stelle `pos` des Notenbilds (Zeichenwerkzeug).
    fn insert_note_at(&mut self, part: EventId, frame: &Frame, pos: Pos2, q: u64) {
        let Some((tick, pitch)) = locate(frame, pos, q, self.project.key.sane()) else {
            return;
        };
        let Some(ev) = self.project.event(part) else {
            return;
        };
        let Ok(start) = i64::try_from(tick.saturating_sub(ev.start.0)) else {
            return;
        };
        let v = self.score_view.value;
        let length = if self.score_view.dotted { v + v / 2 } else { v };
        let before = self.project.clone();
        let mut index = None;
        if let Some(e) = self.project.event_mut(part) {
            e.notes.push(Note {
                pitch,
                velocity: 100,
                start,
                length,
                spelling: 0,
            });
            index = Some(e.notes.len() - 1);
        }
        self.key_editor.selection = index.into_iter().collect();
        self.checkpoint(before);
    }
}

/// Tick (auf die Anzeige-Quantisierung gerastert) und Tonhöhe an der Stelle `pos`.
fn locate(frame: &Frame, pos: Pos2, q: u64, key: KeySignature) -> Option<(u64, u8)> {
    let (bar, t) = frame.bars.iter().find_map(|b| {
        b.y.contains(&pos.y)
            .then(|| b.tick_at(pos.x).map(|t| (b, t)))?
    })?;
    let (start, end) = (bar.pts.first()?.0, bar.pts.last()?.0);
    let q = q.max(1);
    let snapped = start + (t - start) / q * q;
    let tick = snapped.min(end.saturating_sub(q).max(start));
    let (clef, p) = bar.staff_at(pos.y)?;
    Some((tick, super::edit::pitch_at(clef, p, key)?))
}

/// Ungerasteter Tick und diatonische Stufe an der Stelle `pos` (zum Ziehen).
fn locate_raw(frame: &Frame, pos: Pos2) -> Option<(u64, i32)> {
    let (bar, t) = frame.bars.iter().find_map(|b| {
        b.y.contains(&pos.y)
            .then(|| b.tick_at(pos.x).map(|t| (b, t)))?
    })?;
    let (clef, p) = bar.staff_at(pos.y)?;
    Some((t, clef.middle() + p))
}

fn quantize_name(q: u64) -> &'static str {
    match q {
        960 => "1/4",
        480 => "1/8",
        240 => "1/16",
        _ => "1/32",
    }
}

fn key_label(fifths: i8) -> String {
    match fifths {
        0 => tr("Keine Vorzeichen").to_owned(),
        f if f > 0 => trf("{n} Kreuz(e)", &[("n", &f)]),
        f => trf("{n} Be", &[("n", &f.unsigned_abs())]),
    }
}

/// Noten-Editor in der unteren Zone oder im eigenen Fenster.
pub(crate) fn show(w: &mut ProjectWindow, ui: &mut Ui) {
    // Die Schrift gilt erst ab dem nächsten Bild; bis dahin nur die Werkzeugzeile.
    let family = FontFamily::Name(FAMILY.into());
    let font_loaded = ui
        .ctx()
        .fonts(|f| f.definitions().families.contains_key(&family));
    if !font_loaded && !w.score_view.font_ready {
        install_font(ui.ctx());
        w.score_view.font_ready = true;
        ui.ctx().request_repaint();
    }
    let Some(part) = w.list_part() else {
        ui.centered_and_justified(|ui| {
            ui.weak(tr(
                "Einen MIDI-Part wählen oder öffnen, um ihn hier als Noten zu sehen",
            ))
        });
        return;
    };
    let Some((name, q)) = w.project.event(part).map(|e| {
        (
            e.name.clone(),
            studiocraft_project::score_quantize(e.score_quantize),
        )
    }) else {
        return;
    };
    ui.horizontal(|ui| {
        ui.strong(&name);
        ui.separator();
        ui.label(tr("Anzeige-Quantisierung:"));
        let mut chosen = q;
        let combo = egui::ComboBox::from_id_salt("noten-quantisierung")
            .selected_text(quantize_name(q))
            .show_ui(ui, |ui| {
                for v in SCORE_QUANTIZE {
                    ui.selectable_value(&mut chosen, v, quantize_name(v));
                }
            })
            .response;
        combo.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::ComboBox, true, tr("Anzeige-Quantisierung"))
        });
        combo.on_hover_text(tr(
            "Rundet Anfang und Dauer nur für die Anzeige; der Part bleibt, wie er ist",
        ));
        if chosen != q {
            w.set_score_quantize(part, chosen);
        }
        ui.separator();
        ui.label(tr("Tonart:"));
        let fifths = w.project.key.sane().fifths;
        let mut f = fifths;
        egui::ComboBox::from_id_salt("noten-tonart")
            .selected_text(key_label(fifths))
            .show_ui(ui, |ui| {
                for v in -7..=7i8 {
                    ui.selectable_value(&mut f, v, key_label(v));
                }
            })
            .response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, tr("Tonart")));
        if f != fifths {
            w.set_key_fifths(f);
        }
        ui.separator();
        ui.toggle_value(&mut w.follow_playback, tr("Folgen (F)"));
        ui.separator();
        if ui.button(tr("Exportieren …")).clicked() {
            w.open_score_export();
        }
    });
    let any = !w.key_editor.selection.is_empty();
    ui.horizontal(|ui| {
        ui.label(tr("Notenwert:"));
        let mut value = w.score_view.value;
        for (ticks, name) in VALUES {
            ui.selectable_value(&mut value, ticks, tr(name))
                .on_hover_text(tr("Wert für neue Noten; ändert auch die gewählten"));
        }
        let mut dotted = w.score_view.dotted;
        ui.checkbox(&mut dotted, tr("Punktiert"));
        if (value, dotted) != (w.score_view.value, w.score_view.dotted) {
            w.score_view.value = value;
            w.score_view.dotted = dotted;
            if any {
                let length = if dotted { value + value / 2 } else { value };
                w.edit_selected(part, move |n, _| n.length = length);
            }
        }
        ui.separator();
        if ui
            .add_enabled(any, egui::Button::new(tr("Kreuz")))
            .on_hover_text(tr("Gewählte Noten einen Halbton höher, mit Kreuz"))
            .clicked()
        {
            w.edit_selected(part, |n, _| super::edit::sharpen(n));
        }
        if ui
            .add_enabled(any, egui::Button::new(tr("Be")))
            .on_hover_text(tr("Gewählte Noten einen Halbton tiefer, mit Be"))
            .clicked()
        {
            w.edit_selected(part, |n, _| super::edit::flatten(n));
        }
        if ui
            .add_enabled(any, egui::Button::new(tr("Enharmonisch verwechseln")))
            .on_hover_text(tr("Kreuz als Be schreiben und umgekehrt"))
            .clicked()
        {
            w.edit_selected(part, super::edit::respell);
        }
        if ui
            .add_enabled(any, egui::Button::new(tr("Löschen")))
            .clicked()
        {
            let sel = w.key_editor.selection.clone();
            w.delete_notes(part, &sel);
        }
    });
    ui.separator();
    if !font_loaded {
        return;
    }

    let colors = Colors::of(ui.ctx());
    let tool = w.tool;
    let key = w.project.key.sane();
    let mut frame = Frame {
        sel: w.key_editor.selection.clone(),
        ..Frame::default()
    };
    let playing = w.is_playing();
    let follow = w.follow_playback && playing;
    let cursor = w.cursor;
    let avail = ui.available_width();
    let Some(score) = w.score_of(part) else {
        return;
    };
    let staves = score.clefs.len();
    let systems = break_systems(score, ((avail - 2.0 * SP) / SP).max(30.0));
    let sys_h = system_height(staves) * SP;
    let total = Vec2::new(avail, sys_h * systems.len() as f32 + SP);
    let mut scroll_to: Option<Rect> = None;
    let out = egui::ScrollArea::vertical()
        .id_salt("noten-scroll")
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            let (rect, resp) = ui.allocate_exact_size(total, Sense::click_and_drag());
            ui.ctx().accesskit_node_builder(resp.id, |n| {
                n.set_role(egui::accesskit::Role::Image);
                n.set_label(trf(
                    "Partitur: {name}, {bars} Takte",
                    &[("name", &name), ("bars", &score.bars.len())],
                ));
            });
            let p = ui.painter_at(rect);
            for (k, sys) in systems.iter().enumerate() {
                let top = rect.top() + k as f32 * sys_h;
                let sys_rect = Rect::from_min_size(pos2(rect.left(), top), Vec2::new(avail, sys_h));
                let visible = sys_rect.translate(-rect.min.to_vec2()).intersects(viewport);
                let cur = draw_system(
                    &p,
                    score,
                    sys,
                    pos2(rect.left() + SP, top + PAD * SP),
                    k + 1 == systems.len(),
                    &colors,
                    cursor,
                    visible,
                    &mut frame,
                );
                if follow && let Some(x) = cur {
                    scroll_to = Some(Rect::from_x_y_ranges(x..=x + 1.0, sys_rect.y_range()));
                }
            }
            resp
        });
    if let Some(r) = scroll_to {
        ui.scroll_to_rect(r, Some(egui::Align::Center));
    }
    interact(w, ui, part, &out.inner, &frame, tool, key, q);
    w.score_view.last = frame;
}

/// Klicken, Ziehen und Löschen im Notenbild.
#[allow(clippy::too_many_arguments)]
fn interact(
    w: &mut ProjectWindow,
    ui: &Ui,
    part: EventId,
    resp: &egui::Response,
    frame: &Frame,
    tool: crate::tools::Tool,
    key: KeySignature,
    q: u64,
) {
    use crate::tools::Tool;
    let head_at = |p: Pos2| {
        frame
            .heads
            .iter()
            .find(|h| h.0.expand(0.15 * SP).contains(p))
            .map(|h| h.1)
    };
    let shift = ui.input(|i| i.modifiers.shift);
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        match (tool, head_at(p)) {
            (Tool::Erase, Some(n)) => w.delete_notes(part, &[n]),
            (Tool::Draw, None) => w.insert_note_at(part, frame, p, q),
            (_, Some(n)) if shift => {
                let sel = &mut w.key_editor.selection;
                if let Some(k) = sel.iter().position(|&s| s == n) {
                    sel.remove(k);
                } else {
                    sel.push(n);
                }
            }
            (_, Some(n)) => w.key_editor.selection = vec![n],
            (_, None) => w.key_editor.selection.clear(),
        }
    }
    // Ziehen: gewählte Noten in der Zeit (gerastert) und auf den Linien verschieben.
    // Gezogen wird ab dem Punkt, an dem die Taste gedrückt wurde.
    if resp.drag_started()
        && matches!(tool, Tool::Select | Tool::Draw)
        && let Some(p) = ui.input(|i| i.pointer.press_origin())
        && let Some(n) = head_at(p)
        && let Some((tick, step)) = locate_raw(frame, p)
    {
        if !w.key_editor.selection.contains(&n) {
            w.key_editor.selection = vec![n];
        }
        w.score_view.drag = Some(Drag {
            before: w.project.clone(),
            part,
            notes: w.key_editor.selection.clone(),
            tick,
            step,
        });
    }
    if let Some(d) = &w.score_view.drag
        && resp.dragged()
        && let Some(p) = resp.interact_pointer_pos()
        && let Some((tick, step)) = locate_raw(frame, p)
        && let Some(orig) = d.before.event(d.part).map(|e| e.notes.clone())
    {
        let q = q.max(1) as i64;
        let raw = tick as i64 - d.tick as i64;
        let dt = (raw + raw.signum() * q / 2) / q * q;
        let steps = step - d.step;
        let mut notes = orig;
        for &i in &d.notes {
            if let Some(n) = notes.get_mut(i) {
                n.start = n.start.saturating_add(dt).max(0);
                super::edit::move_steps(n, steps, key);
            }
        }
        let part = d.part;
        if let Some(e) = w.project.event_mut(part) {
            e.notes = notes;
        }
    }
    if resp.drag_stopped()
        && let Some(d) = w.score_view.drag.take()
    {
        w.checkpoint(d.before);
    }
}

pub(crate) struct Colors {
    pub(crate) ink: Color32,
    line: Color32,
    playhead: Color32,
    weak: Color32,
    selected: Color32,
}

/// Was beim Zeichnen für die Bedienung anfällt: Notenköpfe und die Lage der Takte.
#[derive(Default)]
pub(crate) struct Frame {
    /// Gewählte Noten (Index im Part).
    pub sel: Vec<usize>,
    /// Notenköpfe: Fläche und Index der Note.
    pub heads: Vec<(Rect, usize)>,
    pub bars: Vec<BarHit>,
}

/// Lage eines gezeichneten Takts: senkrechter Bereich seines Systems, Oberkanten und
/// Schlüssel der Notensysteme, Spalten (Tick, x) vom Anfang bis zum Ende.
#[derive(Debug, Clone)]
pub(crate) struct BarHit {
    pub y: std::ops::RangeInclusive<f32>,
    pub tops: Vec<f32>,
    pub clefs: Vec<Clef>,
    pub pts: Vec<(u64, f32)>,
}

impl BarHit {
    /// Tick an der Stelle `x` (zwischen den Spalten verteilt).
    pub(crate) fn tick_at(&self, x: f32) -> Option<u64> {
        let (first, last) = (self.pts.first()?, self.pts.last()?);
        if x < first.1 || x > last.1 {
            return None;
        }
        // Nahe einer Spalte: ihr Einsatz.
        if let Some(p) = self.pts[1..self.pts.len() - 1]
            .iter()
            .find(|p| (x - p.1).abs() <= 0.8 * SP)
        {
            return Some(p.0);
        }
        self.pts
            .windows(2)
            .find(|w| x >= w[0].1 && x <= w[1].1)
            .map(|w| {
                let f = ((x - w[0].1) / (w[1].1 - w[0].1).max(1e-3)).clamp(0.0, 1.0);
                w[0].0 + ((w[1].0 - w[0].0) as f32 * f) as u64
            })
    }

    /// Notensystem und Lage (halbe Zwischenräume über der Mittellinie) bei `y`.
    pub(crate) fn staff_at(&self, y: f32) -> Option<(Clef, i32)> {
        let (k, top) = self.tops.iter().enumerate().min_by(|a, b| {
            let d = |t: f32| (y - (t + 2.0 * SP)).abs();
            d(*a.1).total_cmp(&d(*b.1))
        })?;
        let pos = ((top + 2.0 * SP - y) / (SP / 2.0)).round() as i32;
        Some((*self.clefs.get(k)?, pos.clamp(-30, 30)))
    }
}

impl Colors {
    /// Für den Druck: schwarz auf weiß, ohne Cursor.
    pub(crate) fn print() -> Self {
        Self {
            ink: Color32::BLACK,
            line: Color32::BLACK,
            playhead: Color32::TRANSPARENT,
            weak: Color32::from_gray(90),
            selected: Color32::BLACK,
        }
    }

    fn of(ctx: &egui::Context) -> Self {
        let t = crate::theme::tokens(ctx);
        Self {
            ink: t.text,
            line: t.text.gamma_multiply(0.75),
            playhead: t.playhead,
            weak: t.text_dim,
            selected: t.accent,
        }
    }
}

/// y einer Lage (halbe Zwischenräume über der Mittellinie) im System mit Oberkante `top`.
fn y_of(top: f32, pos: i32) -> f32 {
    top + 2.0 * SP - pos as f32 * SP / 2.0
}

/// Zeichnet ein System (wenn `visible`) und gibt die x-Lage des Cursors zurück, falls er darin
/// liegt.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_system(
    p: &dyn Canvas,
    score: &Score,
    sys: &System,
    origin: Pos2,
    last_system: bool,
    c: &Colors,
    cursor: u64,
    visible: bool,
    out: &mut Frame,
) -> Option<f32> {
    let size = 4.0 * SP;
    let tops: Vec<f32> = (0..score.clefs.len())
        .map(|s| origin.y + s as f32 * STAFF_GAP * SP)
        .collect();
    let grand = tops.len() == 2;
    let left = origin.x + if grand { 1.5 * SP } else { 0.0 };
    let right = sys.bars.last().map_or(left + sys.header * SP, |b| {
        origin.x + (b.1 + b.2.width) * SP
    });
    // Cursor
    let mut cursor_x = None;
    for (i, x0, cols) in &sys.bars {
        let bar = &score.bars[*i];
        if cursor >= bar.start && cursor < bar.end() {
            let bx = origin.x + x0 * SP;
            let mut pts: Vec<(u64, f32)> =
                cols.cols.iter().map(|&(t, x)| (t, bx + x * SP)).collect();
            pts.push((bar.end(), bx + cols.width * SP));
            if pts.len() == 1 || pts[0].0 > bar.start {
                pts.insert(0, (bar.start, bx));
            }
            let x = pts
                .windows(2)
                .find(|w| cursor >= w[0].0 && cursor < w[1].0)
                .map_or(bx, |w| {
                    let f = (cursor - w[0].0) as f32 / (w[1].0 - w[0].0).max(1) as f32;
                    w[0].1 + f * (w[1].1 - w[0].1)
                });
            cursor_x = Some(x);
        }
    }
    if !visible {
        return cursor_x;
    }
    let bottom = tops.last().copied().unwrap_or(origin.y) + 4.0 * SP;
    for (i, x0, cols) in &sys.bars {
        let bar = &score.bars[*i];
        let bx = origin.x + x0 * SP;
        let mut pts: Vec<(u64, f32)> = vec![(bar.start, bx)];
        pts.extend(cols.cols.iter().map(|&(t, x)| (t, bx + x * SP)));
        pts.push((bar.end(), bx + cols.width * SP));
        out.bars.push(BarHit {
            y: (tops[0] - PAD * SP)..=(bottom + PAD * SP),
            tops: tops.clone(),
            clefs: score.clefs.clone(),
            pts,
        });
    }
    // Linien
    for &top in &tops {
        for l in 0..5 {
            let y = top + l as f32 * SP;
            p.hline(left..=right, y, Stroke::new(0.11 * SP, c.line));
        }
    }
    // Anfangsstrich, Klammer
    p.vline(left, tops[0]..=bottom, Stroke::new(0.18 * SP, c.ink));
    if grand {
        let h = bottom - tops[0];
        glyph(
            p,
            BRACE,
            pos2(origin.x, bottom),
            size * h / (4.0 * SP),
            c.ink,
        );
    }
    // Kopf: Schlüssel, Vorzeichen, Taktart (am Anfang)
    let first_bar = sys.bars.first().map(|b| &score.bars[b.0]);
    for (s, &clef) in score.clefs.iter().enumerate() {
        let top = tops[s];
        let (ch, pos) = match clef {
            Clef::Treble => (G_CLEF, -2),
            Clef::Bass => (F_CLEF, 2),
        };
        glyph(p, ch, pos2(left + 0.5 * SP, y_of(top, pos)), size, c.ink);
        let acc = if score.key.fifths >= 0 { SHARP } else { FLAT };
        for (k, kp) in clef.key_positions(score.key.fifths).iter().enumerate() {
            let x = left + (3.6 + k as f32 * 1.1) * SP;
            glyph(p, acc, pos2(x, y_of(top, *kp)), size, c.ink);
        }
        if let Some(bar) = first_bar.filter(|b| b.show_sig) {
            let x = left + (header_width(score.key, false, false) - 0.3) * SP;
            time_sig(p, bar.sig, x, top, size, c.ink);
        }
    }
    // Taktnummer am Systemanfang
    if let Some(bar) = first_bar {
        p.text(
            pos2(left, tops[0] - 2.5 * SP),
            egui::Align2::LEFT_BOTTOM,
            bar.number.to_string(),
            FontId::proportional(SP * 1.3),
            c.weak,
        );
    }
    // Takte
    let mut ties: Vec<(Pos2, bool, u8, usize)> = Vec::new();
    for (bi, (i, x0, cols)) in sys.bars.iter().enumerate() {
        let bar = &score.bars[*i];
        let bx = origin.x + x0 * SP;
        if bar.show_sig && bi > 0 {
            for &top in &tops {
                time_sig(p, bar.sig, bx - 2.7 * SP, top, size, c.ink);
            }
        }
        for (s, items) in bar.staves.iter().enumerate() {
            let top = tops[s];
            let col_x = |t: u64| {
                cols.cols
                    .iter()
                    .find(|c| c.0 == t)
                    .map_or(bx + SP, |c| bx + c.1 * SP)
            };
            draw_items(
                p, items, top, bx, cols.width, &col_x, size, c, &mut ties, s, out,
            );
        }
        // Taktstrich
        let x = bx + cols.width * SP;
        let final_bar = last_system && bi + 1 == sys.bars.len();
        if final_bar {
            p.vline(
                x - 0.9 * SP,
                tops[0]..=bottom,
                Stroke::new(0.18 * SP, c.ink),
            );
            p.vline(
                x - 0.2 * SP,
                tops[0]..=bottom,
                Stroke::new(0.55 * SP, c.ink),
            );
        } else {
            p.vline(x, tops[0]..=bottom, Stroke::new(0.18 * SP, c.ink));
        }
    }
    // Haltebögen: zum nächsten Kopf gleicher Tonhöhe im selben System, sonst bis zum Rand.
    draw_ties(p, &ties, right, c.ink);
    if let Some(x) = cursor_x {
        p.vline(
            x,
            (tops[0] - 2.0 * SP)..=(bottom + 2.0 * SP),
            Stroke::new(1.5, c.playhead),
        );
    }
    cursor_x
}

fn time_sig(
    p: &dyn Canvas,
    sig: studiocraft_project::TimeSignature,
    x: f32,
    top: f32,
    size: f32,
    ink: Color32,
) {
    let digits = |n: u32| -> String {
        n.to_string()
            .chars()
            .filter_map(|d| d.to_digit(10))
            .filter_map(|d| char::from_u32(TIME_DIGIT_0 + d))
            .collect()
    };
    for (n, pos) in [(sig.numerator, 2), (sig.denominator, -2)] {
        let mut cx = x;
        for ch in digits(n).chars() {
            cx += glyph(p, ch, pos2(cx, y_of(top, pos)), size, ink);
        }
    }
}

/// Hals eines Elements: x, y am Kopf, y der Spitze, nach oben, Zahl der Balken, Balkengruppe.
type Stem = (f32, f32, f32, bool, u8, Option<u32>);

/// Noten, Pausen, Hälse, Fähnchen, Balken und Punkte eines Takts in einem Notensystem.
#[allow(clippy::too_many_arguments)]
fn draw_items(
    p: &dyn Canvas,
    items: &[Item],
    top: f32,
    bx: f32,
    width: f32,
    col_x: &dyn Fn(u64) -> f32,
    size: f32,
    c: &Colors,
    ties: &mut Vec<(Pos2, bool, u8, usize)>,
    staff: usize,
    out: &mut Frame,
) {
    let mut stems: Vec<Option<Stem>> = vec![None; items.len()];
    for (k, it) in items.iter().enumerate() {
        let x = col_x(it.start);
        match &it.kind {
            Kind::BarRest => {
                let cx = bx + width * SP / 2.0 - 0.65 * SP;
                glyph(p, '\u{E4E3}', pos2(cx, y_of(top, 2)), size, c.ink);
            }
            Kind::Rest => {
                let (ch, pos) = match it.value.base {
                    Base::Whole => ('\u{E4E3}', 2),
                    Base::Half => ('\u{E4E4}', 0),
                    Base::Quarter => ('\u{E4E5}', 0),
                    Base::Eighth => ('\u{E4E6}', 0),
                    Base::Sixteenth => ('\u{E4E7}', 0),
                    Base::ThirtySecond => ('\u{E4E8}', 0),
                };
                let w = glyph(p, ch, pos2(x, y_of(top, pos)), size, c.ink);
                if it.value.dotted {
                    glyph(p, DOT, pos2(x + w + 0.3 * SP, y_of(top, 1)), size, c.ink);
                }
            }
            Kind::Chord(heads) => {
                let head = match it.value.base {
                    Base::Whole => HEAD_WHOLE,
                    Base::Half => HEAD_HALF,
                    _ => HEAD_BLACK,
                };
                let hw = if it.value.base == Base::Whole {
                    WHOLE_W
                } else {
                    HEAD_W
                };
                let mut sorted: Vec<_> = heads.iter().collect();
                sorted.sort_by_key(|h| h.pos);
                let mut acc_shift = 0;
                for (j, h) in sorted.iter().enumerate() {
                    // Sekunde: der zweite Kopf auf die andere Seite des Halses.
                    let second = j > 0 && h.pos - sorted[j - 1].pos == 1;
                    let hx = if second {
                        if it.stem_up { x + hw * SP } else { x - hw * SP }
                    } else {
                        x
                    };
                    let y = y_of(top, h.pos);
                    ledger(p, x, hw, top, h.pos, c.line);
                    let ink = if out.sel.contains(&h.note) {
                        c.selected
                    } else {
                        c.ink
                    };
                    glyph(p, head, pos2(hx, y), size, ink);
                    out.heads.push((
                        Rect::from_min_size(pos2(hx, y - 0.6 * SP), Vec2::new(hw * SP, 1.2 * SP)),
                        h.note,
                    ));
                    if let Some(a) = h.accidental {
                        let (ch, aw) = match a {
                            Accidental::Sharp => (SHARP, 0.98),
                            Accidental::Flat => (FLAT, 0.81),
                            Accidental::Natural => (NATURAL, 0.68),
                        };
                        let ax = x - (aw + 0.45 + acc_shift as f32 * 1.0) * SP;
                        glyph(p, ch, pos2(ax, y), size, c.ink);
                        acc_shift = (acc_shift + 1) % 2;
                    }
                    if it.value.dotted {
                        let dy = if h.pos % 2 == 0 {
                            y_of(top, h.pos + 1)
                        } else {
                            y
                        };
                        glyph(p, DOT, pos2(x + (hw + 0.35) * SP, dy), size, c.ink);
                    }
                    if h.tie {
                        ties.push((pos2(hx + hw * SP, y), it.stem_up, h.pitch, staff));
                    } else {
                        // Ende eines Bogens: der Kopf, an dem er ankommt.
                        ties.push((pos2(hx, y), it.stem_up, h.pitch, usize::MAX - staff));
                    }
                }
                if it.value.base == Base::Whole {
                    continue;
                }
                let lo = sorted.first().map_or(0, |h| h.pos);
                let hi = sorted.last().map_or(0, |h| h.pos);
                let (sx, from, tip) = if it.stem_up {
                    let sx = x + (HEAD_W - 0.05) * SP;
                    (sx, y_of(top, lo) - 0.16 * SP, y_of(top, hi) - STEM_LEN * SP)
                } else {
                    let sx = x + 0.05 * SP;
                    (sx, y_of(top, hi) + 0.17 * SP, y_of(top, lo) + STEM_LEN * SP)
                };
                stems[k] = Some((sx, from, tip, it.stem_up, it.value.base.beams(), it.beam));
            }
        }
    }
    // Balkengruppen: gemeinsame waagerechte Spitze.
    let mut k = 0;
    while k < items.len() {
        let Some((_, _, _, up, _, Some(id))) = stems[k] else {
            if let Some((sx, from, tip, up, beams, None)) = stems[k] {
                p.line_segment(
                    [pos2(sx, from), pos2(sx, tip)],
                    Stroke::new(0.1 * SP, c.ink),
                );
                if beams > 0 {
                    let ch = match (beams, up) {
                        (1, true) => '\u{E240}',
                        (1, false) => '\u{E241}',
                        (2, true) => '\u{E242}',
                        (2, false) => '\u{E243}',
                        (_, true) => '\u{E244}',
                        (_, false) => '\u{E245}',
                    };
                    glyph(p, ch, pos2(sx - 0.05 * SP, tip), 4.0 * SP, c.ink);
                }
            }
            k += 1;
            continue;
        };
        let end = (k..items.len())
            .take_while(|&j| stems[j].is_some_and(|s| s.5 == Some(id)))
            .last()
            .unwrap_or(k);
        let group: Vec<(f32, f32, f32, u8)> = (k..=end)
            .filter_map(|j| stems[j].map(|s| (s.0, s.1, s.2, s.4)))
            .collect();
        let tip = if up {
            group.iter().map(|g| g.2).fold(f32::MAX, f32::min)
        } else {
            group.iter().map(|g| g.2).fold(f32::MIN, f32::max)
        };
        for g in &group {
            p.line_segment(
                [pos2(g.0, g.1), pos2(g.0, tip)],
                Stroke::new(0.1 * SP, c.ink),
            );
        }
        let thick = 0.5 * SP;
        let step = 0.75 * SP;
        let dir = if up { 1.0 } else { -1.0 };
        let beam = |x0: f32, x1: f32, level: u8| {
            let y = tip + dir * f32::from(level) * step;
            let y2 = y + dir * thick;
            Shape::convex_polygon(
                vec![
                    pos2(x0, y.min(y2)),
                    pos2(x1, y.min(y2)),
                    pos2(x1, y.max(y2)),
                    pos2(x0, y.max(y2)),
                ],
                c.ink,
                Stroke::NONE,
            )
        };
        if let (Some(a), Some(b)) = (group.first(), group.last()) {
            p.add(beam(a.0 - 0.05 * SP, b.0 + 0.05 * SP, 0));
        }
        // Zweite und dritte Balken: zwischen Nachbarn mit genug Balken, sonst ein Stummel.
        for level in 1..3u8 {
            for (j, g) in group.iter().enumerate() {
                if g.3 <= level {
                    continue;
                }
                let next = group.get(j + 1).filter(|n| n.3 > level);
                let prev = j
                    .checked_sub(1)
                    .and_then(|i| group.get(i))
                    .filter(|n| n.3 > level);
                if let Some(n) = next {
                    p.add(beam(g.0, n.0, level));
                } else if prev.is_none() {
                    let (x0, x1) = if j + 1 < group.len() {
                        (g.0, g.0 + SP)
                    } else {
                        (g.0 - SP, g.0)
                    };
                    p.add(beam(x0, x1, level));
                }
            }
        }
        k = end + 1;
    }
}

/// Hilfslinien über und unter dem System.
fn ledger(p: &dyn Canvas, x: f32, hw: f32, top: f32, pos: i32, color: Color32) {
    let stroke = Stroke::new(0.16 * SP, color);
    let ext = 0.33 * SP;
    let draw = |q: i32| {
        p.hline((x - ext)..=(x + hw * SP + ext), y_of(top, q), stroke);
    };
    let mut q = 6;
    while q <= pos {
        draw(q);
        q += 2;
    }
    let mut q = -6;
    while q >= pos {
        draw(q);
        q -= 2;
    }
}

/// Haltebögen: vom Kopf mit Bogen zum nächsten Kopf gleicher Tonhöhe desselben Notensystems.
fn draw_ties(p: &dyn Canvas, heads: &[(Pos2, bool, u8, usize)], right: f32, ink: Color32) {
    for (k, &(from, up, pitch, staff)) in heads.iter().enumerate() {
        if staff >= usize::MAX / 2 {
            continue;
        }
        let to = heads[k + 1..]
            .iter()
            .find(|h| h.2 == pitch && (h.3 == staff || h.3 == usize::MAX - staff) && h.0.x > from.x)
            .map_or(pos2(right, from.y), |h| h.0);
        let dir = if up { 1.0 } else { -1.0 };
        let y = from.y + dir * 0.6 * SP;
        let a = pos2(from.x + 0.1 * SP, y);
        let b = pos2(to.x - 0.1 * SP, y);
        let bend = dir * (0.8 * SP).min((b.x - a.x) * 0.25);
        let c1 = pos2(a.x + (b.x - a.x) * 0.25, y + bend);
        let c2 = pos2(a.x + (b.x - a.x) * 0.75, y + bend);
        p.add(
            egui::epaint::CubicBezierShape::from_points_stroke(
                [a, c1, c2, b],
                false,
                Color32::TRANSPARENT,
                Stroke::new(0.18 * SP, ink),
            )
            .into(),
        );
    }
}
