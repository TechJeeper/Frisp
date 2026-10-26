#![allow(dead_code)]

use anyhow::anyhow;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const FRAME: usize = nnnoiseless::DenoiseState::FRAME_SIZE;
const MODEL_RATE: u32 = 48_000;

pub const PREVIEW_SAMPLES: usize = 48_000 * 4;

pub struct CompareClip {
    pub original: Vec<f32>,
    pub cleaned: Vec<f32>,
}

struct Recorder {
    active: bool,
    original: Vec<f32>,
    cleaned: Vec<f32>,
}

pub struct Shared {
    pub status: Mutex<String>,
    pub devices: Mutex<Vec<String>>,
    pub default_input: Mutex<Option<String>>,
    pub input_peak: AtomicU32,
    pub output_peak: AtomicU32,
    pub noise_cancel: AtomicBool,
    pub strength: AtomicU32,
    pub clarity_boost: AtomicBool,
    pub voice_leveler: AtomicBool,
    pub low_cut: AtomicBool,
    pub voice_active: AtomicBool,
    pub virtual_ready: AtomicBool,
    pub compare: Mutex<Option<CompareClip>>,
    recorder: Mutex<Recorder>,
    pub playing: AtomicU32,
    play_gen: AtomicU32,
    pub capture_owner: AtomicU32,
}

impl Shared {
    fn new(
        noise_cancel: bool,
        strength: f32,
        clarity_boost: bool,
        voice_leveler: bool,
        low_cut: bool,
    ) -> Self {
        Self {
            status: Mutex::new("Looking for microphones.".into()),
            devices: Mutex::new(Vec::new()),
            default_input: Mutex::new(None),
            input_peak: AtomicU32::new(0),
            output_peak: AtomicU32::new(0),
            noise_cancel: AtomicBool::new(noise_cancel),
            strength: AtomicU32::new(strength.clamp(0.0, 1.0).to_bits()),
            clarity_boost: AtomicBool::new(clarity_boost),
            voice_leveler: AtomicBool::new(voice_leveler),
            low_cut: AtomicBool::new(low_cut),
            voice_active: AtomicBool::new(false),
            virtual_ready: AtomicBool::new(false),
            compare: Mutex::new(None),
            recorder: Mutex::new(Recorder {
                active: false,
                original: Vec::new(),
                cleaned: Vec::new(),
            }),
            playing: AtomicU32::new(0),
            play_gen: AtomicU32::new(0),
            capture_owner: AtomicU32::new(0),
        }
    }

    pub fn start_recording(&self) {
        self.playing.store(0, Ordering::Relaxed);
        if let Ok(mut clip) = self.compare.lock() {
            *clip = None;
        }
        if let Ok(mut recorder) = self.recorder.lock() {
            recorder.original.clear();
            recorder.cleaned.clear();
            recorder.original.reserve(PREVIEW_SAMPLES);
            recorder.cleaned.reserve(PREVIEW_SAMPLES);
            recorder.active = true;
        }
    }

    pub fn recording(&self) -> bool {
        self.recorder.lock().map(|recorder| recorder.active).unwrap_or(false)
    }

    pub fn record_progress(&self) -> f32 {
        let Some(recorder) = self.recorder.lock().ok() else {
            return 0.0;
        };
        if !recorder.active && recorder.original.is_empty() {
            return 0.0;
        }
        (recorder.original.len() as f32 / PREVIEW_SAMPLES as f32).min(1.0)
    }

    pub fn has_clip(&self) -> bool {
        self.compare.lock().ok().and_then(|clip| clip.as_ref().map(|item| !item.original.is_empty())).unwrap_or(false)
    }
}

enum Command {
    UseInput(Option<String>),
    Shutdown,
}

pub struct Engine {
    cmd_tx: Sender<Command>,
    play_stop: Mutex<Arc<AtomicBool>>,
    shared: Arc<Shared>,
}

