// Filename: src/app.rs

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::Duration,
};

use eframe::egui;

use crate::{
    audio::AudioEngine,
    dsp::{AudioSettings, NoiseKind},
    experiments::{self, BoundaryCondition, SweepConfig, SweepEvent, SweepPoint, SweepWorker},
    export,
    harmonic_3d::GpuHarmonicRenderer,
    visualization,
};

pub struct GatewayApp {
    settings: AudioSettings,
    audio: Option<AudioEngine>,
    audio_error: Option<String>,
    export_receiver: Option<Receiver<Result<PathBuf, String>>>,
    exporting: bool,
    export_status: String,
    visualizer_enabled: bool,
    visualizer_mode: Arc<AtomicU8>,
    visualizer_close_signal: Arc<AtomicBool>,
    harmonic_3d: Option<GpuHarmonicRenderer>,
    sweep_config: SweepConfig,
    sweep_worker: Option<SweepWorker>,
    sweep_points: Vec<SweepPoint>,
    sweep_total: usize,
    sweep_completed: usize,
    sweep_status: String,
    sweep_export_receiver: Option<Receiver<Result<PathBuf, String>>>,
    sweep_exporting: bool,
}

impl GatewayApp {
    pub fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        let settings = AudioSettings::default();
        let harmonic_3d = GpuHarmonicRenderer::new(creation_context);
        let sweep_config = SweepConfig {
            carrier_hz: settings.carrier_hz,
            noise_level: settings.noise_level,
            ..SweepConfig::default()
        };
        match AudioEngine::new(settings) {
            Ok(audio) => Self {
                settings,
                visualizer_enabled: false,
                visualizer_mode: Arc::new(AtomicU8::new(
                    visualization::VisualizationMode::default() as u8,
                )),
                visualizer_close_signal: Arc::new(AtomicBool::new(false)),
                harmonic_3d,
                sweep_config,
                sweep_worker: None,
                sweep_points: Vec::new(),
                sweep_total: 0,
                sweep_completed: 0,
                sweep_status: "Ready. Configure a sweep from 0.5 to 30 Hz.".to_owned(),
                sweep_export_receiver: None,
                sweep_exporting: false,
                audio: Some(audio),
                audio_error: None,
                export_receiver: None,
                exporting: false,
                export_status: "Ready. Use stereo headphones for channel-separated tones."
                    .to_owned(),
            },
            Err(error) => Self {
                settings,
                visualizer_enabled: false,
                visualizer_mode: Arc::new(AtomicU8::new(
                    visualization::VisualizationMode::default() as u8,
                )),
                visualizer_close_signal: Arc::new(AtomicBool::new(false)),
                harmonic_3d,
                sweep_config,
                sweep_worker: None,
                sweep_points: Vec::new(),
                sweep_total: 0,
                sweep_completed: 0,
                sweep_status: "Ready. Configure a sweep from 0.5 to 30 Hz.".to_owned(),
                sweep_export_receiver: None,
                sweep_exporting: false,
                audio: None,
                audio_error: Some(error.clone()),
                export_receiver: None,
                exporting: false,
                export_status: format!(
                    "Live output unavailable: {error}. WAV export is still available."
                ),
            },
        }
    }

    fn begin_export(&mut self) {
        if self.exporting {
            return;
        }

        let settings = self.settings.validated();
        let sample_rate = self
            .audio
            .as_ref()
            .map(AudioEngine::sample_rate)
            .unwrap_or(48_000);
        let (sender, receiver) = mpsc::channel();
        self.export_receiver = Some(receiver);
        self.exporting = true;
        self.export_status =
            "Choose a destination. WAV rendering will run in the background.".to_owned();

        thread::spawn(move || {
            // The native save dialog and file rendering are isolated from both the
            // egui event loop and the CPAL real-time audio callback.
            let result = rfd::FileDialog::new()
                .add_filter("WAV audio", &["wav"])
                .set_file_name("gateway-audio-session.wav")
                .save_file()
                .ok_or_else(|| "Export cancelled.".to_owned())
                .and_then(|path| export::write_wav(&path, sample_rate, settings).map(|()| path));
            let _ = sender.send(result);
        });
    }

    fn begin_sweep(&mut self) {
        if self.sweep_worker.is_some() {
            return;
        }

        self.sweep_config.carrier_hz = self.settings.carrier_hz;
        self.sweep_config.noise_level = self.settings.noise_level;
        self.sweep_config = self.sweep_config.validated();
        self.sweep_total = self.sweep_config.sample_count();
        self.sweep_completed = 0;
        self.sweep_points.clear();
        self.sweep_status = format!(
            "Running {} frequency points on a background worker pool…",
            self.sweep_total
        );
        self.sweep_worker = Some(experiments::start_sweep(self.sweep_config));
    }

    fn cancel_sweep(&mut self) {
        if let Some(worker) = &self.sweep_worker {
            worker.cancel();
            self.sweep_status = "Cancellation requested; active points will finish safely.".to_owned();
        }
    }

    fn export_sweep(&mut self) {
        if self.sweep_exporting || self.sweep_points.is_empty() {
            return;
        }

        let config = self.sweep_config;
        let points = self.sweep_points.clone();
        let (sender, receiver) = mpsc::channel();
        self.sweep_export_receiver = Some(receiver);
        self.sweep_exporting = true;
        self.sweep_status = "Choose a filename. CSV, JSON, SVG, and HTML animation will be written in the background.".to_owned();

        thread::spawn(move || {
            let result = rfd::FileDialog::new()
                .add_filter("CSV data", &["csv"])
                .set_file_name("gateway-frequency-sweep.csv")
                .save_file()
                .ok_or_else(|| "Sweep export cancelled.".to_owned())
                .and_then(|path| experiments::export_bundle(&path, config, &points));
            let _ = sender.send(result);
        });
    }

    fn format_time(seconds: f64) -> String {
        let total = seconds.max(0.0) as u64;
        format!(
            "{:02}:{:02}:{:02}",
            total / 3_600,
            (total / 60) % 60,
            total % 60
        )
    }
}

