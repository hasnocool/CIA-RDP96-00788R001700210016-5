// Filename: src/audio.rs
// CPAL output is continuously serviced by its own callback thread. GUI controls
// cross the boundary through atomics only; the audio callback never takes a
// mutex, allocates memory, or waits on a channel.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering},
};

use cpal::{
    Device, FromSample, SampleFormat, SizedSample, Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};

use crate::dsp::{AudioSettings, NoiseKind, StereoSynth};
use crate::visualization::AudioTap;

struct SharedControl {
    playing: AtomicBool,
    generation: AtomicU64,
    published_frames: AtomicU64,
    carrier_hz: AtomicU32,
    beat_hz: AtomicU32,
    tuning_hz: AtomicU32,
    tuning_seconds: AtomicU64,
    binaural_minutes: AtomicU64,
    tone_level: AtomicU32,
    noise_kind: AtomicU8,
    noise_level: AtomicU32,
    fade_seconds: AtomicU64,
}

impl SharedControl {
    fn new(settings: AudioSettings) -> Self {
        let shared = Self {
            playing: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            published_frames: AtomicU64::new(0),
            carrier_hz: AtomicU32::new(0),
            beat_hz: AtomicU32::new(0),
            tuning_hz: AtomicU32::new(0),
            tuning_seconds: AtomicU64::new(0),
            binaural_minutes: AtomicU64::new(0),
            tone_level: AtomicU32::new(0),
            noise_kind: AtomicU8::new(0),
            noise_level: AtomicU32::new(0),
            fade_seconds: AtomicU64::new(0),
        };
        shared.store_settings(settings);
        shared
    }

    fn store_settings(&self, settings: AudioSettings) {
        let settings = settings.validated();
        self.carrier_hz.store(settings.carrier_hz.to_bits(), Ordering::Relaxed);
        self.beat_hz.store(settings.beat_hz.to_bits(), Ordering::Relaxed);
        self.tuning_hz.store(settings.tuning_hz.to_bits(), Ordering::Relaxed);
        self.tuning_seconds.store(u64::from(settings.tuning_seconds), Ordering::Relaxed);
        self.binaural_minutes.store(u64::from(settings.binaural_minutes), Ordering::Relaxed);
        self.tone_level.store(settings.tone_level.to_bits(), Ordering::Relaxed);
        self.noise_kind.store(settings.noise_kind as u8, Ordering::Relaxed);
        self.noise_level.store(settings.noise_level.to_bits(), Ordering::Relaxed);
        self.fade_seconds.store(u64::from(settings.fade_seconds), Ordering::Relaxed);
    }

    fn load_settings(&self) -> AudioSettings {
        AudioSettings {
            carrier_hz: f32::from_bits(self.carrier_hz.load(Ordering::Relaxed)),
            beat_hz: f32::from_bits(self.beat_hz.load(Ordering::Relaxed)),
            tuning_hz: f32::from_bits(self.tuning_hz.load(Ordering::Relaxed)),
            tuning_seconds: self.tuning_seconds.load(Ordering::Relaxed) as u32,
            binaural_minutes: self.binaural_minutes.load(Ordering::Relaxed) as u32,
            tone_level: f32::from_bits(self.tone_level.load(Ordering::Relaxed)),
            noise_kind: NoiseKind::from_u8(self.noise_kind.load(Ordering::Relaxed)),
            noise_level: f32::from_bits(self.noise_level.load(Ordering::Relaxed)),
            fade_seconds: self.fade_seconds.load(Ordering::Relaxed) as u32,
        }
        .validated()
    }
}

pub struct AudioEngine {
    control: Arc<SharedControl>,
    visualization: Arc<AudioTap>,
    // Keeping the stream alive owns the device callback for the lifetime of the app.
    _stream: Stream,
    sample_rate: u32,
    channels: u16,
    device_name: String,
}

