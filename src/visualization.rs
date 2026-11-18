// Filename: src/visualization.rs
// Live, measured stereo visualization. The audio callback only writes samples
// into a fixed-size atomic ring; FFT work and all allocations happen on the GUI
// thread, never on CPAL's real-time thread.

use std::f32::consts::TAU;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Vec2};

use crate::dsp::AudioSettings;

const RING_CAPACITY: usize = 16_384;
const FFT_SIZE: usize = 16_384;
const WAVEFORM_POINTS: usize = 1_024;
const SPECTRUM_MAX_HZ: f32 = 1_000.0;

const BACKGROUND: Color32 = Color32::from_rgb(7, 12, 23);
const GRID: Color32 = Color32::from_rgb(35, 49, 67);
const MUTED: Color32 = Color32::from_rgb(139, 158, 179);
const LEFT_COLOR: Color32 = Color32::from_rgb(72, 204, 255);
const RIGHT_COLOR: Color32 = Color32::from_rgb(244, 116, 202);
const TRACE_COLOR: Color32 = Color32::from_rgb(169, 227, 255);

struct StereoSlot {
    sequence: AtomicU64,
    left: AtomicU32,
    right: AtomicU32,
}

/// Single-writer / multi-reader lock-free sample tap shared with the audio UI.
/// A slot's sequence number is published only after both samples are stored.
pub struct AudioTap {
    slots: Box<[StereoSlot]>,
    published_frames: AtomicU64,
    history_start: AtomicU64,
}

