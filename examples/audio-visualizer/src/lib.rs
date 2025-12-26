use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use waterui::app::App;
use waterui::component::slider::slider;
use waterui::graphics::gpu_surface::{GpuContext, GpuFrame, GpuRenderer, GpuSurface};
use waterui::prelude::*;
use waterui::prelude::stack::Alignment;
use waterui::reactive::{Binding, binding};

use waterkit_audio::AudioRecorderBuilder;

const SAMPLES_COUNT: usize = 1024;
const BUFFER_SIZE: usize = SAMPLES_COUNT * 4;

/// Visualization mode
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum VisualizerMode {
    Waveform = 0,
    Spectrum = 1,
    Spectrogram = 2,
    Phase = 3,
}

impl VisualizerMode {
    fn name(&self) -> &'static str {
        match self {
            Self::Waveform => "Waveform",
            Self::Spectrum => "Spectrum",
            Self::Spectrogram => "Spectrogram",
            Self::Phase => "Phase",
        }
    }
}

/// Visualizer Theme Properties
#[derive(Clone, Copy, Debug)]
pub struct VisualizerTheme {
    pub bg_color: [f32; 3],
    pub primary_color: [f32; 3],
    pub secondary_color: [f32; 3],
    pub grid_color: [f32; 3],
    pub line_width: f32,
    pub grid_opacity: f32,
    pub fill_opacity: f32,
    pub mirror_y: bool,
    pub render_style: u32, // 0 = Line, 1 = Bar
    pub sensitivity: f32,  // Amplitude multiplier (0.5 to 5.0)
    pub _pad: f32,
}

impl VisualizerTheme {
    pub fn cyber() -> Self {
        Self {
            bg_color: [0.05, 0.05, 0.1],      // Dark sci-fi blue
            primary_color: [0.0, 1.0, 0.8],   // Cyan/Teal
            secondary_color: [0.8, 0.0, 1.0], // Purple/Magenta
            grid_color: [0.2, 0.3, 0.5],      // Blue-ish grid
            line_width: 2.0,
            grid_opacity: 0.5,
            fill_opacity: 0.8,
            mirror_y: false,
            render_style: 0,
            sensitivity: 1.0,
            _pad: 0.0,
        }
    }

    pub fn recorder() -> Self {
        Self {
            bg_color: [0.05, 0.05, 0.05],     // Almost black
            primary_color: [1.0, 0.2, 0.2],   // Red
            secondary_color: [0.9, 0.9, 0.9], // White
            grid_color: [0.3, 0.3, 0.3],      // Subtle Gray
            line_width: 3.0,
            grid_opacity: 0.2,
            fill_opacity: 0.9,
            mirror_y: true,
            render_style: 1, // Bar
            sensitivity: 2.0, // Recorder style is usually more sensitive
            _pad: 0.0,
        }
    }
}

#[derive(Clone)]
struct AudioState {
    samples: Arc<Mutex<Vec<f32>>>,
    mode: Arc<AtomicU32>,
}

impl AudioState {
    fn new() -> Self {
        Self {
            samples: Arc::new(Mutex::new(vec![0.0; SAMPLES_COUNT])),
            mode: Arc::new(AtomicU32::new(VisualizerMode::Waveform as u32)),
        }
    }

    fn set_mode(&self, mode: VisualizerMode) {
        self.mode.store(mode as u32, Ordering::Relaxed);
    }

    fn get_mode(&self) -> u32 {
        self.mode.load(Ordering::Relaxed)
    }
}

struct VisualizerRenderer {
    state: AudioState,
    theme: Binding<VisualizerTheme>,
    sensitivity: Binding<f64>,
    render_pipeline: Option<wgpu::RenderPipeline>,
    compute_pipeline: Option<wgpu::ComputePipeline>,
    bind_group: Option<wgpu::BindGroup>,
    uniform_buffer: Option<wgpu::Buffer>,
    audio_buffer: Option<wgpu::Buffer>,
    frequency_buffer: Option<wgpu::Buffer>,
    start_time: Instant,
}

