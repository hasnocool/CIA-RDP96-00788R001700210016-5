// Parametric 3D surface sampled entirely in the vertex shader.

struct HarmonicUniforms {
    params: vec4<f32>,
    audio_params: vec4<f32>,
    camera: vec4<f32>,
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

    let rot_x = uniforms.params.x;
    let rot_y = uniforms.params.y;
    let rot_z = uniforms.params.z;

    let beat_hz = clamp(uniforms.audio_params.x, 0.5, 30.0);
    let carrier_ratio = clamp(uniforms.audio_params.y, 1.0, 1.5);
    let noise_level = clamp(uniforms.audio_params.z, 0.0, 0.10);

    let mode_x = 2.0 + floor(beat_hz / 5.0);
    let mode_y = 2.0 + floor(beat_hz / 7.0);
    let plate_mode = sin(3.14159265 * mode_x * u + rot_y * 0.35)
                   * sin(3.14159265 * mode_y * v - rot_y * 0.28);

    let driven_x = 1.0 + beat_hz / 5.0;
    let driven_y = 1.0 + beat_hz / 7.0;
    let interference = 0.20
        * sin(6.2831853 * driven_x * u + rot_y)
        * cos(6.2831853 * driven_y * v - rot_y * carrier_ratio);

    let procedural_noise = sin(u * 73.1 + v * 43.7 + rot_y)
                         * cos(v * 59.3 - u * 27.9);
    let height = 0.32 * plate_mode + interference
               + noise_level * 0.35 * procedural_noise;

    let x = (u - 0.5) * 1.9;
    let y = (v - 0.5) * 1.55;
    let z = height;

    // Apply 3D rotation
    let cx = cos(rot_x);
    let sx = sin(rot_x);
    let cy = cos(rot_y);
    let sy = sin(rot_y);
    let cz = cos(rot_z);
    let sz = sin(rot_z);

    // Y-axis rotation
    var x_rot = x * cy + z * sy;
    var z_rot = -x * sy + z * cy;
    var y_rot = y;

    // X-axis rotation
    let y_rot2 = y_rot * cx - z_rot * sx;
    z_rot = y_rot * sx + z_rot * cx;
    y_rot = y_rot2;

    // Z-axis rotation
    let x_rot2 = x_rot * cz - y_rot * sz;
    y_rot = x_rot * sz + y_rot * cz;
    x_rot = x_rot2;

    // Camera transform - translate camera along -Z (view direction)
    let cam_distance = uniforms.camera.x;
    let cam_target_x = uniforms.camera.y;
    let cam_target_y = uniforms.camera.z;
    let perspective_scale = uniforms.camera.w;

    x_rot -= cam_target_x;
    y_rot -= cam_target_y;

    // Move camera back along -Z axis (camera at +Z looking at origin)
    let z_camera = z_rot - cam_distance;

    let perspective = perspective_scale / max(z_camera, 0.1);

    var output: VertexOutput;
    output.clip_position = vec4<f32>(
        x_rot * perspective,
        y_rot * perspective,
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
