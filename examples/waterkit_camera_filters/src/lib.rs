//! WaterUI + Waterkit Camera Filter Lab
//!
//! This example demonstrates collaboration between:
//! - WaterUI: camera-style interface + real-time reactive filter pipeline
//! - Waterkit Permission: camera permission check/request
//! - Waterkit Camera: native camera streaming + device enumeration

use std::sync::{Arc, Mutex};

use futures::{FutureExt, StreamExt, future::LocalBoxFuture};
use shaderloom::CompiledShader;
use waterkit_camera::{Camera, FrameConverter};
use waterkit_permission::{Permission, PermissionStatus, check, request};
use waterui::app::App;
use waterui::graphics::{Context, Frame, GpuContent, GpuContentView, bytemuck};
use waterui::prelude::slider::slider;
use waterui::prelude::theme_color::Surface;
use waterui::prelude::*;
use waterui::preview;

#[state]
#[derive(Clone)]
struct CameraLabState {
    active_filter: Binding<usize>,
    filter_strength: Binding<f64>,
    reconnect_ticket: Binding<usize>,
    preview_status: Binding<Str>,
    waterkit_status: Binding<Str>,
    permission_status: Binding<Str>,
    camera_inventory: Binding<Str>,
}

impl CameraLabState {
    fn live() -> Self {
        Self {
            active_filter: Binding::usize(0),
            filter_strength: Binding::f64(0.75),
            reconnect_ticket: Binding::usize(0),
            preview_status: Binding::container(Str::from("Starting camera renderer...")),
            waterkit_status: Binding::container(Str::from(
                "Not synced. Tap the button below to query Waterkit.",
            )),
            permission_status: Binding::container(Str::from("Permission status: unknown")),
            camera_inventory: Binding::container(Str::from("No camera inventory yet.")),
        }
    }

    fn preview() -> Self {
        Self {
            active_filter: Binding::usize(0),
            filter_strength: Binding::f64(0.75),
            reconnect_ticket: Binding::usize(0),
            preview_status: Binding::container(Str::from(
                "Synthetic preview frame for Hydrolysis perf.",
            )),
            waterkit_status: Binding::container(Str::from(
                "Preview does not query Waterkit camera devices.",
            )),
            permission_status: Binding::container(Str::from("Permission status: preview")),
            camera_inventory: Binding::container(Str::from("Synthetic camera frame.")),
        }
    }
}

/// Root view: live camera filter lab.
pub fn demo() -> impl View {
    let state = CameraLabState::live();
    let preview = camera_surface(
        state.active_filter.clone(),
        state.filter_strength.clone(),
        state.reconnect_ticket.clone(),
        state.preview_status.clone(),
    );
    camera_filter_lab(preview, state)
}

#[preview]
fn camera_filter_lab_preview() -> impl View {
    let state = CameraLabState::preview();
    let preview =
        synthetic_camera_surface(state.active_filter.clone(), state.filter_strength.clone());
    camera_filter_lab(preview, state)
}

fn camera_filter_lab(preview: impl View, state: CameraLabState) -> impl View {
    let filter_label = state.active_filter.clone().map(filter_name);
    let filter_strength = state.filter_strength.clone();
    let preview_status = state.preview_status.clone();
    let waterkit_status = state.waterkit_status.clone();
    let permission_status = state.permission_status.clone();
    let camera_inventory = state.camera_inventory.clone();

    let header = vstack((
        text("WaterUI + Waterkit Camera Filter Lab").title().bold(),
        text("Live camera preview via waterkit-camera, rendered and filtered with WaterUI GpuContentView.")
            .body()
            .muted(),
        Divider,
    ))
    .spacing(8.0);

    let preview_section = vstack((
        text!("Filter: {filter_label}   |   Strength: {filter_strength:.2}").body(),
        preview,
        text!("{preview_status}").caption().muted(),
        hstack((button("Reconnect Camera Stream")
            .action(
                |State(ticket): State<Binding<usize>>, State(status): State<Binding<Str>>| {
                    let next = ticket.snapshot().saturating_add(1);
                    ticket.set(next);
                    status.set(Str::from("Reconnecting camera stream..."));
                },
            )
            .state(&state.reconnect_ticket)
            .state(&state.preview_status),)),
    ))
    .spacing(10.0);

    let filter_section = vstack((
        text("Filter Presets").headline(),
        hstack((
            filter_button("Natural", 0, &state.active_filter),
            filter_button("Cinematic", 1, &state.active_filter),
            filter_button("Noir", 2, &state.active_filter),
            filter_button("Vintage", 3, &state.active_filter),
            filter_button("Neon", 4, &state.active_filter),
            filter_button("Dream", 5, &state.active_filter),
        ))
        .spacing(8.0),
        slider("Filter strength", &state.filter_strength).hide_label(),
    ))
    .spacing(8.0);

    let waterkit_section = vstack((
        Divider,
        text("Waterkit Bridge").headline(),
        text!("{waterkit_status}").body(),
        text!("{permission_status}").body(),
        text!("{camera_inventory}").footnote().muted(),
        button("Sync with Waterkit Camera")
            .action_async(|state: CameraLabState| async move {
                sync_waterkit_camera(
                    state.waterkit_status,
                    state.permission_status,
                    state.camera_inventory,
                )
                .await;
            })
            .bordered_prominent()
            .state(&state),
    ))
    .spacing(8.0);

    scroll(
        vstack((header, preview_section, filter_section, waterkit_section))
            .spacing(12.0)
            .padding_with(16.0),
    )
}

