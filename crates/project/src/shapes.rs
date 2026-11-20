//! Formen zum Zeichnen in Automationsspuren und Controller-Lanes (Spec „Automation“ →
//! Kurvenbearbeitung, „Controller-Lanes im Key-Editor“; Issue #45).
//!
//! Mit dem Zeichnen-Werkzeug zieht man von einem Anfangspunkt zu einem Endpunkt; die Form
//! füllt den Bereich dazwischen:
//!
//! - **Linie**: gerade vom Anfangs- zum Endwert.
//! - **Parabel**: beginnt flach und wird steiler (quadratisch).
//! - **Sinus**: schwingt zwischen Anfangs- und Endwert, eine Periode je `period` Ticks
//!   (das Raster), beginnend beim Anfangswert.
//! - **Rechteck**: springt je halbe Periode zwischen Anfangs- und Endwert.
//!
//! [`vertices`] liefert Stützpunkte für Automationskurven (zwischen den Punkten wird linear
//! interpoliert), [`samples`] gleichmäßig verteilte Werte für Controller-Lanes.

use std::f64::consts::TAU;

/// Höchstens so viele Punkte erzeugt eine Form (lange Bereiche mit feinem Raster).
pub const MAX_SHAPE_POINTS: usize = 4096;

/// Stützpunkte für Parabel und Sinus je Abschnitt bzw. Periode.
const SEGMENTS: u64 = 16;

/// Form des Zeichnen-Werkzeugs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DrawShape {
    /// Freihand: setzt Punkte entlang der Mausbewegung (keine Form).
    #[default]
    Free,
    Line,
    Parabola,
    Sine,
    Square,
}

impl DrawShape {
    pub const ALL: [Self; 5] = [
        Self::Free,
        Self::Line,
        Self::Parabola,
        Self::Sine,
        Self::Square,
    ];

    /// Deutscher Name (die Oberfläche übersetzt ihn).
    pub fn label(self) -> &'static str {
        match self {
            Self::Free => "Freihand",
            Self::Line => "Linie",
            Self::Parabola => "Parabel",
            Self::Sine => "Sinus",
            Self::Square => "Rechteck",
        }
    }

    /// Wert der Form bei `tick` zwischen `from` und `to` (Ticks, Wert).
    pub fn value(self, tick: u64, from: (u64, f64), to: (u64, f64), period: u64) -> f64 {
        let ((t0, v0), (t1, v1)) = ordered(from, to);
        let span = t1.saturating_sub(t0);
        let u = if span == 0 {
            1.0
        } else {
            (tick.clamp(t0, t1) - t0) as f64 / span as f64
        };
        let phase = (tick.saturating_sub(t0)) as f64 / period.max(1) as f64;
        match self {
            Self::Free | Self::Line => v0 + (v1 - v0) * u,
            Self::Parabola => v0 + (v1 - v0) * u * u,
            Self::Sine => {
                let mid = (v0 + v1) * 0.5;
                let amp = (v0 - v1) * 0.5;
                mid + amp * (TAU * phase).cos()
            }
            Self::Square => {
                if phase.fract() < 0.5 {
                    v0
                } else {
                    v1
                }
            }
        }
    }
}

/// Von links nach rechts: der frühere Punkt zuerst.
fn ordered(a: (u64, f64), b: (u64, f64)) -> ((u64, f64), (u64, f64)) {
    if a.0 <= b.0 { (a, b) } else { (b, a) }
}

/// Schrittweite, mit der höchstens [`MAX_SHAPE_POINTS`] Punkte entstehen.
fn capped_step(span: u64, step: u64) -> u64 {
    let min = span.div_ceil(MAX_SHAPE_POINTS as u64 - 1).max(1);
    step.max(min)
}

/// Gleichmäßige Werte in `step` Ticks Abstand von Anfang bis Ende (Controller-Lanes).
pub fn samples(
    shape: DrawShape,
    from: (u64, f64),
    to: (u64, f64),
    period: u64,
    step: u64,
) -> Vec<(u64, f64)> {
    let ((t0, _), (t1, _)) = ordered(from, to);
    let step = capped_step(t1 - t0, step);
    let mut out: Vec<(u64, f64)> = (t0..=t1)
        .step_by(usize::try_from(step).unwrap_or(usize::MAX))
        .map(|t| (t, shape.value(t, from, to, period)))
        .collect();
    if out.last().is_none_or(|&(t, _)| t != t1) {
        out.push((t1, shape.value(t1, from, to, period)));
    }
    out
}

