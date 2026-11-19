//! Mithören (Monitor): Der Eingangs-Callback schiebt seine Samples zusätzlich in einen
//! eigenen Ringpuffer, den der Audio-Thread Block für Block liest und in die Kanäle der
//! Spuren mit eingeschaltetem Monitor mischt (Spec „Spurkopf“: Monitor, Issue #19).
//!
//! Eingang und Ausgang sind getrennte Streams mit eigenem Takt. Damit die Verzögerung nicht
//! wächst, verwirft [`MonitorInput::read`] alles, was über [`MAX_LAG_FRAMES`] hinaus wartet;
//! fehlen Samples, wird mit Stille aufgefüllt.

/// Höchstens so viele Frames dürfen zwischen Eingang und Ausgang liegen (rund 43 ms bei 48 kHz).
pub const MAX_LAG_FRAMES: usize = 2048;

/// Puffer zwischen Eingangs-Callback und Audio-Thread in Sekunden.
const RING_SECONDS_DIV: usize = 2;

/// Lesende Seite im Audio-Thread. Der Puffer wird im Hauptthread angelegt.
pub struct MonitorInput {
    consumer: rtrb::Consumer<f32>,
    channels: usize,
    buf: Box<[f32]>,
}

impl std::fmt::Debug for MonitorInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MonitorInput")
            .field("channels", &self.channels)
            .finish_non_exhaustive()
    }
}

/// Ringpuffer für einen Eingang mit `channels` Kanälen bei `rate`. `max_frames` ist der größte
/// Block, den der Audio-Thread auf einmal liest.
pub fn monitor_ring(
    channels: u16,
    rate: u32,
    max_frames: usize,
) -> (rtrb::Producer<f32>, MonitorInput) {
    let channels = usize::from(channels.max(1));
    let capacity = (rate as usize / RING_SECONDS_DIV).max(max_frames + MAX_LAG_FRAMES) * channels;
    let (producer, consumer) = rtrb::RingBuffer::new(capacity);
    let input = MonitorInput {
        consumer,
        channels,
        buf: vec![0.0; max_frames * channels].into_boxed_slice(),
    };
    (producer, input)
}

/// Eingangs-Callback: schreibt nur ganze Frames; ist der Puffer voll, fällt der Rest weg.
/// Echtzeitsicher.
pub fn push_frames(
    producer: &mut rtrb::Producer<f32>,
    channels: usize,
    data: impl ExactSizeIterator<Item = f32>,
) {
    let channels = channels.max(1);
    let room = producer.slots() / channels * channels;
    let n = data.len().min(room) / channels * channels;
    if n == 0 {
        return;
    }
    if let Ok(mut chunk) = producer.write_chunk_uninit(n) {
        let (a, b) = chunk.as_mut_slices();
        let mut src = data;
        for slot in a.iter_mut().chain(b.iter_mut()) {
            slot.write(src.next().unwrap_or(0.0));
        }
        // SAFETY: alle `n` Plätze wurden oben beschrieben.
        unsafe { chunk.commit_all() };
    }
}

impl MonitorInput {
    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Audio-Thread: die nächsten `frames` Frames, verschachtelt mit [`Self::channels`]
    /// Kanälen. Echtzeitsicher (keine Allokation).
    pub fn read(&mut self, frames: usize) -> &[f32] {
        let ch = self.channels;
        let frames = frames.min(self.buf.len() / ch);
        let mut avail = self.consumer.slots() / ch;
        // Zu viel Vorlauf: die ältesten Frames verwerfen.
        if avail > frames + MAX_LAG_FRAMES {
            let skip = (avail - frames - MAX_LAG_FRAMES) * ch;
            if let Ok(chunk) = self.consumer.read_chunk(skip) {
                chunk.commit_all();
            }
            avail = self.consumer.slots() / ch;
        }
        let take = avail.min(frames) * ch;
        let out = &mut self.buf[..frames * ch];
        let mut filled = 0;
        if let Ok(chunk) = self.consumer.read_chunk(take) {
            let (a, b) = chunk.as_slices();
            out[..a.len()].copy_from_slice(a);
            out[a.len()..a.len() + b.len()].copy_from_slice(b);
            filled = a.len() + b.len();
            chunk.commit_all();
        }
        out[filled..].fill(0.0);
        out
    }
}

/// Mischt Kanal `first` (und bei `stereo` `first + 1`) aus `input` in das Stereo-Paar `dst`.
/// Mono liegt auf beiden Seiten. Kanäle, die das Gerät nicht hat, bleiben still.
pub fn mix_into(dst: &mut [f32], input: &[f32], channels: usize, first: u16, stereo: bool) {
    let channels = channels.max(1);
    let l = usize::from(first);
    let r = if stereo { l + 1 } else { l };
    if l >= channels {
        return;
    }
    for (o, frame) in dst.chunks_exact_mut(2).zip(input.chunks_exact(channels)) {
        let left = frame[l];
        let right = frame.get(r).copied().unwrap_or(0.0);
        o[0] += left;
        o[1] += right;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_whole_frames_and_pads_with_silence() {
        let (mut tx, mut rx) = monitor_ring(2, 48_000, 64);
        push_frames(&mut tx, 2, [1.0, 2.0, 3.0, 4.0].into_iter());
        let got = rx.read(4).to_vec();
        assert_eq!(got, [1.0, 2.0, 3.0, 4.0, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn drops_old_frames_beyond_the_lag_limit() {
        let (mut tx, mut rx) = monitor_ring(1, 48_000, 16);
        let total = MAX_LAG_FRAMES + 100;
        push_frames(&mut tx, 1, (0..total).map(|i| i as f32));
        let got = rx.read(16).to_vec();
        // Only the newest MAX_LAG_FRAMES + 16 frames are kept; reading starts at the oldest of them.
        assert_eq!(got[0], (total - MAX_LAG_FRAMES - 16) as f32);
    }

    #[test]
    fn partial_frames_are_never_written() {
        let (mut tx, _rx) = monitor_ring(2, 48_000, 16);
        push_frames(&mut tx, 2, [1.0, 2.0, 3.0].into_iter());
        assert_eq!(tx.slots() % 2, 0);
    }

    #[test]
    fn mono_goes_to_both_sides_and_missing_channels_are_silent() {
        let input = [0.5, 0.25, 0.1, 0.2];
        let mut dst = [0.0; 4];
        mix_into(&mut dst, &input, 2, 1, false);
        assert_eq!(dst, [0.25, 0.25, 0.2, 0.2]);
        let mut dst = [0.0; 4];
        mix_into(&mut dst, &input, 2, 1, true);
        assert_eq!(dst, [0.25, 0.0, 0.2, 0.0]);
        let mut dst = [0.0; 4];
        mix_into(&mut dst, &input, 2, 5, true);
        assert_eq!(dst, [0.0; 4]);
    }
}
