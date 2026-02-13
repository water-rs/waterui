use std::{
    rc::Rc,
    time::{Duration, Instant},
};

use nami::Signal;
use waterkit_audio::{MediaMetadata, MediaSession, PlaybackState};
use waterkit_codec::{CodecType, Decoder};
use waterkit_video::VideoReader;
use waterui_controls::{button, slider::slider};
use waterui_core::{AnyView, Binding, Environment, View, dynamic::Dynamic};
use waterui_graphics::{GpuContext, GpuFrame, GpuRenderer, GpuSurface, wgpu};
use waterui_layout::{hstack, overlay, stack::Alignment, vstack};

use crate::Url;
use crate::video::{AspectRatio, Event, VideoConfig, VideoPlayerConfig, Volume};

const SEEK_EPSILON: f64 = 0.005;
const PRESENT_TOLERANCE: Duration = Duration::from_millis(3);

type OnEvent = Rc<dyn Fn(Event) + 'static>;

#[derive(Clone)]
struct PlayerBindings {
    is_playing: Binding<bool>,
    progress: Binding<f64>,
}

pub(crate) fn install_platform_hooks(env: &mut Environment) {
    env.insert_hook::<VideoConfig, AnyView>(|_env, config| {
        let VideoConfig {
            source,
            volume,
            aspect_ratio,
            loops,
            on_event,
        } = config;

        let on_event: OnEvent = Rc::from(on_event);
        AnyView::new(Dynamic::watch(source, move |url| {
            AnyView::new(VideoSurface::new(
                url,
                volume.clone(),
                aspect_ratio,
                loops,
                on_event.clone(),
                None,
            ))
        }))
    });

    env.insert_hook::<VideoPlayerConfig, AnyView>(|_env, config| {
        let VideoPlayerConfig {
            source,
            volume,
            aspect_ratio,
            show_controls,
            on_event,
        } = config;

        let on_event: OnEvent = Rc::from(on_event);
        AnyView::new(Dynamic::watch(source, move |url| {
            let player = PlayerBindings {
                is_playing: Binding::bool(true),
                progress: Binding::f64(0.0),
            };

            let surface = VideoSurface::new(
                url,
                volume.clone(),
                aspect_ratio,
                true,
                on_event.clone(),
                Some(player.clone()),
            );

            if show_controls {
                let controls = player_controls(
                    player.is_playing.clone(),
                    player.progress.clone(),
                    volume.clone(),
                );
                AnyView::new(overlay(surface, controls).alignment(Alignment::Bottom))
            } else {
                AnyView::new(surface)
            }
        }))
    });
}

fn player_controls(
    is_playing: Binding<bool>,
    progress: Binding<f64>,
    volume: Binding<Volume>,
) -> impl View {
    let volume_level = Binding::mapping(
        &volume,
        |current| f64::from(current.abs().clamp(0.0, 1.0)),
        |volume_binding, level| {
            let magnitude = (level as f32).clamp(0.0, 1.0).max(0.01);
            let muted = volume_binding.get().is_sign_negative();
            volume_binding.set(if muted { -magnitude } else { magnitude });
        },
    );

    let muted = Binding::mapping(
        &volume,
        |current| current.is_sign_negative(),
        |volume_binding, is_muted| {
            let magnitude = volume_binding.get().abs().max(0.1);
            volume_binding.set(if is_muted { -magnitude } else { magnitude });
        },
    );

    let transport = hstack((
        button("Back")
            .with_state(&progress)
            .action(|value| value.set((value.get() - 0.05).max(0.0))),
        button("Play/Pause")
            .with_state(&is_playing)
            .action(|playing| playing.set(!playing.get())),
        button("Mute")
            .with_state(&muted)
            .action(|is_muted| is_muted.set(!is_muted.get())),
        slider(0.0..=1.0, &volume_level),
        button("Forward")
            .with_state(&progress)
            .action(|value| value.set((value.get() + 0.05).min(1.0))),
    ))
    .spacing(8.0);

    vstack((slider(0.0..=1.0, &progress), transport)).spacing(8.0)
}

struct VideoSurface {
    renderer: VideoRenderer,
}