fn camera_surface(
    active_filter: Binding<usize>,
    filter_strength: Binding<f64>,
    reconnect_ticket: Binding<usize>,
    preview_status: Binding<Str>,
) -> impl View {
    CameraFilterRenderer::new(
        active_filter,
        filter_strength,
        reconnect_ticket,
        preview_status,
    )
    .into_view()
    .size(960.0, 540.0)
    .background(Surface)
    .padding_with(8.0)
}

fn synthetic_camera_surface(
    active_filter: Binding<usize>,
    filter_strength: Binding<f64>,
) -> impl View {
    SyntheticCameraPreviewRenderer::new(active_filter, filter_strength)
        .into_view()
        .size(960.0, 540.0)
        .background(Surface)
        .padding_with(8.0)
}

fn filter_button(
    label: &'static str,
    filter_index: usize,
    active_filter: &Binding<usize>,
) -> impl View {
    button(label)
        .action(move |State(selected): State<Binding<usize>>| selected.set(filter_index))
        .state(active_filter)
}

/// The newest filter parameters posted from the UI side to the render side.
///
/// `Binding`s are UI-thread state and cannot cross into a `GpuContent`, so
/// the value the shader sees travels through this mailbox instead.
#[derive(Clone, Copy)]
struct FilterUniforms([f32; 5]);

/// Render-side half of [`SyntheticCameraPreviewRenderer`]: draws the last
/// posted uniform set as a flat color.
struct SyntheticCameraPreviewContent {
    uniforms: Arc<Mutex<FilterUniforms>>,
}

impl GpuContent for SyntheticCameraPreviewContent {
    fn setup(&mut self, _gpu: &Context<'_>) {}

    fn render(&mut self, frame: &mut Frame<'_>) {
        let FilterUniforms([brightness, saturation, contrast, tint, vignette]) = *self
            .uniforms
            .lock()
            .expect("synthetic preview uniform mailbox poisoned");
        let red = (0.32 + brightness + tint * 0.08).clamp(0.0, 1.0);
        let green = (0.46 + brightness + saturation * 0.04 - vignette * 0.03).clamp(0.0, 1.0);
        let blue = (0.58 + brightness - tint * 0.08 + contrast * 0.03).clamp(0.0, 1.0);

        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Synthetic Camera Preview Encoder"),
            });
        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Synthetic Camera Preview Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(red),
                            g: f64::from(green),
                            b: f64::from(blue),
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }
        frame.queue.submit([encoder.finish()]);
    }
}

/// UI-side half of the synthetic preview: owns the filter bindings and posts
/// their values to the render side once per frame.
struct SyntheticCameraPreviewRenderer {
    active_filter: Binding<usize>,
    filter_strength: Binding<f64>,
}

impl SyntheticCameraPreviewRenderer {
    fn new(active_filter: Binding<usize>, filter_strength: Binding<f64>) -> Self {
        Self {
            active_filter,
            filter_strength,
        }
    }

    fn into_view(self) -> GpuContentView {
        let uniforms = Arc::new(Mutex::new(FilterUniforms(filter_params(0, 0.75))));
        let bridge = std::cell::RefCell::new(self);
        GpuContentView::new(SyntheticCameraPreviewContent {
            uniforms: Arc::clone(&uniforms),
        })
        .on_frame(move || {
            let bridge = bridge.borrow();
            *uniforms
                .lock()
                .expect("synthetic preview uniform mailbox poisoned") =
                FilterUniforms(filter_params(
                    bridge.active_filter.snapshot(),
                    bridge.filter_strength.snapshot() as f32,
                ));
        })
    }
}