impl AudioEngine {
    pub fn new(settings: AudioSettings) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "No default audio output device was found.".to_owned())?;
        let device_name = device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|_| "Default output device".to_string());
        let supported_config = device
            .default_output_config()
            .map_err(|error| format!("Could not read output configuration: {error}"))?;
        let sample_format = supported_config.sample_format();
        let stream_config: StreamConfig = supported_config.into();
        let sample_rate = stream_config.sample_rate;
        let channels = stream_config.channels;
        if channels == 0 {
            return Err("The selected output device reports zero channels.".to_owned());
        }

        let control = Arc::new(SharedControl::new(settings));
        let visualization = Arc::new(AudioTap::new());
        let stream = match sample_format {
            SampleFormat::F32 => build_stream::<f32>(&device, stream_config, Arc::clone(&control), Arc::clone(&visualization)),
            SampleFormat::F64 => build_stream::<f64>(&device, stream_config, Arc::clone(&control), Arc::clone(&visualization)),
            SampleFormat::I8 => build_stream::<i8>(&device, stream_config, Arc::clone(&control), Arc::clone(&visualization)),
            SampleFormat::I16 => build_stream::<i16>(&device, stream_config, Arc::clone(&control), Arc::clone(&visualization)),
            SampleFormat::I24 => build_stream::<cpal::I24>(&device, stream_config, Arc::clone(&control), Arc::clone(&visualization)),
            SampleFormat::I32 => build_stream::<i32>(&device, stream_config, Arc::clone(&control), Arc::clone(&visualization)),
            SampleFormat::I64 => build_stream::<i64>(&device, stream_config, Arc::clone(&control), Arc::clone(&visualization)),
            SampleFormat::U8 => build_stream::<u8>(&device, stream_config, Arc::clone(&control), Arc::clone(&visualization)),
            SampleFormat::U16 => build_stream::<u16>(&device, stream_config, Arc::clone(&control), Arc::clone(&visualization)),
            SampleFormat::U32 => build_stream::<u32>(&device, stream_config, Arc::clone(&control), Arc::clone(&visualization)),
            SampleFormat::U64 => build_stream::<u64>(&device, stream_config, Arc::clone(&control), Arc::clone(&visualization)),
            other => return Err(format!("The output device uses unsupported sample format: {other:?}")),
        }
        .map_err(|error| format!("Could not create the audio stream: {error}"))?;

        stream
            .play()
            .map_err(|error| format!("Could not start the audio stream: {error}"))?;

        Ok(Self {
            control,
            visualization,
            _stream: stream,
            sample_rate,
            channels,
            device_name: device_name.to_string(),
        })
    }

    pub fn set_settings(&self, settings: AudioSettings) {
        self.control.store_settings(settings);
    }

    pub fn start_or_resume(&self, settings: AudioSettings) {
        self.set_settings(settings);
        let total_frames = settings.total_seconds().saturating_mul(u64::from(self.sample_rate));
        let current_frames = self.control.published_frames.load(Ordering::Acquire);
        if current_frames >= total_frames {
            self.control.playing.store(false, Ordering::Release);
            self.control.generation.fetch_add(1, Ordering::AcqRel);
            self.control.published_frames.store(0, Ordering::Release);
        }
        self.control.playing.store(true, Ordering::Release);
    }

    pub fn pause(&self) {
        self.control.playing.store(false, Ordering::Release);
    }

    pub fn stop(&self) {
        self.control.playing.store(false, Ordering::Release);
        self.control.generation.fetch_add(1, Ordering::AcqRel);
        self.control.published_frames.store(0, Ordering::Release);
    }

    pub fn is_playing(&self) -> bool {
        self.control.playing.load(Ordering::Acquire)
    }

    pub fn elapsed_seconds(&self) -> f64 {
        self.control.published_frames.load(Ordering::Acquire) as f64
            / f64::from(self.sample_rate)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u16 {
        self.channels
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub fn visualization_tap(&self) -> Arc<AudioTap> {
        Arc::clone(&self.visualization)
    }
}

fn build_stream<T>(
    device: &Device,
    config: StreamConfig,
    control: Arc<SharedControl>,
    visualization: Arc<AudioTap>,
) -> Result<Stream, cpal::Error>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels);
    let sample_rate = config.sample_rate;
    let mut synth = StereoSynth::new(sample_rate);
    let mut callback_generation = u64::MAX;
    let mut frame_index = 0_u64;
    let mut visualization_cursor = 0_u64;

    device.build_output_stream(
        config,
        move |output: &mut [T], _callback_info| {
            let generation = control.generation.load(Ordering::Acquire);
            if generation != callback_generation {
                synth.reset();
                frame_index = 0;
                visualization.begin_take(visualization_cursor);
                callback_generation = generation;
            }

            if !control.playing.load(Ordering::Acquire) {
                let silence = T::from_sample(0.0_f32);
                output.fill(silence);
                return;
            }

            let settings = control.load_settings();
            let total_frames = settings.total_seconds().saturating_mul(u64::from(sample_rate));
            let silence = T::from_sample(0.0_f32);

            for frame in output.chunks_mut(channels) {
                if frame_index >= total_frames {
                    control.playing.store(false, Ordering::Release);
                    frame.fill(silence);
                    continue;
                }

                let (left, right) = synth.render_frame(&settings, frame_index);
                if channels == 1 {
                    // Record the actual mono mix if the selected output is mono.
                    let mono = (left + right) * 0.5;
                    visualization.push_frame(&mut visualization_cursor, mono, mono);
                    frame[0] = T::from_sample(mono);
                } else {
                    // The visualizer receives the same L/R samples written to the device.
                    visualization.push_frame(&mut visualization_cursor, left, right);
                    frame[0] = T::from_sample(left);
                    frame[1] = T::from_sample(right);
                    for sample in frame.iter_mut().skip(2) {
                        *sample = silence;
                    }
                }
                frame_index = frame_index.saturating_add(1);
            }

            // Publish progress once per callback. The generation guard prevents an
            // older callback from overwriting a newly-reset session counter.
            if control.generation.load(Ordering::Acquire) == generation {
                control.published_frames.store(frame_index, Ordering::Release);
            }
            visualization.publish(visualization_cursor);
        },
        |error| eprintln!("Audio stream error: {error}"),
        None,
    )
}
