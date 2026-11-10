//! The HLSL behind the embedded DXIL is byte-identical to what wgpu-hal's
//! DX12 backend writes for the same module, entry points and pipeline
//! layout (issue #2726). Every fixed pipeline builds once through the
//! trusted-WGSL path on a DX12 device with wgpu-hal's shader log
//! captured, and each logged stage must equal the `.hlsl` `build.rs`
//! compiled to DXIL.

use std::path::Path;
use std::sync::{Arc, Mutex, Once};

use super::{ComputeProgram, ENGINE, RenderProgram, ShaderDelivery};
use crate::interop::wgpu;
use crate::render::{bindings, layout_entries, programs};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;

/// wgpu-hal's `Naga generated shader for {entry:?} at {stage:?}:\n{hlsl}`
/// debug records, prefix stripped.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<String>>>);

impl Capture {
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut self.0.lock().unwrap())
    }
}

#[derive(Default)]
struct Message(Option<String>);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = Some(format!("{value:?}"));
        }
    }
}

impl<S> Layer<S> for Capture
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        if *event.metadata().level() > tracing::Level::DEBUG {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        if let Some(text) = message.0.and_then(|text| {
            text.strip_prefix("Naga generated shader for ")
                .map(str::to_owned)
        }) {
            self.0.lock().unwrap().push(text);
        }
    }
}

/// A WARP DX12 device whose wgpu-hal emits shader model 6.0 HLSL, the
/// model `build.rs` writes, with the stage log captured.
struct Harness {
    device: wgpu::Device,
    capture: Capture,
    _guard: tracing::dispatcher::DefaultGuard,
}

impl Harness {
    fn new() -> Self {
        static LOG_TRACER: Once = Once::new();
        LOG_TRACER.call_once(|| {
            tracing_log::LogTracer::init_with_filter(tracing_log::log::LevelFilter::Debug)
                .expect("the log-to-tracing forwarder");
        });
        let capture = Capture::default();
        let guard =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(capture.clone()));
        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = wgpu::Backends::DX12;
        // SAFETY: caps the compiler at the shader model `build.rs` targets;
        // native half arithmetic requires shader model 6.2.
        descriptor.backend_options.dx12.force_shader_model =
            unsafe { wgpu::ForceShaderModelToken::with_shader_model(wgpu::DxcShaderModel::V6_2) };
        let instance = wgpu::Instance::new(descriptor);
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            force_fallback_adapter: true,
            ..wgpu::RequestAdapterOptions::default()
        }))
        .expect("the WARP adapter");
        let (device, _queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                required_features: adapter.features(),
                required_limits: adapter.limits(),
                ..wgpu::DeviceDescriptor::default()
            }))
            .expect("a WARP device");
        Self {
            device,
            capture,
            _guard: guard,
        }
    }

    fn layout(&self, groups: &[&[bindings::Entry]]) -> wgpu::PipelineLayout {
        let layouts: Vec<_> = groups
            .iter()
            .map(|entries| {
                self.device
                    .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: None,
                        entries: &layout_entries(entries),
                    })
            })
            .collect();
        self.device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &layouts.iter().map(Some).collect::<Vec<_>>(),
                immediate_size: 0,
            })
    }

    /// Builds `program` of module `name` on `groups`; no color target and
    /// a depth attachment keep every fragment stage valid.
    fn render(
        &self,
        name: &str,
        groups: &[&[bindings::Entry]],
        program: &RenderProgram,
        spec: programs::Program,
    ) {
        let programs::Program::Render { vertex, fragment } = spec else {
            panic!("{name}: a render program");
        };
        assert_eq!(
            (program.vertex_entry, program.fragment_entry),
            (vertex, fragment),
            "{name}: the program's stages"
        );
        let layout = self.layout(groups);
        self.capture.take();
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let _pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(name),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &program.vertex,
                    entry_point: Some(vertex),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &program.fragment,
                    entry_point: Some(fragment),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
        if let Some(error) = pollster::block_on(scope.pop()) {
            panic!("{name} {vertex}/{fragment}: {error}");
        }
        self.expect(&[
            (vertex, "Vertex", format!("{name}.{vertex}.{fragment}.vs")),
            (fragment, "Fragment", format!("{name}.{fragment}.ps")),
        ]);
    }

    /// Builds coverage kernel `program` on its declared layout.
    fn compute(&self, program: &ComputeProgram) {
        let layout = self.layout(program.layout.groups());
        let entry = program.entry;
        self.capture.take();
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let _pipeline = self
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(&layout),
                module: &program.module,
                entry_point: Some(entry),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });
        if let Some(error) = pollster::block_on(scope.pop()) {
            panic!("coverage {entry}: {error}");
        }
        self.expect(&[(entry, "Compute", format!("coverage.{entry}.cs"))]);
    }

    /// The stages logged since the last build, each once and in
    /// wgpu-hal's order, against `build.rs`'s `<stem>.hlsl`.
    fn expect(&self, stages: &[(&str, &str, String)]) {
        let logged = self.capture.take();
        assert_eq!(logged.len(), stages.len(), "{stages:?}: logged stages");
        for (entry, stage, stem) in stages {
            let header = format!("{entry:?} at {stage}:\n");
            let source = logged
                .iter()
                .find_map(|text| text.strip_prefix(&header))
                .unwrap_or_else(|| panic!("{stem}: wgpu-hal logged no {header}"));
            let path = Path::new(env!("OUT_DIR")).join(format!("{stem}.hlsl"));
            let built = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            if let Some((line, (logged, built))) = source
                .lines()
                .zip(built.lines())
                .enumerate()
                .find(|(_, (logged, built))| logged != built)
            {
                panic!(
                    "{stem}: line {}: wgpu-hal wrote `{logged}`, build.rs `{built}`",
                    line + 1
                );
            }
            assert!(source == built, "{stem}: wgpu-hal's HLSL differs in length");
        }
    }
}