impl AudioTap {
    pub fn new() -> Self {
        let slots = (0..RING_CAPACITY)
            .map(|_| StereoSlot {
                sequence: AtomicU64::new(0),
                left: AtomicU32::new(0),
                right: AtomicU32::new(0),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();

        Self {
            slots,
            published_frames: AtomicU64::new(0),
            history_start: AtomicU64::new(0),
        }
    }

    /// Begin a new playback take without clearing the ring on the real-time thread.
    pub fn begin_take(&self, cursor: u64) {
        self.history_start.store(cursor, Ordering::Release);
    }

    /// Called by the single CPAL producer. Does not allocate or block.
    pub fn push_frame(&self, cursor: &mut u64, left: f32, right: f32) {
        let sequence = *cursor;
        let slot = &self.slots[sequence as usize & (RING_CAPACITY - 1)];

        slot.left.store(left.to_bits(), Ordering::Relaxed);
        slot.right.store(right.to_bits(), Ordering::Relaxed);
        slot.sequence.store(sequence + 1, Ordering::Release);

        *cursor = sequence.wrapping_add(1);
    }

    /// Publish progress once per CPAL callback, not once per sample.
    pub fn publish(&self, cursor: u64) {
        self.published_frames.store(cursor, Ordering::Release);
    }

    /// Copy a stable-enough view for the GUI. All allocations happen here,
    /// on the UI thread. Frames overwritten during a concurrent read are skipped.
    pub fn snapshot(&self) -> Vec<(f32, f32)> {
        let end = self.published_frames.load(Ordering::Acquire);
        let start = end
            .saturating_sub(RING_CAPACITY as u64)
            .max(self.history_start.load(Ordering::Acquire));
        let mut frames = Vec::with_capacity((end - start) as usize);

        for sequence in start..end {
            let slot = &self.slots[sequence as usize & (RING_CAPACITY - 1)];
            let expected = sequence + 1;
            let before = slot.sequence.load(Ordering::Acquire);

            if before != expected {
                continue;
            }

            let left = f32::from_bits(slot.left.load(Ordering::Relaxed));
            let right = f32::from_bits(slot.right.load(Ordering::Relaxed));
            let after = slot.sequence.load(Ordering::Acquire);

            if before == after {
                frames.push((left, right));
            }
        }

        frames
    }
}

impl Default for AudioTap {
    fn default() -> Self {
        Self::new()
    }
}

pub fn show(
    ui: &mut egui::Ui,
    frames: &[(f32, f32)],
    sample_rate: u32,
    settings: AudioSettings,
    elapsed_seconds: f64,
) {
    let settings = settings.validated();
    let (left_hz, right_hz) = settings.channel_frequencies(elapsed_seconds);

    ui.horizontal_wrapped(|ui| {
        ui.heading("Live harmonic visualizer");
        ui.separator();
        ui.colored_label(LEFT_COLOR, format!("LEFT  {left_hz:.2} Hz"));
        ui.colored_label(RIGHT_COLOR, format!("RIGHT  {right_hz:.2} Hz"));
        ui.strong(format!("Δ {:.2} Hz", right_hz - left_hz));
    });
    ui.label(format!(
        "Measured output · {sample_rate} Hz sample rate · {} captured frames",
        frames.len()
    ));
    ui.label(
        "The phase figure and spectrum are derived from the samples sent to the audio device, not a simulated animation.",
    );
    ui.add_space(8.0);

    let total_width = ui.available_width().max(640.0);
    ui.horizontal_top(|ui| {
        draw_lissajous(
            ui,
            frames,
            Vec2::new(total_width * 0.57, 340.0),
        );
        ui.vertical(|ui| {
            let chart_width = (total_width * 0.40).max(250.0);
            draw_waveform(
                ui,
                frames,
                Vec2::new(chart_width, 162.0),
                0,
            );
            ui.add_space(8.0);
            draw_waveform(
                ui,
                frames,
                Vec2::new(chart_width, 162.0),
                1,
            );
        });
    });

    ui.add_space(10.0);
    ui.separator();
    ui.horizontal_wrapped(|ui| {
        ui.heading("Frequency spectrum");
        ui.label("FFT · dBFS · 0–1,000 Hz");
    });
    draw_spectrum(
        ui,
        frames,
        sample_rate,
        left_hz,
        right_hz,
        Vec2::new(total_width, 285.0),
    );

    ui.add_space(8.0);
    ui.colored_label(
        MUTED,
        "Interpretation: the pure sine carriers create strong fundamental peaks. Pink/white noise adds a broadband floor. The 10 Hz binaural difference is the separation between the ear signals; it is not an extra 10 Hz acoustic tone in either channel.",
    );
}

fn draw_lissajous(ui: &mut egui::Ui, frames: &[(f32, f32)], size: Vec2) {
    ui.vertical(|ui| {
        ui.label("STEREO PHASE / LISSAJOUS");
        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, BACKGROUND);

        let plot = Rect::from_min_max(
            rect.min + Vec2::new(24.0, 16.0),
            rect.max - Vec2::new(16.0, 26.0),
        );
        draw_grid(&painter, plot, 4, 4);

        painter.circle_stroke(
            plot.center(),
            plot.width().min(plot.height()) * 0.42,
            Stroke::new(1.0, GRID),
        );

        if frames.len() < 8 {
            draw_centered_text(&painter, rect, "Waiting for live audio samples");
            return;
        }

        let start = frames.len().saturating_sub(RING_CAPACITY);
        let visible = &frames[start..];
        let peak = visible
            .iter()
            .fold(0.0_f32, |m, (l, r)| m.max(l.abs()).max(r.abs()))
            .max(0.0001);
        let scale = peak * 1.05;
        let half = plot.size() * 0.46;
        let mut points = Vec::with_capacity(visible.len());

        for &(left, right) in visible {
            let x = plot.center().x + (left / scale) * half.x;
            let y = plot.center().y - (right / scale) * half.y;
            points.push(Pos2::new(x, y));
        }

        if points.len() >= 2 {
            painter.add(Shape::line(
                points,
                Stroke::new(1.45, TRACE_COLOR),
            ));
        }

        painter.text(
            Pos2::new(plot.center().x, rect.bottom() - 16.0),
            Align2::CENTER_CENTER,
            "LEFT CHANNEL → X     RIGHT CHANNEL → Y",
            FontId::monospace(10.0),
            MUTED,
        );
        painter.text(
            plot.left_top() + Vec2::new(2.0, 2.0),
            Align2::LEFT_TOP,
            format!("normalized to peak {peak:.3}"),
            FontId::monospace(10.0),
            MUTED,
        );
    });
}

fn draw_waveform(
    ui: &mut egui::Ui,
    frames: &[(f32, f32)],
    size: Vec2,
    channel: usize,
) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, BACKGROUND);

    let (name, color) = if channel == 0 {
        ("LEFT WAVEFORM", LEFT_COLOR)
    } else {
        ("RIGHT WAVEFORM", RIGHT_COLOR)
    };
    painter.text(
        rect.min + Vec2::new(10.0, 8.0),
        Align2::LEFT_TOP,
        name,
        FontId::monospace(11.0),
        color,
    );

    let plot = Rect::from_min_max(
        rect.min + Vec2::new(10.0, 28.0),
        rect.max - Vec2::new(10.0, 10.0),
    );
    draw_grid(&painter, plot, 4, 2);
    painter.line_segment(
        [Pos2::new(plot.left(), plot.center().y), Pos2::new(plot.right(), plot.center().y)],
        Stroke::new(1.0, MUTED),
    );

    if frames.len() < 2 {
        return;
    }

    let start = frames.len().saturating_sub(WAVEFORM_POINTS);
    let visible = &frames[start..];
    let value_at = |frame: &(f32, f32)| if channel == 0 { frame.0 } else { frame.1 };
    let peak = visible
        .iter()
        .fold(0.0_f32, |m, frame| m.max(value_at(frame).abs()))
        .max(0.02);
    let scale = peak * 1.1;
    let denominator = visible.len().saturating_sub(1).max(1) as f32;
    let points = visible
        .iter()
        .enumerate()
        .map(|(index, frame)| {
            let x = plot.left() + (index as f32 / denominator) * plot.width();
            let y = plot.center().y - (value_at(frame) / scale) * (plot.height() * 0.46);
            Pos2::new(x, y)
        })
        .collect::<Vec<_>>();

    if points.len() >= 2 {
        painter.add(Shape::line(points, Stroke::new(1.4, color)));
    }

    painter.text(
        plot.left_bottom() - Vec2::new(0.0, 1.0),
        Align2::LEFT_BOTTOM,
        format!("±{scale:.3} FS"),
        FontId::monospace(9.0),
        MUTED,
    );
}

