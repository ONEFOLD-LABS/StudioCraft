//! PDF-Export der Partitur (Issue #183): eigener, kleiner PDF-Writer. Die Notenzeichen kommen
//! als Vektorpfade aus der Leland-Schrift (gelesen mit `skrifa`), Texte (Titel, Namen,
//! Taktnummern) in der Standardschrift Helvetica, die jeder PDF-Leser mitbringt. So wird keine
//! Schrift eingebettet und das Ergebnis ist überall gleich.

use std::cell::RefCell;
use std::fmt::Write as _;

use egui::{Align2, Color32, FontId, Pos2, Shape, Stroke};
use skrifa::{
    FontRef, MetadataProvider,
    instance::{LocationRef, Size},
    outline::{DrawSettings, OutlinePen},
};

use super::canvas::Canvas;
use super::musicxml::PartScore;
use super::view::{Colors, Frame, SP, break_systems, draw_system, system_height};

/// Papierformat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Paper {
    #[default]
    A4,
    Letter,
}

impl Paper {
    pub(crate) const ALL: [Self; 2] = [Self::A4, Self::Letter];

    /// Breite und Höhe in Punkten (1/72 Zoll).
    fn size(self) -> (f32, f32) {
        match self {
            Self::A4 => (595.28, 841.89),
            Self::Letter => (612.0, 792.0),
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::A4 => "A4",
            Self::Letter => "Letter",
        }
    }
}

/// Seiteneinrichtung.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Layout {
    pub paper: Paper,
    /// Ränder in Millimetern.
    pub margin_mm: f32,
    /// Abstand zweier Notenlinien in Millimetern.
    pub space_mm: f32,
    pub title: String,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            paper: Paper::A4,
            margin_mm: 15.0,
            space_mm: 1.75,
            title: String::new(),
        }
    }
}

const MM: f32 = 72.0 / 25.4;

/// Zeichenfläche einer Seite: sammelt PDF-Operatoren in Notensatz-Einheiten (y nach unten).
struct Page<'f> {
    ops: RefCell<String>,
    font: &'f FontRef<'f>,
}

fn rgb(c: Color32) -> (f32, f32, f32) {
    (
        f32::from(c.r()) / 255.0,
        f32::from(c.g()) / 255.0,
        f32::from(c.b()) / 255.0,
    )
}

impl Page<'_> {
    fn stroke_line(&self, a: Pos2, b: Pos2, stroke: Stroke) {
        let (r, g, bl) = rgb(stroke.color);
        let _ = writeln!(
            self.ops.borrow_mut(),
            "{r:.3} {g:.3} {bl:.3} RG {:.3} w {:.3} {:.3} m {:.3} {:.3} l S",
            stroke.width,
            a.x,
            a.y,
            b.x,
            b.y
        );
    }
}

/// Wandelt die Umrisse eines Zeichens in Pfad-Operatoren (y der Schrift zeigt nach oben).
struct Pen<'a> {
    out: &'a mut String,
    at: Pos2,
}

impl OutlinePen for Pen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.out, "{:.3} {:.3} m ", self.at.x + x, self.at.y - y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let _ = write!(self.out, "{:.3} {:.3} l ", self.at.x + x, self.at.y - y);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        // Quadratische Kurve als kubische (PDF kennt nur kubische).
        let _ = write!(
            self.out,
            "{:.3} {:.3} {:.3} {:.3} v ",
            self.at.x + cx0,
            self.at.y - cy0,
            self.at.x + x,
            self.at.y - y
        );
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let _ = write!(
            self.out,
            "{:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c ",
            self.at.x + cx0,
            self.at.y - cy0,
            self.at.x + cx1,
            self.at.y - cy1,
            self.at.x + x,
            self.at.y - y
        );
    }
    fn close(&mut self) {
        self.out.push_str("h ");
    }
}

/// Text für Helvetica (WinAnsi): Latin-1 als Bytes, anderes als „?“, Sonderzeichen maskiert.
fn pdf_string(s: &str) -> String {
    let mut out = String::from("(");
    for c in s.chars() {
        let b = u32::from(c);
        match c {
            '(' | ')' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if (0x20..0x7f).contains(&b) => out.push(c),
            _ if (0xa0..=0xff).contains(&b) => {
                let _ = write!(out, "\\{b:03o}");
            }
            _ => out.push('?'),
        }
    }
    out.push(')');
    out
}

