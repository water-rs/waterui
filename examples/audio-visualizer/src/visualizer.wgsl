// Audio Visualizer Shader - Practical/Functional Design
// Supports 4 modes: Waveform, Spectrum, Spectrogram, Phase
// Supports dynamic theming via uniforms

const SAMPLES_COUNT: u32 = 1024u;
const FREQ_BINS: u32 = 512u;
const PI: f32 = 3.14159265359;

struct Uniforms {
    resolution: vec2<f32>,
    time: f32,
    mode: u32,
    // Theme Bundle
    bg_color: vec3<f32>,
    _pad0: f32,
    primary_color: vec3<f32>,
    _pad1: f32,
    secondary_color: vec3<f32>,
    _pad2: f32,
    grid_color: vec3<f32>,
    _pad3: f32,
    line_width: f32,
    grid_opacity: f32,
    fill_opacity: f32,
    mirror_y: f32,
    render_style: f32,
    sensitivity: f32,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var<storage, read> audio_samples: array<f32, 1024>;
@group(0) @binding(2) var<storage, read_write> frequency_data: array<f32, 512>;

// ==================== Compute Shader: DFT ====================
@compute @workgroup_size(64)
fn dft_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let freq_idx = id.x;
    if (freq_idx >= FREQ_BINS) { return; }

    var real: f32 = 0.0;
    var imag: f32 = 0.0;

    for (var n: u32 = 0u; n < SAMPLES_COUNT; n = n + 1u) {
        let sample = audio_samples[n];
        // Hann window
        let window = 0.5 - 0.5 * cos(2.0 * PI * f32(n) / f32(SAMPLES_COUNT - 1u));
        let windowed = sample * window;
        
        let angle = 2.0 * PI * f32(freq_idx) * f32(n) / f32(SAMPLES_COUNT);
        real += windowed * cos(angle);
        imag -= windowed * sin(angle);
    }

    let magnitude = sqrt(real * real + imag * imag) / f32(SAMPLES_COUNT);
    // Log scale for better visibility
    let db = 20.0 * log(max(magnitude, 0.0001)) / log(10.0);
    let normalized = clamp((db + 60.0) / 60.0, 0.0, 1.0);
    
    frequency_data[freq_idx] = normalized;
}

// ==================== Vertex Shader ====================
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0)
    );
    var out: VertexOutput;
    out.position = vec4<f32>(positions[idx], 0.0, 1.0);
    out.uv = positions[idx] * 0.5 + 0.5;
    return out;
}

// ==================== Fragment Shader ====================
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let mode = uniforms.mode;

    // Background from theme
    let bg = uniforms.bg_color;
    var color = bg;

    if (mode == 0u) {
        // === WAVEFORM ===
        color = draw_waveform(uv, bg);
    } else if (mode == 1u) {
        // === SPECTRUM BARS ===
        color = draw_spectrum(uv, bg);
    } else if (mode == 2u) {
        // === SPECTROGRAM (simplified) ===
        color = draw_spectrogram(uv, bg);
    } else {
        // === PHASE SCOPE ===
        color = draw_phase(uv, bg);
    }

    return vec4<f32>(color, 1.0);
}

// --- Waveform: Time-domain oscilloscope ---
fn get_interpolated_sample(t: f32) -> f32 {
    let idx_f = t * f32(SAMPLES_COUNT - 1u);
    let i = u32(idx_f);
    let f = fract(idx_f);
    let s0 = audio_samples[i];
    let s1 = audio_samples[min(i + 1u, SAMPLES_COUNT - 1u)];
    return mix(s0, s1, f);
}

// --- Waveform: Time-domain oscilloscope ---
fn draw_waveform(uv: vec2<f32>, bg: vec3<f32>) -> vec3<f32> {
    var color = bg;
    let grid_color = uniforms.grid_color;
    let grid_alpha = uniforms.grid_opacity;
    let is_bar = uniforms.render_style > 0.5;
    let is_mirrored = uniforms.mirror_y > 0.5;
    
    // Scale amplitude
    let scale = 0.4 * uniforms.sensitivity;

    // Grid lines
    if (abs(uv.y - 0.5) < 0.002) {
        color = mix(color, grid_color, grid_alpha); 
    }
    if (!is_mirrored) {
        if (abs(uv.y - 0.25) < 0.001 || abs(uv.y - 0.75) < 0.001) {
            color = mix(color, grid_color, grid_alpha * 0.7); 
        }
    }

    if (is_bar) {
        // Bar Style (Voice Memo)
        // Use nearest neighbor for bars to keep them crisp, or interpolate for smooth envelope?
        // Voice memos usually use discrete bars.
        let num_bars = 100.0;
        let bar_width = 1.0 / num_bars;
        let bar_center_x = (floor(uv.x * num_bars) + 0.5) / num_bars;
        
        let raw_sample = get_interpolated_sample(bar_center_x); 
        let sample_mag = abs(raw_sample);
        var height = sample_mag * scale;
        // Ensure minimal height for silence
        height = max(height, 0.004);

        let center_y = 0.5;
        var in_bar = false;
        
        // Anti-aliased width
        let dist_x = abs(uv.x - bar_center_x);
        let bar_mask_x = smoothstep(bar_width * 0.45, bar_width * 0.4, dist_x);

        if (is_mirrored) {
            let dist_y = abs(uv.y - center_y);
            in_bar = dist_y < height;
        } else {
             if (raw_sample >= 0.0) {
                in_bar = uv.y >= center_y && uv.y <= center_y + height;
            } else {
                in_bar = uv.y <= center_y && uv.y >= center_y - height;
            }
        }
        
        if (in_bar) {
             let alpha = bar_mask_x * uniforms.fill_opacity;
             color = mix(color, uniforms.primary_color, alpha);
        }

    } else {
        // Line Style (Oscilloscope) with GLOW
        let sample = get_interpolated_sample(uv.x);
        let y_pos = 0.5 + sample * scale;
        
        let dist = abs(uv.y - y_pos);
        let line_width = max(uniforms.line_width, 1.0) / uniforms.resolution.y;

        // Core Line
        let core_intensity = smoothstep(line_width, line_width * 0.5, dist);
        
        // Glow (Cyber effect)
        let glow_width = line_width * 4.0;
        let glow_intensity = exp(-dist / glow_width) * 0.6; // Exponential falloff
        
        // Additive blendingish behavior for glow
        let final_alpha = max(core_intensity, glow_intensity) * uniforms.fill_opacity;
        if (final_alpha > 0.01) {
            color = mix(color, uniforms.primary_color + uniforms.secondary_color * 0.5, final_alpha);
        }
    }
    
    return color;
}

