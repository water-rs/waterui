//! WaterUI + Waterkit Camera Filter Lab
//!
//! This playground example demonstrates collaboration between:
//! - WaterUI: camera-style interface + real-time reactive filter pipeline
//! - Waterkit Permission: camera permission check/request
//! - Waterkit Camera: native camera streaming + device enumeration

use std::sync::Arc;

use futures::{FutureExt, StreamExt, future::LocalBoxFuture};
use shaderloom::CompiledShader;
use waterkit_camera::Camera;
use waterkit_permission::{Permission, PermissionStatus, check, request};
use waterui::app::App;
use waterui::graphics::{GpuContext, GpuFrame, GpuSurface, GpuView, bytemuck};
use waterui::prelude::slider::slider;
use waterui::prelude::theme_color::{MutedForeground, Surface};
use waterui::prelude::*;
use waterui::preview;

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
        text("Live camera preview via waterkit-camera, rendered and filtered with WaterUI GpuSurface.")
            .body()
            .foreground(MutedForeground),
        Divider,
    ))
    .spacing(8.0);

    let preview_section = vstack((
        text!("Filter: {filter_label}   |   Strength: {filter_strength:.2}").body(),
        preview,
        text!("{preview_status}")
            .caption()
            .foreground(MutedForeground),
        hstack((button("Reconnect Camera Stream")
            .action(
                |State(ticket): State<Binding<usize>>, State(status): State<Binding<Str>>| {
                    let next = ticket.get().saturating_add(1);
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
        text!("{camera_inventory}")
            .footnote()
            .foreground(MutedForeground),
        button("Sync with Waterkit Camera")
            .action_async(
                |State(status): State<Binding<Str>>,
                 State(permission): State<Binding<Str>>,
                 State(inventory): State<Binding<Str>>| async move {
                    sync_waterkit_camera(status, permission, inventory).await;
                },
            )
            .bordered_prominent()
            .state(&state.waterkit_status)
            .state(&state.permission_status)
            .state(&state.camera_inventory),
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
    GpuSurface::new(CameraFilterRenderer::new(
        active_filter,
        filter_strength,
        reconnect_ticket,
        preview_status,
    ))
    .size(960.0, 540.0)
    .background(Surface)
    .padding_with(8.0)
}

fn synthetic_camera_surface(
    active_filter: Binding<usize>,
    filter_strength: Binding<f64>,
) -> impl View {
    GpuSurface::new(SyntheticCameraPreviewRenderer::new(
        active_filter,
        filter_strength,
    ))
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
}

impl GpuView for SyntheticCameraPreviewRenderer {
    async fn setup(&mut self, _ctx: &GpuContext<'_>, _env: &mut waterui::Environment) {}

    fn render(&mut self, frame: &mut GpuFrame) {
        let (brightness, saturation, contrast, tint, vignette) =
            filter_params(self.active_filter.get(), self.filter_strength.get() as f32);
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
                    view: &frame.view,
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

struct CameraFilterRenderer {
    active_filter: Binding<usize>,
    filter_strength: Binding<f64>,
    reconnect_ticket: Binding<usize>,
    preview_status: Binding<Str>,
    last_reconnect_ticket: usize,

    camera: Option<Camera>,
    camera_open_task: Option<LocalBoxFuture<'static, Result<Camera, String>>>,
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
            camera: None,
            camera_open_task: None,
            pipeline: None,
            bind_group_layout: None,
            sampler: None,
            uniform_buffer: None,
            latest_texture: None,
            latest_bind_group: None,
            pipeline_format: None,
        }
    }

    fn ensure_pipeline(&mut self, ctx: &GpuContext) {
        if self.pipeline.is_some() && self.pipeline_format == Some(ctx.surface_format) {
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
                        format: ctx.surface_format,
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
        self.pipeline_format = Some(ctx.surface_format);
    }

    fn start_camera_open(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, reconnect: bool) {
        let status = if reconnect {
            "Reconnecting camera stream..."
        } else {
            "Opening camera stream..."
        };
        self.preview_status.set(status.to_string().into());
        self.camera = None;
        self.latest_texture = None;
        self.latest_bind_group = None;

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

    fn update_filter_uniform(&self, frame: &GpuFrame) {
        let Some(uniform_buffer) = &self.uniform_buffer else {
            return;
        };

        let (brightness, saturation, contrast, tint, vignette) =
            filter_params(self.active_filter.get(), self.filter_strength.get() as f32);

        let uniforms: [f32; 8] = [
            brightness, saturation, contrast, tint, vignette, 0.0, 0.0, 0.0,
        ];
        frame
            .queue
            .write_buffer(uniform_buffer, 0, bytemuck::cast_slice(&uniforms));
    }

    fn pull_latest_frame(&mut self) {
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
            std::task::Poll::Ready(Some(frame)) => {
                self.latest_texture = Some(frame.into_texture());
            }
            std::task::Poll::Ready(None) => {
                self.camera = None;
                self.latest_texture = None;
                self.latest_bind_group = None;
                self.preview_status
                    .set(Str::from("Camera stream ended unexpectedly."));
            }
            std::task::Poll::Pending => {}
        }
    }
}

impl GpuView for CameraFilterRenderer {
    async fn setup(&mut self, ctx: &GpuContext<'_>, _env: &mut waterui::Environment) {
        self.ensure_pipeline(ctx);
        self.start_camera_open(ctx.device, ctx.queue, false);
    }

    fn render(&mut self, frame: &mut GpuFrame) {
        let reconnect_ticket = self.reconnect_ticket.get();
        if reconnect_ticket != self.last_reconnect_ticket {
            self.last_reconnect_ticket = reconnect_ticket;
            self.start_camera_open(frame.device, frame.queue, true);
        }

        self.poll_camera_open();
        self.pull_latest_frame();
        self.update_filter_uniform(frame);

        if let (Some(layout), Some(sampler), Some(uniform_buffer), Some(texture)) = (
            &self.bind_group_layout,
            &self.sampler,
            &self.uniform_buffer,
            &self.latest_texture,
        ) {
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

fn filter_params(filter_kind: usize, strength: f32) -> (f32, f32, f32, f32, f32) {
    match filter_kind {
        // Cinematic
        1 => (
            -0.05 * strength,
            1.1 + 0.25 * strength,
            1.0 + 0.3 * strength,
            0.35 * strength,
            0.45 * strength,
        ),
        // Noir
        2 => (
            -0.12 * strength,
            1.0 - 0.95 * strength,
            1.15 + 0.4 * strength,
            0.0,
            0.2 * strength,
        ),
        // Vintage
        3 => (
            0.02 * strength,
            1.0 - 0.3 * strength,
            1.0 + 0.1 * strength,
            0.65 * strength,
            0.35 * strength,
        ),
        // Neon
        4 => (
            0.08 * strength,
            1.3 + 0.55 * strength,
            1.15 + 0.45 * strength,
            -0.55 * strength,
            0.15 * strength,
        ),
        // Dream
        5 => (
            0.1 * strength,
            1.05 + 0.2 * strength,
            0.92 + 0.08 * strength,
            0.2 * strength,
            0.6 * strength,
        ),
        // Natural
        _ => (0.0, 1.0, 1.0, 0.0, 0.0),
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