impl eframe::App for GatewayApp {
    fn logic(&mut self, context: &egui::Context, _frame: &mut eframe::Frame) {
        let completed =
            self.export_receiver
                .as_ref()
                .and_then(|receiver| match receiver.try_recv() {
                    Ok(result) => Some(result),
                    Err(mpsc::TryRecvError::Empty) => None,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        Some(Err("The WAV export worker stopped unexpectedly.".to_owned()))
                    }
                });

        if let Some(result) = completed {
            self.exporting = false;
            self.export_receiver = None;
            self.export_status = match result {
                Ok(path) => format!("WAV exported: {}", path.display()),
                Err(error) => error,
            };
        }

        let mut sweep_finished = false;
        loop {
            let event = match self.sweep_worker.as_ref() {
                Some(worker) => worker.try_recv(),
                None => break,
            };
            match event {
                Ok(SweepEvent::Point(point)) => {
                    self.sweep_completed += 1;
                    self.sweep_points.push(point);
                }
                Ok(SweepEvent::Finished {
                    worker_count,
                    cancelled,
                }) => {
                    self.sweep_status = if cancelled {
                        format!(
                            "Sweep cancelled after {} of {} points using {} workers.",
                            self.sweep_completed, self.sweep_total, worker_count
                        )
                    } else {
                        format!(
                            "Sweep complete: {} points calculated by {} parallel workers.",
                            self.sweep_completed, worker_count
                        )
                    };
                    sweep_finished = true;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.sweep_status =
                        "Sweep worker exited without sending a completion event.".to_owned();
                    sweep_finished = true;
                    break;
                }
            }
        }
        if sweep_finished {
            self.sweep_worker = None;
            self.sweep_points
                .sort_by(|left, right| left.beat_hz.total_cmp(&right.beat_hz));
        }