/// What the UI side of the camera filter posts to the render side.
struct CameraShared {
    /// The render device's handles, published once at setup so the camera
    /// opens on the device the engine draws with.
    gpu_handles: Option<(wgpu::Device, wgpu::Queue)>,
    /// The newest filter parameters the bindings produced.
    uniforms: FilterUniforms,
    /// The newest camera frame the UI side pulled; `None` once the render
    /// side took it.
    latest_texture: Option<wgpu::Texture>,
}

/// UI-side half of the camera filter: owns the camera, its open future, and
/// the bindings — all of them confined to the UI thread.
struct CameraFilterRenderer {
    active_filter: Binding<usize>,
    filter_strength: Binding<f64>,
    reconnect_ticket: Binding<usize>,
    preview_status: Binding<Str>,
    last_reconnect_ticket: usize,
    camera_started: bool,

    camera: Option<Camera>,
    camera_open_task: Option<LocalBoxFuture<'static, Result<Camera, String>>>,
    converter: Option<FrameConverter>,
    shared: Arc<Mutex<CameraShared>>,
}

/// Render-side half of the camera filter: pipelines, resources and the
/// newest camera texture — all `Send`.
struct CameraFilterContent {
    shared: Arc<Mutex<CameraShared>>,
    pipeline: Option<wgpu::RenderPipeline>,
    bind_group_layout: Option<wgpu::BindGroupLayout>,
    sampler: Option<wgpu::Sampler>,
    uniform_buffer: Option<wgpu::Buffer>,
    latest_texture: Option<wgpu::Texture>,
    latest_bind_group: Option<wgpu::BindGroup>,
    pipeline_format: Option<wgpu::TextureFormat>,
}

impl CameraFilterRenderer {
    fn new(
        active_filter: Binding<usize>,
        filter_strength: Binding<f64>,
        reconnect_ticket: Binding<usize>,
        preview_status: Binding<Str>,
    ) -> Self {
        Self {
            active_filter,
            filter_strength,
            reconnect_ticket,
            preview_status,
            last_reconnect_ticket: 0,
            camera_started: false,
            camera: None,
            camera_open_task: None,
            converter: None,
            shared: Arc::new(Mutex::new(CameraShared {
                gpu_handles: None,
                uniforms: FilterUniforms(filter_params(0, 0.75)),
                latest_texture: None,
            })),
        }
    }

    fn into_view(self) -> GpuContentView {
        let shared = Arc::clone(&self.shared);
        let bridge = std::cell::RefCell::new(self);
        GpuContentView::new(CameraFilterContent {
            shared,
            pipeline: None,
            bind_group_layout: None,
            sampler: None,
            uniform_buffer: None,
            latest_texture: None,
            latest_bind_group: None,
            pipeline_format: None,
        })
        .on_frame(move || bridge.borrow_mut().frame())
    }

    /// Drives the camera pipeline once per presented frame.
    fn frame(&mut self) {
        let gpu_handles = {
            let mut shared = self.shared.lock().expect("camera mailbox poisoned");
            shared.uniforms = FilterUniforms(filter_params(
                self.active_filter.snapshot(),
                self.filter_strength.snapshot() as f32,
            ));
            shared.gpu_handles.clone()
        };
        let Some((device, queue)) = gpu_handles else {
            return;
        };

        let reconnect_ticket = self.reconnect_ticket.snapshot();
        if !self.camera_started || reconnect_ticket != self.last_reconnect_ticket {
            self.last_reconnect_ticket = reconnect_ticket;
            self.start_camera_open(&device, &queue, self.camera_started);
            self.camera_started = true;
        }

        self.poll_camera_open();
        self.pull_latest_frame(&device, &queue);
    }

    fn start_camera_open(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, reconnect: bool) {
        let status = if reconnect {
            "Reconnecting camera stream..."
        } else {
            "Opening camera stream..."
        };
        self.preview_status.set(status.to_string().into());
        self.camera = None;
        self.shared
            .lock()
            .expect("camera mailbox poisoned")
            .latest_texture = None;

        let device = Arc::new(device.clone());
        let queue = Arc::new(queue.clone());
        self.camera_open_task = Some(Box::pin(async move {
            Camera::open_default(device, queue)
                .await
                .map_err(|error| error.to_string())
        }));
    }

    fn poll_camera_open(&mut self) {
        let Some(mut task) = self.camera_open_task.take() else {
            return;
        };

        match task.as_mut().now_or_never() {
            Some(Ok(camera)) => {
                self.camera = Some(camera);
                self.preview_status.set(Str::from("Camera stream active."));
            }
            Some(Err(error)) => {
                self.preview_status
                    .set(format!("Failed to open camera stream: {error}").into());
            }
            None => {
                self.camera_open_task = Some(task);
            }
        };
    }