impl VideoSurface {
    fn new(
        source: Url,
        volume: Binding<Volume>,
        aspect_ratio: AspectRatio,
        loops: bool,
        on_event: OnEvent,
        player: Option<PlayerBindings>,
    ) -> Self {
        Self {
            renderer: VideoRenderer::new(source, volume, aspect_ratio, loops, on_event, player),
        }
    }
}

impl core::fmt::Debug for VideoSurface {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VideoSurface").finish_non_exhaustive()
    }
}

impl View for VideoSurface {
    fn body(self, _env: &Environment) -> impl View {
        GpuSurface::new(self.renderer).continuous()
    }
}

struct VideoRenderer {
    source: Url,
    _volume: Binding<Volume>,
    _aspect_ratio: AspectRatio,
    loops: bool,
    on_event: OnEvent,
    player: Option<PlayerBindings>,
    decode: Option<DecodeState>,
    render_pipeline: Option<wgpu::RenderPipeline>,
    bind_group_layout: Option<wgpu::BindGroupLayout>,
    sampler: Option<wgpu::Sampler>,
    rgba_texture: Option<wgpu::Texture>,
    bind_group: Option<wgpu::BindGroup>,
    pending_frame: Option<DecodedVideoFrame>,
    media_session: Option<MediaSessionState>,
    ready_sent: bool,
    ended_sent: bool,
    playback_anchor_pts: Duration,
    playback_anchor_instant: Option<Instant>,
    last_playing_state: bool,
    last_reported_progress: f64,
}

impl core::fmt::Debug for VideoRenderer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("VideoRenderer")
            .field("source", &self.source)
            .field("loops", &self.loops)
            .field("has_decode", &self.decode.is_some())
            .field("has_pending_frame", &self.pending_frame.is_some())
            .finish_non_exhaustive()
    }
}

impl VideoRenderer {
    fn new(
        source: Url,
        volume: Binding<Volume>,
        aspect_ratio: AspectRatio,
        loops: bool,
        on_event: OnEvent,
        player: Option<PlayerBindings>,
    ) -> Self {
        let last_playing_state = player.as_ref().is_none_or(|p| p.is_playing.get());

        Self {
            source,
            _volume: volume,
            _aspect_ratio: aspect_ratio,
            loops,
            on_event,
            player,
            decode: None,
            render_pipeline: None,
            bind_group_layout: None,
            sampler: None,
            rgba_texture: None,
            bind_group: None,
            pending_frame: None,
            media_session: None,
            ready_sent: false,
            ended_sent: false,
            playback_anchor_pts: Duration::ZERO,
            playback_anchor_instant: None,
            last_playing_state,
            last_reported_progress: 0.0,
        }
    }

    fn ensure_pipeline(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
        if self.render_pipeline.is_some() {
            return;
        }

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Video render shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/video_render.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Video bind group layout"),
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
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Video pipeline layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Video render pipeline"),
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
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Video sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        self.bind_group_layout = Some(bind_group_layout);
        self.render_pipeline = Some(render_pipeline);
        self.sampler = Some(sampler);
    }

    fn open_decode_state(&mut self) {
        match DecodeState::open(&self.source) {
            Ok(state) => {
                self.decode = Some(state);
                self.pending_frame = None;
                self.ended_sent = false;
                self.playback_anchor_pts = Duration::ZERO;
                self.playback_anchor_instant = None;
                self.update_ui_progress();

                if self.media_session.is_none() {
                    self.media_session = MediaSessionState::new(&self.source);
                }

                if !self.ready_sent {
                    (self.on_event)(Event::ReadyToPlay);
                    self.ready_sent = true;
                }
            }
            Err(message) => {
                (self.on_event)(Event::Error { message });
                self.decode = None;
            }
        }
    }

    fn should_play(&self) -> bool {
        self.player
            .as_ref()
            .is_none_or(|player| player.is_playing.get())
    }

    fn playback_position(&self, now: Instant) -> Duration {
        match self.playback_anchor_instant {
            Some(anchor) => self
                .playback_anchor_pts
                .saturating_add(now.saturating_duration_since(anchor)),
            None => self.playback_anchor_pts,
        }
    }

    fn set_playback_position(&mut self, pts: Duration, should_play: bool) {
        self.playback_anchor_pts = pts;
        self.playback_anchor_instant = should_play.then(Instant::now);
        self.last_playing_state = should_play;
    }

