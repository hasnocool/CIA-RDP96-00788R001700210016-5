// Filename: src/export.rs
// WAV rendering runs on a worker thread started by the UI, never on the audio
// callback or the immediate-mode GUI thread.

use std::path::Path;

use crate::dsp::{AudioSettings, StereoSynth};

pub fn write_wav(path: &Path, sample_rate: u32, settings: AudioSettings) -> Result<(), String> {
    let settings = settings.validated();
    let sample_rate = sample_rate.max(8_000);
    let total_frames = settings
        .total_seconds()
        .checked_mul(u64::from(sample_rate))
        .ok_or_else(|| "The selected WAV duration is too large.".to_owned())?;
    let estimated_data_bytes = total_frames
        .checked_mul(4)
        .ok_or_else(|| "The selected WAV duration is too large.".to_owned())?;
    if estimated_data_bytes > u64::from(u32::MAX) - 44 {
        return Err(
            "This session is too long for a standard WAV file at the selected sample rate. Reduce the session duration or export at a lower device sample rate.".to_owned(),
        );
    }

    let spec = hound::WavSpec {
        channels: 2,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .map_err(|error| format!("Could not create WAV file: {error}"))?;
    let mut synth = StereoSynth::new(sample_rate);

    for frame_index in 0..total_frames {
        let (left, right) = synth.render_frame(&settings, frame_index);
        let left_sample = (left.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        let right_sample = (right.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer
            .write_sample(left_sample)
            .map_err(|error| format!("WAV write failed: {error}"))?;
        writer
            .write_sample(right_sample)
            .map_err(|error| format!("WAV write failed: {error}"))?;
    }

    writer
        .finalize()
        .map_err(|error| format!("Could not finalize WAV file: {error}"))?;
    Ok(())
}