impl Engine {
    pub fn spawn(
        noise_cancel: bool,
        strength: f32,
        clarity_boost: bool,
        voice_leveler: bool,
        low_cut: bool,
    ) -> (Self, Arc<Shared>) {
        let shared = Arc::new(Shared::new(
            noise_cancel,
            strength,
            clarity_boost,
            voice_leveler,
            low_cut,
        ));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let shared_thread = Arc::clone(&shared);
        thread::spawn(move || engine_loop(cmd_rx, shared_thread));
        (
            Self {
                cmd_tx,
                play_stop: Mutex::new(Arc::new(AtomicBool::new(false))),
                shared: Arc::clone(&shared),
            },
            shared,
        )
    }

    pub fn preview(&self, cleaned: bool) {
        let stop = {
            let Ok(mut current) = self.play_stop.lock() else {
                return;
            };
            current.store(true, Ordering::Relaxed);
            let next = Arc::new(AtomicBool::new(false));
            *current = Arc::clone(&next);
            next
        };
        let Some(samples) = self.shared.compare.lock().ok().and_then(|clip| {
            clip.as_ref().map(|item| {
                if cleaned {
                    item.cleaned.clone()
                } else {
                    item.original.clone()
                }
            })
        }) else {
            return;
        };
        let generation = self.shared.play_gen.fetch_add(1, Ordering::Relaxed) + 1;
        self.shared
            .playing
            .store(if cleaned { 2 } else { 1 }, Ordering::Relaxed);
        let shared = Arc::clone(&self.shared);
        thread::spawn(move || {
            play_preview(samples, stop);
            if shared.play_gen.load(Ordering::Relaxed) == generation {
                shared.playing.store(0, Ordering::Relaxed);
            }
        });
    }

    pub fn begin_sample(&self, device_name: Option<String>) {
        if let Ok(stop) = self.play_stop.lock() {
            stop.store(true, Ordering::Relaxed);
        }
        let side = !self.shared.virtual_ready.load(Ordering::Relaxed);
        self.shared
            .capture_owner
            .store(if side { 2 } else { 1 }, Ordering::Relaxed);
        self.shared.start_recording();
        if !side {
            return;
        }
        let Some(name) = device_name else {
            return;
        };
        let shared = Arc::clone(&self.shared);
        thread::spawn(move || {
            if let Err(err) = capture_without_cable(&name, &shared) {
                set_status(&shared, &format!("Couldn't record a sample: {err}"));
                if let Ok(mut recorder) = shared.recorder.lock() {
                    recorder.active = false;
                }
            }
            shared.capture_owner.store(0, Ordering::Relaxed);
        });
    }

    pub fn set_input(&self, name: Option<String>) {
        let _ = self.cmd_tx.send(Command::UseInput(name));
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(Command::Shutdown);
    }
}

fn engine_loop(cmd_rx: Receiver<Command>, shared: Arc<Shared>) {
    let mut wanted: Option<String> = None;
    let mut session: Option<Session> = None;
    let mut last_scan = Instant::now() - Duration::from_secs(5);
    loop {
        if last_scan.elapsed() >= Duration::from_secs(1) {
            publish_devices(&shared);
            last_scan = Instant::now();
        }
        match cmd_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Command::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(Command::UseInput(name)) => {
                wanted = name;
                drop(session.take());
                session = open_session(wanted.clone(), &shared);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let failed = session.as_ref().is_some_and(|item| item.failed.load(Ordering::Relaxed));
                if failed {
                    drop(session.take());
                    session = open_session(wanted.clone(), &shared);
                }
            }
        }
    }
    set_status(&shared, "Stopped.");
}

fn publish_devices(shared: &Shared) {
    let names = list_inputs();
    let default = default_input_name().filter(|name| names.iter().any(|device| device == name));
    if let Ok(mut devices) = shared.devices.lock() {
        *devices = names;
    }
    if let Ok(mut slot) = shared.default_input.lock() {
        *slot = default;
    }
}

