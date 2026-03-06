use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;

use gdk4::prelude::{PaintableExt, TextureExtManual};
use glib::Bytes;
use gtk4::prelude::*;
use gtk4::{Picture, Widget};
use waterui_graphics::filter_view::{AppliedFilter, FilterContext, FilterInput, FilterOutput};
use waterui_graphics::shared_context::{init_shared_context, shared_context};

struct AppliedFilterState {
    filter: AppliedFilter,
    capture_host: gtk4::Box,
    capture_widget: Widget,
    picture: Picture,
    paintable: gtk4::WidgetPaintable,
    setup_done: bool,
    dirty: bool,
    filtered_output_revealed: bool,
    last_size: Option<(i32, i32)>,
}

impl AppliedFilterState {
    fn mark_dirty(&mut self) {
        self.dirty = true;
    }
}

pub fn wrap_filtered_content(content: Widget, filter: AppliedFilter) -> Widget {
    let capture_host = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    capture_host.set_halign(gtk4::Align::Fill);
    capture_host.set_valign(gtk4::Align::Fill);
    capture_host.set_hexpand(true);
    capture_host.set_vexpand(true);
    capture_host.append(&content);

    let picture = Picture::new();
    picture.set_halign(gtk4::Align::Fill);
    picture.set_valign(gtk4::Align::Fill);
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    picture.set_can_target(false);
    picture.set_visible(false);

    let overlay = gtk4::Overlay::new();
    overlay.set_halign(gtk4::Align::Fill);
    overlay.set_valign(gtk4::Align::Fill);
    overlay.set_hexpand(true);
    overlay.set_vexpand(true);
    overlay.set_child(Some(&capture_host));
    overlay.add_overlay(&picture);
    overlay.set_measure_overlay(&picture, false);
    overlay.set_clip_overlay(&picture, true);

    let paintable = gtk4::WidgetPaintable::new(Some(&content));
    let state = Rc::new(RefCell::new(AppliedFilterState {
        filter,
        capture_host,
        capture_widget: content,
        picture,
        paintable,
        setup_done: false,
        dirty: true,
        filtered_output_revealed: false,
        last_size: None,
    }));

    state.borrow().paintable.connect_invalidate_contents({
        let state = state.clone();
        move |_| {
            state.borrow_mut().mark_dirty();
        }
    });
    state.borrow().paintable.connect_invalidate_size({
        let state = state.clone();
        move |_| {
            state.borrow_mut().mark_dirty();
        }
    });

    overlay.add_tick_callback(move |_, _| {
        render_if_needed(&state);
        glib::ControlFlow::Continue
    });

    overlay.upcast()
}

fn render_if_needed(state: &Rc<RefCell<AppliedFilterState>>) {
    let (width, height) = {
        let state = state.borrow();
        let width = state.capture_widget.width();
        let height = state.capture_widget.height();
        if width <= 0 || height <= 0 {
            return;
        }
        (width, height)
    };

    {
        let mut state = state.borrow_mut();
        state.filter.sync_targets();
        let size = (width, height);
        let size_changed = state.last_size != Some(size);
        if !state.dirty && !size_changed && !state.filter.redraw_hint() {
            return;
        }
        state.last_size = Some(size);
    }

    let capture_texture = match capture_widget_texture(state) {
        Some(texture) => texture,
        None => return,
    };
    let (capture_bytes, capture_stride) = download_texture_bytes(&capture_texture);
    let output_bytes = render_filter_output(
        state,
        width as u32,
        height as u32,
        &capture_bytes,
        capture_stride,
    );

    let memory_texture = gdk4::MemoryTexture::new(
        width,
        height,
        gdk4::MemoryFormat::R8g8b8a8,
        &Bytes::from_owned(output_bytes),
        (width as usize) * 4,
    );

    let mut state = state.borrow_mut();
    state.picture.set_paintable(Some(&memory_texture));
    state.picture.set_visible(true);
    if !state.filtered_output_revealed {
        state.capture_host.set_opacity(0.0);
        state.filtered_output_revealed = true;
    }
    state.dirty = state.filter.redraw_hint();
}

fn capture_widget_texture(state: &Rc<RefCell<AppliedFilterState>>) -> Option<gdk4::Texture> {
    let state = state.borrow();
    let native = state.capture_widget.native()?;
    let renderer = native.renderer()?;
    let width = state.capture_widget.width();
    let height = state.capture_widget.height();
    if width <= 0 || height <= 0 {
        return None;
    }

    let snapshot = gtk4::Snapshot::new();
    let image = state.paintable.current_image();
    image.snapshot(&snapshot, f64::from(width), f64::from(height));
    let node = snapshot.to_node()?;
    let viewport = gtk4::graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
    Some(renderer.render_texture(&node, Some(&viewport)))
}

