//! Exercises the Metal attachment interface through wgpu passthrough.
#![cfg(target_vendor = "apple")]

use std::borrow::Cow;

use objc2_metal::{MTLResource, MTLStorageMode};

#[test]
fn memoryless_producer_and_consumer_share_one_wgpu_pass() {
    attachment_read("produce", "composite", [0.75, 0.5, 2.0, 1.0]);
}

#[test]
fn float_attachment_reads_preserve_rgba16f_storage_rounding() {
    attachment_read("produce_float", "composite_float", [0.0; 4]);
}

#[expect(
    clippy::too_many_lines,
    reason = "the fixture covers one complete native attachment contract"
)]
fn attachment_read(producer: &str, consumer: &str, expected: [f32; 4]) {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::METAL,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .expect("a Metal adapter is required");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: wgpu::Features::PASSTHROUGH_SHADERS,
        ..Default::default()
    }))
    .expect("Metal passthrough device");
    // SAFETY: the fixture declares exactly the entry points and attachment
    // formats below, has no resources, and only indexes its three vertices.
    let module = unsafe {
        device.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
            label: Some("attachment-read contract"),
            metallib: Some(Cow::Borrowed(include_bytes!(concat!(
                env!("OUT_DIR"),
                "/attachment_read.metallib"
            )))),
            entry_points: Cow::Owned(
                [
                    "vertex_main",
                    "produce",
                    "composite",
                    "produce_float",
                    "composite_float",
                ]
                .into_iter()
                .map(|name| wgpu::PassthroughShaderEntryPoint {
                    name: Cow::Borrowed(name),
                    workgroup_size: (0, 0, 0),
                })
                .collect(),
            ),
            ..Default::default()
        })
    };
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[],
        immediate_size: 0,
    });
    let pipeline = |entry, destination| {
        let targets = [0, 1].map(|slot| {
            Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba16Float,
                blend: None,
                write_mask: if slot == destination {
                    wgpu::ColorWrites::ALL
                } else {
                    wgpu::ColorWrites::empty()
                },
            })
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(entry),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vertex_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &targets,
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    };
    let produce = pipeline(producer, 1);
    let composite = pipeline(consumer, 0);
    let texture = |label, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: 7,
                height: 3,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage,
            view_formats: &[],
        })
    };
    let target = texture(
        "persistent",
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
    );
    let temporary = texture(
        "memoryless",
        wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TRANSIENT_ATTACHMENT,
    );
    // SAFETY: the guard keeps the wgpu texture alive; this reads its storage
    // mode without changing any native state or resource ownership.
    {
        let temporary_hal = unsafe { temporary.as_hal::<wgpu::hal::metal::Api>() }.unwrap();
        assert_eq!(
            temporary_hal.raw_handle().storageMode(),
            MTLStorageMode::Memoryless
        );
    }
    let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let temporary_view = temporary.create_view(&wgpu::TextureViewDescriptor::default());
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("attachment-read result"),
        size: 3 * 256,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("producer and consumer"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &temporary_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Discard,
                    },
                }),
            ],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&produce);
        pass.draw(0..3, 0..1);
        pass.set_pipeline(&composite);
        pass.draw(0..3, 0..1);
    }
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(3),
            },
        },
        target.size(),
    );
    let (tx, rx) = std::sync::mpsc::channel();
    encoder.map_buffer_on_submit(&readback, wgpu::MapMode::Read, .., move |result| {
        tx.send(result).unwrap();
    });
    let submission = queue.submit([encoder.finish()]);
    device
        .poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(30)),
        })
        .unwrap();
    rx.recv().unwrap().unwrap();
    let data = readback.get_mapped_range(..).unwrap();
    let expected = expected.map(|v| half::f16::from_f32(v).to_bits());
    for row in data.as_chunks::<256>().0 {
        for pixel in row[..7 * 8].as_chunks::<8>().0 {
            assert_eq!(bytemuck::cast_slice::<u8, u16>(pixel), expected);
        }
    }
}
