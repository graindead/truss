//! Low-latency multi-stem audio playback for the chart preview.
//!
//! Stems are decoded lazily from the playback position (never the whole song up front),
//! mixed to stereo at the device rate, and handed to the audio callback through a
//! lock-free ring buffer. The number of frames the callback has consumed is the clock
//! that drives the highway, so audio and visuals can't drift apart.

use rtrb::{Consumer, RingBuffer};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::*};
use std::sync::{Arc, LazyLock, mpsc};
use std::time::Duration;
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::Time;

static CODECS: LazyLock<CodecRegistry> = LazyLock::new(|| {
    let mut reg = CodecRegistry::new();
    symphonia::default::register_enabled_codecs(&mut reg);
    reg.register_audio_decoder::<symphonia_adapter_libopus::OpusDecoder>();
    reg
});

/// One decodable audio file.
struct Stem {
    reader: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    rate: u32,
    channels: usize,
    /// Stereo frames at the *output* rate, ready to mix.
    pending: VecDeque<[f32; 2]>,
    scratch: Vec<f32>,
    resampler: Resampler,
    eof: bool,
    length_secs: f64,
}

impl Stem {
    fn open(data: StemData, ext: &str, out_rate: u32) -> Result<Self, String> {
        let mss = MediaSourceStream::new(
            match data {
                StemData::Memory(bytes) => Box::new(std::io::Cursor::new(ArcBytes(bytes))),
                StemData::Sng(slice) => Box::new(slice),
            },
            MediaSourceStreamOptions::default(),
        );
        let mut hint = Hint::new();
        hint.with_extension(ext);
        let reader = symphonia::default::get_probe()
            .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
            .map_err(|e| format!("Unsupported audio ({ext}): {e}"))?;
        let track = reader.default_track(TrackType::Audio).ok_or("No audio track")?;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or("No audio codec parameters")?
            .clone();
        let track_id = track.id;
        let rate = params.sample_rate.ok_or("Unknown sample rate")?;
        let channels = params.channels.as_ref().map(|c| c.count()).unwrap_or(2).max(1);
        let length_secs = match (track.num_frames, track.time_base) {
            (Some(n), Some(tb)) => {
                let t = tb.calc_time(symphonia::core::units::Timestamp::new(n as i64));
                t.map(|t| t.as_secs_f64()).unwrap_or(n as f64 / rate as f64)
            }
            (Some(n), None) => n as f64 / rate as f64,
            _ => 0.0,
        };
        let decoder = CODECS
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .map_err(|e| format!("No decoder for {ext}: {e}"))?;
        Ok(Self {
            reader,
            decoder,
            track_id,
            rate,
            channels,
            pending: VecDeque::with_capacity(8192),
            scratch: Vec::new(),
            resampler: Resampler::new(rate, out_rate),
            eof: false,
            length_secs,
        })
    }

    fn seek(&mut self, secs: f64) {
        self.pending.clear();
        self.resampler.reset();
        self.eof = false;
        let time = Time::try_from_secs_f64(secs.max(0.0)).unwrap_or_default();
        if self
            .reader
            .seek(SeekMode::Accurate, SeekTo::Time { time, track_id: Some(self.track_id) })
            .is_err()
        {
            // Seeking past the end (or unsupported): treat as finished.
            self.eof = secs > 0.0;
        }
        self.decoder.reset();
    }

    /// Decodes one packet into `pending`. Returns false at end of stream.
    fn decode_more(&mut self) -> bool {
        if self.eof {
            return false;
        }
        loop {
            let packet = match self.reader.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) | Err(_) => {
                    self.eof = true;
                    return false;
                }
            };
            if packet.track_id != self.track_id {
                continue;
            }
            let Ok(buf) = self.decoder.decode(&packet) else { continue }; // skip corrupt packets
            buf.copy_to_vec_interleaved(&mut self.scratch);
            let ch = self.channels;
            let frames = self.scratch.chunks_exact(ch).map(|f| {
                if ch == 1 { [f[0], f[0]] } else { [f[0], f[1]] }
            });
            self.resampler.process(frames, &mut self.pending);
            return true;
        }
    }
}

/// Streaming linear-interpolation resampler (only used when a stem's rate differs from the device).
struct Resampler {
    step: f64,
    pos: f64,
    prev: [f32; 2],
    bypass: bool,
}

impl Resampler {
    fn new(from: u32, to: u32) -> Self {
        Self { step: from as f64 / to as f64, pos: 0.0, prev: [0.0; 2], bypass: from == to }
    }
    fn reset(&mut self) {
        self.pos = 0.0;
        self.prev = [0.0; 2];
    }
    fn process(&mut self, input: impl Iterator<Item = [f32; 2]>, out: &mut VecDeque<[f32; 2]>) {
        if self.bypass {
            out.extend(input);
            return;
        }
        // `pos` is the fractional position between `prev` (0.0) and the current sample (1.0).
        for cur in input {
            while self.pos <= 1.0 {
                let t = self.pos as f32;
                out.push_back([
                    self.prev[0] + (cur[0] - self.prev[0]) * t,
                    self.prev[1] + (cur[1] - self.prev[1]) * t,
                ]);
                self.pos += self.step;
            }
            self.pos -= 1.0;
            self.prev = cur;
        }
    }
}

