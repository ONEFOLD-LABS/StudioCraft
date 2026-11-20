//! Time-Stretch für Audio-Events, die dem Projekttempo folgen (Issue #55).
//!
//! Verfahren: WSOLA (Überlappen und Addieren mit Ähnlichkeitssuche). Die Ausgabe entsteht in
//! Körnern fester Länge mit halber Überlappung und Hann-Fenster. Für jedes Korn gibt eine
//! Abbildung „Ausgabe-Frame → Quell-Frame“ die ideale Stelle im Clip vor; um sie herum wird
//! der Ausschnitt gesucht, der am besten an das vorige Korn anschließt (Kreuzkorrelation,
//! erst grob auf jedem achten Sample, dann fein). So bleibt die Tonhöhe erhalten, und die
//! Abbildung darf sich beliebig ändern (Tempospur, Rampen).
//!
//! Gerendert wird außerhalb des Audio-Threads; ein Zwischenspeicher hält die Ergebnisse, damit
//! nur geänderte Events neu entstehen, und der Audio-Thread spielt sie wie einen gewöhnlichen
//! Clip. Beim Export wird gewartet ([`cached`]). Während der Arbeit im Projekt rechnet ein
//! eigener Arbeits-Thread ([`request`]): das Event bleibt still, bis sein Ergebnis da ist, und
//! [`take_ready`] meldet dem Haupt-Thread, dass er die Abspielliste neu bauen soll. Aufträge,
//! die bis zu ihrem Start von einer neueren Abspielliste überholt wurden, fallen weg.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
/// Höchstens so viele Samples (f32) hält der Zwischenspeicher (512 MB).
const CACHE_SAMPLES: usize = 128 << 20;
/// Höchste Länge eines gerenderten Events in Frames (eine Stunde bei 192 kHz).
pub const MAX_RENDER_FRAMES: u64 = 192_000 * 60 * 60;

/// Korngröße für eine Abtastrate aus der Größe bei 48 kHz: gerade, mindestens 256.
fn grain(rate: u32, grain_48k: usize) -> usize {
    let g = (grain_48k as f64 * f64::from(rate) / 48_000.0).round() as usize;
    (g.clamp(256, 16_384) / 2) * 2
}

/// Periodisches Hann-Fenster: bei halber Überlappung ergibt die Summe überall 1.
fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let x = std::f64::consts::TAU * i as f64 / n as f64;
            (0.5 - 0.5 * x.cos()) as f32
        })
        .collect()
}

/// Mono-Wert an Frame `f` (außerhalb: Stille).
#[inline]
fn mono(src: &[f32], f: i64) -> f32 {
    if f < 0 {
        return 0.0;
    }
    let i = f as usize * 2;
    match (src.get(i), src.get(i + 1)) {
        (Some(l), Some(r)) => 0.5 * (l + r),
        _ => 0.0,
    }
}

/// Ähnlichkeit des Ausschnitts ab `a` mit dem ab `b` (`len` Frames, jedes `step`-te Frame),
/// normiert auf die Energie von `a`: die genaue Fortsetzung schneidet am besten ab.
fn correlation(src: &[f32], a: i64, b: i64, len: usize, step: usize) -> f32 {
    let (mut cross, mut energy) = (0.0f32, 0.0f32);
    let mut k = 0;
    while k < len {
        let x = mono(src, a + k as i64);
        cross += x * mono(src, b + k as i64);
        energy += x * x;
        k += step;
    }
    cross / (energy + 1e-9).sqrt()
}

