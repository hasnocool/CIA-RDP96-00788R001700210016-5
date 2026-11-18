// Filename: src/harmonic_3d.rs
// WGPU-backed procedural 3D harmonic surface embedded in egui.
// GPU resources are created once during app startup; each frame only updates
// a tiny uniform buffer. No CPU mesh generation or audio-thread work is used.

use std::{num::NonZeroU64, time::{Duration, Instant}};

use eframe::{
    CreationContext,
    egui::{self, Color32, Sense, Vec2},
    egui_wgpu::{self, wgpu},
};

const GRID_SIZE: u32 = 40;
const VERTEX_COUNT: u32 = 4 * GRID_SIZE * (GRID_SIZE - 1);


pub struct GpuHarmonicRenderer {
    rotation: [f32; 3],
    camera_distance: f32,
    camera_target: [f32; 3],
    auto_rotate: bool,
    animation_origin: Instant,
}

impl GpuHarmonicRenderer {
    pub fn new(creation_context: &CreationContext<'_>) -> Option<Self> {
        let render_state = creation_context.wgpu_render_state.as_ref()?;
        let device = &render_state.device;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gateway-harmonic-surface-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("harmonic_3d.wgsl").into()),
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("gateway-harmonic-uniform-layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(48),
                    },
                    count: None,
                }],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gateway-harmonic-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gateway-harmonic-wireframe-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(render_state.target_format.into())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let initial_uniforms = HarmonicUniforms {
            params: [0.0, 0.0, 0.0, 0.0, 10.0, 1.05, 0.0, 0.0, 0.0, 2.7, 1.55, 1.0],
        };
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gateway-harmonic-uniform-buffer"),
            size: 48,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        render_state
            .queue
            .write_buffer(&uniform_buffer, 0, &initial_uniforms.to_bytes());

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gateway-harmonic-bind-group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        render_state
            .renderer
            .write()
            .callback_resources
            .insert(HarmonicRenderResources {
                pipeline,
                bind_group,
                uniform_buffer,
            });

        Some(Self {
            rotation: [0.0, 0.0, 0.0],
            camera_distance: 2.7,
            camera_target: [0.0, 0.0, 0.0],
            auto_rotate: false,
            animation_origin: Instant::now(),
        })
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        carrier_hz: f32,
        beat_hz: f32,
        noise_level: f32,
        elapsed_seconds: f64,
    ) {
        ui.ctx().request_repaint_after(Duration::from_millis(16));
        ui.horizontal_wrapped(|ui| {
            ui.strong("GPU-generated standing-wave surface");
            ui.separator();
            ui.label(format!("Carrier {:.2} Hz · Δ {:.2} Hz", carrier_hz, beat_hz));
            if ui.button("Reset view").clicked() {
                self.rotation = [0.0, 0.0, 0.0];
                self.camera_distance = 2.7;
                self.camera_target = [0.0, 0.0, 0.0];
            }
            ui.checkbox(&mut self.auto_rotate, "Auto-rotate");
        });

        let width = ui.available_width().max(420.0);
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, 380.0), Sense::drag());

        if response.dragged() {
            let motion = response.drag_motion();
            let sensitivity = 0.006;
            if ui.input(|i| i.modifiers.shift) {
                self.rotation[2] += motion.x * sensitivity;
            } else {
                self.rotation[0] += motion.y * sensitivity;
                self.rotation[1] += motion.x * sensitivity;
            }
        }

        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.camera_distance = (self.camera_distance - scroll * 0.005).clamp(0.8, 10.0);
            }
        }

        ui.painter()
            .rect_filled(rect, 7.0, Color32::from_rgb(7, 12, 23));

        let auto_angle = if self.auto_rotate {
            (self.animation_origin.elapsed().as_secs_f32() + elapsed_seconds as f32) * 0.17
        } else {
            0.0
        };

        let uniforms = HarmonicUniforms {
            params: [
                self.rotation[0],
                self.rotation[1],
                self.rotation[2] + auto_angle,
                0.0,
                beat_hz.clamp(0.5, 30.0),
                (1.0 + beat_hz / carrier_hz.max(1.0)).clamp(1.0, 1.5),
                noise_level.clamp(0.0, 0.10),
                0.0,
                self.camera_target[0],
                self.camera_target[1],
                self.camera_distance,
                1.0,
            ],
        };

        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            rect,
            HarmonicCallback { uniforms },
        ));
        ui.small(
            "Drag: rotate X/Y. Shift+drag: rotate Z. Scroll: zoom. Auto-rotate: continuous Y rotation.",
        );
    }
}

#[derive(Clone, Copy)]
struct HarmonicUniforms {
    params: [f32; 12],
}

impl HarmonicUniforms {
    fn to_bytes(self) -> [u8; 48] {
        let mut bytes = [0_u8; 48];
        for (index, value) in self.params.iter().enumerate() {
            bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }
}

struct HarmonicRenderResources {
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
}

impl HarmonicRenderResources {
    fn prepare(&self, queue: &wgpu::Queue, uniforms: HarmonicUniforms) {
        queue.write_buffer(&self.uniform_buffer, 0, &uniforms.to_bytes());
    }

    fn paint(&self, render_pass: &mut wgpu::RenderPass<'_>) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.bind_group, &[]);
        render_pass.draw(0..VERTEX_COUNT * 2, 0..1);
    }
}

struct HarmonicCallback {
    uniforms: HarmonicUniforms,
}

impl egui_wgpu::CallbackTrait for HarmonicCallback {
    fn prepare(
        &self,
        _device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &egui_wgpu::ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(resources) = callback_resources.get_mut::<HarmonicRenderResources>() {
            resources.prepare(queue, self.uniforms);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(resources) = callback_resources.get::<HarmonicRenderResources>() {
            resources.paint(render_pass);
        }
    }
}