        let sweep_export_completed = self
            .sweep_export_receiver
            .as_ref()
            .and_then(|receiver| match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("The sweep export worker stopped unexpectedly.".to_owned()))
                }
            });
        if let Some(result) = sweep_export_completed {
            self.sweep_exporting = false;
            self.sweep_export_receiver = None;
            self.sweep_status = match result {
                Ok(path) => format!(
                    "Research bundle exported: {} (+ JSON, SVG, and HTML animation).",
                    path.display()
                ),
                Err(error) => error,
            };
        }

        if self.visualizer_close_signal.swap(false, Ordering::AcqRel) {
            self.visualizer_enabled = false;
            context.request_repaint();
        }

        let live_playing = self.audio.as_ref().is_some_and(AudioEngine::is_playing);
        if live_playing
            || self.exporting
            || self.visualizer_enabled
            || self.sweep_worker.is_some()
            || self.sweep_exporting
        {
            context.request_repaint_after(Duration::from_millis(150));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(10.0);
            ui.heading("Gateway Audio Lab");
            ui.label("Stereo binaural-tone and noise generator · Rust desktop application");
            ui.add_space(6.0);
            ui.separator();

            ui.group(|ui| {
                ui.heading("Session controls");
                ui.horizontal_wrapped(|ui| {
                    let playing = self.audio.as_ref().is_some_and(AudioEngine::is_playing);
                    let elapsed = self.audio.as_ref().map(AudioEngine::elapsed_seconds).unwrap_or(0.0);
                    let total = self.settings.total_seconds() as f64;

                    if let Some(audio) = &self.audio {
                        if playing {
                            if ui.button("Pause").clicked() {
                                audio.pause();
                            }
                        } else {
                            let label = if elapsed > 0.0 && elapsed < total {
                                "Resume"
                            } else {
                                "Start session"
                            };
                            if ui.button(label).clicked() {
                                audio.start_or_resume(self.settings);
                            }
                        }
                        if ui.button("Stop / reset").clicked() {
                            audio.stop();
                        }
                    } else {
                        ui.add_enabled(false, egui::Button::new("Live output unavailable"));
                    }

                    if ui
                        .add_enabled(!self.exporting, egui::Button::new("Export WAV"))
                        .clicked()
                    {
                        self.begin_export();
                    }
                });

                let elapsed = self.audio.as_ref().map(AudioEngine::elapsed_seconds).unwrap_or(0.0);
                let total = self.settings.total_seconds() as f64;
                let progress = if total > 0.0 { (elapsed / total).clamp(0.0, 1.0) as f32 } else { 0.0 };
                ui.add(
                    egui::ProgressBar::new(progress)
                        .desired_height(14.0)
                        .show_percentage(),
                );
                let stage = self.settings.stage_label(elapsed);
                ui.horizontal_wrapped(|ui| {
                    ui.strong(stage);
                    ui.label(format!("Elapsed {} / {}", Self::format_time(elapsed), Self::format_time(total)));
                    ui.label(format!("Remaining {}", Self::format_time((total - elapsed).max(0.0))));
                });
                ui.label(&self.export_status);
            });

            ui.add_space(8.0);
            ui.group(|ui| {
                ui.heading("Live visualization");
                let previous = self.visualizer_enabled;
                let response = ui.add_enabled(
                    self.audio.is_some(),
                    egui::Checkbox::new(
                        &mut self.visualizer_enabled,
                        "Open harmonic visualization in a separate window",
                    ),
                );
                if response.changed() && self.visualizer_enabled {
                    self.visualizer_close_signal.store(false, Ordering::Release);
                } else if previous && !self.visualizer_enabled {
                    ui.ctx().send_viewport_cmd_to(
                        egui::ViewportId::from_hash_of("gateway_harmonic_visualizer"),
                        egui::ViewportCommand::Close,
                    );
                }
                if self.audio.is_some() {
                    ui.label("Opens a live stereo phase figure, left/right waveforms, and measured FFT spectrum.");
                } else {
                    ui.label("Live visualization requires a working output audio device.");
                }
            });

            ui.add_space(8.0);
            ui.group(|ui| {
                ui.heading("3D harmonic geometry · WGPU");
                if let Some(renderer) = &mut self.harmonic_3d {
                    let elapsed = self.audio.as_ref()
                        .map(AudioEngine::elapsed_seconds)
                        .unwrap_or(0.0);
                    renderer.show(
                        ui,
                        self.settings.carrier_hz,
                        self.settings.beat_hz,
                        self.settings.noise_level,
                        elapsed,
                    );
                } else {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        "The WGPU render state is unavailable; the existing 2D visualizer remains available.",
                    );
                }
            });

            ui.add_space(8.0);
            ui.group(|ui| {
                ui.heading("Parallel frequency experiments");
                ui.label(
                    "Sweep the binaural difference and record how this deterministic Chladni-style surrogate responds. Each frequency point is calculated independently on a bounded background worker pool.",
                );
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::Slider::new(&mut self.sweep_config.start_beat_hz, 0.5..=30.0)
                            .text("Start Δf (Hz)"),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.sweep_config.end_beat_hz, 0.5..=30.0)
                            .text("End Δf (Hz)"),
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::Slider::new(&mut self.sweep_config.step_hz, 0.1..=5.0)
                            .text("Step (Hz)"),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.sweep_config.carrier_hz, 80.0..=1_000.0)
                            .text("Carrier (Hz)"),
                    );
                });
                ui.horizontal_wrapped(|ui| {
                    ui.add(
                        egui::Slider::new(&mut self.sweep_config.mode_x, 1..=12)
                            .text("Plate mode X"),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.sweep_config.mode_y, 1..=12)
                            .text("Plate mode Y"),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.sweep_config.noise_level, 0.0..=0.10)
                            .text("Model noise"),
                    );
                });
                egui::ComboBox::from_label("Boundary condition")
                    .selected_text(self.sweep_config.boundary.label())
                    .show_ui(ui, |ui| {
                        for boundary in BoundaryCondition::ALL {
                            ui.selectable_value(
                                &mut self.sweep_config.boundary,
                                boundary,
                                boundary.label(),
                            );
                        }
                    });

                ui.horizontal_wrapped(|ui| {
                    if self.sweep_worker.is_some() {
                        if ui.button("Cancel sweep").clicked() {
                            self.cancel_sweep();
                        }
                    } else if ui.button("Run parallel sweep").clicked() {
                        self.begin_sweep();
                    }

                    if ui
                        .add_enabled(
                            !self.sweep_points.is_empty() && !self.sweep_exporting,
                            egui::Button::new("Export research bundle"),
                        )
                        .clicked()
                    {
                        self.export_sweep();
                    }
                    if self.sweep_exporting {
                        ui.spinner();
                    }
                });

                let progress = if self.sweep_total == 0 {
                    0.0
                } else {
                    (self.sweep_completed as f32 / self.sweep_total as f32).clamp(0.0, 1.0)
                };
                ui.add(
                    egui::ProgressBar::new(progress)
                        .desired_height(12.0)
                        .show_percentage(),
                );
                ui.label(&self.sweep_status);
                ui.small(format!(
                    "{} of {} points · {} stored samples · grid {}×{}",
                    self.sweep_completed,
                    self.sweep_total,
                    self.sweep_points.len(),
                    48,
                    48
                ));

                if let Some(point) = self.sweep_points.last() {
                    ui.separator();
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.strong(format!("Δf = {:.2} Hz", point.beat_hz));
                            ui.label(format!(
                                "Nodal crossings: {} · density {:.4}",
                                point.nodal_crossings, point.nodal_density
                            ));
                            ui.label(format!("RMS model amplitude: {:.5}", point.rms_amplitude));
                            ui.label(format!(
                                "Carrier / difference: {:.5}",
                                point.carrier_to_beat_ratio
                            ));
                            ui.label(format!("Base-12 ratio: {}", point.ratio_base12));
                            ui.label(format!("Base-60 ratio: {}", point.ratio_base60));
                            ui.small("Base conversions are alternate representations of the same ratio, not separate physical predictions.");
                        });
                        ui.add_space(12.0);
                        experiments::draw_pattern(
                            ui,
                            point,
                            egui::Vec2::new(250.0, 250.0),
                        );
                    });
                }
                ui.small(
                    "Boundary labels marked “approximation” use simplified basis functions. The beat-coupled overlay is a declared mathematical experiment; a binaural beat is an auditory percept and does not itself create a plate's physical nodal pattern.",
                );
            });

            ui.add_space(8.0);
            ui.columns(2, |columns| {
                columns[0].group(|ui| {
                    ui.heading("Binaural signal");
                    ui.label("The report's worked example uses a 10 Hz difference between the two ears.");
                    ui.add(
                        egui::Slider::new(&mut self.settings.carrier_hz, 80.0..=1_000.0)
                            .text("Left carrier (Hz)"),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.settings.beat_hz, 0.5..=30.0)
                            .text("Binaural difference (Hz)"),
                    );
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Use report example: 10 Hz").clicked() {
                            self.settings.beat_hz = 10.0;
                        }
                        if ui.button("Reset signal").clicked() {
                            self.settings.carrier_hz = 200.0;
                            self.settings.beat_hz = 10.0;
                        }
                    });
                    let elapsed = self.audio.as_ref().map(AudioEngine::elapsed_seconds).unwrap_or(0.0);
                    let (left, right) = self.settings.channel_frequencies(elapsed);
                    ui.separator();
                    ui.label(format!("Left channel:  {left:.2} Hz"));
                    ui.label(format!("Right channel: {right:.2} Hz"));
                    ui.strong(format!("Difference: {:.2} Hz", right - left));
                    ui.small("Use headphones. A mono speaker output cannot preserve the ear-to-ear separation.");
                });

                columns[1].group(|ui| {
                    ui.heading("Resonant tuning stage");
                    ui.label("The report describes humming with a chorus before Hemi-Sync is introduced.");
                    ui.add(
                        egui::Slider::new(&mut self.settings.tuning_seconds, 0..=600)
                            .text("Tuning duration (seconds)"),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.settings.tuning_hz, 80.0..=500.0)
                            .text("Synthetic tuning tone (Hz)"),
                    );
                    ui.small("This equal-channel sine tone is a configurable approximation, not a human-humming recording or an original Gateway track.");
                });
            });

            ui.add_space(8.0);
            ui.columns(2, |columns| {
                columns[0].group(|ui| {
                    ui.heading("Noise layer");
                    egui::ComboBox::from_label("Noise type")
                        .selected_text(self.settings.noise_kind.label())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.settings.noise_kind, NoiseKind::Off, "Off");
                            ui.selectable_value(&mut self.settings.noise_kind, NoiseKind::White, "White noise");
                            ui.selectable_value(&mut self.settings.noise_kind, NoiseKind::Pink, "Pink noise");
                            ui.selectable_value(&mut self.settings.noise_kind, NoiseKind::SurfLike, "Synthetic surf-like noise");
                        });
                    ui.add(
                        egui::Slider::new(&mut self.settings.noise_level, 0.0..=0.10)
                            .text("Noise mix"),
                    );
                    ui.small("Noise begins after the tuning stage and fades in over two seconds. Surf-like noise is procedural, not a field recording.");
                });

                columns[1].group(|ui| {
                    ui.heading("Session and level");
                    ui.add(
                        egui::Slider::new(&mut self.settings.binaural_minutes, 1..=120)
                            .text("Binaural phase (minutes)"),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.settings.fade_seconds, 1..=60)
                            .text("Fade in / out (seconds)"),
                    );
                    ui.add(
                        egui::Slider::new(&mut self.settings.tone_level, 0.0..=0.20)
                            .text("Tone level (digital amplitude)"),
                    );
                    ui.small(format!("Total session: {}", Self::format_time(self.settings.total_seconds() as f64)));
                });
            });

            ui.add_space(8.0);
            ui.group(|ui| {
                ui.heading("Output device");
                if let Some(audio) = &self.audio {
                    ui.label(format!("{} · {} Hz · {} channel(s)", audio.device_name(), audio.sample_rate(), audio.channels()));
                    if audio.channels() < 2 {
                        ui.colored_label(
                            egui::Color32::YELLOW,
                            "Warning: the current device is mono. Select a stereo output device / headphones in your operating system.",
                        );
                    }
                } else {
                    ui.colored_label(egui::Color32::YELLOW, self.audio_error.as_deref().unwrap_or("No output device."));
                    ui.label("You can still export a WAV file for playback in another application.");
                }
            });

            ui.add_space(8.0);
            ui.group(|ui| {
                ui.heading("How this relates to the 1983 report");
                ui.label("The report describes: (1) a resonant humming/tuning exercise, (2) an affirmation, (3) stereo Hemi-Sync beat frequencies, and (4) progressive relaxation with pink/white noise. This app models the sound stages; it does not speak or record the affirmation.");
                ui.label("The report's numerical illustration is one channel 10 Hz below the other. It does not publish the original tape's exact carrier frequencies, detailed beat schedule, durations, or sound mix.");
                ui.label("This app generates fresh synthetic tones/noise. It does not contain Monroe Institute recordings, reproduce the proprietary Hemi-Sync tapes, or validate the report's claims about consciousness or brain entrainment.");
                ui.hyperlink_to(
                    "Open the declassified report (PDF)",
                    "https://upload.wikimedia.org/wikipedia/commons/d/dc/Analysis_and_Assessment_of_Gateway_Process.pdf",
                );
            });

            ui.add_space(8.0);
            ui.group(|ui| {
                ui.heading("Listening safety");
                ui.label("Begin at a low device volume and raise it only slightly if needed. Avoid listening while driving, cycling, or operating machinery. Stop if you experience discomfort, dizziness, anxiety, or a headache. This is an audio experiment, not a medical treatment or a substitute for professional care.");
            });
        });

        if let Some(audio) = &self.audio {
            audio.set_settings(self.settings);
        }

        if self.visualizer_enabled
            && let Some(audio) = &self.audio
        {
            let tap = audio.visualization_tap();
            let sample_rate = audio.sample_rate();
            let elapsed_seconds = audio.elapsed_seconds();
            let settings = self.settings.validated();
            let close_signal = Arc::clone(&self.visualizer_close_signal);
            let visualizer_mode = Arc::clone(&self.visualizer_mode);
            let viewport_id = egui::ViewportId::from_hash_of("gateway_harmonic_visualizer");
            let viewport_builder = egui::ViewportBuilder::default()
                .with_title("Gateway Harmonic Visualizer")
                .with_inner_size([1120.0, 850.0])
                .with_min_inner_size([780.0, 650.0]);

            ui.ctx().show_viewport_deferred(
                viewport_id,
                viewport_builder,
                move |visualizer_ui, _viewport_class| {
                    if visualizer_ui.input(|input| input.viewport().close_requested()) {
                        close_signal.store(true, Ordering::Release);
                        return;
                    }

                    visualizer_ui
                        .ctx()
                        .request_repaint_after(Duration::from_millis(66));

                    let frames = tap.snapshot();
                    let mut mode = visualization::VisualizationMode::from(
                        visualizer_mode.load(Ordering::Acquire),
                    );
                    visualization::show(
                        visualizer_ui,
                        &frames,
                        sample_rate,
                        settings,
                        elapsed_seconds,
                        &mut mode,
                    );
                    visualizer_mode.store(mode as u8, Ordering::Release);
                },
            );
        }
    }
}
