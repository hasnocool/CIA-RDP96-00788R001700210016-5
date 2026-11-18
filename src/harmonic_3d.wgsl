// Filename: src/harmonic_3d.wgsl
// Parametric 3D surface sampled entirely in the vertex shader.

struct HarmonicUniforms {
    params: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> uniforms: HarmonicUniforms;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

const GRID_SIZE: u32 = 40u;
const HORIZONTAL_EDGES: u32 = GRID_SIZE * (GRID_SIZE - 1u);

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let line_index = vertex_index / 2u;
    let endpoint = vertex_index % 2u;

    var grid_x: u32;
    var grid_y: u32;

    if (line_index < HORIZONTAL_EDGES) {
        let row = line_index / (GRID_SIZE - 1u);
        let column = line_index % (GRID_SIZE - 1u);
        grid_x = column + endpoint;
        grid_y = row;
    } else {
        let vertical_index = line_index - HORIZONTAL_EDGES;
        let row = vertical_index / GRID_SIZE;
        let column = vertical_index % GRID_SIZE;
        grid_x = column;
        grid_y = row + endpoint;
    }

    let u = f32(grid_x) / f32(GRID_SIZE - 1u);
    let v = f32(grid_y) / f32(GRID_SIZE - 1u);

    let angle = uniforms.params.x;
    let beat_hz = clamp(uniforms.params.y, 0.5, 30.0);
    let carrier_ratio = clamp(uniforms.params.z, 1.0, 1.5);
    let noise_level = clamp(uniforms.params.w, 0.0, 0.10);

    let mode_x = 2.0 + floor(beat_hz / 5.0);
    let mode_y = 2.0 + floor(beat_hz / 7.0);
    let plate_mode = sin(3.14159265 * mode_x * u + angle * 0.35)
                   * sin(3.14159265 * mode_y * v - angle * 0.28);

    let driven_x = 1.0 + beat_hz / 5.0;
    let driven_y = 1.0 + beat_hz / 7.0;
    let interference = 0.20
        * sin(6.2831853 * driven_x * u + angle)
        * cos(6.2831853 * driven_y * v - angle * carrier_ratio);

    let procedural_noise = sin(u * 73.1 + v * 43.7 + angle)
                         * cos(v * 59.3 - u * 27.9);
    let height = 0.32 * plate_mode + interference
               + noise_level * 0.35 * procedural_noise;

    let x = (u - 0.5) * 1.9;
    let y = (v - 0.5) * 1.55;
    let z = height;

    let cosine_y = cos(angle * 0.65);
    let sine_y = sin(angle * 0.65);
    let x_rotated = x * cosine_y + z * sine_y;
    let z_rotated = -x * sine_y + z * cosine_y;

    let camera_tilt = 0.78;
    let y_projected = y * cos(camera_tilt) - z_rotated * sin(camera_tilt);
    let z_projected = y * sin(camera_tilt) + z_rotated * cos(camera_tilt);
    let perspective = 1.55 / (2.7 + z_projected);

    var output: VertexOutput;
    output.clip_position = vec4<f32>(
        x_rotated * perspective,
        y_projected * perspective * 1.05,
        0.0,
        1.0,
    );

    let intensity = clamp(height * 1.4 + 0.5, 0.0, 1.0);
    output.color = vec4<f32>(
        0.18 + intensity * 0.35,
        0.42 + (1.0 - intensity) * 0.28,
        0.72 + intensity * 0.24,
        1.0,
    );
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return input.color;
}
