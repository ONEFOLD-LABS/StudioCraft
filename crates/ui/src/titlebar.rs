//! Titelleiste wie in den anderen Craft-Apps: App-Symbol, Menü, Projekt-Titel und Fensterknöpfe
//! in einer Zeile, statt Titelzeile des Systems plus Menüzeile.
//!
//! Wer den Rahmen zeichnet, entscheidet `craft-window` (storytold/craft-libs) für alle Craft-Apps
//! gleich: auf Windows, GNOME, KDE und den meisten Desktops die App, auf Tiling-Desktops das
//! System, auf macOS die App mit den Ampelknöpfen über ihrer Leiste. Die Knöpfe stehen in der
//! Reihenfolge des Desktops. `STUDIOCRAFT_TITLEBAR=system` (oder `CRAFT_TITLEBAR`) behält den
//! Rahmen des Systems, `=app` erzwingt die eigene Leiste.
//!
//! Die leere Fläche der Leiste verschiebt das Fenster (Doppelklick nach Systemeinstellung), an
//! den Fensterrändern ändert Ziehen die Größe.

use craft_window::Button;
use craft_window::policy::{
    Backend, CaptionFamily, Captions, Frame, TitleBarPref, apply, decide_for,
};
use egui::{
    Color32, Context, CursorIcon, FontId, Rect, ResizeDirection, Sense, Stroke, Ui,
    ViewportBuilder, ViewportCommand, pos2, vec2,
};

use crate::ProjectWindow;

/// Breite eines Fensterknopfs (Windows-Stil).
const CAPTION_W: f32 = 40.0;
/// Breite eines runden Fensterknopfs (Adwaita, Breeze, generisch).
const ROUND_W: f32 = 30.0;
/// Fangbreite der Fensterränder zum Ziehen der Größe.
const EDGE: f32 = 5.0;

/// Entscheidet einmal beim Start, wer den Fensterrahmen zeichnet.
pub fn decide_frame() -> Frame {
    decide_for(
        craft_window::Prefix::new("STUDIOCRAFT"),
        Backend::from_env(),
        TitleBarPref::Integrated,
    )
    .frame
}

/// Setzt die Entscheidung auf den Fenster-Builder (vor dem Öffnen des Fensters).
pub fn apply_frame(builder: ViewportBuilder, frame: &Frame) -> ViewportBuilder {
    apply(builder, frame)
}

/// Eigene Leiste mit allgemeinen Knöpfen rechts (für Tests und Vorschauen).
pub fn integrated_frame() -> Frame {
    Frame::Integrated(Captions {
        family: CaptionFamily::Generic,
        layout: craft_window::Layout::default(),
    })
}

fn caption_width(family: CaptionFamily) -> f32 {
    if family == CaptionFamily::Windows {
        CAPTION_W
    } else {
        ROUND_W
    }
}

