//! Hitpoints und Tempoerkennung (Spec „Editoren“ → Sample-Editor; Issue #41).
//!
//! Beides arbeitet auf verschachteltem Stereo (`L, R, L, R, …`) und braucht keine
//! Bibliothek:
//!
//! - **Einsatzstärke**: Die Energie wird in kurzen Fenstern (etwa 10 ms) gemessen; wo sie
//!   logarithmisch sprunghaft steigt, beginnt ein Ereignis (Anschlag, Silbe, Schlag).
//! - **Hitpoints**: Spitzen der Einsatzstärke über einer gleitenden Schwelle, mit
//!   Mindestabstand. Die Empfindlichkeit (0–1) senkt die Schwelle.
//! - **Tempo**: Autokorrelation der Einsatzstärke über die Perioden von 60 bis 200 BPM; die
//!   stärkste Periode, fein interpoliert, ergibt das Tempo. Halbe und doppelte Tempi werden in
//!   den Bereich 70–180 BPM gefaltet.

/// Länge eines Analysefensters in Sekunden.
const HOP_SECONDS: f64 = 0.01;
/// Leiser als das (Energie je Sample) zählt als Stille.
const SILENCE: f32 = 1e-7;
/// Mindestabstand zweier Hitpoints in Sekunden.
const MIN_GAP_SECONDS: f64 = 0.05;
/// Tempobereich der Suche und der Bereich, in den gefaltet wird.
const BPM_SEARCH: (f64, f64) = (60.0, 200.0);
const BPM_FOLD: (f64, f64) = (70.0, 180.0);

/// Einsatzstärke je Fenster.
#[derive(Debug, Clone, PartialEq)]
pub struct OnsetStrength {
    /// Frames je Fenster.
    pub hop: usize,
    /// Anstieg der log. Energie je Fenster (≥ 0).
    pub flux: Vec<f32>,
    /// Energie je Fenster (mittleres Quadrat).
    pub energy: Vec<f32>,
}

/// Misst die Einsatzstärke von `samples` (Stereo) bei `rate`.
pub fn onset_strength(samples: &[f32], rate: u32) -> OnsetStrength {
    let hop = ((f64::from(rate) * HOP_SECONDS) as usize).max(16);
    let frames = samples.len() / 2;
    let n = frames / hop;
    let mut energy = Vec::with_capacity(n);
    for k in 0..n {
        let mut sum = 0.0f64;
        for f in k * hop..(k + 1) * hop {
            let l = samples.get(f * 2).copied().unwrap_or(0.0);
            let r = samples.get(f * 2 + 1).copied().unwrap_or(0.0);
            let m = if l.is_finite() && r.is_finite() {
                (l + r) * 0.5
            } else {
                0.0
            };
            sum += f64::from(m) * f64::from(m);
        }
        energy.push((sum / hop as f64) as f32);
    }
    let log = |e: f32| (e.max(SILENCE)).ln();
    let flux = (0..n)
        .map(|k| {
            if k == 0 {
                0.0
            } else {
                (log(energy[k]) - log(energy[k - 1])).max(0.0)
            }
        })
        .collect();
    OnsetStrength { hop, flux, energy }
}