struct Session {
    failed: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    _input: cpal::Stream,
    _output: cpal::Stream,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn open_session(input_name: Option<String>, shared: &Arc<Shared>) -> Option<Session> {
    let Some(input_name) = input_name.filter(|name| !name.is_empty()) else {
        shared.virtual_ready.store(false, Ordering::Relaxed);
        set_status(shared, "Choose a microphone.");
        return None;
    };

    let host = cpal::default_host();
    let Some(output_device) = find_publish_output(&host) else {
        shared.virtual_ready.store(false, Ordering::Relaxed);
        set_status(shared, missing_cable_status());
        return None;
    };
    let output_name = output_device.name().unwrap_or_default();
    let Some(input_device) = find_input(&host, &input_name) else {
        shared.virtual_ready.store(false, Ordering::Relaxed);
        set_status(shared, &format!("Can't find \"{input_name}\"."));
        return None;
    };

    match start_streams(input_device, output_device, Arc::clone(shared)) {
        Ok(session) => {
            shared.virtual_ready.store(true, Ordering::Relaxed);
            if output_name.to_ascii_lowercase().contains("frisp") {
                set_status(shared, "Frisp Mic is live.");
            } else {
                set_status(shared, unnamed_cable_status());
            }
            Some(session)
        }
        Err(err) => {
            shared.virtual_ready.store(false, Ordering::Relaxed);
            set_status(shared, &format!("Audio didn't start: {err}"));
            None
        }
    }
}

fn start_streams(
    input_device: cpal::Device,
    output_device: cpal::Device,
    shared: Arc<Shared>,
) -> anyhow::Result<Session> {
    let input_supported = input_device.default_input_config()?;
    let output_supported = pick_output_config(&output_device)?;
    let in_rate = input_supported.sample_rate().0;
    let out_rate = output_supported.sample_rate().0;
    let in_channels = input_supported.channels() as usize;
    let out_channels = output_supported.channels() as usize;
    if in_channels == 0 || out_channels == 0 {
        return Err(anyhow!("the audio device has no channels"));
    }

    let input_queue = Arc::new(Mutex::new(VecDeque::<f32>::new()));
    let output_queue = Arc::new(Mutex::new(VecDeque::<f32>::new()));
    let failed = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let primed = Arc::new(AtomicBool::new(false));

    let input_stream = build_input(
        &input_device,
        &input_supported,
        Arc::clone(&input_queue),
        Arc::clone(&shared),
        Arc::clone(&failed),
    )?;
    let output_stream = build_output(
        &output_device,
        &output_supported,
        Arc::clone(&output_queue),
        Arc::clone(&shared),
        Arc::clone(&failed),
        Arc::clone(&primed),
    )?;
    input_stream.play()?;
    output_stream.play()?;

    let worker_stop = Arc::clone(&stop);
    let worker_in = Arc::clone(&input_queue);
    let worker_out = Arc::clone(&output_queue);
    let worker_shared = Arc::clone(&shared);
    let worker = thread::spawn(move || {
        process_audio(worker_stop, worker_shared, worker_in, worker_out, in_rate, out_rate, false);
    });

    Ok(Session {
        failed,
        stop,
        worker: Some(worker),
        _input: input_stream,
        _output: output_stream,
    })
}

fn build_input(
    device: &cpal::Device,
    supported: &cpal::SupportedStreamConfig,
    queue: Arc<Mutex<VecDeque<f32>>>,
    shared: Arc<Shared>,
    failed: Arc<AtomicBool>,
) -> anyhow::Result<cpal::Stream> {
    let channels = supported.channels() as usize;
    let config = supported.config();
    let stream = match supported.sample_format() {
        SampleFormat::F32 => {
            let queue = Arc::clone(&queue);
            let peak = Arc::clone(&shared);
            let failed = Arc::clone(&failed);
            device.build_input_stream(
                &config,
                move |data: &[f32], _| push_input(&queue, &peak.input_peak, data, channels),
                move |err| fail(&failed, err),
                None,
            )?
        }
        SampleFormat::I16 => {
            let queue = Arc::clone(&queue);
            let peak = Arc::clone(&shared);
            let failed = Arc::clone(&failed);
            device.build_input_stream(
                &config,
                move |data: &[i16], _| {
                    let samples: Vec<f32> = data.iter().map(|s| *s as f32 / 32768.0).collect();
                    push_input(&queue, &peak.input_peak, &samples, channels);
                },
                move |err| fail(&failed, err),
                None,
            )?
        }
        SampleFormat::U16 => {
            let failed = Arc::clone(&failed);
            device.build_input_stream(
                &config,
                move |data: &[u16], _| {
                    let samples: Vec<f32> = data
                        .iter()
                        .map(|s| (*s as f32 - 32768.0) / 32768.0)
                        .collect();
                    push_input(&queue, &shared.input_peak, &samples, channels);
                },
                move |err| fail(&failed, err),
                None,
            )?
        }
        other => return Err(anyhow!("unsupported microphone format {other}")),
    };
    Ok(stream)
}

fn build_output(
    device: &cpal::Device,
    supported: &cpal::SupportedStreamConfig,
    queue: Arc<Mutex<VecDeque<f32>>>,
    shared: Arc<Shared>,
    failed: Arc<AtomicBool>,
    primed: Arc<AtomicBool>,
) -> anyhow::Result<cpal::Stream> {
    let channels = supported.channels() as usize;
    let config = supported.config();
    let prime_samples = (supported.sample_rate().0 / 50).max(1) as usize;
    let stream = match supported.sample_format() {
        SampleFormat::F32 => {
            let failed = Arc::clone(&failed);
            device.build_output_stream(
                &config,
                move |data: &mut [f32], _| {
                    pull_output(
                        &queue,
                        &shared.output_peak,
                        &primed,
                        data,
                        channels,
                        prime_samples,
                    );
                },
                move |err| fail(&failed, err),
                None,
            )?
        }
        SampleFormat::I16 => {
            let failed = Arc::clone(&failed);
            device.build_output_stream(
                &config,
                move |data: &mut [i16], _| {
                    let mut floats = vec![0f32; data.len()];
                    pull_output(
                        &queue,
                        &shared.output_peak,
                        &primed,
                        &mut floats,
                        channels,
                        prime_samples,
                    );
                    for (dst, src) in data.iter_mut().zip(floats) {
                        *dst = (src.clamp(-1.0, 1.0) * 32767.0) as i16;
                    }
                },
                move |err| fail(&failed, err),
                None,
            )?
        }
        other => return Err(anyhow!("unsupported virtual microphone format {other}")),
    };
    Ok(stream)
}

fn fail(flag: &AtomicBool, err: cpal::StreamError) {
    eprintln!("audio stream error: {err}");
    flag.store(true, Ordering::Relaxed);
}

fn push_input(queue: &Mutex<VecDeque<f32>>, peak: &AtomicU32, data: &[f32], channels: usize) {
    if channels == 0 || data.is_empty() {
        return;
    }
    let mut max_abs = 0f32;
    let mut mono = Vec::with_capacity(data.len() / channels);
    for frame in data.chunks_exact(channels) {
        let sample = if channels == 1 {
            frame[0]
        } else {
            // For stereo and multi-channel arrays (such as 4-channel Intel SST), channels 0 & 1 form the primary stereo voice pair
            (frame[0] + frame[1]) * 0.5
        };
        max_abs = max_abs.max(sample.abs());
        mono.push(sample);
    }
    peak.store(max_abs.to_bits(), Ordering::Relaxed);
    enqueue(queue, &mono, MODEL_RATE as usize);
}

fn pull_output(
    queue: &Mutex<VecDeque<f32>>,
    peak: &AtomicU32,
    primed: &AtomicBool,
    data: &mut [f32],
    channels: usize,
    prime_samples: usize,
) {
    if channels == 0 {
        data.fill(0.0);
        return;
    }
    let mut queue = queue.lock().unwrap_or_else(|err| err.into_inner());
    if !primed.load(Ordering::Relaxed) {
        if queue.len() < prime_samples {
            data.fill(0.0);
            return;
        }
        primed.store(true, Ordering::Relaxed);
    }
    let frames = data.len() / channels;
    let mut max_abs = 0f32;
    for frame in 0..frames {
        let sample = queue.pop_front().unwrap_or(0.0);
        max_abs = max_abs.max(sample.abs());
        for channel in 0..channels {
            data[frame * channels + channel] = sample;
        }
    }
    peak.store(max_abs.to_bits(), Ordering::Relaxed);
}

fn capture_without_cable(name: &str, shared: &Arc<Shared>) -> anyhow::Result<()> {
    let host = cpal::default_host();
    let device = find_input(&host, name).ok_or_else(|| anyhow!("that microphone is not available"))?;
    let supported = device.default_input_config()?;
    let queue = Arc::new(Mutex::new(VecDeque::<f32>::new()));
    let output = Arc::new(Mutex::new(VecDeque::<f32>::new()));
    let failed = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let stream = build_input(
        &device,
        &supported,
        Arc::clone(&queue),
        Arc::clone(shared),
        Arc::clone(&failed),
    )?;
    stream.play()?;
    let worker_stop = Arc::clone(&stop);
    let worker_shared = Arc::clone(shared);
    let worker_in = Arc::clone(&queue);
    let rate = supported.sample_rate().0;
    let worker = thread::spawn(move || {
        process_audio(worker_stop, worker_shared, worker_in, output, rate, MODEL_RATE, true);
    });
    let deadline = Instant::now() + Duration::from_secs(6);
    while Instant::now() < deadline && shared.recording() && !failed.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(30));
    }
    stop.store(true, Ordering::Relaxed);
    let _ = worker.join();
    drop(stream);
    publish_partial(shared);
    Ok(())
}

