//! Offline-Bearbeitung von Clips (Spec „Editoren“ → Sample-Editor; Issue #18).
//!
//! Alle Funktionen arbeiten auf verschachteltem Stereo (`L, R, L, R, …`) im Frame-Bereich
//! `from..to` und lassen den Rest unverändert. Bereiche außerhalb der Samples werden begrenzt.

use std::ops::Range;

/// Begrenzt einen Frame-Bereich auf die vorhandenen Frames.
fn frames(samples: &[f32], range: Range<u64>) -> Range<usize> {
    let n = samples.len() / 2;
    let a = usize::try_from(range.start).unwrap_or(n).min(n);
    let b = usize::try_from(range.end).unwrap_or(n).min(n);
    a..b.max(a)
}

/// Höchster Betrag im Bereich.
pub fn peak(samples: &[f32], range: Range<u64>) -> f32 {
    let r = frames(samples, range);
    samples[r.start * 2..r.end * 2]
        .iter()
        .filter(|v| v.is_finite())
        .fold(0.0, |m, v| m.max(v.abs()))
}

/// Verstärkt den Bereich um `db` Dezibel.
pub fn gain(samples: &mut [f32], range: Range<u64>, db: f32) {
    let g = 10f32.powf(db / 20.0);
    if !g.is_finite() {
        return;
    }
    let r = frames(samples, range);
    for v in &mut samples[r.start * 2..r.end * 2] {
        *v *= g;
    }
}

/// Hebt die Spitze im Bereich auf `target_db` (0 = Vollaussteuerung). Stille bleibt Stille.
/// Gibt die angewendete Verstärkung in dB zurück.
pub fn normalize(samples: &mut [f32], range: Range<u64>, target_db: f32) -> f32 {
    let p = peak(samples, range.clone());
    if p <= 1e-9 {
        return 0.0;
    }
    let db = target_db - 20.0 * p.log10();
    gain(samples, range, db);
    db
}

/// Kehrt den Bereich zeitlich um (die Kanäle bleiben getrennt).
pub fn reverse(samples: &mut [f32], range: Range<u64>) {
    let r = frames(samples, range);
    let slice = &mut samples[r.start * 2..r.end * 2];
    let n = slice.len() / 2;
    for i in 0..n / 2 {
        let j = n - 1 - i;
        slice.swap(2 * i, 2 * j);
        slice.swap(2 * i + 1, 2 * j + 1);
    }
}

/// Lineares Einblenden über den Bereich (0 → 1).
pub fn fade_in(samples: &mut [f32], range: Range<u64>) {
    let r = frames(samples, range);
    let n = (r.end - r.start).max(1) as f32;
    for (i, f) in samples[r.start * 2..r.end * 2]
        .chunks_exact_mut(2)
        .enumerate()
    {
        let g = i as f32 / n;
        f[0] *= g;
        f[1] *= g;
    }
}

/// Lineares Ausblenden über den Bereich (1 → 0).
pub fn fade_out(samples: &mut [f32], range: Range<u64>) {
    let r = frames(samples, range);
    let n = (r.end - r.start).max(1) as f32;
    for (i, f) in samples[r.start * 2..r.end * 2]
        .chunks_exact_mut(2)
        .enumerate()
    {
        let g = 1.0 - (i + 1) as f32 / n;
        f[0] *= g;
        f[1] *= g;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(n: usize) -> Vec<f32> {
        (0..n)
            .flat_map(|i| [i as f32 * 0.1, -(i as f32) * 0.1])
            .collect()
    }

    #[test]
    fn gain_and_normalize_touch_only_the_range() {
        let mut s = vec![0.25f32; 8];
        gain(&mut s, 1..3, 6.0206);
        assert!((s[2] - 0.5).abs() < 1e-4 && (s[5] - 0.5).abs() < 1e-4);
        assert_eq!((s[0], s[6]), (0.25, 0.25));
        let mut s = vec![0.0, 0.0, 0.5, -0.25, 0.1, 0.1];
        let db = normalize(&mut s, 0..10, 0.0);
        assert!((db - 6.0206).abs() < 1e-3);
        assert!((s[2] - 1.0).abs() < 1e-5 && (s[3] + 0.5).abs() < 1e-5);
        let mut silence = vec![0.0f32; 4];
        assert_eq!(normalize(&mut silence, 0..2, 0.0), 0.0);
    }

    #[test]
    fn reverse_keeps_channels_and_the_rest() {
        let mut s = ramp(5);
        reverse(&mut s, 1..4);
        let left: Vec<f32> = s.chunks(2).map(|f| (f[0] * 10.0).round()).collect();
        let right: Vec<f32> = s.chunks(2).map(|f| (f[1] * 10.0).round()).collect();
        assert_eq!(left, [0.0, 3.0, 2.0, 1.0, 4.0]);
        assert_eq!(right, [0.0, -3.0, -2.0, -1.0, -4.0]);
    }

    #[test]
    fn fades_go_from_and_to_silence() {
        let mut s = vec![1.0f32; 2 * 4];
        fade_in(&mut s, 0..4);
        assert_eq!(s[0], 0.0);
        assert!(s[6] > 0.7 && s[6] < 1.0);
        let mut s = vec![1.0f32; 2 * 4];
        fade_out(&mut s, 0..4);
        assert_eq!(s[6], 0.0);
        assert!(s[0] > 0.7);
    }

    #[test]
    fn ranges_beyond_the_clip_are_clamped() {
        let mut s = vec![0.5f32; 4];
        gain(&mut s, 1..99, 6.0206);
        // A reversed range (from user input) is treated as empty.
        let (from, to) = (5, 2);
        reverse(&mut s, from..to);
        fade_in(&mut s, u64::MAX - 1..u64::MAX);
        assert!((s[0] - 0.5).abs() < 1e-6 && (s[2] - 1.0).abs() < 1e-3);
    }
}