    fn update_playing_state(&mut self, should_play: bool) {
        if self.last_playing_state == should_play {
            return;
        }

        if should_play {
            self.playback_anchor_instant = Some(Instant::now());
        } else {
            self.playback_anchor_pts = self.playback_position(Instant::now());
            self.playback_anchor_instant = None;
        }
        self.last_playing_state = should_play;
    }

    fn sync_media_session(&mut self, should_play: bool) {
        let position = self.playback_position(Instant::now());
        if let Some(session) = self.media_session.as_mut() {
            session.sync(should_play, position);
        }
    }

    fn maybe_seek_from_ui(&mut self, should_play: bool) {
        let Some(player) = self.player.as_ref() else {
            return;
        };
        let Some(decode) = self.decode.as_mut() else {
            return;
        };

        let requested = player.progress.get().clamp(0.0, 1.0);
        if (requested - self.last_reported_progress).abs() <= SEEK_EPSILON {
            return;
        }

        match decode.seek_to_progress(requested) {
            Ok(target_pts) => {
                self.pending_frame = None;
                self.rgba_texture = None;
                self.ended_sent = false;
                self.set_playback_position(target_pts, should_play);
                self.last_reported_progress = requested;
                self.sync_media_session(should_play);
            }
            Err(message) => {
                (self.on_event)(Event::Error { message });
            }
        }
    }

    fn update_ui_progress(&mut self) {
        let Some(player) = self.player.as_ref() else {
            return;
        };

        let progress = self.decode.as_ref().map_or(0.0, DecodeState::progress);
        player.progress.set(progress);
        self.last_reported_progress = progress;
    }

    fn ensure_texture_resources(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let need_texture = self
            .rgba_texture
            .as_ref()
            .is_none_or(|texture| texture.width() != width || texture.height() != height);
        if !need_texture {
            return;
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Video RGBA texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let Some(bind_group_layout) = self.bind_group_layout.as_ref() else {
            return;
        };
        let Some(sampler) = self.sampler.as_ref() else {
            return;
        };

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Video bind group"),
            layout: bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });

        self.rgba_texture = Some(texture);
        self.bind_group = Some(bind_group);
    }

    fn upload_frame_texture(&mut self, frame: &GpuFrame, decoded: &DecodedVideoFrame) {
        self.ensure_texture_resources(frame.device, decoded.width, decoded.height);
        let Some(texture) = self.rgba_texture.as_ref() else {
            return;
        };

        frame.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &decoded.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(decoded.width * 4),
                rows_per_image: Some(decoded.height),
            },
            wgpu::Extent3d {
                width: decoded.width,
                height: decoded.height,
                depth_or_array_layers: 1,
            },
        );
    }

    fn step_decoder_if_needed(&mut self, frame: &GpuFrame) {
        if self.decode.is_none() {
            self.open_decode_state();
        }

        let should_play = self.should_play();
        self.update_playing_state(should_play);
        self.maybe_seek_from_ui(should_play);

        let Some(decode) = self.decode.as_mut() else {
            return;
        };

        if !should_play && self.pending_frame.is_none() && self.rgba_texture.is_some() {
            self.sync_media_session(false);
            return;
        }

        if self.pending_frame.is_none() {
            match decode.next_frame() {
                Ok(Some(decoded)) => {
                    self.pending_frame = Some(decoded);
                    self.ended_sent = false;
                }
                Ok(None) => {
                    if !self.ended_sent {
                        (self.on_event)(Event::Ended);
                        self.ended_sent = true;
                    }
                    if self.loops {
                        if let Err(message) = decode.reopen() {
                            (self.on_event)(Event::Error { message });
                            self.decode = None;
                            self.pending_frame = None;
                            return;
                        }
                        self.set_playback_position(Duration::ZERO, should_play);
                        self.update_ui_progress();
                    } else {
                        if let Some(player) = self.player.as_ref() {
                            player.is_playing.set(false);
                        }
                        self.set_playback_position(decode.duration(), false);
                        self.sync_media_session(false);
                    }
                    return;
                }
                Err(message) => {
                    (self.on_event)(Event::Error { message });
                    self.decode = None;
                    self.pending_frame = None;
                    return;
                }
            }
        }

        let Some(pending) = self.pending_frame.as_ref() else {
            return;
        };

        let present_immediately = self.rgba_texture.is_none();
        let due = should_play
            && self
                .playback_position(Instant::now())
                .saturating_add(PRESENT_TOLERANCE)
                >= pending.pts;

        if !(present_immediately || due) {
            return;
        }

        let Some(decoded) = self.pending_frame.take() else {
            return;
        };

        self.upload_frame_texture(frame, &decoded);
        self.set_playback_position(decoded.pts, should_play);

        if let Some(player) = self.player.as_ref() {
            let progress = decode.progress_for_sample(decoded.sample_index);
            player.progress.set(progress);
            self.last_reported_progress = progress;
        }

        self.sync_media_session(should_play);
    }

    fn render_surface(&mut self, frame: &GpuFrame) {
        let Some(pipeline) = self.render_pipeline.as_ref() else {
            return;
        };
        let Some(bind_group) = self.bind_group.as_ref() else {
            return;
        };

        let mut encoder = frame
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Video render encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Video render pass"),
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
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..6, 0..1);
        }

        frame.queue.submit([encoder.finish()]);
    }
}

