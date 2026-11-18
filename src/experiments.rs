// Filename: src/experiments.rs
// Deterministic Chladni-style model and frequency-sweep research utilities.
// Heavy sweeps run on a bounded set of background workers. No code in this
// module is called from the CPAL real-time audio callback.

use std::{
    f32::consts::{PI, TAU},
    fmt::Write as FmtWrite,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
};

const GRID_RESOLUTION: usize = 48;
const MAX_SWEEP_POINTS: usize = 500;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BoundaryCondition {
    #[default]
    SimplySupported,
    ClampedApproximation,
    FreeApproximation,
}

impl BoundaryCondition {
    pub const ALL: [Self; 3] = [
        Self::SimplySupported,
        Self::ClampedApproximation,
        Self::FreeApproximation,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::SimplySupported => "Simply supported",
            Self::ClampedApproximation => "Clamped (approximation)",
            Self::FreeApproximation => "Free edge (approximation)",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::SimplySupported => "simply_supported",
            Self::ClampedApproximation => "clamped_approximation",
            Self::FreeApproximation => "free_approximation",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SweepConfig {
    pub carrier_hz: f32,
    pub start_beat_hz: f32,
    pub end_beat_hz: f32,
    pub step_hz: f32,
    pub noise_level: f32,
    pub mode_x: usize,
    pub mode_y: usize,
    pub boundary: BoundaryCondition,
}

impl Default for SweepConfig {
    fn default() -> Self {
        Self {
            carrier_hz: 200.0,
            start_beat_hz: 0.5,
            end_beat_hz: 30.0,
            step_hz: 0.5,
            noise_level: 0.015,
            mode_x: 3,
            mode_y: 2,
            boundary: BoundaryCondition::SimplySupported,
        }
    }
}

impl SweepConfig {
    pub fn validated(self) -> Self {
        let carrier_hz = if self.carrier_hz.is_finite() {
            self.carrier_hz.clamp(80.0, 1_000.0)
        } else {
            200.0
        };
        let start_beat_hz = if self.start_beat_hz.is_finite() {
            self.start_beat_hz.clamp(0.5, 30.0)
        } else {
            0.5
        };
        let end_beat_hz = if self.end_beat_hz.is_finite() {
            self.end_beat_hz.clamp(start_beat_hz, 30.0)
        } else {
            30.0
        };
        let step_hz = if self.step_hz.is_finite() {
            self.step_hz.clamp(0.1, 5.0)
        } else {
            0.5
        };

        Self {
            carrier_hz,
            start_beat_hz,
            end_beat_hz,
            step_hz,
            noise_level: if self.noise_level.is_finite() {
                self.noise_level.clamp(0.0, 0.10)
            } else {
                0.0
            },
            mode_x: self.mode_x.clamp(1, 12),
            mode_y: self.mode_y.clamp(1, 12),
            boundary: self.boundary,
        }
    }

    pub fn beat_values(self) -> Vec<f32> {
        let config = self.validated();
        let count = (((config.end_beat_hz - config.start_beat_hz) / config.step_hz).floor()
            as usize
            + 1)
            .clamp(1, MAX_SWEEP_POINTS);
        (0..count)
            .map(|index| config.start_beat_hz + index as f32 * config.step_hz)
            .filter(|value| *value <= config.end_beat_hz + 0.0001)
            .collect()
    }

    pub fn sample_count(self) -> usize {
        self.beat_values().len()
    }
}

#[derive(Clone, Debug)]
pub struct SweepPoint {
    pub beat_hz: f32,
    pub left_hz: f32,
    pub right_hz: f32,
    pub carrier_to_beat_ratio: f32,
    pub ratio_base12: String,
    pub ratio_base60: String,
    pub mode_x: usize,
    pub mode_y: usize,
    pub boundary: BoundaryCondition,
    pub nodal_crossings: u32,
    pub nodal_density: f32,
    pub rms_amplitude: f32,
    pub grid_resolution: usize,
    pub grid: Vec<f32>,
}

#[derive(Debug)]
pub enum SweepEvent {
    Point(SweepPoint),
    Finished {
        worker_count: usize,
        cancelled: bool,
    },
}

pub struct SweepWorker {
    receiver: Receiver<SweepEvent>,
    cancel: Arc<AtomicBool>,
}

impl SweepWorker {
    pub fn try_recv(&self) -> Result<SweepEvent, TryRecvError> {
        self.receiver.try_recv()
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}

/// Start a sweep on a detached coordinator thread. The coordinator fans points
/// out across at most eight workers, leaving CPU headroom for the UI and audio.
/// The UI consumes events with try_recv(), so it never waits for this work.
pub fn start_sweep(config: SweepConfig) -> SweepWorker {
    let config = config.validated();
    let values = Arc::new(config.beat_values());
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    let (sender, receiver) = mpsc::channel::<SweepEvent>();

    thread::spawn(move || {
        let count = values.len();
        let worker_count = thread::available_parallelism()
            .map(|parallelism| parallelism.get())
            .unwrap_or(1)
            .clamp(1, 8)
            .min(count.max(1));
        let next_index = AtomicUsize::new(0);

        thread::scope(|scope| {
            for _ in 0..worker_count {
                let values = Arc::clone(&values);
                let sender = sender.clone();
                let cancel = Arc::clone(&worker_cancel);
                let next_index = &next_index;

                scope.spawn(move || loop {
                    if cancel.load(Ordering::Acquire) {
                        break;
                    }

                    let index = next_index.fetch_add(1, Ordering::Relaxed);
                    let Some(&beat_hz) = values.get(index) else {
                        break;
                    };

                    let point = analyze_point(config, beat_hz);
                    if sender.send(SweepEvent::Point(point)).is_err() {
                        break;
                    }
                });
            }
        });

        let _ = sender.send(SweepEvent::Finished {
            worker_count,
            cancelled: worker_cancel.load(Ordering::Acquire),
        });
    });

    SweepWorker { receiver, cancel }
}

/// Generates a reproducible two-dimensional standing-wave model. The plate
/// eigenmode is approximated by selectable edge-condition basis functions.
/// A small, explicitly synthetic beat-coupled interference term makes the
/// 0.5–30 Hz sweep investigable; it is not a physical model of binaural sound
/// propagating through a plate or a claim that binaural beats create cymatics.
pub fn analyze_point(config: SweepConfig, beat_hz: f32) -> SweepPoint {
    let config = config.validated();
    let resolution = GRID_RESOLUTION;
    let beat_phase = TAU * beat_hz / 30.0;
    let carrier_phase = TAU * (config.carrier_hz / 1_000.0);
    let drive_x = 1.0 + beat_hz / 5.0;
    let drive_y = 1.0 + beat_hz / 7.0;
    let carrier_scale = (config.carrier_hz / 200.0).clamp(0.4, 5.0);
    let beat_mix = 0.28 * (beat_hz / 30.0).clamp(0.0, 1.0);

    let mut grid = Vec::with_capacity(resolution * resolution);
    let mut sum_squares = 0.0_f64;

    for row in 0..resolution {
        let y = row as f32 / (resolution - 1) as f32;
        for column in 0..resolution {
            let x = column as f32 / (resolution - 1) as f32;
            let plate_x = boundary_basis(x, config.mode_x, config.boundary);
            let plate_y = boundary_basis(y, config.mode_y, config.boundary);
            let plate_mode = plate_x * plate_y;

            // A parameterized spatial overlay, included to test how the chosen
            // mathematical model responds when binaural difference is varied.
            let driven = (TAU * drive_x * x + beat_phase).sin()
                * (TAU * drive_y * y - carrier_phase).cos()
                * carrier_scale.sqrt().recip();
            let deterministic_noise =
                hashed_noise(column as u32, row as u32, beat_hz.to_bits());
            let value = plate_mode
                + beat_mix * driven
                + config.noise_level * deterministic_noise;
            grid.push(value);
            sum_squares += f64::from(value) * f64::from(value);
        }
    }

    let mut nodal_crossings = 0_u32;
    for row in 0..resolution {
        for column in 0..resolution {
            let index = row * resolution + column;
            let value = grid[index];
            if column + 1 < resolution && opposite_sign(value, grid[index + 1]) {
                nodal_crossings = nodal_crossings.saturating_add(1);
            }
            if row + 1 < resolution && opposite_sign(value, grid[index + resolution]) {
                nodal_crossings = nodal_crossings.saturating_add(1);
            }
        }
    }

    let edge_count = 2 * resolution * (resolution - 1);
    let rms_amplitude = (sum_squares / grid.len() as f64).sqrt() as f32;
    let carrier_to_beat_ratio = config.carrier_hz / beat_hz.max(0.0001);

    SweepPoint {
        beat_hz,
        left_hz: config.carrier_hz,
        right_hz: config.carrier_hz + beat_hz,
        carrier_to_beat_ratio,
        ratio_base12: format_radix(carrier_to_beat_ratio, 12, 5),
        ratio_base60: format_radix(carrier_to_beat_ratio, 60, 4),
        mode_x: config.mode_x,
        mode_y: config.mode_y,
        boundary: config.boundary,
        nodal_crossings,
        nodal_density: nodal_crossings as f32 / edge_count as f32,
        rms_amplitude,
        grid_resolution: resolution,
        grid,
    }
}

fn boundary_basis(position: f32, mode: usize, boundary: BoundaryCondition) -> f32 {
    let mode = mode as f32;
    match boundary {
        BoundaryCondition::SimplySupported => (PI * mode * position).sin(),
        BoundaryCondition::ClampedApproximation => 1.0 - (TAU * mode * position).cos(),
        BoundaryCondition::FreeApproximation => (PI * mode * position).cos(),
    }
}

fn opposite_sign(left: f32, right: f32) -> bool {
    (left < 0.0 && right >= 0.0) || (left >= 0.0 && right < 0.0)
}

fn hashed_noise(x: u32, y: u32, seed: u32) -> f32 {
    let mut value = x
        .wrapping_mul(0x9E37_79B9)
        .wrapping_add(y.wrapping_mul(0x85EB_CA6B))
        .wrapping_add(seed.rotate_left(11));
    value ^= value >> 16;
    value = value.wrapping_mul(0x7FEB_352D);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846C_A68B);
    value ^= value >> 16;
    (value as f32 / u32::MAX as f32).mul_add(2.0, -1.0)
}

fn format_radix(value: f32, radix: u32, fractional_digits: usize) -> String {
    if !value.is_finite() || value < 0.0 || !(2..=60).contains(&radix) {
        return "n/a".to_owned();
    }

    let mut integer = value.floor() as u64;
    let mut digits = Vec::new();
    if integer == 0 {
        digits.push(0);
    }
    while integer > 0 {
        digits.push((integer % u64::from(radix)) as u32);
        integer /= u64::from(radix);
    }
    digits.reverse();

    let integer_text = if radix == 12 {
        digits.iter().map(|digit| radix12_digit(*digit)).collect::<String>()
    } else {
        digits.iter().map(|digit| format!("{digit:02}")).collect::<Vec<_>>().join(":")
    };

    let mut fraction = f64::from(value.fract());
    let mut fractional = Vec::with_capacity(fractional_digits);
    for _ in 0..fractional_digits {
        fraction *= f64::from(radix);
        let digit = fraction.floor() as u32;
        fraction -= f64::from(digit);
        fractional.push(digit);
    }

    if fractional.is_empty() {
        integer_text
    } else if radix == 12 {
        format!(
            "{integer_text}.{}",
            fractional.iter().map(|digit| radix12_digit(*digit)).collect::<String>()
        )
    } else {
        format!(
            "{integer_text};{}",
            fractional.iter().map(|digit| format!("{digit:02}")).collect::<Vec<_>>().join(":")
        )
    }
}

fn radix12_digit(digit: u32) -> char {
    match digit {
        0..=9 => char::from(b'0' + digit as u8),
        10 => 'A',
        11 => 'B',
        _ => '?',
    }
}

pub fn draw_pattern(ui: &mut eframe::egui::Ui, point: &SweepPoint, size: eframe::egui::Vec2) {
    use eframe::egui::{Color32, Rect, Sense, Vec2};

    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, Color32::from_rgb(7, 12, 23));

    let resolution = point.grid_resolution.max(2);
    let cell_width = rect.width() / resolution as f32;
    let cell_height = rect.height() / resolution as f32;

    for row in 0..resolution {
        for column in 0..resolution {
            let value = point.grid[row * resolution + column];
            let magnitude = value.abs().clamp(0.0, 1.0);
            let color = if magnitude < 0.035 {
                Color32::from_rgb(236, 245, 255)
            } else if value > 0.0 {
                Color32::from_rgb(
                    (25.0 + 65.0 * magnitude) as u8,
                    (80.0 + 120.0 * magnitude) as u8,
                    (145.0 + 105.0 * magnitude) as u8,
                )
            } else {
                Color32::from_rgb(
                    (105.0 + 130.0 * magnitude) as u8,
                    (38.0 + 75.0 * magnitude) as u8,
                    (85.0 + 125.0 * magnitude) as u8,
                )
            };
            let cell = Rect::from_min_size(
                rect.min + Vec2::new(column as f32 * cell_width, row as f32 * cell_height),
                Vec2::new(cell_width + 0.4, cell_height + 0.4),
            );
            painter.rect_filled(cell, 0.0, color);
        }
    }

    painter.rect_stroke(
        rect,
        4.0,
        eframe::egui::Stroke::new(1.0, Color32::from_rgb(87, 113, 143)),
        eframe::egui::StrokeKind::Inside,
    );
}

pub fn export_bundle(selected_path: &Path, config: SweepConfig, points: &[SweepPoint]) -> Result<PathBuf, String> {
    if points.is_empty() {
        return Err("There are no completed sweep points to export.".to_owned());
    }

    let base = selected_path.with_extension("");
    let csv_path = base.with_extension("csv");
    let json_path = base.with_extension("json");
    let svg_path = base.with_extension("svg");
    let html_path = base.with_extension("html");

    fs::write(&csv_path, build_csv(points))
        .map_err(|error| format!("Could not write {}: {error}", csv_path.display()))?;
    fs::write(&json_path, build_json(config, points))
        .map_err(|error| format!("Could not write {}: {error}", json_path.display()))?;
    let last = points.last().expect("non-empty points checked above");
    fs::write(&svg_path, build_svg(last))
        .map_err(|error| format!("Could not write {}: {error}", svg_path.display()))?;
    fs::write(&html_path, build_animation_html(points))
        .map_err(|error| format!("Could not write {}: {error}", html_path.display()))?;

    Ok(csv_path)
}

fn build_csv(points: &[SweepPoint]) -> String {
    let mut output = String::from(
        "beat_hz,left_carrier_hz,right_carrier_hz,carrier_to_beat_ratio,ratio_base12,ratio_base60,mode_x,mode_y,boundary,nodal_crossings,nodal_density,rms_amplitude,grid_resolution\n",
    );
    for point in points {
        let _ = writeln!(
            output,
            "{:.4},{:.4},{:.4},{:.6},{},{},{},{},{},{},{:.8},{:.8},{}",
            point.beat_hz,
            point.left_hz,
            point.right_hz,
            point.carrier_to_beat_ratio,
            point.ratio_base12,
            point.ratio_base60,
            point.mode_x,
            point.mode_y,
            point.boundary.key(),
            point.nodal_crossings,
            point.nodal_density,
            point.rms_amplitude,
            point.grid_resolution,
        );
    }
    output
}

fn build_json(config: SweepConfig, points: &[SweepPoint]) -> String {
    let config = config.validated();
    let mut output = String::new();
    let _ = writeln!(output, "{{");
    let _ = writeln!(output, "  \"schema_version\": 1,");
    let _ = writeln!(output, "  \"model\": \"parametric_chladni_plate_surrogate\",");
    let _ = writeln!(output, "  \"interpretation\": \"Mathematical model; not measured acoustic cymatics or a physical binaural-to-plate coupling.\",");
    let _ = writeln!(output, "  \"parameters\": {{");
    let _ = writeln!(output, "    \"carrier_hz\": {:.6},", config.carrier_hz);
    let _ = writeln!(output, "    \"start_beat_hz\": {:.6},", config.start_beat_hz);
    let _ = writeln!(output, "    \"end_beat_hz\": {:.6},", config.end_beat_hz);
    let _ = writeln!(output, "    \"step_hz\": {:.6},", config.step_hz);
    let _ = writeln!(output, "    \"noise_level\": {:.6},", config.noise_level);
    let _ = writeln!(output, "    \"mode_x\": {},", config.mode_x);
    let _ = writeln!(output, "    \"mode_y\": {},", config.mode_y);
    let _ = writeln!(output, "    \"boundary\": \"{}\",", config.boundary.key());
    let _ = writeln!(output, "    \"grid_resolution\": {}", GRID_RESOLUTION);
    let _ = writeln!(output, "  }},");
    let _ = writeln!(output, "  \"samples\": [");

    for (index, point) in points.iter().enumerate() {
        let comma = if index + 1 == points.len() { "" } else { "," };
        let _ = writeln!(
            output,
            "    {{\"beat_hz\":{:.6},\"left_hz\":{:.6},\"right_hz\":{:.6},\"carrier_to_beat_ratio\":{:.8},\"ratio_base12\":\"{}\",\"ratio_base60\":\"{}\",\"mode_x\":{},\"mode_y\":{},\"boundary\":\"{}\",\"nodal_crossings\":{},\"nodal_density\":{:.8},\"rms_amplitude\":{:.8},\"grid_resolution\":{},\"grid\":[{}]}}{}",
            point.beat_hz,
            point.left_hz,
            point.right_hz,
            point.carrier_to_beat_ratio,
            point.ratio_base12,
            point.ratio_base60,
            point.mode_x,
            point.mode_y,
            point.boundary.key(),
            point.nodal_crossings,
            point.nodal_density,
            point.rms_amplitude,
            point.grid_resolution,
            point.grid.iter().map(|value| format!("{value:.5}")).collect::<Vec<_>>().join(","),
            comma,
        );
    }

    let _ = writeln!(output, "  ]");
    let _ = writeln!(output, "}}");
    output
}

fn color_for_svg(value: f32) -> (u8, u8, u8) {
    let magnitude = value.abs().clamp(0.0, 1.0);
    if magnitude < 0.035 {
        (238, 245, 255)
    } else if value >= 0.0 {
        ((25.0 + 65.0 * magnitude) as u8, (80.0 + 120.0 * magnitude) as u8, (145.0 + 105.0 * magnitude) as u8)
    } else {
        ((105.0 + 130.0 * magnitude) as u8, (38.0 + 75.0 * magnitude) as u8, (85.0 + 125.0 * magnitude) as u8)
    }
}

fn build_svg(point: &SweepPoint) -> String {
    let resolution = point.grid_resolution.max(2);
    let cell = 640.0 / resolution as f32;
    let mut output = String::from(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"640\" height=\"680\" viewBox=\"0 0 640 680\">\n<rect width=\"640\" height=\"680\" fill=\"#070c17\"/>\n<text x=\"24\" y=\"28\" fill=\"#edf5ff\" font-family=\"sans-serif\" font-size=\"18\">Chladni-style model · Δf = ",
    );
    let _ = write!(output, "{:.3} Hz</text>\n<g transform=\"translate(0 40)\">\n", point.beat_hz);
    for row in 0..resolution {
        for column in 0..resolution {
            let value = point.grid[row * resolution + column];
            let (red, green, blue) = color_for_svg(value);
            let _ = writeln!(
                output,
                "<rect x=\"{:.3}\" y=\"{:.3}\" width=\"{:.3}\" height=\"{:.3}\" fill=\"rgb({red},{green},{blue})\"/>",
                column as f32 * cell,
                row as f32 * cell,
                cell + 0.2,
                cell + 0.2,
            );
        }
    }
    let _ = writeln!(
        output,
        "</g><text x=\"24\" y=\"670\" fill=\"#9ab0c8\" font-family=\"sans-serif\" font-size=\"13\">Boundary: {} · Mode ({}, {}) · Nodal crossings: {}</text></svg>",
        point.boundary.label(),
        point.mode_x,
        point.mode_y,
        point.nodal_crossings,
    );
    output
}

fn build_animation_html(points: &[SweepPoint]) -> String {
    let mut frames = String::from("[");
    for (index, point) in points.iter().enumerate() {
        if index > 0 {
            frames.push(',');
        }
        let _ = write!(
            frames,
            "{{\"beat\":{:.5},\"crossings\":{},\"density\":{:.8},\"values\":[{}]}}",
            point.beat_hz,
            point.nodal_crossings,
            point.nodal_density,
            point.grid.iter().map(|value| format!("{value:.5}")).collect::<Vec<_>>().join(","),
        );
    }
    frames.push(']');

    format!(
        r#"<!doctype html>
<html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Gateway Audio Lab · Frequency sweep</title>
<style>
body{{margin:0;background:#070c17;color:#edf5ff;font:16px system-ui,sans-serif;display:grid;place-items:center;min-height:100vh}}
main{{width:min(92vw,760px)}}canvas{{width:100%;height:auto;background:#070c17;border:1px solid #34465f;border-radius:10px}}
.row{{display:flex;gap:16px;align-items:center;flex-wrap:wrap;margin:12px 0}}button,input{{accent-color:#71ccff}}
small{{color:#9ab0c8}}
</style>
<main><h1>Chladni-style frequency sweep</h1><canvas id="plate" width="640" height="640"></canvas>
<div class="row"><button id="toggle">Pause</button><input id="seek" type="range" min="0" max="{}" value="0"><span id="readout"></span></div>
<small>Generated deterministic model frames; these are mathematical visualizations, not measurements of physical cymatics.</small></main>
<script>
const frames={};
const canvas=document.getElementById("plate"),ctx=canvas.getContext("2d");
const seek=document.getElementById("seek"),readout=document.getElementById("readout"),toggle=document.getElementById("toggle");
let index=0,playing=true,last=0;
function paint(i){{const frame=frames[i],n=Math.sqrt(frame.values.length)|0,cell=canvas.width/n;
ctx.fillStyle="#070c17";ctx.fillRect(0,0,canvas.width,canvas.height);
for(let y=0;y<n;y++)for(let x=0;x<n;x++){{const v=frame.values[y*n+x],a=Math.min(1,Math.abs(v));
if(a<.035)ctx.fillStyle="#eef5ff";else if(v>0)ctx.fillStyle="rgb("+Math.round(25+65*a)+","+Math.round(80+120*a)+","+Math.round(145+105*a)+")";else ctx.fillStyle="rgb("+Math.round(105+130*a)+","+Math.round(38+75*a)+","+Math.round(85+125*a)+")";
ctx.fillRect(x*cell,y*cell,cell+.4,cell+.4);}}
readout.textContent="Δf "+frame.beat.toFixed(2)+" Hz · "+frame.crossings+" nodal crossings · density "+frame.density.toFixed(4);
seek.value=String(i);}
function tick(t){{if(playing&&t-last>140){{index=(index+1)%frames.length;paint(index);last=t;}}requestAnimationFrame(tick);}}
seek.addEventListener("input",()=>{{index=Number(seek.value);paint(index);}});
toggle.addEventListener("click",()=>{{playing=!playing;toggle.textContent=playing?"Pause":"Play";}});
paint(0);requestAnimationFrame(tick);
</script></html>"#,
        points.len().saturating_sub(1),
        frames,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        BoundaryCondition, SweepConfig, analyze_point, format_radix, opposite_sign,
    };

    #[test]
    fn frequency_sweep_includes_both_endpoints() {
        let config = SweepConfig {
            start_beat_hz: 0.5,
            end_beat_hz: 30.0,
            step_hz: 0.5,
            ..SweepConfig::default()
        };
        let values = config.beat_values();
        assert_eq!(values.first().copied(), Some(0.5));
        assert_eq!(values.last().copied(), Some(30.0));
        assert_eq!(values.len(), 60);
    }

    #[test]
    fn base_twelve_and_base_sixty_represent_the_same_ratio() {
        assert_eq!(format_radix(20.0, 12, 3), "18.000");
        assert_eq!(format_radix(20.0, 60, 2), "20;00:00");
        assert_eq!(format_radix(400.0, 60, 1), "06:40;00");
    }

    #[test]
    fn plate_model_is_deterministic_and_finite() {
        let config = SweepConfig {
            boundary: BoundaryCondition::ClampedApproximation,
            noise_level: 0.04,
            ..SweepConfig::default()
        };
        let a = analyze_point(config, 7.5);
        let b = analyze_point(config, 7.5);
        assert_eq!(a.grid, b.grid);
        assert!(a.grid.iter().all(|value| value.is_finite()));
        assert!(a.nodal_density.is_finite());
        assert!(a.rms_amplitude.is_finite());
    }

    #[test]
    fn sign_crossing_test_ignores_same_side_samples() {
        assert!(opposite_sign(-1.0, 1.0));
        assert!(!opposite_sign(1.0, 2.0));
        assert!(!opposite_sign(-1.0, -2.0));
    }

    #[test]
    fn parallel_sweep_returns_every_requested_frequency() {
        use super::{SweepEvent, start_sweep};
        use std::sync::mpsc::TryRecvError;

        let worker = start_sweep(SweepConfig {
            start_beat_hz: 0.5,
            end_beat_hz: 1.5,
            step_hz: 0.5,
            ..SweepConfig::default()
        });
        let mut beats = Vec::new();
        let mut finished = false;

        while !finished {
            match worker.try_recv() {
                Ok(SweepEvent::Point(point)) => beats.push(point.beat_hz),
                Ok(SweepEvent::Finished { cancelled, .. }) => {
                    assert!(!cancelled);
                    finished = true;
                }
                Err(TryRecvError::Empty) => std::thread::yield_now(),
                Err(TryRecvError::Disconnected) => panic!("worker ended without a completion event"),
            }
        }

        beats.sort_by(f32::total_cmp);
        assert_eq!(beats, vec![0.5, 1.0, 1.5]);
    }

    #[test]
    fn mode_and_boundary_selection_are_preserved() {
        let point = analyze_point(
            SweepConfig {
                mode_x: 5,
                mode_y: 4,
                boundary: BoundaryCondition::FreeApproximation,
                ..SweepConfig::default()
            },
            12.0,
        );
        assert_eq!((point.mode_x, point.mode_y), (5, 4));
        assert_eq!(point.boundary, BoundaryCondition::FreeApproximation);
    }
}