/// Dehnt verschachtelte Stereo-Samples `src` auf `out_frames` Frames. `map(j)` gibt für
/// Ausgabe-Frame `j` die Stelle im Quellmaterial in Frames (darf außerhalb liegen: Stille).
/// `grain_48k`: Korngröße in Frames bei 48 kHz (kürzer: knackiger, länger: weicher).
pub fn render(
    src: &[f32],
    rate: u32,
    grain_48k: usize,
    out_frames: usize,
    map: impl Fn(f64) -> f64,
) -> Vec<f32> {
    let n = grain(rate, grain_48k);
    let hop = n / 2;
    let delta = (n / 4) as i64;
    let win = hann(n);
    let mut out = vec![0.0f32; out_frames * 2];
    let mut norm = vec![0.0f32; out_frames];
    let mut prev: Option<i64> = None;
    // Das erste Korn beginnt eine halbe Kornlänge vor dem Anfang: jedes Frame liegt in zwei
    // Körnern, deren Fenster sich zu 1 ergänzen.
    let mut out_start = -(hop as i64);
    while out_start < out_frames as i64 {
        let center = out_start as f64 + hop as f64;
        let ideal = map(center);
        let ideal = if ideal.is_finite() {
            (ideal - hop as f64).round() as i64
        } else {
            i64::MIN / 4
        };
        let pos = match prev {
            None => ideal,
            Some(p) => {
                // Natürliche Fortsetzung des vorigen Korns.
                let target = p + hop as i64;
                let len = n / 2;
                // Grob: jede achte Verschiebung, jedes vierte Frame.
                let mut best = (f32::MIN, 0i64);
                let mut s = -delta;
                while s <= delta {
                    let c = correlation(src, ideal + s, target, len, 4);
                    if c > best.0 + 1e-6 || (c >= best.0 - 1e-6 && s.abs() < best.1.abs()) {
                        best = (c, s);
                    }
                    s += 8;
                }
                // Fein: ±8 um das grobe Ergebnis, jedes zweite Frame.
                let coarse = best.1;
                for s in (coarse - 8).max(-delta)..=(coarse + 8).min(delta) {
                    let c = correlation(src, ideal + s, target, len, 2);
                    if c > best.0 + 1e-6 || (c >= best.0 - 1e-6 && s.abs() < best.1.abs()) {
                        best = (c, s);
                    }
                }
                ideal + best.1
            }
        };
        for (k, w) in win.iter().enumerate() {
            let j = out_start + k as i64;
            if j < 0 || j as usize >= out_frames {
                continue;
            }
            let j = j as usize;
            let f = pos + k as i64;
            if f >= 0 {
                let i = f as usize * 2;
                if let (Some(l), Some(r)) = (src.get(i), src.get(i + 1)) {
                    out[j * 2] += l * w;
                    out[j * 2 + 1] += r * w;
                }
            }
            norm[j] += w;
        }
        prev = Some(pos);
        out_start += hop as i64;
    }
    for (j, &w) in norm.iter().enumerate() {
        if w > 1e-3 {
            out[j * 2] /= w;
            out[j * 2 + 1] /= w;
        }
    }
    out
}

/// Kennung eines gerenderten Events: Clip-Samples, Abbildung und Länge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key(u64);

impl Key {
    /// Kennung aus allem, was das Ergebnis bestimmt.
    pub fn new(samples: &Arc<[f32]>, parts: impl Hash) -> Self {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (Arc::as_ptr(samples) as *const f32 as usize).hash(&mut h);
        samples.len().hash(&mut h);
        parts.hash(&mut h);
        Self(h.finish())
    }
}