struct Spectrum {
    left: Vec<f32>,
    right: Vec<f32>,
    bin_hz: f32,
}

fn spectrum(frames: &[(f32, f32)], sample_rate: u32) -> Option<Spectrum> {
    if frames.len() < FFT_SIZE || sample_rate == 0 {
        return None;
    }

    let window = &frames[frames.len() - FFT_SIZE..];
    let left = fft_channel(window, sample_rate, 0);
    let right = fft_channel(window, sample_rate, 1);

    Some(Spectrum {
        left,
        right,
        bin_hz: sample_rate as f32 / FFT_SIZE as f32,
    })
}

fn fft_channel(frames: &[(f32, f32)], sample_rate: u32, channel: usize) -> Vec<f32> {
    let mut values = vec![(0.0_f32, 0.0_f32); FFT_SIZE];
    let mut window_sum = 0.0_f32;

    for (index, frame) in frames.iter().enumerate() {
        let sample = if channel == 0 { frame.0 } else { frame.1 };
        let window = 0.5 - 0.5 * (TAU * index as f32 / (FFT_SIZE - 1) as f32).cos();
        values[index].0 = sample * window;
        window_sum += window;
    }

    fft_in_place(&mut values);

    let max_bin = ((SPECTRUM_MAX_HZ * FFT_SIZE as f32 / sample_rate as f32).floor()
        as usize)
        .min(FFT_SIZE / 2);
    let mut magnitudes = Vec::with_capacity(max_bin + 1);

    for &(real, imaginary) in values.iter().take(max_bin + 1) {
        let magnitude = (real.mul_add(real, imaginary * imaginary)).sqrt();
        // One-sided amplitude normalization compensates for the Hann window.
        magnitudes.push((2.0 * magnitude / window_sum.max(1.0)).max(1.0e-8));
    }

    magnitudes
}