/// Where a stem's (compressed) bytes come from.
pub enum StemData {
    Memory(Arc<[u8]>),
    /// Streamed from a .sng file on disk, unmasked on the fly.
    Sng(crate::sng::MaskedSlice),
}

impl symphonia::core::io::MediaSource for crate::sng::MaskedSlice {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        Some(self.len())
    }
}

struct ArcBytes(Arc<[u8]>);
impl AsRef<[u8]> for ArcBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// State shared between the UI, the decoder thread, and the audio callback.
struct Shared {
    rate: u32,
    playing: AtomicBool,
    /// Chart time (ms, f64 bits) at the last seek.
    seek_ms: AtomicU64,
    /// Frames consumed by the callback since the last seek.
    frames: AtomicU64,
    /// Seek generation requested by the decoder / acknowledged (buffer flushed) by the callback.
    generation: AtomicU32,
    acked: AtomicU32,
    volume: AtomicU32,
    /// Output latency reported by the device, in microseconds.
    latency_us: AtomicU64,
    /// Decoder has produced everything up to the chart end.
    source_done: AtomicBool,
    ended: AtomicBool,
}

enum Cmd {
    Seek(f64),
    Quit,
}

pub struct Player {
    shared: Arc<Shared>,
    cmd: mpsc::Sender<Cmd>,
    end_ms: f64,
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::Quit);
    }
}

impl Player {
    /// `stems`: (data, extension). `start_delay_ms`: chart time at which audio time 0 plays.
    /// `fallback_length_ms`: preview length when there is no audio.
    pub fn new(stems: Vec<(StemData, String)>, start_delay_ms: f64, fallback_length_ms: f64) -> Result<Self, String> {
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(Arc<Shared>, f64), String>>();
        let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();

        std::thread::Builder::new()
            .name("preview-audio".into())
            .spawn(move || audio_thread(stems, start_delay_ms, fallback_length_ms, cmd_rx, ready_tx))
            .map_err(|e| e.to_string())?;

        let (shared, end_ms) = ready_rx.recv().map_err(|_| "Audio thread failed to start".to_string())??;
        Ok(Self { shared, cmd: cmd_tx, end_ms })
    }

    pub fn end_ms(&self) -> f64 {
        self.end_ms
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Relaxed)
    }

    pub fn has_ended(&self) -> bool {
        self.shared.ended.load(Relaxed)
    }

    pub fn play(&self) {
        if self.has_ended() {
            self.seek(0.0);
        }
        self.shared.playing.store(true, Relaxed);
    }

    pub fn pause(&self) {
        self.shared.playing.store(false, Relaxed);
    }

    pub fn seek(&self, ms: f64) {
        let ms = ms.clamp(0.0, self.end_ms);
        // Publish the target time immediately so the UI doesn't jump back while the buffer refills.
        self.shared.seek_ms.store(ms.to_bits(), Relaxed);
        self.shared.ended.store(false, Relaxed);
        let _ = self.cmd.send(Cmd::Seek(ms));
    }

    /// Volume 0..=1 (perceptual; squared like the original player).
    pub fn set_volume(&self, v: f32) {
        let v = v.clamp(0.0, 1.0);
        self.shared.volume.store((v * v).to_bits(), Relaxed);
    }

    /// Current audible chart position in ms.
    pub fn position_ms(&self) -> f64 {
        let s = &self.shared;
        let seek = f64::from_bits(s.seek_ms.load(Relaxed));
        if s.generation.load(Acquire) != s.acked.load(Acquire) {
            return seek; // seek in flight
        }
        let played = s.frames.load(Relaxed) as f64 * 1000.0 / s.rate as f64;
        let latency = if played > 0.0 { s.latency_us.load(Relaxed) as f64 / 1000.0 } else { 0.0 };
        (seek + (played - latency).max(0.0)).min(self.end_ms)
    }
}