fn caption_button(ui: &mut Ui, which: Button, family: CaptionFamily, maximized: bool) {
    let label = match which {
        Button::Minimize => crate::i18n::tr("Minimieren"),
        Button::Maximize if maximized => crate::i18n::tr("Wiederherstellen"),
        Button::Maximize => crate::i18n::tr("Maximieren"),
        Button::Close => crate::i18n::tr("Schließen"),
    };
    let size = vec2(caption_width(family), ui.available_height());
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    let t = crate::theme::tokens(ui.ctx());
    let hot = resp.hovered();
    let close = which == Button::Close;
    let p = ui.painter();
    let c = rect.center();
    let fg = if family == CaptionFamily::Windows {
        // Windows: ganze Fläche hinterlegt, Schließen rot.
        if hot {
            let fill = if close {
                Color32::from_rgb(0xc4, 0x2b, 0x1c)
            } else {
                t.button_hi
            };
            p.rect_filled(rect, 0.0, fill);
        }
        if hot && close { Color32::WHITE } else { t.text }
    } else {
        // Adwaita, Breeze, generisch: runder Knopf.
        let fill = if hot { t.button_hi } else { t.button };
        p.circle_filled(c, 11.0, fill);
        t.text
    };
    let s = Stroke::new(1.2, fg);
    match which {
        Button::Minimize => {
            p.hline(c.x - 4.5..=c.x + 4.5, c.y + 0.5, s);
        }
        Button::Maximize if maximized => {
            p.rect_stroke(
                Rect::from_center_size(c + vec2(-1.5, 1.5), vec2(7.0, 7.0)),
                0.0,
                s,
                egui::StrokeKind::Inside,
            );
            p.line(
                vec![
                    c + vec2(-2.0, -3.5),
                    c + vec2(3.5, -3.5),
                    c + vec2(3.5, 2.0),
                ],
                s,
            );
        }
        Button::Maximize => {
            p.rect_stroke(
                Rect::from_center_size(c, vec2(9.0, 9.0)),
                0.0,
                s,
                egui::StrokeKind::Inside,
            );
        }
        Button::Close => {
            p.line_segment([c + vec2(-4.5, -4.5), c + vec2(4.5, 4.5)], s);
            p.line_segment([c + vec2(-4.5, 4.5), c + vec2(4.5, -4.5)], s);
        }
    }
    let resp = resp.on_hover_text(label);
    if resp.clicked() {
        let cmd = match which {
            Button::Minimize => ViewportCommand::Minimized(true),
            Button::Maximize => ViewportCommand::Maximized(!maximized),
            Button::Close => ViewportCommand::Close,
        };
        ui.ctx().send_viewport_cmd(cmd);
    }
}

impl ProjectWindow {
    /// Wer den Fensterrahmen zeichnet (siehe [`decide_frame`]).
    pub fn set_window_frame(&mut self, frame: Frame) {
        self.window_frame = frame;
    }

    /// Die App zeichnet die Titelleiste selbst (auch macOS mit Ampelknöpfen über der Leiste).
    pub(crate) fn draws_title_bar(&self) -> bool {
        !matches!(self.window_frame, Frame::Native)
    }

