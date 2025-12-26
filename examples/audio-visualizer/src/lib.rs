use std::sync::{Arc, Mutex, atomic::{AtomicU32, Ordering}};
use std::time::{Instant, Duration};

use waterui::app::App;
use waterui::graphics::gpu_surface::{GpuContext, GpuFrame, GpuRenderer, GpuSurface};
use waterui::prelude::*;
use waterui::prelude::stack::Alignment;

use waterkit_audio::{AudioRecorder, AudioRecorderBuilder};

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

/// Shared state for raw PCM samples
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
    render_pipeline: Option<wgpu::RenderPipeline>,
    compute_pipeline: Option<wgpu::ComputePipeline>,
    bind_group: Option<wgpu::BindGroup>,
    uniform_buffer: Option<wgpu::Buffer>,
    audio_buffer: Option<wgpu::Buffer>,
    frequency_buffer: Option<wgpu::Buffer>,
    start_time: Instant,
}

impl VisualizerRenderer {
    fn new(state: AudioState) -> Self {
        Self {
            state,
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

        let time = self.start_time.elapsed().as_secs_f32();
        let uniforms = ShaderUniforms {
            resolution: [frame.width as f32, frame.height as f32],
            time,
            mode: self.state.get_mode(),
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

    // Spawn microphone capture thread
    std::thread::spawn(move || {
        // Build and start recorder
        let mut recorder = match AudioRecorderBuilder::new().build() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Failed to create AudioRecorder: {:?}", e);
                return;
            }
        };

        if let Err(e) = pollster::block_on(recorder.start()) {
            eprintln!("Failed to start recording: {:?}", e);
            return;
        }

        let mut write_pos: usize = 0;

        loop {
            // Non-blocking read
            if let Some(buffer) = recorder.try_read() {
                let samples = buffer.samples();
                if let Ok(mut lock) = state_clone.samples.try_lock() {
                    for &s in samples {
                        lock[write_pos] = s;
                        write_pos = (write_pos + 1) % SAMPLES_COUNT;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(5));
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

    zstack((
        GpuSurface::new(VisualizerRenderer::new(state.clone()))
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
                mode_button("Phase", VisualizerMode::Phase, state_for_buttons),
            )).spacing(8.0),
        )).padding().alignment(Alignment::Top)
    ))
}

fn mode_button(label: &'static str, mode: VisualizerMode, state: AudioState) -> impl View {
    button(label)
        .action(move || {
            state.set_mode(mode);
        })
}

pub fn app(env: Environment) -> App {
    App::new(main_view, env)
}

waterui_ffi::export!();