/// Stützpunkte einer Automationskurve von Anfang bis Ende; dazwischen interpoliert die
/// Kurve linear. Rechtecke bekommen je Flanke zwei Punkte (einen Tick davor und darauf).
pub fn vertices(
    shape: DrawShape,
    from: (u64, f64),
    to: (u64, f64),
    period: u64,
) -> Vec<(u64, f64)> {
    let ((t0, v0), (t1, v1)) = ordered(from, to);
    let span = t1 - t0;
    match shape {
        DrawShape::Free | DrawShape::Line => {
            if span == 0 {
                vec![(t1, v1)]
            } else {
                vec![(t0, v0), (t1, v1)]
            }
        }
        DrawShape::Parabola => samples(shape, from, to, period, span.div_ceil(SEGMENTS).max(1)),
        DrawShape::Sine => samples(shape, from, to, period, (period / SEGMENTS).max(1)),
        DrawShape::Square => {
            let half = (period / 2).max(1);
            let mut out = vec![(t0, v0)];
            let mut edge = t0 + half;
            let mut high = false;
            while edge < t1 && out.len() + 2 < MAX_SHAPE_POINTS {
                let (before, after) = if high { (v1, v0) } else { (v0, v1) };
                if edge - 1 > out.last().map_or(t0, |p| p.0) {
                    out.push((edge - 1, before));
                }
                out.push((edge, after));
                high = !high;
                edge += half;
            }
            let last = if high { v1 } else { v0 };
            if out.last().is_some_and(|p| p.0 < t1) {
                out.push((t1, last));
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_and_parabola_run_from_start_to_end_value() {
        let (a, b) = ((100, 0.0), (300, 1.0));
        assert_eq!(vertices(DrawShape::Line, a, b, 120), vec![a, b]);
        assert_eq!(DrawShape::Line.value(200, a, b, 120), 0.5);
        // The parabola starts flat: halfway it is at a quarter.
        assert_eq!(DrawShape::Parabola.value(200, a, b, 120), 0.25);
        let p = vertices(DrawShape::Parabola, a, b, 120);
        assert_eq!(p.first(), Some(&a));
        assert_eq!(p.last(), Some(&b));
        assert!(p.windows(2).all(|w| w[0].0 < w[1].0 && w[0].1 <= w[1].1));
    }

    #[test]
    fn dragging_right_to_left_gives_the_same_points_mirrored_in_time() {
        let (a, b) = ((300, 1.0), (100, 0.0));
        assert_eq!(
            vertices(DrawShape::Line, a, b, 120),
            vec![(100, 0.0), (300, 1.0)]
        );
    }

    #[test]
    fn sine_starts_at_the_first_value_and_swings_to_the_second() {
        let (a, b) = ((0, 1.0), (480, 0.0));
        let v = |t| DrawShape::Sine.value(t, a, b, 240);
        assert!((v(0) - 1.0).abs() < 1e-9);
        assert!(
            (v(120) - 0.0).abs() < 1e-9,
            "half a period: the other value"
        );
        assert!((v(240) - 1.0).abs() < 1e-9, "a full period: back");
        let pts = vertices(DrawShape::Sine, a, b, 240);
        assert!(pts.len() >= 2 * 16);
        assert!(pts.iter().all(|&(_, v)| (-1e-9..=1.0 + 1e-9).contains(&v)));
        assert_eq!(pts.last().map(|p| p.0), Some(480));
    }

    #[test]
    fn square_jumps_every_half_period_with_two_points_per_edge() {
        let pts = vertices(DrawShape::Square, (0, 0.0), (240, 1.0), 120);
        assert_eq!(
            pts,
            vec![
                (0, 0.0),
                (59, 0.0),
                (60, 1.0),
                (119, 1.0),
                (120, 0.0),
                (179, 0.0),
                (180, 1.0),
                (240, 1.0),
            ]
        );
    }

    #[test]
    fn samples_are_evenly_spaced_and_end_on_the_last_tick() {
        let s = samples(DrawShape::Line, (0, 0.0), (100, 100.0), 120, 30);
        assert_eq!(
            s,
            vec![(0, 0.0), (30, 30.0), (60, 60.0), (90, 90.0), (100, 100.0)]
        );
    }

    #[test]
    fn long_ranges_with_a_fine_grid_stay_capped() {
        let far = (10_000_000, 1.0);
        for shape in DrawShape::ALL {
            assert!(vertices(shape, (0, 0.0), far, 1).len() <= MAX_SHAPE_POINTS);
            assert!(samples(shape, (0, 0.0), far, 1, 1).len() <= MAX_SHAPE_POINTS + 1);
        }
    }

    #[test]
    fn a_zero_length_drag_gives_one_point() {
        assert_eq!(
            vertices(DrawShape::Line, (50, 0.3), (50, 0.7), 120).len(),
            1
        );
        assert_eq!(
            samples(DrawShape::Sine, (50, 0.3), (50, 0.7), 120, 10).len(),
            1
        );
    }
}