#[derive(Default)]
struct Cache {
    items: HashMap<Key, (Arc<[f32]>, u64)>,
    samples: usize,
    clock: u64,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

fn lookup(key: Key) -> Option<Arc<[f32]>> {
    let mut guard = CACHE.lock().ok()?;
    let cache = guard.get_or_insert_with(Cache::default);
    cache.clock += 1;
    let now = cache.clock;
    let (s, used) = cache.items.get_mut(&key)?;
    *used = now;
    Some(s.clone())
}

/// Legt ein Ergebnis ab. Wird der Speicher zu groß, fallen die am längsten nicht benutzten
/// Einträge heraus.
fn insert(key: Key, rendered: Arc<[f32]>) {
    let Ok(mut guard) = CACHE.lock() else {
        return;
    };
    let cache = guard.get_or_insert_with(Cache::default);
    let now = cache.clock;
    cache.samples += rendered.len();
    if let Some((old, _)) = cache.items.insert(key, (rendered, now)) {
        cache.samples -= old.len();
    }
    while cache.samples > CACHE_SAMPLES && cache.items.len() > 1 {
        let Some(oldest) = cache
            .items
            .iter()
            .filter(|(k, _)| **k != key)
            .min_by_key(|(_, (_, used))| *used)
            .map(|(k, _)| *k)
        else {
            break;
        };
        if let Some((s, _)) = cache.items.remove(&oldest) {
            cache.samples -= s.len();
        }
    }
}

/// Holt ein gerendertes Event aus dem Zwischenspeicher oder rendert es sofort mit `make`
/// (Export, Tests). Nicht im Audio-Thread aufrufen.
pub fn cached(key: Key, make: impl FnOnce() -> Vec<f32>) -> Arc<[f32]> {
    if let Some(hit) = lookup(key) {
        return hit;
    }
    let rendered: Arc<[f32]> = make().into();
    insert(key, rendered.clone());
    rendered
}

/// Ein Auftrag an den Arbeits-Thread.
type Job = Box<dyn FnOnce() -> Vec<f32> + Send>;

/// Aufträge in Arbeit oder in der Warteschlange, mit der Generation der Abspielliste, die
/// sie zuletzt brauchte.
static PENDING: Mutex<Vec<(Key, u64)>> = Mutex::new(Vec::new());
/// Generation der Abspielliste: steigt mit jedem Bau über [`new_generation`].
static GENERATION: AtomicU64 = AtomicU64::new(0);
/// Neue Ergebnisse seit dem letzten [`take_ready`].
static READY: AtomicBool = AtomicBool::new(false);
static WORKER: Mutex<Option<std::sync::mpsc::Sender<(Key, Job)>>> = Mutex::new(None);

/// Beginnt eine neue Abspielliste: Aufträge, die sie nicht mehr anfordert, fallen weg.
pub fn new_generation() -> u64 {
    GENERATION.fetch_add(1, Ordering::SeqCst) + 1
}

/// Ob seit dem letzten Aufruf Ergebnisse fertig geworden sind (dann die Abspielliste neu
/// bauen).
pub fn take_ready() -> bool {
    READY.swap(false, Ordering::SeqCst)
}

/// Anzahl offener Aufträge (für die Statuszeile).
pub fn busy() -> usize {
    PENDING.lock().map_or(0, |p| p.len())
}

fn worker() -> Option<std::sync::mpsc::Sender<(Key, Job)>> {
    let mut guard = WORKER.lock().ok()?;
    if guard.is_none() {
        let (tx, rx) = std::sync::mpsc::channel::<(Key, Job)>();
        std::thread::Builder::new()
            .name("studiocraft-stretch".into())
            .spawn(move || {
                for (key, job) in rx {
                    let wanted = PENDING.lock().ok().and_then(|p| {
                        p.iter()
                            .find(|(k, _)| *k == key)
                            .map(|(_, g)| *g >= GENERATION.load(Ordering::SeqCst))
                    });
                    if wanted == Some(true) {
                        insert(key, job().into());
                        READY.store(true, Ordering::SeqCst);
                    }
                    if let Ok(mut p) = PENDING.lock() {
                        p.retain(|(k, _)| *k != key);
                    }
                }
            })
            .ok()?;
        *guard = Some(tx);
    }
    guard.clone()
}

/// Wie [`cached`], aber ohne zu warten: Fehlt das Ergebnis, rechnet es der Arbeits-Thread,
/// und es gibt `None` (das Event bleibt bis dahin still). `generation` aus
/// [`new_generation`]. Im Browser (ohne Threads) wird sofort gerendert.
pub fn request(
    key: Key,
    generation: u64,
    make: impl FnOnce() -> Vec<f32> + Send + 'static,
) -> Option<Arc<[f32]>> {
    if let Some(hit) = lookup(key) {
        return Some(hit);
    }
    if cfg!(target_arch = "wasm32") {
        return Some(cached(key, make));
    }
    if let Ok(mut p) = PENDING.lock() {
        if let Some(entry) = p.iter_mut().find(|(k, _)| *k == key) {
            entry.1 = generation;
            return None;
        }
        p.push((key, generation));
    }
    let job: Job = Box::new(make);
    let job = match worker() {
        Some(tx) => match tx.send((key, job)) {
            Ok(()) => return None,
            Err(e) => (e.0).1,
        },
        None => job,
    };
    // Kein Arbeits-Thread: sofort rendern.
    if let Ok(mut p) = PENDING.lock() {
        p.retain(|(k, _)| *k != key);
    }
    Some(cached(key, job))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f64, rate: u32, frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let v = (std::f64::consts::TAU * freq * i as f64 / f64::from(rate)).sin() as f32;
                [v, v]
            })
            .collect()
    }

    /// Nulldurchgänge (aufwärts) im linken Kanal zwischen zwei Frames.
    fn crossings(s: &[f32], from: usize, to: usize) -> usize {
        (from + 1..to)
            .filter(|&i| s[(i - 1) * 2] < 0.0 && s[i * 2] >= 0.0)
            .count()
    }

    #[test]
    fn stretching_keeps_the_pitch_and_hits_the_length() {
        let rate = 48_000;
        let src = sine(440.0, rate, rate as usize);
        // Halbe Geschwindigkeit: eine Sekunde Quelle wird zwei Sekunden lang.
        let out = render(&src, rate, 2048, 2 * rate as usize, |j| j / 2.0);
        assert_eq!(out.len(), 4 * rate as usize);
        let n = crossings(&out, 4800, 4800 + 48_000);
        assert!((435..=445).contains(&n), "{n} Schwingungen pro Sekunde");
        // Doppelte Geschwindigkeit.
        let out = render(&src, rate, 2048, rate as usize / 2, |j| j * 2.0);
        let n = crossings(&out, 2400, 2400 + 12_000);
        assert!((107..=113).contains(&n), "{n} Schwingungen in 0,25 s");
        // Kein Sprung im Pegel: das Ergebnis bleibt nah an Amplitude 1.
        let peak = out[4800..20_000].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((0.85..=1.15).contains(&peak), "Spitze {peak}");
    }

    #[test]
    fn identity_mapping_reproduces_the_source() {
        let rate = 44_100;
        let src = sine(220.0, rate, 20_000);
        let out = render(&src, rate, 2048, 20_000, |j| j);
        let err = src[8000..30_000]
            .iter()
            .zip(&out[8000..30_000])
            .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        // Die Suche rechnet auf ausgedünnten Frames: kleine Phasenfehler (unter −40 dB) bleiben.
        assert!(err < 1e-2, "Abweichung {err}");
    }

    #[test]
    fn outside_the_source_is_silence_and_nothing_panics() {
        let src = sine(100.0, 48_000, 1000);
        let out = render(&src, 48_000, 2048, 5000, |j| j - 2000.0);
        assert!(out[..2000].iter().all(|v| v.abs() < 1e-6));
        // Die Suche verschiebt Material um höchstens eine Viertel-Kornlänge (512 Frames).
        assert!(out[(3000 + 512 + 64) * 2..].iter().all(|v| v.abs() < 1e-6));
        // Leere Quelle, leere Ausgabe, NaN-Abbildung.
        assert!(
            render(&[], 48_000, 2048, 100, |j| j)
                .iter()
                .all(|v| *v == 0.0)
        );
        assert!(render(&src, 48_000, 2048, 0, |j| j).is_empty());
        assert!(
            render(&src, 48_000, 2048, 100, |_| f64::NAN)
                .iter()
                .all(|v| *v == 0.0)
        );
    }

    #[test]
    fn outdated_background_jobs_are_skipped() {
        let a: Arc<[f32]> = vec![0.25; 64].into();
        let old = Key::new(&a, "alt");
        let g = new_generation();
        // Den Arbeits-Thread mit einem langsamen Auftrag belegen, dann einen veralteten
        // dahinter stellen.
        let slow = Key::new(&a, "langsam");
        assert!(
            request(slow, g, || {
                std::thread::sleep(std::time::Duration::from_millis(200));
                vec![1.0; 4]
            })
            .is_none()
        );
        assert!(request(old, g, || vec![2.0; 4]).is_none());
        // Die nächste Abspielliste braucht nur noch `slow`.
        let g2 = new_generation();
        let _ = request(slow, g2, || unreachable!("ist schon in Arbeit"));
        for _ in 0..100 {
            if lookup(slow).is_some() && busy_without(old) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(lookup(slow).is_some());
        assert!(
            lookup(old).is_none(),
            "überholter Auftrag wurde nicht gerechnet"
        );
    }

    /// Ob `key` nicht mehr in Arbeit ist.
    fn busy_without(key: Key) -> bool {
        PENDING
            .lock()
            .is_ok_and(|p| !p.iter().any(|(k, _)| *k == key))
    }

    #[test]
    fn the_cache_returns_the_same_render() {
        let a: Arc<[f32]> = vec![0.5; 64].into();
        let key = Key::new(&a, (1u64, 2u64));
        let first = cached(key, || vec![1.0; 8]);
        let second = cached(key, || panic!("nicht neu rendern"));
        assert!(Arc::ptr_eq(&first, &second));
        assert_ne!(key, Key::new(&a, (1u64, 3u64)));
    }
}