#[test]
fn every_fixed_pipeline_matches_the_embedded_hlsl() {
    let harness = Harness::new();
    let device = &harness.device;
    let wgsl = ShaderDelivery::Wgsl;
    for variant in 0..ENGINE.len() {
        let programs = wgsl.engine_programs(device, variant, wgpu::TextureFormat::Rgba16Float);
        for (program, spec) in programs.iter().zip(programs::ENGINE) {
            let name = format!("engine{variant}");
            harness.render(&name, bindings::ENGINE_GROUPS, program, *spec);
        }
    }
    let single = [
        (
            "present",
            bindings::PRESENT_GROUPS,
            wgsl.present_program(device),
            programs::MAIN[0],
        ),
        (
            "external",
            bindings::EXTERNAL_GROUPS,
            wgsl.external_program(device),
            programs::EXTERNAL[0],
        ),
        (
            "projective",
            bindings::PROJECTIVE_GROUPS,
            wgsl.projective_program(device),
            programs::PROJECTIVE[0],
        ),
        (
            "mip",
            bindings::MIP_GROUPS,
            wgsl.mip_program(device),
            programs::MAIN[0],
        ),
        (
            "resolve",
            bindings::RESOLVE_GROUPS,
            wgsl.resolve_program(device),
            programs::MAIN[0],
        ),
        (
            "reduce",
            bindings::REDUCE_GROUPS,
            wgsl.reduce_program(device),
            programs::MAIN[0],
        ),
        (
            "shadow",
            bindings::SHADOW_GROUPS,
            wgsl.shadow_program(device),
            programs::SHADOW[0],
        ),
    ];
    for (name, groups, program, spec) in &single {
        harness.render(name, groups, program, *spec);
    }
    let conversions = wgsl.space_convert_programs(device);
    for (program, spec) in conversions.iter().zip(programs::SPACE_CONVERT) {
        harness.render(
            "space_convert",
            bindings::SPACE_CONVERT_GROUPS,
            program,
            *spec,
        );
    }
    let kernels = wgsl.coverage_programs(device);
    assert_eq!(kernels.len(), programs::COVERAGE.len());
    for (program, spec) in kernels.iter().zip(programs::COVERAGE) {
        let programs::Program::Compute { entry, layout } = *spec else {
            panic!("programs::COVERAGE is compute only");
        };
        assert_eq!(program.entry, entry);
        assert_eq!(program.layout, layout);
        harness.compute(program);
    }
}