impl Canvas for Page<'_> {
    fn hline(&self, x: std::ops::RangeInclusive<f32>, y: f32, stroke: Stroke) {
        self.stroke_line(Pos2::new(*x.start(), y), Pos2::new(*x.end(), y), stroke);
    }
    fn vline(&self, x: f32, y: std::ops::RangeInclusive<f32>, stroke: Stroke) {
        self.stroke_line(Pos2::new(x, *y.start()), Pos2::new(x, *y.end()), stroke);
    }
    fn line_segment(&self, points: [Pos2; 2], stroke: Stroke) {
        self.stroke_line(points[0], points[1], stroke);
    }
    fn add(&self, shape: Shape) {
        let mut ops = self.ops.borrow_mut();
        match shape {
            Shape::Path(path) if path.points.len() >= 3 => {
                let (r, g, b) = rgb(path.fill);
                let _ = write!(ops, "{r:.3} {g:.3} {b:.3} rg ");
                for (k, p) in path.points.iter().enumerate() {
                    let op = if k == 0 { "m" } else { "l" };
                    let _ = write!(ops, "{:.3} {:.3} {op} ", p.x, p.y);
                }
                ops.push_str("h f\n");
            }
            Shape::CubicBezier(c) => {
                let (r, g, b) = match c.stroke.color {
                    egui::epaint::ColorMode::Solid(col) => rgb(col),
                    _ => (0.0, 0.0, 0.0),
                };
                let p = c.points;
                let _ = writeln!(
                    ops,
                    "{r:.3} {g:.3} {b:.3} RG {:.3} w {:.3} {:.3} m {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c S",
                    c.stroke.width, p[0].x, p[0].y, p[1].x, p[1].y, p[2].x, p[2].y, p[3].x, p[3].y
                );
            }
            _ => {}
        }
    }
    fn text(&self, pos: Pos2, _anchor: Align2, text: String, font: FontId, color: Color32) {
        let (r, g, b) = rgb(color);
        // Die Seite ist gespiegelt (y nach unten); Text wird zurückgespiegelt.
        let _ = writeln!(
            self.ops.borrow_mut(),
            "BT {r:.3} {g:.3} {b:.3} rg /F1 {:.3} Tf 1 0 0 -1 {:.3} {:.3} Tm {} Tj ET",
            font.size,
            pos.x,
            pos.y,
            pdf_string(&text)
        );
    }
    fn glyph(&self, ch: char, at: Pos2, size: f32, color: Color32) -> f32 {
        let Some(gid) = self.font.charmap().map(ch) else {
            return 0.0;
        };
        let advance = self
            .font
            .glyph_metrics(Size::new(size), LocationRef::default())
            .advance_width(gid)
            .unwrap_or(0.0);
        let Some(outline) = self.font.outline_glyphs().get(gid) else {
            return advance;
        };
        let mut path = String::new();
        let mut pen = Pen { out: &mut path, at };
        let settings = DrawSettings::unhinted(Size::new(size), LocationRef::default());
        if outline.draw(settings, &mut pen).is_ok() && !path.is_empty() {
            let (r, g, b) = rgb(color);
            let _ = writeln!(self.ops.borrow_mut(), "{r:.3} {g:.3} {b:.3} rg {path}f");
        }
        advance
    }
}

/// Die Partitur der Parts als PDF-Dokument.
pub(crate) fn write(parts: &[PartScore], layout: &Layout) -> Result<Vec<u8>, String> {
    let font = FontRef::new(super::view::FONT).map_err(|e| format!("Leland: {e}"))?;
    let (w, h) = layout.paper.size();
    let margin = (layout.margin_mm.clamp(5.0, 50.0) * MM).min(w / 4.0);
    // Notensatz-Einheiten → Punkte: ein Zwischenraum (SP Einheiten) wird `space_mm` groß.
    let k = layout.space_mm.clamp(1.0, 3.0) * MM / SP;
    let width_units = (w - 2.0 * margin) / k;
    let height_units = (h - 2.0 * margin) / k;
    let colors = Colors::print();
    let mut pages: Vec<String> = Vec::new();
    let new_page = |font| Page {
        ops: RefCell::new(String::new()),
        font,
    };
    let mut page = new_page(&font);
    let mut y = 0.0f32;
    if !layout.title.trim().is_empty() {
        let size = 18.0 / k;
        let text_w = layout.title.chars().count() as f32 * size * 0.5;
        page.text(
            Pos2::new(((width_units - text_w) / 2.0).max(0.0), size),
            Align2::LEFT_BOTTOM,
            layout.title.clone(),
            FontId::proportional(size),
            colors.ink,
        );
        y = size * 2.0;
    }
    for part in parts {
        let staves = part.score.clefs.len();
        let sys_h = system_height(staves) * SP;
        let systems = break_systems(&part.score, width_units / SP - 2.0);
        let name_h = 11.0 / k * 1.6;
        if y + name_h + sys_h > height_units && y > 0.0 {
            pages.push(page.ops.take());
            page = new_page(&font);
            y = 0.0;
        }
        page.text(
            Pos2::new(0.0, y + 11.0 / k),
            Align2::LEFT_BOTTOM,
            part.name.clone(),
            FontId::proportional(11.0 / k),
            colors.ink,
        );
        y += name_h;
        for (n, sys) in systems.iter().enumerate() {
            if y + sys_h > height_units && y > 0.0 {
                pages.push(page.ops.take());
                page = new_page(&font);
                y = 0.0;
            }
            let mut frame = Frame::default();
            draw_system(
                &page,
                &part.score,
                sys,
                Pos2::new(SP, y + super::view::PAD * SP),
                n + 1 == systems.len(),
                &colors,
                u64::MAX,
                true,
                &mut frame,
            );
            y += sys_h;
        }
        y += SP * 2.0;
    }
    pages.push(page.ops.take());
    Ok(document(&pages, w, h, margin, k, &layout.title))
}

