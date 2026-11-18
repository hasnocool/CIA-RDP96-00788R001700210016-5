// Filename: src/harmonic_3d.rs
// WGPU-backed procedural 3D harmonic surface embedded in egui.
// GPU resources are created once during app startup; each frame only updates
// a tiny uniform buffer. No CPU mesh generation or audio-thread work is used.

use std::num::NonZeroU64;

use eframe::{
    CreationContext,
    egui::{self, Color32, Sense, Vec2},
    egui_wgpu::{self, wgpu},
};

const GRID_SIZE: u32 = 40;
const VERTEX_COUNT: u32 = 4 * GRID_SIZE * (GRID_SIZE - 1);

#[repr(C)]
#[repr(C)]
#[derive(Clone, Copy)]
struct HarmonicUniforms {
    params: [f32; 4],
}

impl HarmonicUniforms {
    fn to_bytes(self) -> [u8; 16] {
        let mut bytes = [0_u8; 16];
        for (index, value) in self.params.iter().enumerate() {
            bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_ne_bytes());
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
        render_pass.draw(0..VERTEX_COUNT, 0..1);
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
        _encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(resources) = callback_resources.get::<HarmonicRenderResources>() {
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

pub struct GpuHarmonicRenderer {
    user_rotation: f32,
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
                        min_binding_size: NonZeroU64::new(16),
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
            params: [0.0, 10.0, 1.05, 0.0],
        };
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gateway-harmonic-uniform-buffer"),
            size: 16,
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

        // The pipeline must be stored in the renderer's callback resource map
        // so the callback can borrow it for the lifetime of egui's render pass.
        render_state
            .renderer
            .write()
            .callback_resources
            .insert(HarmonicRenderResources {
                pipeline,
                bind_group,
                uniform_buffer,
            });

        Some(Self { user_rotation: 0.0 })
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        carrier_hz: f32,
        beat_hz: f32,
        noise_level: f32,
        elapsed_seconds: f64,
    ) {
        ui.horizontal_wrapped(|ui| {
            ui.strong("GPU-generated standing-wave surface");
            ui.separator();
            ui.label(format!("Carrier {:.2} Hz · Δ {:.2} Hz", carrier_hz, beat_hz));
            if ui.button("Reset rotation").clicked() {
                self.user_rotation = 0.0;
            }
        });

        let width = ui.available_width().max(420.0);
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, 285.0), Sense::drag());
        if response.dragged() {
            self.user_rotation += response.drag_motion().x * 0.012;
        }

        ui.painter()
            .rect_filled(rect, 7.0, Color32::from_rgb(7, 12, 23));

        let uniforms = HarmonicUniforms {
            params: [
                self.user_rotation + elapsed_seconds as f32 * 0.17,
                beat_hz.clamp(0.5, 30.0),
                (1.0 + beat_hz / carrier_hz.max(1.0)).clamp(1.0, 1.5),
                noise_level.clamp(0.0, 0.10),
            ],
        };

        ui.painter().add(egui_wgpu::Callback::new_paint_callback(
            rect,
            HarmonicCallback { uniforms },
        ));
        ui.small("Drag horizontally to rotate. The GPU draws a procedural 3D wireframe model; its beat-dependent surface is a mathematical visualization, not a measured plate response.");
    }
}