    fn pull_latest_frame(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let Some(camera) = self.camera.as_ref() else {
            return;
        };

        let poll_result = {
            let mut frame_stream = camera.frames().boxed_local();
            let waker = futures::task::noop_waker_ref();
            let mut cx = std::task::Context::from_waker(waker);
            frame_stream.as_mut().poll_next(&mut cx)
        };

        match poll_result {
            std::task::Poll::Ready(Some(Ok(frame))) => {
                // Frames arrive in the platform's plane layout and
                // orientation, so convert each to an upright RGBA texture
                // before the filter pass samples it.
                let converter = self.converter.get_or_insert_with(|| {
                    FrameConverter::check_device(device).expect(
                        "the GPU runtime's device loads the converter's precompiled shaders",
                    );
                    FrameConverter::new(device)
                });
                let texture = converter.convert(device, queue, &frame);
                self.shared
                    .lock()
                    .expect("camera mailbox poisoned")
                    .latest_texture = Some(texture);
            }
            std::task::Poll::Ready(Some(Err(error))) => {
                // A capture failure is the stream's last item, then it ends.
                self.camera = None;
                self.shared
                    .lock()
                    .expect("camera mailbox poisoned")
                    .latest_texture = None;
                self.preview_status
                    .set(format!("Camera stream failed: {error}").into());
            }
            std::task::Poll::Ready(None) => {
                self.camera = None;
                self.shared
                    .lock()
                    .expect("camera mailbox poisoned")
                    .latest_texture = None;
                self.preview_status
                    .set(Str::from("Camera stream ended unexpectedly."));
            }
            std::task::Poll::Pending => {}
        }
    }
}

impl CameraFilterContent {
    fn ensure_pipeline(&mut self, ctx: &Context) {
        if self.pipeline.is_some() && self.pipeline_format == Some(ctx.format) {
            return;
        }

        let (vertex_shader, fragment_shader) =
            CAMERA_FILTER_SHADER.create_render_stages(ctx.device, "vs_main", "fs_main");

        let bind_group_layout =
            ctx.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Camera Filter Bind Group Layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                    ],
                });

        let pipeline_layout = ctx
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Camera Filter Pipeline Layout"),
                bind_group_layouts: &[Some(&bind_group_layout)],
                immediate_size: 0,
            });

        let pipeline = ctx
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Camera Filter Pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: vertex_shader.module(),
                    entry_point: Some(vertex_shader.entry_point()),
                    buffers: &[],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: fragment_shader.module(),
                    entry_point: Some(fragment_shader.entry_point()),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: ctx.format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        let uniform_buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Camera Filter Uniform Buffer"),
            size: (core::mem::size_of::<f32>() * 8) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let sampler = ctx.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Camera Filter Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        self.pipeline = Some(pipeline);
        self.bind_group_layout = Some(bind_group_layout);
        self.sampler = Some(sampler);
        self.uniform_buffer = Some(uniform_buffer);
        self.pipeline_format = Some(ctx.format);
    }
}

impl GpuContent for CameraFilterContent {
    fn setup(&mut self, context: &Context<'_>) {
        self.ensure_pipeline(context);
        self.shared
            .lock()
            .expect("camera mailbox poisoned")
            .gpu_handles = Some((context.device.clone(), context.queue.clone()));
    }

    fn render(&mut self, frame: &mut Frame<'_>) {
        let (incoming, uniforms) = {
            let mut shared = self.shared.lock().expect("camera mailbox poisoned");
            (shared.latest_texture.take(), shared.uniforms)
        };
        if incoming.is_some() {
            self.latest_texture = incoming;
            self.latest_bind_group = None;
        }

        if let Some(uniform_buffer) = &self.uniform_buffer {
            let FilterUniforms([brightness, saturation, contrast, tint, vignette]) = uniforms;
            let uniforms: [f32; 8] = [
                brightness, saturation, contrast, tint, vignette, 0.0, 0.0, 0.0,
            ];
            frame
                .queue
                .write_buffer(uniform_buffer, 0, bytemuck::cast_slice(&uniforms));
        }

        if let (Some(layout), Some(sampler), Some(uniform_buffer), Some(texture)) = (
            &self.bind_group_layout,
            &self.sampler,
            &self.uniform_buffer,
            &self.latest_texture,
        ) && self.latest_bind_group.is_none()
        {
            let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.latest_bind_group =
                Some(frame.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Camera Filter Bind Group"),
                    layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&texture_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: uniform_buffer.as_entire_binding(),
                        },
                    ],
                }));
        }

        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Camera Filter Encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Camera Filter Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame.view,
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
                multiview_mask: None,
            });

            if let (Some(pipeline), Some(bind_group)) = (&self.pipeline, &self.latest_bind_group) {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }

        frame.queue.submit([encoder.finish()]);
        frame.request_redraw();
    }
}