/// Setzt die Objekte zusammen: Katalog, Seitenbaum, Schrift, Seiten mit Inhalt, Info.
fn document(pages: &[String], w: f32, h: f32, margin: f32, k: f32, title: &str) -> Vec<u8> {
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets: Vec<usize> = Vec::new();
    let mut obj = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        let n = offsets.len();
        out.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    let first_page = 5;
    let kids: Vec<String> = (0..pages.len())
        .map(|i| format!("{} 0 R", first_page + 2 * i))
        .collect();
    obj(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    obj(
        &mut out,
        format!(
            "<< /Type /Pages /Kids [{}] /Count {} >>",
            kids.join(" "),
            pages.len()
        )
        .as_bytes(),
    );
    obj(
        &mut out,
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    );
    obj(
        &mut out,
        format!("<< /Title {} /Producer (StudioCraft) >>", pdf_string(title)).as_bytes(),
    );
    for (i, content) in pages.iter().enumerate() {
        // Spiegeln (y nach unten), an den Rand rücken, auf Notensatz-Einheiten skalieren.
        let stream = format!(
            "q 1 0 0 -1 0 {h:.3} cm {k:.5} 0 0 {k:.5} {margin:.3} {margin:.3} cm\n{content}Q\n"
        );
        let n = first_page + 2 * i;
        obj(
            &mut out,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w:.2} {h:.2}] \
                 /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
                n + 1
            )
            .as_bytes(),
        );
        let mut body = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
        body.extend_from_slice(stream.as_bytes());
        body.extend_from_slice(b"endstream");
        obj(&mut out, &body);
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes(),
    );
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R /Info 4 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len() + 1
        )
        .as_bytes(),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::super::notation::{Input, layout};
    use super::*;
    use studiocraft_project::{KeySignature, Note, TempoMap};

    fn part(bars: u64) -> PartScore {
        let tempo = TempoMap::default();
        let notes: Vec<Note> = (0..bars as i64 * 8)
            .map(|k| Note {
                start: k * 480,
                length: 480,
                pitch: 60 + (k % 12) as u8,
                velocity: 100,
                spelling: 0,
            })
            .collect();
        PartScore {
            name: "Geige (Solo)".into(),
            score: layout(&Input {
                notes: &notes,
                part_start: 0,
                part_len: bars * 3840,
                tempo: &tempo,
                key: KeySignature::C_MAJOR,
                quantize: 240,
            }),
        }
    }

    #[test]
    fn writes_a_valid_looking_pdf_with_pages() {
        let l = Layout {
            title: "Übung (1)".into(),
            ..Layout::default()
        };
        let pdf = write(&[part(4)], &l).unwrap();
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.starts_with("%PDF-1.4"));
        assert!(text.trim_end().ends_with("%%EOF"));
        assert!(text.contains("/Count 1"));
        assert!(text.contains("(\\334bung \\(1\\)) Tj"), "title in WinAnsi");
        assert!(text.contains(" c "), "glyph outlines as curves");
        // The xref offsets point at the objects.
        let xref_at: usize = text
            .rsplit("startxref\n")
            .next()
            .and_then(|s| s.lines().next())
            .and_then(|s| s.parse().ok())
            .unwrap();
        // Byte offsets (the header has binary bytes, so not in the lossy text).
        assert!(pdf[xref_at..].starts_with(b"xref"));
        let entry = String::from_utf8_lossy(&pdf[xref_at..]);
        let first: usize = entry.lines().nth(3).unwrap()[..10].parse().unwrap();
        assert!(pdf[first..].starts_with(b"1 0 obj"));
    }

    /// Mit `STUDIOCRAFT_SHOTS`: Beispieldateien zum Ansehen (PDF und MusicXML).
    #[test]
    fn sample_files_for_review() {
        let Ok(dir) = std::env::var("STUDIOCRAFT_SHOTS") else {
            return;
        };
        let parts = [part(6)];
        let l = Layout {
            title: "Übung".into(),
            ..Layout::default()
        };
        let dir = std::path::Path::new(&dir);
        std::fs::write(dir.join("score.pdf"), write(&parts, &l).unwrap()).unwrap();
        let xml = super::super::musicxml::write("Übung", &parts);
        std::fs::write(dir.join("score.musicxml"), xml).unwrap();
    }

    #[test]
    fn long_parts_and_large_staves_flow_onto_more_pages() {
        let big = Layout {
            space_mm: 3.0,
            ..Layout::default()
        };
        let pdf = write(&[part(60), part(10)], &big).unwrap();
        let text = String::from_utf8_lossy(&pdf);
        let count: usize = text
            .split("/Count ")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse().ok())
            .unwrap();
        assert!(count > 2, "{count} pages");
        assert_eq!(text.matches("/Type /Page ").count(), count);
        let letter = Layout {
            paper: Paper::Letter,
            ..Layout::default()
        };
        let pdf = write(&[part(1)], &letter).unwrap();
        assert!(String::from_utf8_lossy(&pdf).contains("/MediaBox [0 0 612.00 792.00]"));
    }
}