fn download_texture_bytes(texture: &gdk4::Texture) -> (Vec<u8>, usize) {
    let stride = usize::try_from(texture.width())
        .expect("GTK AppliedFilter texture width must fit usize")
        * 4;
    let mut bytes = vec![
        0_u8;
        stride
            * usize::try_from(texture.height())
                .expect("GTK AppliedFilter texture height must fit usize")
    ];
    texture.download(&mut bytes, stride);
    (bytes, stride)
}

fn render_filter_output(
    state: &Rc<RefCell<AppliedFilterState>>,
    width: u32,
    height: u32,
    input_rgba: &[u8],
    input_stride: usize,
) -> Vec<u8> {
    assert!(width > 0, "GTK AppliedFilter width must be positive");
    assert!(height > 0, "GTK AppliedFilter height must be positive");
    let stride_u32 =
        u32::try_from(input_stride).expect("GTK AppliedFilter input stride must fit u32");
    assert!(
        input_rgba.len() >= input_stride * (height as usize),
        "GTK AppliedFilter capture buffer is smaller than expected image size"
    );

    init_shared_context().unwrap_or_else(|error| {
        panic!("GTK AppliedFilter failed to initialize shared GPU context: {error}")
    });
    let shared = shared_context();
    let guard = shared.read();
    let device = guard.device.as_ref();
    let queue = guard.queue.as_ref();

    let mut state = state.borrow_mut();
    if !state.setup_done {
        let filter_context = FilterContext {
            device,
            queue,
            input_format: wgpu::TextureFormat::Rgba8Unorm,
            output_format: wgpu::TextureFormat::Rgba8Unorm,
            pipeline_cache: guard.pipeline_cache.as_ref(),
        };
        ready_now_or_panic(
            state.filter.setup(&filter_context),
            "gtk_applied_filter::render_filter_output::setup",
        );
        state.setup_done = true;
    }

    let texture_size = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let input_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gtk_applied_filter_input"),
        size: texture_size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &input_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        input_rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(stride_u32),
            rows_per_image: Some(height),
        },
        texture_size,
    );

    let output_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gtk_applied_filter_output"),
        size: texture_size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    });

    let input = FilterInput {
        device,
        queue,
        texture: &input_texture,
        view: input_texture.create_view(&wgpu::TextureViewDescriptor::default()),
        format: wgpu::TextureFormat::Rgba8Unorm,
        width,
        height,
    };
    let output = FilterOutput {
        device,
        queue,
        texture: &output_texture,
        view: output_texture.create_view(&wgpu::TextureViewDescriptor::default()),
        format: wgpu::TextureFormat::Rgba8Unorm,
        width,
        height,
    };
    let needs_redraw = state.filter.render(&input, &output) || state.filter.redraw_hint();
    state.dirty = needs_redraw;

    readback_texture_rgba8(device, queue, &output_texture, width, height)
}

fn readback_texture_rgba8(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Vec<u8> {
    const BYTES_PER_PIXEL: u32 = 4;
    const COPY_ALIGNMENT: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

    let unpadded_bpr = width * BYTES_PER_PIXEL;
    let padded_bpr = unpadded_bpr.div_ceil(COPY_ALIGNMENT) * COPY_ALIGNMENT;
    let copy_size = u64::from(padded_bpr * height);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gtk_applied_filter_readback"),
        size: copy_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("gtk_applied_filter_readback_encoder"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_bpr),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);

    let slice = buffer.slice(..);
    let (sender, receiver) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = sender.send(result);
    });
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let map_result = receiver
        .recv()
        .expect("GTK AppliedFilter readback channel must stay open");
    map_result.unwrap_or_else(|error| panic!("GTK AppliedFilter readback mapping failed: {error}"));

    let mapped = slice.get_mapped_range();
    let mut output = vec![0_u8; (width * height * BYTES_PER_PIXEL) as usize];
    for row in 0..height as usize {
        let src_start = row * padded_bpr as usize;
        let src_end = src_start + unpadded_bpr as usize;
        let dst_start = row * unpadded_bpr as usize;
        let dst_end = dst_start + unpadded_bpr as usize;
        output[dst_start..dst_end].copy_from_slice(&mapped[src_start..src_end]);
    }
    drop(mapped);
    buffer.unmap();
    output
}

fn ready_now_or_panic<F>(future: F, scope: &'static str) -> F::Output
where
    F: Future,
{
    let mut future = future;
    let mut future = unsafe { Pin::new_unchecked(&mut future) };
    let mut cx = Context::from_waker(core::task::Waker::noop());
    match future.as_mut().poll(&mut cx) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("{scope}: future returned Pending in synchronous path"),
    }
}