fn filter_params(filter_kind: usize, strength: f32) -> [f32; 5] {
    match filter_kind {
        // Cinematic
        1 => [
            -0.05 * strength,
            1.1 + 0.25 * strength,
            1.0 + 0.3 * strength,
            0.35 * strength,
            0.45 * strength,
        ],
        // Noir
        2 => [
            -0.12 * strength,
            1.0 - 0.95 * strength,
            1.15 + 0.4 * strength,
            0.0,
            0.2 * strength,
        ],
        // Vintage
        3 => [
            0.02 * strength,
            1.0 - 0.3 * strength,
            1.0 + 0.1 * strength,
            0.65 * strength,
            0.35 * strength,
        ],
        // Neon
        4 => [
            0.08 * strength,
            1.3 + 0.55 * strength,
            1.15 + 0.45 * strength,
            -0.55 * strength,
            0.15 * strength,
        ],
        // Dream
        5 => [
            0.1 * strength,
            1.05 + 0.2 * strength,
            0.92 + 0.08 * strength,
            0.2 * strength,
            0.6 * strength,
        ],
        // Natural
        _ => [0.0, 1.0, 1.0, 0.0, 0.0],
    }
}

async fn sync_waterkit_camera(
    status: Binding<Str>,
    permission: Binding<Str>,
    inventory: Binding<Str>,
) {
    status.set(Str::from(
        "Checking camera permission via waterkit-permission...",
    ));

    let current = check(Permission::Camera).await;
    permission.set(format!("Permission status: {}", permission_status_text(current)).into());

    let granted = if matches!(current, PermissionStatus::Granted) {
        true
    } else {
        status.set(Str::from(
            "Requesting camera permission via waterkit-permission...",
        ));
        match request(Permission::Camera).await {
            Ok(next) => {
                permission
                    .set(format!("Permission status: {}", permission_status_text(next)).into());
                matches!(next, PermissionStatus::Granted)
            }
            Err(error) => {
                status.set(format!("Permission request failed: {error}").into());
                inventory.set(Str::from(
                    "Waterkit camera inventory unavailable until permission is granted.",
                ));
                return;
            }
        }
    };

    if !granted {
        status.set(Str::from(
            "Camera permission was not granted. Sync cancelled.",
        ));
        inventory.set(Str::from(
            "Waterkit camera inventory unavailable until permission is granted.",
        ));
        return;
    }

    status.set(Str::from(
        "Permission granted. Enumerating cameras via waterkit-camera...",
    ));

    match Camera::list() {
        Ok(cameras) if cameras.is_empty() => {
            status.set(Str::from(
                "Waterkit is connected, but no camera devices were reported.",
            ));
            inventory.set(Str::from("0 camera devices detected."));
        }
        Ok(cameras) => {
            let summary = cameras
                .iter()
                .map(|camera| {
                    let facing = if camera.is_front_facing {
                        "Front"
                    } else {
                        "Back/External"
                    };
                    format!("{facing}: {} ({})", camera.name, camera.id)
                })
                .collect::<Vec<_>>()
                .join(" | ");

            status.set(
                format!(
                    "Waterkit sync complete: {} camera(s) detected.",
                    cameras.len()
                )
                .into(),
            );
            inventory.set(summary.into());
        }
        Err(error) => {
            status.set(format!("Camera enumeration failed: {error}").into());
            inventory.set(Str::from(
                "Waterkit camera list could not be loaded on this platform/runtime.",
            ));
        }
    }
}

fn permission_status_text(status: PermissionStatus) -> &'static str {
    match status {
        PermissionStatus::Granted => "granted",
        PermissionStatus::Denied => "denied",
        PermissionStatus::Restricted => "restricted",
        PermissionStatus::NotDetermined => "not determined",
        _ => "unknown",
    }
}

fn filter_name(index: usize) -> &'static str {
    match index {
        1 => "Cinematic",
        2 => "Noir",
        3 => "Vintage",
        4 => "Neon",
        5 => "Dream",
        _ => "Natural",
    }
}

const CAMERA_FILTER_SHADER: CompiledShader =
    include!(concat!(env!("OUT_DIR"), "/camera_filter.rs"));

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