    /// Die oberste Leiste: mit eigener Titelleiste App-Symbol, Menü, Titel und Fensterknöpfe,
    /// sonst nur das Menü.
    pub(crate) fn top_bar(&mut self, ui: &mut Ui) {
        if !self.draws_title_bar() {
            self.menu_bar(ui);
            return;
        }
        let bar = ui.max_rect();
        // Leere Fläche: Fenster ziehen, Doppelklick wie im System eingestellt. Vor den
        // Bedienelementen, damit die ihre Klicks selbst bekommen.
        craft_window::title_bar::empty_area(ui, bar);
        let maximized = ui.input(|i| i.viewport().maximized).unwrap_or(false);
        let (leading, captions) = match &self.window_frame {
            Frame::Inset(m) => (m.leading, None),
            Frame::Integrated(c) => (0.0, Some(c.clone())),
            Frame::Native => (0.0, None),
        };
        let t = crate::theme::tokens(ui.ctx());
        let mut right_w = 0.0;
        ui.horizontal_centered(|ui| {
            ui.add_space(leading);
            if let Some(c) = &captions {
                ui.spacing_mut().item_spacing.x = 0.0;
                // Links: außen zuerst.
                for &b in c.layout.left.iter() {
                    caption_button(ui, b, c.family, maximized);
                }
                ui.spacing_mut().item_spacing.x = 8.0;
                if !c.layout.left.is_empty() {
                    ui.add_space(6.0);
                }
            }
            // App-Symbol: dasselbe Zeichen wie das App-Icon.
            let (r, _) = ui.allocate_exact_size(vec2(22.0, 20.0), Sense::hover());
            let mark = Rect::from_center_size(r.center(), vec2(19.0, 19.0));
            crate::icons::app_mark(ui.painter(), mark, ui.visuals().panel_fill);
            // Die Menüleiste belegt die ganze Breite; ihr Ende aus den Titeln berechnen.
            let font = egui::TextStyle::Button.resolve(ui.style());
            let pad = ui.spacing().button_padding.x * 2.0 + ui.spacing().item_spacing.x;
            let menus_right = ui.cursor().min.x
                + ["Datei", "Bearbeiten", "Projekt", "Ansicht"]
                    .iter()
                    .map(|l| {
                        ui.painter()
                            .layout_no_wrap(crate::i18n::tr(l).to_owned(), font.clone(), t.text)
                            .size()
                            .x
                            + pad
                    })
                    .sum::<f32>();
            self.menu_bar(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if let Some(c) = &captions {
                    // Rechts: außen zuletzt, im Rechts-nach-links-Layout also rückwärts.
                    for &b in c.layout.right.iter().rev() {
                        caption_button(ui, b, c.family, maximized);
                    }
                    right_w = c.layout.right.len() as f32 * caption_width(c.family);
                }
            });
            // Titel mittig, solange er zwischen Menü und Knöpfe passt.
            let free = Rect::from_x_y_ranges(
                menus_right + 12.0..=bar.right() - right_w - 12.0,
                bar.y_range(),
            );
            let galley =
                ui.painter()
                    .layout_no_wrap(self.title(), FontId::proportional(13.0), t.text_dim);
            let w = galley.size().x;
            if w <= free.width() {
                let x = bar
                    .center()
                    .x
                    .clamp(free.left() + w / 2.0, free.right() - w / 2.0);
                ui.painter().galley(
                    pos2(x - w / 2.0, bar.center().y - galley.size().y / 2.0),
                    galley,
                    t.text_dim,
                );
            }
        });
    }

    /// Nach dem Zeichnen: auf macOS AppKit sagen, ob ein Ziehen in der Leiste das Fenster
    /// bewegen darf (nicht auf Bedienelementen).
    pub(crate) fn sync_title_bar(&mut self, ctx: &Context) {
        #[cfg(target_os = "macos")]
        if self.draws_title_bar() {
            craft_window::title_bar::sync_movable(ctx, &mut self.mac_movable);
        }
        let _ = ctx;
    }

    /// Fensterränder ohne Rahmen des Systems: Mauszeiger zeigt die Richtung, Ziehen ändert die
    /// Größe.
    pub(crate) fn resize_edges(&self, ctx: &Context) {
        if !matches!(self.window_frame, Frame::Integrated(_)) {
            return;
        }
        let (maximized, fullscreen) = ctx.input(|i| {
            (
                i.viewport().maximized.unwrap_or(false),
                i.viewport().fullscreen.unwrap_or(false),
            )
        });
        if maximized || fullscreen {
            return;
        }
        let r = ctx.content_rect();
        let Some(p) = ctx.input(|i| i.pointer.hover_pos()) else {
            return;
        };
        let (l, rt, tp, b) = (
            p.x - r.left() < EDGE,
            r.right() - p.x < EDGE,
            p.y - r.top() < EDGE,
            r.bottom() - p.y < EDGE,
        );
        let dir = match (l, rt, tp, b) {
            (true, _, true, _) => ResizeDirection::NorthWest,
            (_, true, true, _) => ResizeDirection::NorthEast,
            (true, _, _, true) => ResizeDirection::SouthWest,
            (_, true, _, true) => ResizeDirection::SouthEast,
            (true, ..) => ResizeDirection::West,
            (_, true, ..) => ResizeDirection::East,
            (_, _, true, _) => ResizeDirection::North,
            (.., true) => ResizeDirection::South,
            _ => return,
        };
        ctx.set_cursor_icon(match dir {
            ResizeDirection::North | ResizeDirection::South => CursorIcon::ResizeVertical,
            ResizeDirection::East | ResizeDirection::West => CursorIcon::ResizeHorizontal,
            ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::ResizeNwSe,
            ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::ResizeNeSw,
        });
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
        }
    }
}