impl VisualizerRenderer {
    fn new(state: AudioState, theme: Binding<VisualizerTheme>, sensitivity: Binding<f64>) -> Self {
        Self {
            state,
            theme,
            sensitivity,
            render_pipeline: None,
            compute_pipeline: None,
            bind_group: None,
            uniform_buffer: None,
            audio_buffer: None,
            frequency_buffer: None,
            start_time: Instant::now(),
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct ShaderUniforms {
    resolution: [f32; 2],
    time: f32,
    mode: u32,
    // Theme Bundle
    bg_color: [f32; 3],       // 16
    _pad0: f32,
    primary_color: [f32; 3],  // 32
    _pad1: f32,
    secondary_color: [f32; 3],// 48
    _pad2: f32,
    grid_color: [f32; 3],     // 64-76
    _pad3: f32,               // 76-80 (Crucial padding!)
    line_width: f32,          // 80
    grid_opacity: f32,        // 84
    fill_opacity: f32,        // 88
    mirror_y: f32,            // 92
    render_style: f32,        // 96
    sensitivity: f32,         // 100
    // Explicit padding to reach 112 bytes (WGSL struct rounds to 16-byte alignment)
    _pad_end0: f32,           // 104
    _pad_end1: f32,           // 108 -> total 112
}

impl GpuRenderer for VisualizerRenderer {
    fn setup(&mut self, ctx: &GpuContext) -> impl std::future::Future<Output = ()> {
        let device = ctx.device;

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Uniforms"),
            size: std::mem::size_of::<ShaderUniforms>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let audio_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Audio Samples"),
            size: BUFFER_SIZE as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let frequency_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Frequency Data"),
            size: (BUFFER_SIZE / 2) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let shader_src = include_str!("visualizer.wgsl");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Visualizer Shader"),
            source: wgpu::ShaderSource::Wgsl(shader_src.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Main Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::all(),
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Main Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: audio_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: frequency_buffer.as_entire_binding() },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("DFT Compute Pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("dft_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Visualizer Render Pipeline"),
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
                targets: &[Some(wgpu::ColorTargetState {
                    format: ctx.surface_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        self.render_pipeline = Some(render_pipeline);
        self.compute_pipeline = Some(compute_pipeline);
        self.bind_group = Some(bind_group);
        self.uniform_buffer = Some(uniform_buffer);
        self.audio_buffer = Some(audio_buffer);
        self.frequency_buffer = Some(frequency_buffer);

        async {}
    }

    fn render(&mut self, frame: &GpuFrame) {
        let (Some(render_pl), Some(compute_pl), Some(bg), Some(ub), Some(ab)) = (
            &self.render_pipeline,
            &self.compute_pipeline,
            &self.bind_group,
            &self.uniform_buffer,
            &self.audio_buffer,
        ) else { return };

        let audio_data = {
            let lock = self.state.samples.lock().unwrap();
            lock.clone()
        };
        frame.queue.write_buffer(ab, 0, bytemuck::cast_slice(&audio_data));

        // Read reactive state
        let time = self.start_time.elapsed().as_secs_f32();
        let theme = self.theme.get();
        let current_sensitivity = self.sensitivity.get();

        let uniforms = ShaderUniforms {
            resolution: [frame.width as f32, frame.height as f32],
            time,
            mode: self.state.get_mode(),
            bg_color: theme.bg_color,
            _pad0: 0.0,
            primary_color: theme.primary_color,
            _pad1: 0.0,
            secondary_color: theme.secondary_color,
            _pad2: 0.0,
            grid_color: theme.grid_color,
            _pad3: 0.0,
            line_width: theme.line_width,
            grid_opacity: theme.grid_opacity,
            fill_opacity: theme.fill_opacity,
            mirror_y: if theme.mirror_y { 1.0 } else { 0.0 },
            render_style: theme.render_style as f32,
            sensitivity: current_sensitivity as f32,
            _pad_end0: 0.0,
            _pad_end1: 0.0,
        };
        frame.queue.write_buffer(ub, 0, bytemuck::bytes_of(&uniforms));

        let mut encoder = frame.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Visualizer Encoder"),
        });

        // Run compute for spectrum/spectrogram modes
        if uniforms.mode == 1 || uniforms.mode == 2 {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("DFT Pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(compute_pl);
            cpass.set_bind_group(0, bg, &[]);
            cpass.dispatch_workgroups(8, 1, 1);
        }

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Visualizer Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rpass.set_pipeline(render_pl);
            rpass.set_bind_group(0, bg, &[]);
            rpass.draw(0..3, 0..1);
        }

        frame.queue.submit(Some(encoder.finish()));
    }
}

// Audio System using waterkit-audio mic input
struct AudioSystem {
    state: AudioState,
}

static AUDIO_SYSTEM: std::sync::OnceLock<AudioSystem> = std::sync::OnceLock::new();

fn init_audio() -> AudioState {
    if let Some(sys) = AUDIO_SYSTEM.get() {
        return sys.state.clone();
    }

    let state = AudioState::new();
    let state_clone = state.clone();

    // Spawn microphone capture thread (dedicated thread for robustness)
    std::thread::spawn(move || {
        // recorder disabled for debugging
        /*
        let mut recorder = match AudioRecorderBuilder::new().build() {
            Ok(r) => r,
            Err(e) => {
                waterui::log::error!("Failed to create AudioRecorder: {:?}", e);
                return;
            }
        };
        // Use pollster to run async start in this thread
        if let Err(e) = pollster::block_on(recorder.start()) {
            waterui::log::error!("Failed to start recording: {:?}", e);
            return;
        }
        */
        
        waterui::log::info!("Fake Audio Loop Running (Blue Sine Wave)");

        let mut t = 0.0f32;
        loop {
            // DEBUG: Generate fake sine wave
            t += 0.05;
            if let Ok(mut lock) = state_clone.samples.lock() {
                for i in 0..SAMPLES_COUNT {
                    let val = (t * 5.0 + (i as f32) * 0.1).sin() * 0.5;
                    lock[i] = val;
                }
            }
            std::thread::sleep(Duration::from_millis(16));
        }
    });
    let sys = AudioSystem {
        state: state.clone(),
    };
    AUDIO_SYSTEM.set(sys).ok();

    state
}

fn main_view() -> impl View {
    let state = init_audio();
    let state_for_buttons = state.clone();
    
    // Reactive State
    let theme_binding = binding(VisualizerTheme::cyber());
    let sensitivity_binding = binding(1.0f64);
    
    // Clones for closures
    let theme_for_style = theme_binding.clone();
    let sens_for_style = sensitivity_binding.clone();
    
    zstack((
        GpuSurface::new(VisualizerRenderer::new(
            state.clone(),
            theme_binding.clone(),
            sensitivity_binding.clone()
        ))
        .width(800.0).height(500.0),
        
        vstack((
            text("Audio Visualizer").color(Color::srgb(255, 255, 255)).size(20.0),
            text("🎤 Microphone Input").color(Color::srgb(100, 200, 100)).size(12.0),
            spacer(),
            // Mode buttons
            hstack((
                mode_button("Waveform", VisualizerMode::Waveform, state_for_buttons.clone()),
                mode_button("Spectrum", VisualizerMode::Spectrum, state_for_buttons.clone()),
                mode_button("Spectrogram", VisualizerMode::Spectrogram, state_for_buttons.clone()),
                mode_button("Phase", VisualizerMode::Phase, state_for_buttons.clone()),
            )).spacing(8.0),
            
            spacer().height(16.0),
            
            // Style buttons (now use bindings)
            text("🎨 Style").color(Color::srgb(200, 200, 200)).size(12.0),
            hstack((
                style_button(
                    "Cyber", 
                    VisualizerTheme::cyber(), 
                    theme_for_style.clone(), 
                    sens_for_style.clone(), 
                    1.0
                ),
                style_button(
                    "Recorder", 
                    VisualizerTheme::recorder(), 
                    theme_for_style, 
                    sens_for_style, 
                    2.0
                ),
            )).spacing(8.0),

            spacer().height(8.0),
            
            // Sensitivity slider
            text("🔊 Sensitivity").color(Color::srgb(200, 200, 200)).size(12.0),
            slider(0.5..=5.0, &sensitivity_binding),

        )).padding().alignment(Alignment::Top)
    ))
}

fn mode_button(label: &'static str, mode: VisualizerMode, state: AudioState) -> impl View {
    button(label)
        .action(move || {
            state.set_mode(mode);
        })
}

fn style_button(
    label: &'static str, 
    theme: VisualizerTheme, 
    theme_binding: Binding<VisualizerTheme>,
    sens_binding: Binding<f64>,
    sens_value: f64
) -> impl View {
    button(label)
        .action(move || {
            theme_binding.set(theme);
            sens_binding.set(sens_value);
        })
}

pub fn app(env: Environment) -> App {
    App::new(main_view, env)
}

waterui_ffi::export!();