fn audio_thread(
    stems: Vec<(StemData, String)>,
    start_delay_ms: f64,
    fallback_length_ms: f64,
    cmd_rx: mpsc::Receiver<Cmd>,
    ready: mpsc::Sender<Result<(Arc<Shared>, f64), String>>,
) {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    let setup = (|| -> Result<_, String> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or("No audio output device")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let config = supported.config();
        Ok((device, config, supported.sample_format()))
    })();
    let (device, config, sample_format) = match setup {
        Ok(v) => v,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let rate = config.sample_rate;
    let out_channels = config.channels as usize;

    let mut sources: Vec<Stem> = stems
        .into_iter()
        .filter_map(|(data, ext)| Stem::open(data, &ext, rate).ok())
        .collect();
    let audio_len_ms = sources.iter().map(|s| s.length_secs * 1000.0).fold(0.0, f64::max);
    // Same rule as the original player: max(delay + audio, audio, fallback).
    let end_ms = (start_delay_ms + audio_len_ms).max(audio_len_ms).max(fallback_length_ms).max(1.0);

    let shared = Arc::new(Shared {
        rate,
        playing: AtomicBool::new(false),
        seek_ms: AtomicU64::new(0f64.to_bits()),
        frames: AtomicU64::new(0),
        generation: AtomicU32::new(0),
        acked: AtomicU32::new(0),
        volume: AtomicU32::new(0.25f32.to_bits()),
        latency_us: AtomicU64::new(0),
        source_done: AtomicBool::new(false),
        ended: AtomicBool::new(false),
    });

    // ~0.5 s of stereo audio buffered ahead.
    let (mut producer, consumer) = RingBuffer::<f32>::new((rate as usize / 2) * 2);

    let stream = match build_stream(&device, &config, sample_format, consumer, shared.clone(), out_channels) {
        Ok(s) => s,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    if let Err(e) = stream.play() {
        let _ = ready.send(Err(e.to_string()));
        return;
    }
    let _ = ready.send(Ok((shared.clone(), end_ms)));

    // Decoder loop.
    let mut silence_frames: u64 = 0; // leading silence before audio time 0 (positive start delay)
    let mut frames_until_end: u64 = (end_ms / 1000.0 * rate as f64) as u64;
    let mut pending_seek: Option<f64> = Some(0.0);
    let mut seek_generation = 0u32;

    loop {
        // Commands
        loop {
            match cmd_rx.try_recv() {
                Ok(Cmd::Seek(ms)) => pending_seek = Some(ms),
                Ok(Cmd::Quit) | Err(mpsc::TryRecvError::Disconnected) => {
                    drop(stream);
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }

        if let Some(ms) = pending_seek.take() {
            // Ask the callback to flush, then wait for it so no stale audio survives.
            seek_generation = seek_generation.wrapping_add(1);
            shared.seek_ms.store(ms.to_bits(), Relaxed);
            shared.source_done.store(false, Relaxed);
            shared.ended.store(false, Relaxed);
            shared.generation.store(seek_generation, Release);
            let deadline = std::time::Instant::now() + Duration::from_millis(250);
            while shared.acked.load(Acquire) != seek_generation && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
            if shared.acked.load(Acquire) != seek_generation {
                // Callback isn't running (device stalled); acknowledge ourselves.
                shared.frames.store(0, Relaxed);
                shared.acked.store(seek_generation, Release);
            }
            let audio_ms = ms - start_delay_ms;
            silence_frames = if audio_ms < 0.0 { (-audio_ms / 1000.0 * rate as f64) as u64 } else { 0 };
            for s in &mut sources {
                s.seek((audio_ms / 1000.0).max(0.0));
            }
            frames_until_end = (((end_ms - ms) / 1000.0) * rate as f64).max(0.0) as u64;
            continue;
        }

        // Fill the ring buffer.
        let mut wrote = false;
        while producer.slots() >= 2 && frames_until_end > 0 {
            if silence_frames > 0 {
                let n = (producer.slots() / 2).min(silence_frames as usize).min(frames_until_end as usize).min(1024);
                if let Ok(chunk) = producer.write_chunk_uninit(n * 2) {
                    chunk.fill_from_iter(std::iter::repeat(0.0));
                }
                silence_frames -= n as u64;
                frames_until_end -= n as u64;
                wrote = true;
                continue;
            }
            // Make sure each live stem has something to mix.
            for s in sources.iter_mut() {
                while s.pending.is_empty() && s.decode_more() {}
            }
            let available = sources.iter().filter(|s| !s.pending.is_empty()).map(|s| s.pending.len()).min();
            let all_done = sources.iter().all(|s| s.eof && s.pending.is_empty());
            let n = match available {
                Some(a) => a.min(producer.slots() / 2).min(frames_until_end as usize).min(2048),
                None if all_done => {
                    // Audio finished (or none): pad with silence until the chart end.
                    silence_frames = frames_until_end;
                    continue;
                }
                None => 0,
            };
            if n == 0 {
                break;
            }
            if let Ok(chunk) = producer.write_chunk_uninit(n * 2) {
                let mut mixed = Vec::with_capacity(n * 2);
                for _ in 0..n {
                    let mut l = 0.0;
                    let mut r = 0.0;
                    for s in sources.iter_mut() {
                        if let Some([a, b]) = s.pending.pop_front() {
                            l += a;
                            r += b;
                        }
                    }
                    mixed.push(l);
                    mixed.push(r);
                }
                chunk.fill_from_iter(mixed);
            }
            frames_until_end -= n as u64;
            wrote = true;
        }
        if frames_until_end == 0 {
            shared.source_done.store(true, Relaxed);
        }
        if !wrote {
            match cmd_rx.recv_timeout(Duration::from_millis(5)) {
                Ok(Cmd::Seek(ms)) => pending_seek = Some(ms),
                Ok(Cmd::Quit) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

fn build_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    mut consumer: Consumer<f32>,
    shared: Arc<Shared>,
    out_channels: usize,
) -> Result<cpal::Stream, String> {
    use cpal::traits::DeviceTrait;
    use cpal::{FromSample, SizedSample};

    fn callback<T: SizedSample + FromSample<f32>>(
        data: &mut [T],
        info: &cpal::OutputCallbackInfo,
        consumer: &mut Consumer<f32>,
        shared: &Shared,
        my_gen: &mut u32,
        out_channels: usize,
    ) {
        let ts = info.timestamp();
        let latency = ts.playback.duration_since(ts.callback);
        shared.latency_us.store(latency.as_micros() as u64, Relaxed);

        let g = shared.generation.load(Acquire);
        if g != *my_gen {
            // Seek: discard everything buffered and restart the clock.
            let n = consumer.slots();
            if let Ok(c) = consumer.read_chunk(n) {
                c.commit_all();
            }
            shared.frames.store(0, Relaxed);
            *my_gen = g;
            shared.acked.store(g, Release);
        }

        let frames = data.len() / out_channels;
        if !shared.playing.load(Relaxed) {
            data.fill(T::EQUILIBRIUM);
            return;
        }
        let vol = f32::from_bits(shared.volume.load(Relaxed));
        let avail = (consumer.slots() / 2).min(frames);
        if let Ok(chunk) = consumer.read_chunk(avail * 2) {
            let (a, b) = chunk.as_slices();
            let mut it = a.iter().chain(b.iter());
            for frame in data.chunks_exact_mut(out_channels).take(avail) {
                let l = *it.next().unwrap_or(&0.0) * vol;
                let r = *it.next().unwrap_or(&0.0) * vol;
                for (c, s) in frame.iter_mut().enumerate() {
                    *s = T::from_sample(match c {
                        0 => l,
                        1 => r,
                        _ => 0.0,
                    });
                }
            }
            chunk.commit_all();
        }
        for s in &mut data[avail * out_channels..] {
            *s = T::EQUILIBRIUM;
        }
        shared.frames.fetch_add(avail as u64, Relaxed);
        if avail < frames && shared.source_done.load(Relaxed) && consumer.slots() == 0 {
            shared.ended.store(true, Relaxed);
            shared.playing.store(false, Relaxed);
        }
    }

    let err = |e| eprintln!("audio stream error: {e}");
    let mut my_gen = 0u32;
    macro_rules! build {
        ($t:ty) => {
            device.build_output_stream::<$t, _, _>(
                config.clone(),
                move |data, info| callback(data, info, &mut consumer, &shared, &mut my_gen, out_channels),
                err,
                None,
            )
        };
    }
    let stream = match format {
        cpal::SampleFormat::F32 => build!(f32),
        cpal::SampleFormat::I16 => build!(i16),
        cpal::SampleFormat::U16 => build!(u16),
        cpal::SampleFormat::I32 => build!(i32),
        cpal::SampleFormat::F64 => build!(f64),
        other => return Err(format!("Unsupported output sample format {other:?}")),
    };
    stream.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TRUSS_TEST_SNG=<file.sng>: decode every stem from 3:30 and report speed.
    #[test]
    fn decode_and_seek_speed() {
        let Ok(path) = std::env::var("TRUSS_TEST_SNG") else { return };
        let data = std::fs::read(path).unwrap();
        let sng = crate::sng::read_sng(&data, |n| n.ends_with(".opus") || n.ends_with(".ogg")).unwrap();
        for (name, bytes) in sng.files {
            let ext = name.rsplit('.').next().unwrap().to_string();
            let t = std::time::Instant::now();
            let mut s = Stem::open(StemData::Memory(Arc::from(bytes)), &ext, 48000).unwrap();
            let opened = t.elapsed();
            s.seek(210.0);
            let seeked = t.elapsed();
            let mut frames = 0;
            while frames < 48000 * 10 && s.decode_more() {
                frames += s.pending.len();
                s.pending.clear();
            }
            println!(
                "{name}: len {:.1}s rate {} ch {} | open {:?} seek {:?} decode 10s {:?}",
                s.length_secs, s.rate, s.channels, opened, seeked - opened, t.elapsed() - seeked
            );
            assert!(frames >= 48000 * 9);
        }
    }
}
