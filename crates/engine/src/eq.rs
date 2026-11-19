//! Kanal-EQ: Biquad-Filter je Band (Spec „Mixer“ → Signalfluss, Punkt 4; Issue #29).
//!
//! Die Koeffizienten folgen den allgemein bekannten Formeln für digitale Glocken-, Shelf-,
//! Hoch- und Tiefpassfilter (bilineare Transformation analoger Prototypen zweiter Ordnung).
//! Sie werden im Hauptthread berechnet ([`Biquad::design`]); der Audio-Thread rechnet nur noch
//! mit Zuständen, die vorab angelegt sind.

use studiocraft_project::{BandKind, ChannelEq, EqBand};

/// Koeffizienten eines Filters zweiter Ordnung, auf `a0 = 1` normiert.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Biquad {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
}

/// Zustand eines Bands für einen Kanal (Direktform I: zwei Eingänge, zwei Ausgänge zurück).
pub type BandState = [f32; 4];

/// Zustände aller Bänder eines Kanalzugs, links und rechts.
pub type EqState = [[BandState; 2]; 4];

impl Biquad {
    /// Filter für `band` bei Abtastrate `rate`.
    pub fn design(band: &EqBand, rate: u32) -> Self {
        let b = band.sanitized();
        let fs = f64::from(rate.max(1));
        let f = f64::from(b.freq).min(fs * 0.49);
        let w0 = std::f64::consts::TAU * f / fs;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * f64::from(b.q));
        let a = 10f64.powf(f64::from(b.gain) / 40.0);
        let sq = 2.0 * a.sqrt() * alpha;
        let (b0, b1, b2, a0, a1, a2) = match b.kind {
            BandKind::Peak => (
                1.0 + alpha * a,
                -2.0 * cos,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos,
                1.0 - alpha / a,
            ),
            BandKind::LowShelf => (
                a * ((a + 1.0) - (a - 1.0) * cos + sq),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                a * ((a + 1.0) - (a - 1.0) * cos - sq),
                (a + 1.0) + (a - 1.0) * cos + sq,
                -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                (a + 1.0) + (a - 1.0) * cos - sq,
            ),
            BandKind::HighShelf => (
                a * ((a + 1.0) + (a - 1.0) * cos + sq),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                a * ((a + 1.0) + (a - 1.0) * cos - sq),
                (a + 1.0) - (a - 1.0) * cos + sq,
                2.0 * ((a - 1.0) - (a + 1.0) * cos),
                (a + 1.0) - (a - 1.0) * cos - sq,
            ),
            BandKind::HighCut => (
                (1.0 - cos) / 2.0,
                1.0 - cos,
                (1.0 - cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
            BandKind::LowCut => (
                (1.0 + cos) / 2.0,
                -(1.0 + cos),
                (1.0 + cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
        };
        Self {
            b0: (b0 / a0) as f32,
            b1: (b1 / a0) as f32,
            b2: (b2 / a0) as f32,
            a1: (a1 / a0) as f32,
            a2: (a2 / a0) as f32,
        }
    }

    /// Verstärkung in dB bei `freq` (für die Kurvenanzeige).
    pub fn magnitude_db(&self, freq: f32, rate: u32) -> f32 {
        let w = std::f64::consts::TAU * f64::from(freq) / f64::from(rate.max(1));
        let (s1, c1) = w.sin_cos();
        let (s2, c2) = (2.0 * w).sin_cos();
        let (b0, b1, b2) = (f64::from(self.b0), f64::from(self.b1), f64::from(self.b2));
        let (a1, a2) = (f64::from(self.a1), f64::from(self.a2));
        let num = ((b0 + b1 * c1 + b2 * c2).powi(2) + (b1 * s1 + b2 * s2).powi(2)).sqrt();
        let den = ((1.0 + a1 * c1 + a2 * c2).powi(2) + (a1 * s1 + a2 * s2).powi(2)).sqrt();
        (20.0 * (num / den.max(1e-12)).log10()) as f32
    }

    /// Ein Sample durch das Filter. Echtzeitsicher.
    #[inline]
    pub fn tick(&self, s: &mut BandState, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * s[0] + self.b2 * s[1] - self.a1 * s[2] - self.a2 * s[3];
        // Winzige Werte auf null, damit ausklingende Filter keine Denormals rechnen.
        let y = if y.abs() < 1e-20 { 0.0 } else { y };
        *s = [x, s[0], y, s[2]];
        y
    }
}

/// Die wirksamen Bänder eines Kanal-EQs (Platz im EQ, Filter); leer, wenn er nichts ändert.
pub fn design(eq: &ChannelEq, rate: u32) -> Vec<(usize, Biquad)> {
    if !eq.active() {
        return Vec::new();
    }
    eq.bands
        .iter()
        .enumerate()
        .filter(|(_, b)| b.active())
        .map(|(i, b)| (i, Biquad::design(b, rate)))
        .collect()
}

/// Gesamter Frequenzgang in dB bei `freq` (Summe der Bänder).
pub fn response_db(eq: &ChannelEq, freq: f32, rate: u32) -> f32 {
    design(eq, rate)
        .iter()
        .map(|(_, b)| b.magnitude_db(freq, rate))
        .sum()
}

/// Bearbeitet verschachteltes Stereo an Ort und Stelle. Echtzeitsicher.
pub fn process(bands: &[(usize, Biquad)], state: &mut EqState, interleaved: &mut [f32]) {
    for &(i, f) in bands {
        let Some([l, r]) = state.get_mut(i) else {
            continue;
        };
        for fr in interleaved.chunks_exact_mut(2) {
            fr[0] = f.tick(l, fr[0]);
            fr[1] = f.tick(r, fr[1]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn band(kind: BandKind, freq: f32, gain: f32, q: f32) -> EqBand {
        EqBand {
            on: true,
            kind,
            freq,
            gain,
            q,
        }
    }

    #[test]
    fn peak_and_shelves_hit_their_gain() {
        let peak = Biquad::design(&band(BandKind::Peak, 1000.0, 6.0, 1.0), 48_000);
        assert!((peak.magnitude_db(1000.0, 48_000) - 6.0).abs() < 0.05);
        assert!(peak.magnitude_db(50.0, 48_000).abs() < 0.2);
        let low = Biquad::design(&band(BandKind::LowShelf, 200.0, -9.0, 0.7), 48_000);
        assert!((low.magnitude_db(20.0, 48_000) + 9.0).abs() < 0.3);
        assert!(low.magnitude_db(10_000.0, 48_000).abs() < 0.2);
        let high = Biquad::design(&band(BandKind::HighShelf, 5000.0, 4.0, 0.7), 48_000);
        assert!((high.magnitude_db(20_000.0, 48_000) - 4.0).abs() < 0.3);
    }

    #[test]
    fn cut_filters_remove_the_far_side() {
        let lc = Biquad::design(&band(BandKind::LowCut, 200.0, 0.0, 0.7), 48_000);
        assert!(lc.magnitude_db(20.0, 48_000) < -30.0);
        assert!(lc.magnitude_db(5000.0, 48_000).abs() < 0.1);
        let hc = Biquad::design(&band(BandKind::HighCut, 2000.0, 0.0, 0.7), 48_000);
        assert!(hc.magnitude_db(20_000.0, 48_000) < -30.0);
    }

    #[test]
    fn a_sine_through_a_boost_comes_out_louder() {
        let mut eq = ChannelEq::default();
        eq.bands[2].gain = 6.0;
        eq.bands[2].freq = 1000.0;
        let bands = design(&eq, 48_000);
        assert_eq!(bands.len(), 1, "only the band with gain works");
        let mut state: EqState = Default::default();
        let mut buf: Vec<f32> = (0..48_000)
            .flat_map(|i| {
                let v = (std::f32::consts::TAU * 1000.0 * i as f32 / 48_000.0).sin() * 0.25;
                [v, v]
            })
            .collect();
        process(&bands, &mut state, &mut buf);
        // After the filter has settled: amplitude doubled (+6 dB).
        let peak = buf[2 * 24_000..]
            .chunks(2)
            .map(|f| f[0].abs())
            .fold(0.0, f32::max);
        assert!((peak - 0.5).abs() < 0.01, "{peak}");
        assert!(design(&ChannelEq::default(), 48_000).is_empty());
    }
}