fn publish_partial(shared: &Shared) {
    let clip = {
        let Ok(mut recorder) = shared.recorder.lock() else {
            return;
        };
        if !recorder.active {
            return;
        }
        recorder.active = false;
        if recorder.original.is_empty() {
            return;
        }
        Some(CompareClip {
            original: std::mem::take(&mut recorder.original),
            cleaned: std::mem::take(&mut recorder.cleaned),
        })
    };
    if let Some(clip) = clip {
        if let Ok(mut stored) = shared.compare.lock() {
            *stored = Some(clip);
        }
    }
}

fn capture_preview(shared: &Shared, dry: &[f32], wet: &[f32], strength: f32) {
    let finished = {
        let Ok(mut recorder) = shared.recorder.lock() else {
            return;
        };
        if !recorder.active {
            return;
        }
        let room = PREVIEW_SAMPLES.saturating_sub(recorder.original.len());
        let take = room.min(dry.len()).min(wet.len());
        recorder.original.extend_from_slice(&dry[..take]);
        recorder.cleaned.extend(
            dry[..take]
                .iter()
                .zip(wet[..take].iter())
                .map(|(original, cleaned)| original * (1.0 - strength) + cleaned * strength),
        );
        if recorder.original.len() >= PREVIEW_SAMPLES {
            recorder.active = false;
            Some(CompareClip {
                original: std::mem::take(&mut recorder.original),
                cleaned: std::mem::take(&mut recorder.cleaned),
            })
        } else {
            None
        }
    };
    if let Some(clip) = finished {
        if let Ok(mut stored) = shared.compare.lock() {
            *stored = Some(clip);
        }
    }
}