/// Hitpoints in Frames: Einsätze von `samples` (Stereo) bei `rate`. `sensitivity` 0–1:
/// höher findet auch schwächere Einsätze.
pub fn hitpoints(samples: &[f32], rate: u32, sensitivity: f32) -> Vec<u64> {
    let s = onset_strength(samples, rate);
    let n = s.flux.len();
    let sens = if sensitivity.is_finite() {
        sensitivity.clamp(0.0, 1.0)
    } else {
        0.5
    };
    // Schwelle: Mittel der Umgebung plus ein fester Sprung (in log. Energie), der mit der
    // Empfindlichkeit kleiner wird: 0 → etwa 12 dB, 1 → etwa 2 dB Anstieg.
    let jump = 0.5 + 2.3 * (1.0 - sens);
    let gap = ((MIN_GAP_SECONDS * f64::from(rate)) as usize / s.hop).max(1);
    let window = 8usize;
    let peak_energy = s.energy.iter().copied().fold(0.0f32, f32::max);
    let gate = (peak_energy * 1e-4).max(SILENCE * 10.0);
    let mut out: Vec<u64> = Vec::new();
    let mut last: Option<usize> = None;
    for k in 1..n {
        let v = s.flux[k];
        let lo = k.saturating_sub(window);
        let hi = (k + window + 1).min(n);
        let mean = s.flux[lo..hi].iter().sum::<f32>() / (hi - lo) as f32;
        let local_max = s.flux[k.saturating_sub(2)..(k + 3).min(n)]
            .iter()
            .all(|&o| o <= v);
        let loud = s.energy[k] >= gate;
        let spaced = last.is_none_or(|l| k - l >= gap);
        if v >= mean + jump && local_max && loud && spaced {
            // Der Anstieg beginnt im vorigen Fenster.
            out.push(((k - 1) * s.hop) as u64);
            last = Some(k);
        }
    }
    out
}

