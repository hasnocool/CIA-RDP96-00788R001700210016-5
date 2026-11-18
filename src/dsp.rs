// Filename: src/dsp.rs
// Original, deterministic DSP primitives. No locks, allocations, or blocking
// operations are used by StereoSynth::render_frame, so it is suitable for an
// audio callback as well as background WAV rendering.

use std::f32::consts::TAU;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NoiseKind {
    Off = 0,
    White = 1,
    Pink = 2,
    SurfLike = 3,
}

impl NoiseKind {
    pub fn from_u8(value: u8) -> Self {
        match value {
            1 => Self::White,
            2 => Self::Pink,
            3 => Self::SurfLike,
            _ => Self::Off,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::White => "White noise",
            Self::Pink => "Pink noise",
            Self::SurfLike => "Synthetic surf-like noise",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioSettings {
    pub carrier_hz: f32,
    pub beat_hz: f32,
    pub tuning_hz: f32,
    pub tuning_seconds: u32,
    pub binaural_minutes: u32,
    pub tone_level: f32,
    pub noise_kind: NoiseKind,
    pub noise_level: f32,
    pub fade_seconds: u32,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            carrier_hz: 200.0,
            beat_hz: 10.0,
            tuning_hz: 110.0,
            tuning_seconds: 60,
            binaural_minutes: 20,
            tone_level: 0.08,
            noise_kind: NoiseKind::Pink,
            noise_level: 0.015,
            fade_seconds: 10,
        }
    }
}

impl AudioSettings {
    pub fn validated(self) -> Self {
        Self {
            carrier_hz: self.carrier_hz.clamp(80.0, 1_000.0),
            beat_hz: self.beat_hz.clamp(0.5, 30.0),
            tuning_hz: self.tuning_hz.clamp(80.0, 1_000.0),
            tuning_seconds: self.tuning_seconds.min(600),
            binaural_minutes: self.binaural_minutes.clamp(1, 120),
            tone_level: self.tone_level.clamp(0.0, 0.20),
            noise_kind: self.noise_kind,
            noise_level: self.noise_level.clamp(0.0, 0.10),
            fade_seconds: self.fade_seconds.clamp(1, 60),
        }
    }

    pub fn total_seconds(self) -> u64 {
        u64::from(self.tuning_seconds.min(600))
            + u64::from(self.binaural_minutes.clamp(1, 120)) * 60
    }

    pub fn stage_label(self, elapsed_seconds: f64) -> &'static str {
        if elapsed_seconds < f64::from(self.validated().tuning_seconds) {
            "Resonant tuning"
        } else {
            "Binaural / relaxation"
        }
    }

    pub fn channel_frequencies(self, elapsed_seconds: f64) -> (f32, f32) {
        let settings = self.validated();
        if elapsed_seconds < f64::from(settings.tuning_seconds) {
            (settings.tuning_hz, settings.tuning_hz)
        } else {
            (settings.carrier_hz, settings.carrier_hz + settings.beat_hz)
        }
    }
}

/// Stateful stereo synthesizer. Create one instance per audio stream or export.
pub struct StereoSynth {
    sample_rate: f32,
    tuning_phase: f32,
    left_phase: f32,
    right_phase: f32,
    left_noise: NoiseState,
    right_noise: NoiseState,
}

impl StereoSynth {
    pub fn new(sample_rate: u32) -> Self {
        let safe_rate = sample_rate.max(8_000) as f32;
        Self {
            sample_rate: safe_rate,
            tuning_phase: 0.0,
            left_phase: 0.0,
            right_phase: 0.0,
            left_noise: NoiseState::new(0xA341_316C),
            right_noise: NoiseState::new(0xC801_3EA4),
        }
    }

    pub fn reset(&mut self) {
        self.tuning_phase = 0.0;
        self.left_phase = 0.0;
        self.right_phase = 0.0;
        self.left_noise.reset(0xA341_316C);
        self.right_noise.reset(0xC801_3EA4);
    }

