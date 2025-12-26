// Audio Visualizer Shader - Practical/Functional Design
// Supports 4 modes: Waveform, Spectrum, Spectrogram, Phase

const SAMPLES_COUNT: u32 = 1024u;
const FREQ_BINS: u32 = 512u;
const PI: f32 = 3.14159265359;

struct Uniforms {
    resolution: vec2<f32>,
    time: f32,
    mode: u32,  // 0=Waveform, 1=Spectrum, 2=Spectrogram, 3=Phase
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

    // Background
    let bg = vec3<f32>(0.08, 0.08, 0.1);
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
fn draw_waveform(uv: vec2<f32>, bg: vec3<f32>) -> vec3<f32> {
    let x_idx = u32(uv.x * f32(SAMPLES_COUNT - 1u));
    let sample = audio_samples[x_idx];
    
    // Map sample [-1, 1] to screen [0.1, 0.9]
    let y_pos = 0.5 + sample * 0.4;
    let dist = abs(uv.y - y_pos);
    
    // Grid lines
    var color = bg;
    if (abs(uv.y - 0.5) < 0.002) {
        color = vec3<f32>(0.2, 0.2, 0.25); // Center line
    }
    if (abs(uv.y - 0.25) < 0.001 || abs(uv.y - 0.75) < 0.001) {
        color = vec3<f32>(0.15, 0.15, 0.18); // ±50% lines
    }
    
    // Waveform line
    let line_width = 2.5 / uniforms.resolution.y;
    if (dist < line_width) {
        let intensity = 1.0 - dist / line_width;
        color = mix(color, vec3<f32>(0.2, 0.8, 0.4), intensity);
    }
    
    return color;
}

// --- Spectrum: Frequency bars ---
fn draw_spectrum(uv: vec2<f32>, bg: vec3<f32>) -> vec3<f32> {
    let num_bars: u32 = 64u;
    let bar_idx = u32(uv.x * f32(num_bars));
    
    // Average frequency bins for this bar
    let bins_per_bar = FREQ_BINS / num_bars;
    var sum: f32 = 0.0;
    for (var i: u32 = 0u; i < bins_per_bar; i = i + 1u) {
        sum += frequency_data[bar_idx * bins_per_bar + i];
    }
    let height = sum / f32(bins_per_bar);
    
    var color = bg;
    
    // Bar
    let bar_width = 0.8 / f32(num_bars);
    let bar_x = (f32(bar_idx) + 0.1) / f32(num_bars);
    let bar_end_x = bar_x + bar_width;
    
    if (uv.x > bar_x && uv.x < bar_end_x && uv.y < height) {
        // Color gradient: green -> yellow -> red
        let hue = (1.0 - uv.y) * 0.33; // 0.33 = green, 0 = red
        color = hsv_to_rgb(vec3<f32>(hue, 0.9, 0.9));
    }
    
    // Grid
    if (abs(uv.y - 0.25) < 0.001 || abs(uv.y - 0.5) < 0.001 || abs(uv.y - 0.75) < 0.001) {
        color = max(color, vec3<f32>(0.15, 0.15, 0.18));
    }
    
    return color;
}

// --- Spectrogram: Frequency over time (simplified single-column) ---
fn draw_spectrogram(uv: vec2<f32>, bg: vec3<f32>) -> vec3<f32> {
    // Y = frequency, X = time (but we only have current frame, so show vertical spectrum)
    let freq_idx = u32(uv.y * f32(FREQ_BINS - 1u));
    let intensity = frequency_data[freq_idx];
    
    // Heat map: black -> blue -> cyan -> green -> yellow -> red -> white
    let color = heat_map(intensity);
    
    return color;
}

// --- Phase: Lissajous / Stereo correlation ---
fn draw_phase(uv: vec2<f32>, bg: vec3<f32>) -> vec3<f32> {
    var color = bg;
    
    // Draw circle outline
    let center = vec2<f32>(0.5, 0.5);
    let radius = 0.4;
    let dist_to_center = length(uv - center);
    if (abs(dist_to_center - radius) < 0.003) {
        color = vec3<f32>(0.2, 0.2, 0.25);
    }
    if (abs(dist_to_center - radius * 0.5) < 0.002) {
        color = vec3<f32>(0.15, 0.15, 0.18);
    }
    
    // Cross lines
    if (abs(uv.x - 0.5) < 0.001 || abs(uv.y - 0.5) < 0.001) {
        color = vec3<f32>(0.2, 0.2, 0.25);
    }
    
    // Plot samples as points (treating pairs as L/R)
    for (var i: u32 = 0u; i < SAMPLES_COUNT; i = i + 2u) {
        let left = audio_samples[i];
        let right = audio_samples[i + 1u];
        let px = 0.5 + left * radius;
        let py = 0.5 + right * radius;
        
        let d = length(uv - vec2<f32>(px, py));
        if (d < 0.003) {
            color = vec3<f32>(0.2, 0.9, 0.5);
        }
    }
    
    return color;
}

// --- Helpers ---
fn hsv_to_rgb(hsv: vec3<f32>) -> vec3<f32> {
    let h = hsv.x * 6.0;
    let s = hsv.y;
    let v = hsv.z;
    let i = floor(h);
    let f = h - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    
    if (i < 1.0) { return vec3<f32>(v, t, p); }
    if (i < 2.0) { return vec3<f32>(q, v, p); }
    if (i < 3.0) { return vec3<f32>(p, v, t); }
    if (i < 4.0) { return vec3<f32>(p, q, v); }
    if (i < 5.0) { return vec3<f32>(t, p, v); }
    return vec3<f32>(v, p, q);
}

fn heat_map(t: f32) -> vec3<f32> {
    // Black -> Blue -> Cyan -> Green -> Yellow -> Red
    if (t < 0.2) { return mix(vec3<f32>(0.0), vec3<f32>(0.0, 0.0, 0.5), t / 0.2); }
    if (t < 0.4) { return mix(vec3<f32>(0.0, 0.0, 0.5), vec3<f32>(0.0, 0.5, 0.5), (t - 0.2) / 0.2); }
    if (t < 0.6) { return mix(vec3<f32>(0.0, 0.5, 0.5), vec3<f32>(0.0, 0.8, 0.0), (t - 0.4) / 0.2); }
    if (t < 0.8) { return mix(vec3<f32>(0.0, 0.8, 0.0), vec3<f32>(1.0, 1.0, 0.0), (t - 0.6) / 0.2); }
    return mix(vec3<f32>(1.0, 1.0, 0.0), vec3<f32>(1.0, 0.2, 0.0), (t - 0.8) / 0.2);
}