/// Tempo in BPM, wenn `samples` (Stereo) bei `rate` lang genug ist und einen Puls hat.
pub fn tempo(samples: &[f32], rate: u32) -> Option<f64> {
    let s = onset_strength(samples, rate);
    let hop_s = s.hop as f64 / f64::from(rate.max(1));
    let n = s.flux.len();
    // Mindestens vier Schläge bei der langsamsten Suche.
    if (n as f64) * hop_s < 4.0 * 60.0 / BPM_SEARCH.0 {
        return None;
    }
    let mean = s.flux.iter().sum::<f32>() / n as f32;
    let x: Vec<f64> = s.flux.iter().map(|&v| f64::from(v - mean)).collect();
    let energy: f64 = x.iter().map(|v| v * v).sum();
    if energy <= 1e-9 {
        return None;
    }
    let lag_of = |bpm: f64| 60.0 / (bpm * hop_s);
    let (min_lag, max_lag) = (
        lag_of(BPM_SEARCH.1).floor().max(1.0) as usize,
        (lag_of(BPM_SEARCH.0).ceil() as usize).min(n / 2),
    );
    if max_lag <= min_lag + 1 {
        return None;
    }
    let ac = |lag: usize| -> f64 {
        let m = n - lag;
        x[..m]
            .iter()
            .zip(&x[lag..])
            .map(|(a, b)| a * b)
            .sum::<f64>()
            / m as f64
    };
    let values: Vec<f64> = (min_lag..=max_lag).map(ac).collect();
    let (best, &peak) = values
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))?;
    if peak <= 0.0 {
        return None;
    }
    // Parabel durch die Nachbarn für einen Lag zwischen den Fenstern.
    let at = |i: usize| values.get(i).copied();
    let shift = match (best.checked_sub(1).and_then(at), at(best + 1)) {
        (Some(a), Some(c)) => {
            let d = a - 2.0 * peak + c;
            if d.abs() > 1e-12 {
                (0.5 * (a - c) / d).clamp(-0.5, 0.5)
            } else {
                0.0
            }
        }
        _ => 0.0,
    };
    let mut lag = (min_lag + best) as f64 + shift;
    // Oktavfehler: Pulst es auch bei der halben Periode kräftig, ist das schnellere Tempo
    // gemeint (die stärkste Periode ist dann jeder zweite Schlag).
    while lag / 2.0 >= min_lag as f64 {
        let half = (lag / 2.0).round() as usize;
        let strength = (half.saturating_sub(1)..=half + 1)
            .filter(|&l| l >= 1 && l < n)
            .map(ac)
            .fold(f64::MIN, f64::max);
        if strength >= 0.5 * peak {
            lag /= 2.0;
        } else {
            break;
        }
    }
    let mut bpm = 60.0 / (lag * hop_s);
    while bpm < BPM_FOLD.0 {
        bpm *= 2.0;
    }
    while bpm > BPM_FOLD.1 {
        bpm /= 2.0;
    }
    bpm.is_finite().then_some(bpm)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Klicks (kurze, abklingende Rauschstöße) bei `bpm` über `seconds`, leises Rauschen
    /// dazwischen.
    fn clicks(bpm: f64, seconds: f64, rate: u32) -> (Vec<f32>, Vec<u64>) {
        let n = (seconds * f64::from(rate)) as usize;
        let mut s = vec![0.0f32; n * 2];
        let mut seed = 12345u32;
        let mut noise = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        };
        for f in 0..n {
            let v = noise() * 0.001;
            s[f * 2] = v;
            s[f * 2 + 1] = v;
        }
        let period = 60.0 / bpm * f64::from(rate);
        let mut at = Vec::new();
        let mut t = 0.1 * f64::from(rate);
        while (t as usize) < n {
            let start = t as usize;
            at.push(start as u64);
            for k in 0..(f64::from(rate) * 0.05) as usize {
                let f = start + k;
                if f >= n {
                    break;
                }
                let env = (-(k as f32) / (rate as f32 * 0.01)).exp();
                let v = noise() * 0.8 * env;
                s[f * 2] += v;
                s[f * 2 + 1] += v;
            }
            t += period;
        }
        (s, at)
    }

    #[test]
    fn hitpoints_land_on_the_clicks() {
        let rate = 48_000;
        let (s, truth) = clicks(120.0, 4.0, rate);
        let hits = hitpoints(&s, rate, 0.5);
        assert_eq!(hits.len(), truth.len(), "{hits:?} vs {truth:?}");
        let tolerance = (rate / 50) as i64; // 20 ms
        for (h, t) in hits.iter().zip(&truth) {
            assert!((*h as i64 - *t as i64).abs() <= tolerance, "{h} vs {t}");
        }
    }

    #[test]
    fn silence_and_steady_noise_have_no_hitpoints_or_tempo() {
        let rate = 44_100;
        let silent = vec![0.0f32; rate as usize * 2 * 5];
        assert!(hitpoints(&silent, rate, 1.0).is_empty());
        assert_eq!(tempo(&silent, rate), None);
        assert!(hitpoints(&[], rate, 0.5).is_empty());
        assert_eq!(tempo(&[0.5; 10], rate), None);
    }

    #[test]
    fn tempo_is_found_for_several_speeds() {
        let rate = 44_100;
        for bpm in [92.0, 120.0, 140.0, 174.0] {
            let (s, _) = clicks(bpm, 10.0, rate);
            let found = tempo(&s, rate).expect("tempo");
            assert!((found - bpm).abs() < 1.5, "{bpm}: {found}");
        }
    }

    #[test]
    fn sensitivity_controls_how_many_weak_hits_count() {
        // Steady noise at 0.1 with strong hits (0.9) and weak hits (+0.15) alternating.
        let rate = 48_000u32;
        let n = 4 * rate as usize;
        let mut seed = 777u32;
        let mut noise = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        };
        let mut s = vec![0.0f32; n * 2];
        let period = rate as usize / 2;
        for f in 0..n {
            let since = (f + period - rate as usize / 10) % period;
            let beat = (f + period - rate as usize / 10) / period;
            let burst = if f >= rate as usize / 10 && since < rate as usize / 20 {
                let env = (-(since as f32) / (rate as f32 * 0.01)).exp();
                if beat % 2 == 0 { 0.9 * env } else { 0.15 * env }
            } else {
                0.0
            };
            let v = noise() * (0.1 + burst);
            s[f * 2] = v;
            s[f * 2 + 1] = v;
        }
        let strict = hitpoints(&s, rate, 0.0).len();
        let loose = hitpoints(&s, rate, 1.0).len();
        assert!(strict >= 3 && strict < loose, "{strict} < {loose}");
    }
}
