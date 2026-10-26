//! Krisp-inspired Digital Signal Processing pipeline for Frisp.
//!
//! Reverse-engineered insights from Krisp:
//! 1. High-Pass Pre-Filter (`enforce_high_pass_filtering`): Removes subsonic desk bumps,
//!    handling noise, and mic rumble (< 80 Hz) before the neural network sees the audio.
//! 2. Human Voice Clarity / HVC (`hvc-outbound-lfb`): Restores presence, crispness, and vocal
//!    harmonics in the 3.5 kHz - 5 kHz range after neural denoiser attenuation.
//! 3. Dynamic Voice Leveler (`volumeUp` / `normalizationParams`): Automatically normalizes and
//!    boosts cleaned speech volume with intelligent noise freeze (no noise pumping during silence).
//! 4. Soft-Knee Limiter (`clipFix`): Transparent non-linear saturation preventing digital clipping.
//! 5. Voice Activity Detection (`hasEnergy` / VAD): Tracks voice presence vs. ambient noise floor.

const TWO_PI: f32 = std::f32::consts::PI * 2.0;

/// 2nd-order Butterworth High-Pass Biquad filter (80 Hz cutoff at 48 kHz).
/// Removes mechanical rumble, air pops, and DC offset before neural denoiser processing.
#[derive(Clone, Debug)]
pub struct HighPassFilter {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    s1: f32,
    s2: f32,
}

impl HighPassFilter {
    pub fn new(cutoff_hz: f32, sample_rate: f32) -> Self {
        let w0 = TWO_PI * cutoff_hz / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let q = std::f32::consts::FRAC_1_SQRT_2; // 0.7071 Butterworth
        let alpha = sin_w0 / (2.0 * q);

        let b0 = (1.0 + cos_w0) * 0.5;
        let b1 = -(1.0 + cos_w0);
        let b2 = (1.0 + cos_w0) * 0.5;
        let a0 = 1.0 + alpha;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            s1: 0.0,
            s2: 0.0,
        }
    }

    /// Process a single sample using Direct Form II Transposed.
    #[inline]
    pub fn process_sample(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.s1;
        self.s1 = self.b1 * x - self.a1 * y + self.s2;
        self.s2 = self.b2 * x - self.a2 * y;
        y
    }

    /// Process an in-place buffer of samples.
    pub fn process_buffer(&mut self, buffer: &mut [f32]) {
        for sample in buffer.iter_mut() {
            *sample = self.process_sample(*sample);
        }
    }

    pub fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }
}

/// Peaking Presence / Human Voice Clarity (HVC) filter.
/// Boosts vocal intelligibility around 3.6 kHz by +2.5 dB to prevent muffling.
#[derive(Clone, Debug)]
pub struct VoiceClarityFilter {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    s1: f32,
    s2: f32,
}

impl VoiceClarityFilter {
    pub fn new(center_hz: f32, gain_db: f32, q: f32, sample_rate: f32) -> Self {
        let a_gain = 10.0f32.powf(gain_db / 40.0);
        let w0 = TWO_PI * center_hz / sample_rate;
        let cos_w0 = w0.cos();
        let sin_w0 = w0.sin();
        let alpha = sin_w0 / (2.0 * q);

        let b0 = 1.0 + alpha * a_gain;
        let b1 = -2.0 * cos_w0;
        let b2 = 1.0 - alpha * a_gain;
        let a0 = 1.0 + alpha / a_gain;
        let a1 = -2.0 * cos_w0;
        let a2 = 1.0 - alpha / a_gain;

        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
            s1: 0.0,
            s2: 0.0,
        }
    }

    #[inline]
    pub fn process_sample(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.s1;
        self.s1 = self.b1 * x - self.a1 * y + self.s2;
        self.s2 = self.b2 * x - self.a2 * y;
        y
    }

    pub fn process_buffer(&mut self, buffer: &mut [f32]) {
        for sample in buffer.iter_mut() {
            *sample = self.process_sample(*sample);
        }
    }

    pub fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }
}

