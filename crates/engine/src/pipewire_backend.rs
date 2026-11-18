//! Audio-Ausgabe über PipeWire. Der [`EngineCore`] läuft im
//! Echtzeit-Callback des Streams (`RT_PROCESS`).

use std::{
    sync::mpsc,
    thread::{self, JoinHandle},
};

use pipewire as pw;
use pw::{properties::properties, spa};

use crate::EngineCore;

const CHANNELS: usize = 2;
const FRAME_BYTES: usize = CHANNELS * std::mem::size_of::<f32>();
/// Größter Block, der ohne Aufteilung gerendert wird.
const SCRATCH_FRAMES: usize = 8192;
/// Gewünschte Blockgröße in Frames (Spec-Ziel: ≤ 10 ms bei 48 kHz).
pub const DEFAULT_BLOCK_FRAMES: u32 = 256;

/// Laufende PipeWire-Ausgabe. Beim Drop wird der Stream beendet.
pub struct PipeWireOutput {
    quit: Option<pw::channel::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

struct CallbackData {
    core: EngineCore,
    scratch: Box<[f32]>,
}

impl PipeWireOutput {
    /// Startet einen Ausgabestream mit der Abtastrate des Kerns.
    /// Kehrt zurück, sobald der Stream verbunden ist oder der Start scheitert.
    pub fn start(core: EngineCore, name: &str) -> Result<Self, String> {
        let (quit_tx, quit_rx) = pw::channel::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        let name = name.to_owned();

        let thread = thread::Builder::new()
            .name("pipewire".into())
            .spawn(move || {
                if let Err(e) = run(core, &name, quit_rx, &ready_tx) {
                    let _ = ready_tx.send(Err(e));
                }
            })
            .map_err(|e| e.to_string())?;

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self { quit: Some(quit_tx), thread: Some(thread) }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err("PipeWire-Thread wurde unerwartet beendet".into()),
        }
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        if let Some(q) = self.quit.take() {
            let _ = q.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for PipeWireOutput {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run(
    core: EngineCore,
    name: &str,
    quit_rx: pw::channel::Receiver<()>,
    ready: &mpsc::Sender<Result<(), String>>,
) -> Result<(), String> {
    let err = |e: pw::Error| format!("PipeWire: {e}");
    pw::init();
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(err)?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(err)?;
    let pw_core = context
        .connect_rc(None)
        .map_err(|e| format!("Keine Verbindung zum PipeWire-Server: {e}"))?;

    let rate = core.sample_rate();
    let latency = format!("{DEFAULT_BLOCK_FRAMES}/{rate}");
    let stream = pw::stream::StreamBox::new(
        &pw_core,
        name,
        properties! {
            *pw::keys::MEDIA_TYPE => "Audio",
            *pw::keys::MEDIA_ROLE => "Production",
            *pw::keys::MEDIA_CATEGORY => "Playback",
            *pw::keys::AUDIO_CHANNELS => "2",
            *pw::keys::APP_NAME => "StudioCraft",
            *pw::keys::NODE_LATENCY => latency.as_str(),
        },
    )
    .map_err(err)?;

    let data = CallbackData { core, scratch: vec![0.0; SCRATCH_FRAMES * CHANNELS].into() };
    let _listener = stream
        .add_local_listener_with_user_data(data)
        .state_changed(|_, _, old, new| {
            if let pw::stream::StreamState::Error(msg) = &new {
                eprintln!("PipeWire-Stream-Fehler ({old:?} -> Error): {msg}");
            }
        })
        .process(|stream, data| {
            let Some(mut buffer) = stream.dequeue_buffer() else { return };
            // Nur so viele Frames rendern, wie der Graph in diesem Zyklus braucht.
            // Den ganzen Puffer zu füllen würde Audio vorpuffern und die Latenz erhöhen.
            let requested = buffer.requested() as usize;
            let datas = buffer.datas_mut();
            let Some(d) = datas.first_mut() else { return };
            let mut written = 0usize;
            if let Some(bytes) = d.data() {
                let capacity = bytes.len() / FRAME_BYTES;
                let frames = if requested > 0 { requested.min(capacity) } else { capacity };
                while written < frames {
                    let n = (frames - written).min(SCRATCH_FRAMES);
                    let block = &mut data.scratch[..n * CHANNELS];
                    data.core.process(block);
                    let dst = &mut bytes[written * FRAME_BYTES..(written + n) * FRAME_BYTES];
                    for (chunk, s) in dst.chunks_exact_mut(4).zip(block.iter()) {
                        chunk.copy_from_slice(&s.to_le_bytes());
                    }
                    written += n;
                }
            }
            let chunk = d.chunk_mut();
            *chunk.offset_mut() = 0;
            *chunk.stride_mut() = FRAME_BYTES as _;
            *chunk.size_mut() = (written * FRAME_BYTES) as _;
        })
        .register()
        .map_err(err)?;

    let mut info = spa::param::audio::AudioInfoRaw::new();
    info.set_format(spa::param::audio::AudioFormat::F32LE);
    info.set_rate(rate);
    info.set_channels(CHANNELS as u32);
    let mut position = [0; spa::param::audio::MAX_CHANNELS];
    position[0] = spa::sys::SPA_AUDIO_CHANNEL_FL;
    position[1] = spa::sys::SPA_AUDIO_CHANNEL_FR;
    info.set_position(position);

    let bytes: Vec<u8> = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(spa::pod::Object {
            type_: spa::sys::SPA_TYPE_OBJECT_Format,
            id: spa::sys::SPA_PARAM_EnumFormat,
            properties: info.into(),
        }),
    )
    .map_err(|e| format!("Format-Pod: {e:?}"))?
    .0
    .into_inner();
    let mut params = [spa::pod::Pod::from_bytes(&bytes).ok_or("Format-Pod ungültig")?];

    stream
        .connect(
            spa::utils::Direction::Output,
            None,
            pw::stream::StreamFlags::AUTOCONNECT
                | pw::stream::StreamFlags::MAP_BUFFERS
                | pw::stream::StreamFlags::RT_PROCESS,
            &mut params,
        )
        .map_err(err)?;

    let ml = mainloop.clone();
    let _quit = quit_rx.attach(mainloop.loop_(), move |()| ml.quit());

    let _ = ready.send(Ok(()));
    mainloop.run();
    Ok(())
}