impl GpuRenderer for VideoRenderer {
    fn setup(&mut self, ctx: &GpuContext) -> impl core::future::Future<Output = ()> {
        self.ensure_pipeline(ctx.device, ctx.surface_format);
        self.open_decode_state();
        async {}
    }

    fn render(&mut self, frame: &GpuFrame) {
        self.step_decoder_if_needed(frame);
        self.render_surface(frame);
    }
}

struct MediaSessionState {
    session: MediaSession,
}

impl MediaSessionState {
    fn new(source: &Url) -> Option<Self> {
        let session = MediaSession::new().ok()?;
        let _ = session.set_metadata(&MediaMetadata::new().title(source.as_str()));
        Some(Self { session })
    }

    fn sync(&mut self, playing: bool, position: Duration) {
        if playing {
            let _ = self.session.request_audio_focus();
        } else {
            let _ = self.session.abandon_audio_focus();
        }

        let playback = if playing {
            PlaybackState::playing(position)
        } else {
            PlaybackState::paused(position)
        };
        let _ = self.session.set_playback_state(&playback);
    }
}

impl Drop for MediaSessionState {
    fn drop(&mut self) {
        let _ = self.session.set_playback_state(&PlaybackState::stopped());
        let _ = self.session.abandon_audio_focus();
        let _ = self.session.clear();
    }
}

#[derive(Debug)]
struct DecodedVideoFrame {
    rgba: Vec<u8>,
    pts: Duration,
    sample_index: u32,
    width: u32,
    height: u32,
}

struct DecodeState {
    source: Url,
    reader: VideoReader,
    decoder: Decoder,
    width: u32,
    height: u32,
    timescale: u32,
    total_samples: u32,
    duration: Duration,
}

impl DecodeState {
    fn open(source: &Url) -> Result<Self, String> {
        let path = source.as_str();
        let reader = VideoReader::open(path).map_err(|error| error.to_string())?;
        let (width, height) = reader.dimensions();
        let codec_type = detect_codec_type(reader.codec_config())?;
        let decoder = Decoder::new(codec_type, reader.codec_config(), width, height)
            .map_err(|error| error.to_string())?;
        let duration = reader.duration().unwrap_or_default();

        Ok(Self {
            source: source.clone(),
            timescale: reader.timescale(),
            total_samples: reader.sample_count(),
            reader,
            decoder,
            width,
            height,
            duration,
        })
    }

    fn reopen(&mut self) -> Result<(), String> {
        let reopened = Self::open(&self.source)?;
        *self = reopened;
        Ok(())
    }

    fn duration(&self) -> Duration {
        self.duration
    }

    fn progress(&self) -> f64 {
        if self.total_samples <= 1 {
            return 0.0;
        }

        let index = self.reader.current_index().min(self.total_samples as usize) as f64;
        index / f64::from(self.total_samples)
    }

    fn progress_for_sample(&self, sample_index: u32) -> f64 {
        if self.total_samples <= 1 {
            return 0.0;
        }

        f64::from(sample_index.min(self.total_samples - 1)) / f64::from(self.total_samples - 1)
    }

