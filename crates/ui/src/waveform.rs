//! Wellenform-Übersicht (Peaks) eines Clips: Minimum und Maximum je Block,
//! damit die Anzeige bei jedem Zoom nur wenige Werte lesen muss.

/// Frames pro Peak-Block.
pub const BUCKET: usize = 256;

#[derive(Debug, Clone, Default)]
pub struct Peaks {
    pub min: Vec<f32>,
    pub max: Vec<f32>,
}

impl Peaks {
    /// Aus verschachteltem Stereo; die Kanäle werden zu Mono gemittelt.
    pub fn new(stereo: &[f32]) -> Self {
        let mut p = Self::default();
        for block in stereo.chunks(BUCKET * 2) {
            let (mut lo, mut hi) = (0.0f32, 0.0f32);
            for f in block.chunks_exact(2) {
                let v = (f[0] + f[1]) * 0.5;
                lo = lo.min(v);
                hi = hi.max(v);
            }
            p.min.push(lo);
            p.max.push(hi);
        }
        p
    }

    /// Minimum und Maximum zwischen zwei Frame-Positionen.
    pub fn range(&self, from: f64, to: f64) -> (f32, f32) {
        let n = self.min.len();
        if n == 0 || to <= 0.0 {
            return (0.0, 0.0);
        }
        let b0 = (from.max(0.0) as usize / BUCKET).min(n);
        let b1 = ((to.max(0.0) as usize).div_ceil(BUCKET))
            .clamp(b0 + 1, n.max(b0 + 1))
            .min(n);
        if b0 >= b1 {
            return (0.0, 0.0);
        }
        let lo = self.min[b0..b1].iter().copied().fold(0.0, f32::min);
        let hi = self.max[b0..b1].iter().copied().fold(0.0, f32::max);
        (lo, hi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_cover_blocks() {
        let mut s = vec![0.0f32; BUCKET * 2 * 3];
        s[BUCKET * 2 + 10] = 0.8; // Block 1, linker Kanal
        s[BUCKET * 2 + 11] = 0.8;
        s[BUCKET * 4 + 20] = -1.0; // Block 2, nur links → Mono -0,5
        let p = Peaks::new(&s);
        assert_eq!(p.max.len(), 3);
        assert_eq!(p.range(0.0, BUCKET as f64), (0.0, 0.0));
        assert_eq!(p.range(BUCKET as f64, 2.0 * BUCKET as f64), (0.0, 0.8));
        assert_eq!(p.range(0.0, 3.0 * BUCKET as f64), (-0.5, 0.8));
        assert_eq!(
            p.range(10.0 * BUCKET as f64, 11.0 * BUCKET as f64),
            (0.0, 0.0),
            "hinter dem Ende"
        );
    }
}