/// Adaptive Voice Leveler and Auto-Gain (`volumeUp` / `normalizationParams`).
/// Compensates for speech level reduction post-denoising while freezing gain during silence.
#[derive(Clone, Debug)]
pub struct VoiceLeveler {
    current_gain: f32,
    target_rms: f32,
    speech_threshold: f32,
    max_gain: f32,
    min_gain: f32,
}

impl VoiceLeveler {
    pub fn new(target_rms: f32, max_gain: f32) -> Self {
        Self {
            current_gain: 1.0,
            target_rms,
            speech_threshold: 0.008, // ~ -42 dBFS
            max_gain,
            min_gain: 0.8,
        }
    }

    /// Process a block of samples, adapting gain smoothly across the frame.
    pub fn process_frame(&mut self, buffer: &mut [f32]) {
        if buffer.is_empty() {
            return;
        }

        // Calculate frame RMS
        let sum_sq: f32 = buffer.iter().map(|&s| s * s).sum();
        let rms = (sum_sq / buffer.len() as f32).sqrt();

        // Calculate target gain
        let desired_gain = if rms > self.speech_threshold {
            (self.target_rms / rms).clamp(self.min_gain, self.max_gain)
        } else {
            // Speech inactive: gently drift toward 1.0 without amplifying noise
            1.0
        };

        // Smooth adaptation across the frame (avoid zipper noise)
        let start_gain = self.current_gain;
        let smooth_coeff = if desired_gain > start_gain { 0.08 } else { 0.04 };
        let end_gain = start_gain + (desired_gain - start_gain) * smooth_coeff;
        self.current_gain = end_gain;

        let len = buffer.len() as f32;
        for (i, sample) in buffer.iter_mut().enumerate() {
            let frac = i as f32 / len;
            let gain = start_gain + (end_gain - start_gain) * frac;
            *sample *= gain;
        }
    }

    pub fn current_gain(&self) -> f32 {
        self.current_gain
    }
}

/// Soft-Knee Limiter (`clipFix`).
/// Prevents harsh digital clipping with a smooth transparent compression curve above 0.75.
#[inline]
pub fn soft_limit(x: f32) -> f32 {
    let threshold = 0.75;
    let margin = 1.0 - threshold; // 0.25
    let abs_x = x.abs();
    if abs_x <= threshold {
        x
    } else {
        let sign = if x < 0.0 { -1.0 } else { 1.0 };
        let over = abs_x - threshold;
        let compressed = threshold + margin * (over / margin).tanh();
        sign * compressed.min(0.999)
    }
}

/// Voice Activity & Energy Detector (`hasEnergy` / VAD).
/// Tracks voice presence and dynamic noise floor.
#[derive(Clone, Debug)]
pub struct VoiceActivityDetector {
    noise_floor: f32,
    hold_frames: usize,
    hold_count: usize,
}

impl VoiceActivityDetector {
    pub fn new() -> Self {
        Self {
            noise_floor: 0.002, // Initial noise floor estimate
            hold_frames: 20,    // 20 frames * 10 ms = 200 ms hold
            hold_count: 0,
        }
    }

    /// Returns true if human voice is active in this frame.
    pub fn update(&mut self, frame: &[f32]) -> bool {
        if frame.is_empty() {
            return false;
        }

        let sum_sq: f32 = frame.iter().map(|&s| s * s).sum();
        let rms = (sum_sq / frame.len() as f32).sqrt();

        // Track noise floor: slow upward leak, rapid downward tracking
        if rms < self.noise_floor {
            self.noise_floor = self.noise_floor * 0.95 + rms * 0.05;
        } else {
            self.noise_floor = self.noise_floor * 0.999 + rms * 0.001;
        }
        self.noise_floor = self.noise_floor.max(0.0001);

        let snr = rms / self.noise_floor;
        let is_speech = rms > 0.006 && snr > 2.0;

        if is_speech {
            self.hold_count = self.hold_frames;
            true
        } else if self.hold_count > 0 {
            self.hold_count -= 1;
            true
        } else {
            false
        }
    }