    fn seek_to_progress(&mut self, progress: f64) -> Result<Duration, String> {
        if self.total_samples == 0 {
            return Ok(Duration::ZERO);
        }

        let clamped = progress.clamp(0.0, 1.0);
        let target_index =
            (clamped * f64::from(self.total_samples.saturating_sub(1))).round() as usize;
        let keyframe_index = self.reader.nearest_keyframe_at_or_before(target_index);

        self.reopen()?;
        self.reader.seek_to_sample(keyframe_index);

        while self.reader.current_index() < target_index {
            let Some((sample_data, _, _)) = self.reader.read_sample() else {
                break;
            };
            let mut stream = self.decoder.decode(&sample_data);
            while let Some(result) = stream.next() {
                result.map_err(|error| error.to_string())?;
            }
        }

        let pts = self
            .reader
            .sample_info(target_index)
            .map_or(Duration::ZERO, |(pts, _)| {
                pts_to_duration(pts, self.timescale)
            });
        Ok(pts)
    }

    fn next_frame(&mut self) -> Result<Option<DecodedVideoFrame>, String> {
        loop {
            let Some((sample_data, pts, _)) = self.reader.read_sample() else {
                return Ok(None);
            };
            let sample_index = self.reader.current_index().saturating_sub(1) as u32;
            let pts = pts_to_duration(pts, self.timescale);

            let mut stream = self.decoder.decode(&sample_data);
            if let Some(result) = stream.next() {
                let decoded = result.map_err(|error| error.to_string())?;
                let mut nv12 = vec![0; (self.width * self.height * 3 / 2) as usize];
                decoded.copy_to_buffer(&mut nv12);
                return Ok(Some(DecodedVideoFrame {
                    rgba: nv12_to_rgba8(&nv12, self.width, self.height),
                    pts,
                    sample_index,
                    width: self.width,
                    height: self.height,
                }));
            }
        }
    }
}

fn pts_to_duration(pts: u64, timescale: u32) -> Duration {
    if timescale == 0 {
        return Duration::ZERO;
    }

    Duration::from_nanos((pts.saturating_mul(1_000_000_000)) / u64::from(timescale))
}

fn detect_codec_type(config: Option<&[u8]>) -> Result<CodecType, String> {
    let Some(config) = config else {
        return Err(String::from("No codec config found"));
    };

    if config.len() >= 8 && &config[4..8] == b"avcC" {
        return Ok(CodecType::H264);
    }

    if config.len() >= 7 && config[0] == 0x01 {
        let profile = config[1];
        if matches!(profile, 66 | 77 | 88 | 100 | 110 | 122 | 144) {
            return Ok(CodecType::H264);
        }
    }

    if config.len() >= 8 && &config[4..8] == b"hvcC" {
        return Ok(CodecType::H265);
    }

    if config.len() >= 23 && config[0] == 0x01 {
        let level = config[12];
        if level > 0 && level <= 186 {
            return Ok(CodecType::H265);
        }
    }

    Err(String::from(
        "Unknown codec (only H.264 and H.265 are supported)",
    ))
}

fn nv12_to_rgba8(nv12: &[u8], width: u32, height: u32) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let y_plane_len = w * h;

    if nv12.len() < y_plane_len + (y_plane_len / 2) {
        return vec![0; y_plane_len * 4];
    }

    let mut rgba = vec![0u8; y_plane_len * 4];

    for y in 0..h {
        for x in 0..w {
            let y_index = y * w + x;
            let uv_row = y / 2;
            let uv_col = (x / 2) * 2;
            let uv_index = y_plane_len + uv_row * w + uv_col;

            let y_sample = nv12[y_index] as i32;
            let u_sample = nv12[uv_index] as i32;
            let v_sample = nv12[uv_index + 1] as i32;

            let c = (y_sample - 16).max(0);
            let d = u_sample - 128;
            let e = v_sample - 128;

            let r = ((298 * c + 409 * e + 128) >> 8).clamp(0, 255) as u8;
            let g = ((298 * c - 100 * d - 208 * e + 128) >> 8).clamp(0, 255) as u8;
            let b = ((298 * c + 516 * d + 128) >> 8).clamp(0, 255) as u8;

            let dst = y_index * 4;
            rgba[dst] = r;
            rgba[dst + 1] = g;
            rgba[dst + 2] = b;
            rgba[dst + 3] = 255;
        }
    }

    rgba
}