fn play_preview(samples: Vec<f32>, stop: Arc<AtomicBool>) {
    let host = cpal::default_host();
    let Some(device) = find_speakers(&host) else {
        return;
    };
    let Ok(config) = pick_output_config(&device) else {
        return;
    };
    let channels = config.channels() as usize;
    if channels == 0 || samples.is_empty() {
        return;
    }
    let mut audio = Resampler::new(MODEL_RATE, config.sample_rate().0).process(&samples);
    let total = audio.len();
    let fade = (config.sample_rate().0 as usize / 80).min(audio.len() / 4);
    for index in 0..fade {
        let gain = index as f32 / fade.max(1) as f32;
        audio[index] *= gain;
        let tail = audio.len() - 1 - index;
        audio[tail] *= gain;
    }
    let cursor = Arc::new(AtomicUsize::new(0));
    let failed = Arc::new(AtomicBool::new(false));
    let stream = match build_preview(&device, &config, Arc::clone(&cursor), audio, Arc::clone(&stop), Arc::clone(&failed)) {
        Ok(stream) => stream,
        Err(_) => return,
    };
    if stream.play().is_err() {
        return;
    }
    let timeout = Instant::now() + Duration::from_secs(8);
    while !stop.load(Ordering::Relaxed) && !failed.load(Ordering::Relaxed) && Instant::now() < timeout {
        if cursor.load(Ordering::Relaxed) >= total {
            thread::sleep(Duration::from_millis(80));
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn build_preview(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    cursor: Arc<AtomicUsize>,
    audio: Vec<f32>,
    stop: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
) -> anyhow::Result<cpal::Stream> {
    let channels = config.channels() as usize;
    let stream_config = config.config();
    let stream = match config.sample_format() {
        SampleFormat::F32 => {
            let failed_cb = Arc::clone(&failed);
            device.build_output_stream(
                &stream_config,
                move |data: &mut [f32], _| fill_preview(data, channels, &audio, &cursor, &stop),
                move |err| fail(&failed_cb, err),
                None,
            )?
        }
        SampleFormat::I16 => {
            let failed_cb = Arc::clone(&failed);
            device.build_output_stream(
                &stream_config,
                move |data: &mut [i16], _| {
                    let mut floats = vec![0f32; data.len()];
                    fill_preview(&mut floats, channels, &audio, &cursor, &stop);
                    for (dst, src) in data.iter_mut().zip(floats) {
                        *dst = (src.clamp(-1.0, 1.0) * 32767.0) as i16;
                    }
                },
                move |err| fail(&failed_cb, err),
                None,
            )?
        }
        other => return Err(anyhow!("unsupported speaker format {other}")),
    };
    Ok(stream)
}

fn fill_preview(data: &mut [f32], channels: usize, audio: &[f32], cursor: &AtomicUsize, stop: &AtomicBool) {
    if channels == 0 || stop.load(Ordering::Relaxed) {
        data.fill(0.0);
        return;
    }
    let mut index = cursor.load(Ordering::Relaxed);
    for frame in data.chunks_exact_mut(channels) {
        let sample = audio.get(index).copied().unwrap_or(0.0);
        if index < audio.len() {
            index += 1;
        }
        frame.fill(sample);
    }
    cursor.store(index, Ordering::Relaxed);
}

fn find_speakers(host: &cpal::Host) -> Option<cpal::Device> {
    if let Some(device) = host.default_output_device() {
        if device.name().map(|name| !is_monitor(&name)).unwrap_or(false) {
            return Some(device);
        }
    }
    host.output_devices().ok()?.find(|device| device.name().map(|name| !is_monitor(&name)).unwrap_or(false))
}

fn is_monitor(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("frisp") || is_cable_render(&lower) || is_cable_capture(&lower)
}

pub fn virtual_cable_present() -> bool {
    let host = cpal::default_host();
    host.output_devices()
        .ok()
        .into_iter()
        .flatten()
        .any(|device| {
            device
                .name()
                .map(|name| {
                    let lower = name.to_ascii_lowercase();
                    lower.contains("frisp") || is_cable_render(&lower)
                })
                .unwrap_or(false)
        })
}

fn enqueue(queue: &Mutex<VecDeque<f32>>, samples: &[f32], cap: usize) {
    let mut queue = queue.lock().unwrap_or_else(|err| err.into_inner());
    queue.extend(samples);
    if queue.len() > cap {
        let extra = queue.len() - cap;
        queue.drain(0..extra);
    }
}

fn process_audio(
    stop: Arc<AtomicBool>,
    shared: Arc<Shared>,
    input: Arc<Mutex<VecDeque<f32>>>,
    output: Arc<Mutex<VecDeque<f32>>>,
    in_rate: u32,
    out_rate: u32,
    side_capture: bool,
) {
    let mut denoise = nnnoiseless::DenoiseState::new();
    let mut krisp_pipeline = crate::dsp::KrispDenoisePipeline::new(MODEL_RATE as f32);
    let mut to_model = Resampler::new(in_rate, MODEL_RATE);
    let mut to_output = Resampler::new(MODEL_RATE, out_rate);
    let mut pending = Vec::<f32>::new();
    let mut frame_in = [0f32; FRAME];
    let mut frame_out = [0f32; FRAME];

    while !stop.load(Ordering::Relaxed) {
        let batch = {
            let mut queue = input.lock().unwrap_or_else(|err| err.into_inner());
            queue.drain(..).collect::<Vec<_>>()
        };
        if !batch.is_empty() {
            pending.extend(to_model.process(&batch));
        }
        if pending.len() > MODEL_RATE as usize {
            let extra = (pending.len() - MODEL_RATE as usize / 2) / FRAME * FRAME;
            if extra > 0 {
                pending.drain(0..extra);
            }
        }
        let mut produced = false;
        while pending.len() >= FRAME {
            frame_in.copy_from_slice(&pending[..FRAME]);
            pending.drain(0..FRAME);
            let cancel = shared.noise_cancel.load(Ordering::Relaxed);
            let strength = f32::from_bits(shared.strength.load(Ordering::Relaxed)).clamp(0.0, 1.0);
            let owner = shared.capture_owner.load(Ordering::Relaxed);
            let recording = shared.recorder.lock().map(|recorder| recorder.active).unwrap_or(false)
                && ((side_capture && owner == 2) || (!side_capture && owner != 2));

            // Krisp Pre-processing: VAD & Subsonic Rumble Filter (< 80 Hz)
            krisp_pipeline.low_cut_enabled = shared.low_cut.load(Ordering::Relaxed);
            let is_voice = krisp_pipeline.pre_process(&mut frame_in);
            shared.voice_active.store(is_voice, Ordering::Relaxed);

            let denoised = if cancel || recording {
                let mut scaled = [0f32; FRAME];
                for (dst, src) in scaled.iter_mut().zip(frame_in) {
                    *dst = src * 32767.0;
                }
                denoise.process_frame(&mut frame_out, &scaled);
                let mut clean: Vec<f32> = frame_out
                    .iter()
                    .map(|sample| (sample / 32768.0).clamp(-1.0, 1.0))
                    .collect();

                // Krisp Post-processing: Human Voice Clarity, Adaptive Leveler, Soft Limiting
                krisp_pipeline.clarity_enabled = shared.clarity_boost.load(Ordering::Relaxed);
                krisp_pipeline.leveler_enabled = shared.voice_leveler.load(Ordering::Relaxed);
                krisp_pipeline.post_process(&mut clean);

                Some(clean)
            } else {
                None
            };
            if recording {
                if let Some(clean) = &denoised {
                    capture_preview(&shared, &frame_in, clean, strength);
                }
            }
            let mut mixed = match &denoised {
                Some(clean) if cancel => frame_in
                    .iter()
                    .zip(clean.iter())
                    .map(|(dry, wet)| dry * (1.0 - strength) + wet * strength)
                    .collect::<Vec<_>>(),
                _ => frame_in.to_vec(),
            };
            for s in mixed.iter_mut() {
                *s = crate::dsp::soft_limit(*s);
            }
            let rendered = to_output.process(&mixed);
            enqueue(&output, &rendered, out_rate as usize);
            produced = true;
        }
        if !produced {
            thread::sleep(Duration::from_millis(4));
        }
    }
}

fn find_publish_output(host: &cpal::Host) -> Option<cpal::Device> {
    let mut frisp_fallback = None;
    let mut cable = None;
    let mut cable_preferred = None;
    let devices = host.output_devices().ok()?;
    for device in devices {
        let name = device.name().unwrap_or_default();
        let lower = name.to_ascii_lowercase();
        if lower.contains("frisp") {
            if lower.contains("output") || lower.contains("speaker") {
                return Some(device);
            }
            frisp_fallback = Some(device);
        } else if is_cable_render(&lower) {
            if prefers_cable(&lower) {
                cable_preferred = Some(device);
            } else if cable.is_none() {
                cable = Some(device);
            }
        }
    }
    frisp_fallback.or(cable_preferred).or(cable)
}

fn find_input(host: &cpal::Host, wanted: &str) -> Option<cpal::Device> {
    host.input_devices().ok()?.find(|device| {
        device
            .name()
            .map(|name| name == wanted && !is_virtual_mic(&name))
            .unwrap_or(false)
    })
}

pub fn list_inputs() -> Vec<String> {
    let host = cpal::default_host();
    let Ok(devices) = host.input_devices() else {
        return Vec::new();
    };
    let mut names: Vec<String> = devices
        .filter_map(|device| device.name().ok())
        .filter(|name| !is_virtual_mic(name))
        .collect();
    names.sort();
    names.dedup();
    names
}

pub fn default_input_name() -> Option<String> {
    let host = cpal::default_host();
    let device = host.default_input_device()?;
    let name = device.name().ok()?;
    if is_virtual_mic(&name) {
        None
    } else {
        Some(name)
    }
}

fn is_virtual_mic(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("frisp") || is_cable_capture(&lower)
}

fn is_cable_render(lower: &str) -> bool {
    if cfg!(target_os = "macos") {
        lower.contains("blackhole")
    } else {
        lower.contains("cable input")
    }
}

fn is_cable_capture(lower: &str) -> bool {
    if cfg!(target_os = "macos") {
        lower.contains("blackhole")
    } else {
        lower.contains("cable output")
    }
}

fn prefers_cable(lower: &str) -> bool {
    if cfg!(target_os = "macos") {
        lower.contains("2ch")
    } else {
        true
    }
}

fn missing_cable_status() -> &'static str {
    if cfg!(target_os = "macos") {
        "Install BlackHole 2ch from existential.audio/blackhole, run install-macos.sh, then reopen Frisp."
    } else {
        "Preparing the virtual cable."
    }
}

fn unnamed_cable_status() -> &'static str {
    if cfg!(target_os = "macos") {
        "Frisp is live. In other apps, choose BlackHole 2ch, or run install-macos.sh to name it Frisp Mic."
    } else {
        "Frisp is live. In other apps, choose CABLE Output, or run install-windows.ps1 to name it Frisp Mic."
    }
}

fn pick_output_config(device: &cpal::Device) -> anyhow::Result<cpal::SupportedStreamConfig> {
    let configs: Vec<_> = device.supported_output_configs()?.collect();
    if let Some(range) = configs.iter().find(|range| {
        range.channels() >= 1
            && range.sample_format() == SampleFormat::F32
            && range.min_sample_rate().0 <= MODEL_RATE
            && range.max_sample_rate().0 >= MODEL_RATE
    }) {
        return Ok(range.with_sample_rate(cpal::SampleRate(MODEL_RATE)));
    }
    if let Some(range) = configs
        .iter()
        .find(|range| range.sample_format() == SampleFormat::F32)
    {
        return Ok(range.with_max_sample_rate());
    }
    Ok(device.default_output_config()?)
}

fn set_status(shared: &Shared, text: &str) {
    if let Ok(mut status) = shared.status.lock() {
        *status = text.to_string();
    }
}

struct Resampler {
    step: f64,
    pos: f64,
    prev: f32,
}

impl Resampler {
    fn new(from_rate: u32, to_rate: u32) -> Self {
        let from_rate = from_rate.max(1);
        let to_rate = to_rate.max(1);
        let step = from_rate as f64 / to_rate as f64;
        Self {
            step,
            pos: step,
            prev: 0.0,
        }
    }

    fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if input.is_empty() {
            return Vec::new();
        }
        if (self.step - 1.0).abs() < f64::EPSILON {
            self.prev = *input.last().unwrap_or(&self.prev);
            return input.to_vec();
        }
        let mut samples = Vec::with_capacity(input.len() + 1);
        samples.push(self.prev);
        samples.extend_from_slice(input);
        let last = (samples.len() - 1) as f64;
        let mut out = Vec::new();
        while self.pos <= last {
            let index = self.pos.floor() as usize;
            let frac = (self.pos - index as f64) as f32;
            let left = samples[index];
            let right = samples.get(index + 1).copied().unwrap_or(left);
            out.push(left + (right - left) * frac);
            self.pos += self.step;
        }
        self.prev = *input.last().unwrap_or(&self.prev);
        self.pos -= input.len() as f64;
        if self.pos < 0.0 {
            self.pos = 0.0;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::Resampler;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    #[test]
    fn same_rate_copies_samples() {
        let mut resampler = Resampler::new(48_000, 48_000);
        let input: Vec<f32> = (0..100).map(|i| i as f32).collect();
        let output = resampler.process(&input);
        assert_eq!(output, input);
    }

    #[test]
    fn upsample_emits_more_samples() {
        let mut resampler = Resampler::new(44_100, 48_000);
        let input = vec![0.25; 441];
        let output = resampler.process(&input);
        assert!(output.len() > input.len());
        assert!(output.iter().all(|sample| sample.is_finite()));
    }
}