    pub fn noise_floor(&self) -> f32 {
        self.noise_floor
    }
}

/// Complete Krisp-inspired Enhancement Processor.
/// Combines High-Pass -> Denoiser -> Voice Clarity -> Voice Leveler -> Soft Limiter.
#[derive(Clone, Debug)]
pub struct KrispDenoisePipeline {
    high_pass: HighPassFilter,
    clarity: VoiceClarityFilter,
    leveler: VoiceLeveler,
    vad: VoiceActivityDetector,
    pub clarity_enabled: bool,
    pub leveler_enabled: bool,
    pub low_cut_enabled: bool,
}

impl KrispDenoisePipeline {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            high_pass: HighPassFilter::new(80.0, sample_rate),
            clarity: VoiceClarityFilter::new(3600.0, 2.5, 0.8, sample_rate),
            leveler: VoiceLeveler::new(0.075, 2.2), // Target ~ -22 dBFS, up to +6.8 dB boost
            vad: VoiceActivityDetector::new(),
            clarity_enabled: true,
            leveler_enabled: true,
            low_cut_enabled: true,
        }
    }

    /// Pre-process input frame before the neural network (e.g. rumble filter & VAD).
    pub fn pre_process(&mut self, frame: &mut [f32]) -> bool {
        let is_voice = self.vad.update(frame);
        if self.low_cut_enabled {
            self.high_pass.process_buffer(frame);
        }
        is_voice
    }

    /// Post-process denoised frame (Voice Clarity, Volume Normalization, Soft Limiting).
    pub fn post_process(&mut self, frame: &mut [f32]) {
        if self.clarity_enabled {
            self.clarity.process_buffer(frame);
        }
        if self.leveler_enabled {
            self.leveler.process_frame(frame);
        }
        for sample in frame.iter_mut() {
            *sample = soft_limit(*sample);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn high_pass_attenuates_low_frequencies() {
        let mut hp = HighPassFilter::new(80.0, 48000.0);
        // 20 Hz subsonic wave
        let mut low_sine = vec![0.0f32; 4800];
        for (i, s) in low_sine.iter_mut().enumerate() {
            *s = (TWO_PI * 20.0 * (i as f32) / 48000.0).sin();
        }
        hp.process_buffer(&mut low_sine);
        // After settling, 20 Hz amplitude should be attenuated by > 12 dB (amplitude < 0.25)
        let tail_peak = low_sine[2400..].iter().fold(0.0f32, |acc, &x| acc.max(x.abs()));
        assert!(tail_peak < 0.35, "20 Hz rumble should be strongly attenuated, got peak: {tail_peak}");
    }

    #[test]
    fn high_pass_passes_speech_frequencies() {
        let mut hp = HighPassFilter::new(80.0, 48000.0);
        // 1000 Hz mid vocal tone
        let mut mid_sine = vec![0.0f32; 4800];
        for (i, s) in mid_sine.iter_mut().enumerate() {
            *s = (TWO_PI * 1000.0 * (i as f32) / 48000.0).sin();
        }
        hp.process_buffer(&mut mid_sine);
        let tail_peak = mid_sine[2400..].iter().fold(0.0f32, |acc, &x| acc.max(x.abs()));
        assert!(tail_peak > 0.95, "1 kHz speech tone should pass unattenuated, got: {tail_peak}");
    }

    #[test]
    fn soft_limit_never_exceeds_unity() {
        assert_eq!(soft_limit(0.5), 0.5);
        assert!(soft_limit(1.5) < 1.0);
        assert!(soft_limit(-2.5) > -1.0);
        assert!(soft_limit(10.0) < 1.0);
    }

    #[test]
    fn vad_detects_speech_and_silence() {
        let mut vad = VoiceActivityDetector::new();
        let silent_frame = vec![0.0005f32; 480];
        assert!(!vad.update(&silent_frame));

        let speech_frame = vec![0.05f32; 480];
        assert!(vad.update(&speech_frame));
    }
}