    /// Return one left/right sample pair in the range [-0.95, 0.95].
    /// `frame_index` is the zero-based frame position within the current take.
    pub fn render_frame(
        &mut self,
        settings: &AudioSettings,
        frame_index: u64,
    ) -> (f32, f32) {
        // Callers validate settings when they cross the GUI / export boundary;
        // do not repeat clamps and conversions for every sample in real time.
        let sample_rate = self.sample_rate;
        let frame_time = frame_index as f32 / sample_rate;
        let tuning_frames = settings.tuning_seconds as f32 * sample_rate;
        let total_frames = settings.total_seconds() as f32 * sample_rate;

        let tuning_sample = self.tuning_phase.sin();
        self.tuning_phase = wrap_phase(
            self.tuning_phase + TAU * settings.tuning_hz / sample_rate,
        );

        let left_tone = self.left_phase.sin();
        let right_tone = self.right_phase.sin();
        self.left_phase = wrap_phase(
            self.left_phase + TAU * settings.carrier_hz / sample_rate,
        );
        self.right_phase = wrap_phase(
            self.right_phase + TAU * (settings.carrier_hz + settings.beat_hz) / sample_rate,
        );

        let (mut left, mut right) = if (frame_index as f32) < tuning_frames {
            // The report describes a preceding humming/tuning stage. This is a
            // generated equal-channel sine-tone approximation, not a recording
            // of a human voice or the original Gateway chorus.
            (tuning_sample, tuning_sample)
        } else {
            let transition_frames = sample_rate * 1.5;
            let transition = if settings.tuning_seconds == 0 {
                1.0
            } else {
                ((frame_index as f32 - tuning_frames) / transition_frames)
                    .clamp(0.0, 1.0)
            };
            let blend = smoothstep(transition);
            (
                tuning_sample * (1.0 - blend) + left_tone * blend,
                tuning_sample * (1.0 - blend) + right_tone * blend,
            )
        };

        let noise_start = tuning_frames;
        let noise_ramp = if settings.tuning_seconds == 0 {
            1.0
        } else {
            ((frame_index as f32 - noise_start) / (sample_rate * 2.0)).clamp(0.0, 1.0)
        };
        let noise_gain = settings.noise_level * noise_ramp;
        let (left_noise, right_noise) = if settings.noise_kind != NoiseKind::Off && noise_gain > 0.0 {
            (
                self.left_noise.next(settings.noise_kind, frame_time),
                self.right_noise.next(settings.noise_kind, frame_time),
            )
        } else {
            (0.0, 0.0)
        };

        let fade_frames = settings.fade_seconds as f32 * sample_rate;
        let fade_in = if fade_frames > 0.0 {
            (frame_index as f32 / fade_frames).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let remaining_frames = (total_frames - frame_index as f32).max(0.0);
        let fade_out = if fade_frames > 0.0 {
            (remaining_frames / fade_frames).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let envelope = fade_in.min(fade_out);
        let tone_gain = settings.tone_level * envelope;
        let noise_gain = noise_gain * envelope;

        left = (left * tone_gain + left_noise * noise_gain).clamp(-0.95, 0.95);
        right = (right * tone_gain + right_noise * noise_gain).clamp(-0.95, 0.95);
        (left, right)
    }
}

fn wrap_phase(phase: f32) -> f32 {
    if phase >= TAU { phase - TAU } else { phase }
}

fn smoothstep(value: f32) -> f32 {
    value * value * (3.0 - 2.0 * value)
}

struct NoiseState {
    rng: u32,
    b0: f32,
    b1: f32,
    b2: f32,
    b3: f32,
    b4: f32,
    b5: f32,
    b6: f32,
}

impl NoiseState {
    fn new(seed: u32) -> Self {
        Self {
            rng: seed,
            b0: 0.0,
            b1: 0.0,
            b2: 0.0,
            b3: 0.0,
            b4: 0.0,
            b5: 0.0,
            b6: 0.0,
        }
    }

    fn reset(&mut self, seed: u32) {
        self.rng = seed;
        self.b0 = 0.0;
        self.b1 = 0.0;
        self.b2 = 0.0;
        self.b3 = 0.0;
        self.b4 = 0.0;
        self.b5 = 0.0;
        self.b6 = 0.0;
    }

    fn white(&mut self) -> f32 {
        // Xorshift32: deterministic, allocation-free pseudo-random noise.
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        let unit = self.rng as f32 / u32::MAX as f32;
        unit.mul_add(2.0, -1.0)
    }

    fn pink(&mut self) -> f32 {
        let white = self.white();
        self.b0 = 0.99886 * self.b0 + white * 0.055_517_9;
        self.b1 = 0.99332 * self.b1 + white * 0.075_075_9;
        self.b2 = 0.96900 * self.b2 + white * 0.153_852_0;
        self.b3 = 0.86650 * self.b3 + white * 0.310_485_6;
        self.b4 = 0.55000 * self.b4 + white * 0.532_952_2;
        self.b5 = -0.7616 * self.b5 - white * 0.016_898_0;
        let pink = self.b0
            + self.b1
            + self.b2
            + self.b3
            + self.b4
            + self.b5
            + self.b6
            + white * 0.5362;
        self.b6 = white * 0.115_926;
        (pink * 0.11).clamp(-1.0, 1.0)
    }

    fn next(&mut self, kind: NoiseKind, time_seconds: f32) -> f32 {
        match kind {
            NoiseKind::Off => 0.0,
            NoiseKind::White => self.white(),
            NoiseKind::Pink => self.pink(),
            NoiseKind::SurfLike => {
                let swell = (0.5 + 0.5 * (TAU * 0.10 * time_seconds).sin()).powi(3);
                self.pink() * (0.12 + 0.88 * swell)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioSettings, NoiseKind, StereoSynth};

    #[test]
    fn default_settings_use_the_report_illustration_of_ten_hertz() {
        let settings = AudioSettings::default();
        let (left, right) = settings.channel_frequencies(120.0);
        assert!((right - left - 10.0).abs() < 0.001);
    }

    #[test]
    fn tuning_stage_uses_the_same_frequency_in_both_ears() {
        let settings = AudioSettings {
            tuning_seconds: 5,
            ..AudioSettings::default()
        };
        assert_eq!(settings.channel_frequencies(1.0), (110.0, 110.0));
    }

    #[test]
    fn binaural_stage_uses_separate_channel_frequencies() {
        let settings = AudioSettings::default();
        assert_eq!(settings.channel_frequencies(61.0), (200.0, 210.0));
    }

    #[test]
    fn rendered_audio_is_finite_and_bounded() {
        let settings = AudioSettings {
            tone_level: 0.20,
            noise_level: 0.10,
            noise_kind: NoiseKind::SurfLike,
            ..AudioSettings::default()
        };
        let mut synth = StereoSynth::new(48_000);
        for frame in 0..48_000 {
            let (left, right) = synth.render_frame(&settings, frame);
            assert!(left.is_finite() && right.is_finite());
            assert!(left.abs() <= 0.95 && right.abs() <= 0.95);
        }
    }

    #[test]
    fn total_time_includes_tuning_and_binaural_phases() {
        let settings = AudioSettings {
            tuning_seconds: 90,
            binaural_minutes: 12,
            ..AudioSettings::default()
        };
        assert_eq!(settings.total_seconds(), 810);
    }
}