fn fft_in_place(values: &mut [(f32, f32)]) {
    let length = values.len();
    debug_assert!(length.is_power_of_two());

    let mut reversed = 0_usize;
    for index in 1..length {
        let mut bit = length >> 1;
        while reversed & bit != 0 {
            reversed ^= bit;
            bit >>= 1;
        }
        reversed ^= bit;

        if index < reversed {
            values.swap(index, reversed);
        }
    }

    let mut span = 2;
    while span <= length {
        let angle = -TAU / span as f32;
        let step_real = angle.cos();
        let step_imaginary = angle.sin();
        let half = span / 2;

        for base in (0..length).step_by(span) {
            let mut twiddle_real = 1.0_f32;
            let mut twiddle_imaginary = 0.0_f32;

            for offset in 0..half {
                let even = values[base + offset];
                let odd = values[base + offset + half];

                let product_real =
                    odd.0 * twiddle_real - odd.1 * twiddle_imaginary;
                let product_imaginary =
                    odd.0 * twiddle_imaginary + odd.1 * twiddle_real;

                values[base + offset] = (
                    even.0 + product_real,
                    even.1 + product_imaginary,
                );
                values[base + offset + half] = (
                    even.0 - product_real,
                    even.1 - product_imaginary,
                );

                let next_real =
                    twiddle_real * step_real - twiddle_imaginary * step_imaginary;
                twiddle_imaginary =
                    twiddle_real * step_imaginary + twiddle_imaginary * step_real;
                twiddle_real = next_real;
            }
        }

        span <<= 1;
    }
}

fn draw_spectrum(
    ui: &mut egui::Ui,
    frames: &[(f32, f32)],
    sample_rate: u32,
    left_hz: f32,
    right_hz: f32,
    size: Vec2,
) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, BACKGROUND);

    let plot = Rect::from_min_max(
        rect.min + Vec2::new(48.0, 14.0),
        rect.max - Vec2::new(16.0, 30.0),
    );
    draw_grid(&painter, plot, 10, 5);

    let max_hz = SPECTRUM_MAX_HZ.min(sample_rate as f32 * 0.5);
    for step in 0..=10 {
        let hz = max_hz * step as f32 / 10.0;
        let x = plot.left() + plot.width() * step as f32 / 10.0;
        painter.text(
            Pos2::new(x, plot.bottom() + 5.0),
            Align2::CENTER_TOP,
            format!("{hz:.0}"),
            FontId::monospace(10.0),
            MUTED,
        );
    }

    for step in 0..=5 {
        let db = -(step as i32) * 20;
        let y = plot.bottom() - plot.height() * step as f32 / 5.0;
        painter.text(
            Pos2::new(plot.left() - 8.0, y),
            Align2::RIGHT_CENTER,
            format!("{db}"),
            FontId::monospace(10.0),
            MUTED,
        );
    }

    if let Some(measured) = spectrum(frames, sample_rate) {
        draw_spectrum_line(
            &painter,
            plot,
            &measured.left,
            measured.bin_hz,
            max_hz,
            LEFT_COLOR,
        );
        draw_spectrum_line(
            &painter,
            plot,
            &measured.right,
            measured.bin_hz,
            max_hz,
            RIGHT_COLOR,
        );

        draw_frequency_marker(&painter, plot, left_hz, max_hz, LEFT_COLOR);
        draw_frequency_marker(&painter, plot, right_hz, max_hz, RIGHT_COLOR);
    } else {
        let progress = frames.len().min(FFT_SIZE);
        draw_centered_text(
            &painter,
            plot,
            &format!("Collecting FFT window: {progress}/{FFT_SIZE} samples"),
        );
    }

    painter.text(
        Pos2::new(plot.center().x, rect.bottom() - 4.0),
        Align2::CENTER_BOTTOM,
        "FREQUENCY (Hz)",
        FontId::monospace(10.0),
        MUTED,
    );
}

