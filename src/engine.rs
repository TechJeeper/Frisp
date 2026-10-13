use anyhow::anyhow;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const FRAME: usize = nnnoiseless::DenoiseState::FRAME_SIZE;
const MODEL_RATE: u32 = 48_000;

pub struct Shared {
    pub status: Mutex<String>,
    pub input_peak: AtomicU32,
    pub output_peak: AtomicU32,
    pub noise_cancel: AtomicBool,
    pub virtual_ready: AtomicBool,
}

impl Shared {
    fn new(noise_cancel: bool) -> Self {
        Self {
            status: Mutex::new("Starting.".into()),
            input_peak: AtomicU32::new(0),
            output_peak: AtomicU32::new(0),
            noise_cancel: AtomicBool::new(noise_cancel),
            virtual_ready: AtomicBool::new(false),
        }
    }
}

enum Command {
    UseInput(Option<String>),
    Shutdown,
}

pub struct Engine {
    cmd_tx: Sender<Command>,
}

impl Engine {
    pub fn spawn(noise_cancel: bool) -> (Self, Arc<Shared>) {
        let shared = Arc::new(Shared::new(noise_cancel));
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let shared_thread = Arc::clone(&shared);
        thread::spawn(move || engine_loop(cmd_rx, shared_thread));
        (Self { cmd_tx }, shared)
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
    loop {
        let command = if session.is_none() {
            cmd_rx.recv().ok()
        } else {
            match cmd_rx.recv_timeout(Duration::from_millis(500)) {
                Ok(command) => Some(command),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        };
        match command {
            Some(Command::Shutdown) => break,
            Some(Command::UseInput(name)) => {
                wanted = name;
                drop(session.take());
                session = open_session(wanted.clone(), &shared);
            }
            None => {
                let failed = session.as_ref().is_some_and(|s| s.failed.load(Ordering::Relaxed));
                if failed {
                    drop(session.take());
                    session = open_session(wanted.clone(), &shared);
                }
            }
        }
    }
    set_status(&shared, "Stopped.");
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
        set_status(
            shared,
            "Install VB-Audio Virtual Cable from vb-audio.com/Cable, reboot, then reopen Frisp.",
        );
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
                set_status(
                    shared,
                    "Frisp is live. In other apps, choose the microphone named CABLE Output.",
                );
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
        process_audio(worker_stop, worker_shared, worker_in, worker_out, in_rate, out_rate);
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
        let sample = frame.iter().copied().sum::<f32>() / channels as f32;
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
) {
    let mut denoise = nnnoiseless::DenoiseState::new();
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
            let cleaned = if shared.noise_cancel.load(Ordering::Relaxed) {
                let mut scaled = [0f32; FRAME];
                for (dst, src) in scaled.iter_mut().zip(frame_in) {
                    *dst = src * 32767.0;
                }
                denoise.process_frame(&mut frame_out, &scaled);
                frame_out
                    .iter()
                    .map(|sample| (sample / 32768.0).clamp(-1.0, 1.0))
                    .collect::<Vec<_>>()
            } else {
                frame_in.to_vec()
            };
            let rendered = to_output.process(&cleaned);
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
            cable = Some(device);
        }
    }
    frisp_fallback.or(cable)
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
    lower.contains("cable input")
}

fn is_cable_capture(lower: &str) -> bool {
    lower.contains("cable output")
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