// --- Spectrum: Frequency bars ---
fn draw_spectrum(uv: vec2<f32>, bg: vec3<f32>) -> vec3<f32> {
    let num_bars: u32 = 64u;
    let total_bars = f32(num_bars);
    let bar_idx = u32(uv.x * total_bars);
    
    // Average frequency bins for this bar
    let bins_per_bar = FREQ_BINS / num_bars;
    var sum: f32 = 0.0;
    // Simple average
    let start_bin = bar_idx * bins_per_bar;
    for (var i: u32 = 0u; i < bins_per_bar; i = i + 1u) {
        sum += frequency_data[start_bin + i];
    }
    let height = sum / f32(bins_per_bar);
    
    var color = bg;
    
    // Bar
    let bar_center_x = (f32(bar_idx) + 0.5) / total_bars;
    let bar_width = 1.0 / total_bars;
    
    // X-axis smoothing
    let dist_x = abs(uv.x - bar_center_x);
    let alpha_x = smoothstep(bar_width * 0.45, bar_width * 0.4, dist_x);
    
    if (uv.y < height && alpha_x > 0.0) {
        let t = uv.y / height; // 0 at bottom, 1 at top
        // Gradient from primary to secondary color
        let bar_color = mix(uniforms.primary_color, uniforms.secondary_color, t);
        color = mix(bg, bar_color, uniforms.fill_opacity * alpha_x);
    }
    
    // Grid
    let grid_color = uniforms.grid_color;
    let grid_alpha = uniforms.grid_opacity;
    if (abs(uv.y - 0.25) < 0.001 || abs(uv.y - 0.5) < 0.001 || abs(uv.y - 0.75) < 0.001) {
        color = mix(color, grid_color, grid_alpha * 0.7);
    }
    
    return color;
}

// --- Spectrogram: Frequency over time (simplified single-column) ---
fn draw_spectrogram(uv: vec2<f32>, bg: vec3<f32>) -> vec3<f32> {
    // Y = frequency, X = time (but we only have current frame, so show vertical spectrum)
    let freq_idx = u32(uv.y * f32(FREQ_BINS - 1u));
    let intensity = frequency_data[freq_idx];
    
    // Heat map based on primary/secondary colors
    // Low intensity = bg, Mid = Primary, High = Secondary
    var color = bg;
    if (intensity < 0.5) {
        color = mix(bg, uniforms.primary_color, intensity * 2.0);
    } else {
        color = mix(uniforms.primary_color, uniforms.secondary_color, (intensity - 0.5) * 2.0);
    }
    
    return color;
}

// --- Phase: Lissajous / Stereo correlation ---
fn draw_phase(uv: vec2<f32>, bg: vec3<f32>) -> vec3<f32> {
    var color = bg;
    let grid_color = uniforms.grid_color;
    let grid_alpha = uniforms.grid_opacity;
    
    // Draw circle outline
    let center = vec2<f32>(0.5, 0.5);
    let radius = 0.4;
    let dist_to_center = length(uv - center);

    if (abs(dist_to_center - radius) < 0.003) {
        color = mix(color, grid_color, grid_alpha);
    }
    if (abs(dist_to_center - radius * 0.5) < 0.002) {
        color = mix(color, grid_color, grid_alpha * 0.7);
    }
    
    // Cross lines
    if (abs(uv.x - 0.5) < 0.001 || abs(uv.y - 0.5) < 0.001) {
        color = mix(color, grid_color, grid_alpha);
    }
    
    // Plot samples as points (treating pairs as L/R)
    for (var i: u32 = 0u; i < SAMPLES_COUNT; i = i + 2u) {
        let left = audio_samples[i];
        let right = audio_samples[i + 1u];
        let px = 0.5 + left * radius;
        let py = 0.5 + right * radius;
        
        let d = length(uv - vec2<f32>(px, py));
        if (d < 0.003) {
            color = uniforms.primary_color;
        }
    }
    
    return color;
}