fn draw_spectrum_line(
    painter: &egui::Painter,
    plot: Rect,
    magnitudes: &[f32],
    bin_hz: f32,
    max_hz: f32,
    color: Color32,
) {
    let mut points = Vec::with_capacity(magnitudes.len());
    for (bin, magnitude) in magnitudes.iter().enumerate().skip(1) {
        let frequency = bin as f32 * bin_hz;
        if frequency > max_hz {
            break;
        }

        let db = 20.0 * magnitude.max(1.0e-8).log10();
        let normalized = ((db + 100.0) / 100.0).clamp(0.0, 1.0);
        let x = plot.left() + (frequency / max_hz) * plot.width();
        let y = plot.bottom() - normalized * plot.height();
        points.push(Pos2::new(x, y));
    }

    if points.len() >= 2 {
        painter.add(Shape::line(points, Stroke::new(1.45, color)));
    }
}

fn draw_frequency_marker(
    painter: &egui::Painter,
    plot: Rect,
    frequency: f32,
    max_hz: f32,
    color: Color32,
) {
    if !(0.0..=max_hz).contains(&frequency) {
        return;
    }

    let x = plot.left() + (frequency / max_hz) * plot.width();
    let dash_height = 5.0;
    let mut y = plot.top();
    while y < plot.bottom() {
        let end = (y + dash_height).min(plot.bottom());
        painter.line_segment(
            [Pos2::new(x, y), Pos2::new(x, end)],
            Stroke::new(1.0, color),
        );
        y += dash_height * 2.0;
    }
}

fn draw_grid(painter: &egui::Painter, rect: Rect, vertical: usize, horizontal: usize) {
    for step in 0..=vertical {
        let x = rect.left() + rect.width() * step as f32 / vertical.max(1) as f32;
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(0.7, GRID),
        );
    }

    for step in 0..=horizontal {
        let y = rect.top() + rect.height() * step as f32 / horizontal.max(1) as f32;
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(0.7, GRID),
        );
    }
}

fn draw_centered_text(painter: &egui::Painter, rect: Rect, message: &str) {
    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        message,
        FontId::proportional(13.0),
        MUTED,
    );
}

#[cfg(test)]
mod tests {
    use super::{AudioTap, fft_channel, FFT_SIZE};

    #[test]
    fn audio_tap_returns_the_latest_published_stereo_frames() {
        let tap = AudioTap::new();
        let mut cursor = 0;

        tap.push_frame(&mut cursor, 0.25, -0.5);
        tap.push_frame(&mut cursor, 0.75, -0.25);
        tap.publish(cursor);

        assert_eq!(tap.snapshot(), vec![(0.25, -0.5), (0.75, -0.25)]);
    }

    #[test]
    fn fft_finds_the_generated_carrier_frequencies() {
        let sample_rate = 48_000_u32;
        let frames = (0..FFT_SIZE)
            .map(|index| {
                let time = index as f32 / sample_rate as f32;
                (
                    (std::f32::consts::TAU * 200.0 * time).sin(),
                    (std::f32::consts::TAU * 210.0 * time).sin(),
                )
            })
            .collect::<Vec<_>>();

        let left = fft_channel(&frames, sample_rate, 0);
        let right = fft_channel(&frames, sample_rate, 1);
        let peak_bin = |spectrum: &[f32]| {
            spectrum
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .map(|(index, _)| index)
                .unwrap()
        };

        let left_hz = peak_bin(&left) as f32 * sample_rate as f32 / FFT_SIZE as f32;
        let right_hz = peak_bin(&right) as f32 * sample_rate as f32 / FFT_SIZE as f32;

        assert!((left_hz - 200.0).abs() < 4.0, "left peak was {left_hz} Hz");
        assert!((right_hz - 210.0).abs() < 4.0, "right peak was {right_hz} Hz");
    }
}
