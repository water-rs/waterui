//! Engine execution of composed filtrate filters and custom effects.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use rustc_hash::FxHashMap;

use cherenkov::{RenderError, SurfaceVisibility, WakeGate};
use filtrate::{
    Effect, EffectContext, EffectFrameTiming, EffectInput, EffectOutput, ShapeTextures,
};

/// An effect moved to the render thread before its device resources exist.
pub struct EffectBox(pub(crate) Box<dyn Source>);

impl std::fmt::Debug for EffectBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectBox").finish_non_exhaustive()
    }
}

impl<E: Effect + cherenkov::RenderTransfer> From<E> for EffectBox {
    fn from(effect: E) -> Self {
        Self(Box::new(effect))
    }
}

pub trait Source: cherenkov::RenderTransfer {
    fn build(self: Box<Self>) -> Box<dyn Runnable>;
}

impl<E: Effect + cherenkov::RenderTransfer> Source for E {
    fn build(self: Box<Self>) -> Box<dyn Runnable> {
        self
    }
}

pub struct FromFilter<F>(pub F);

impl<F: filtrate_core::Filter + cherenkov::RenderTransfer> Source for FromFilter<F> {
    fn build(self: Box<Self>) -> Box<dyn Runnable> {
        Box::new(filtrate::Executor::new(self.0))
    }
}

/// A filter chain registered for a backdrop group capture.
pub struct FromBackdropChain<K, F>(pub F, pub std::marker::PhantomData<fn() -> K>);

impl<K, F> Source for FromBackdropChain<K, F>
where
    K: filtrate_core::kind::Kind,
    F: cherenkov::BackdropChain<K> + cherenkov::RenderTransfer,
{
    fn build(self: Box<Self>) -> Box<dyn Runnable> {
        Box::new(BackdropRunnable::<K, F>(
            filtrate::Executor::new(self.0),
            std::marker::PhantomData,
        ))
    }
}

/// An `Executor` over a backdrop chain, reporting its footprint bound.
struct BackdropRunnable<K, F: filtrate_core::Filter>(
    filtrate::Executor<F>,
    std::marker::PhantomData<fn() -> K>,
);

impl<K, F> Runnable for BackdropRunnable<K, F>
where
    K: filtrate_core::kind::Kind,
    F: cherenkov::BackdropChain<K> + cherenkov::RenderTransfer,
{
    #[cfg(not(target_arch = "wasm32"))]
    fn setup(&mut self, ctx: &EffectContext<'_>) -> Result<(), filtrate::EffectSetupError> {
        Runnable::setup(&mut self.0, ctx)
    }

    #[cfg(target_arch = "wasm32")]
    fn setup<'a>(
        &'a mut self,
        ctx: &'a EffectContext<'a>,
    ) -> core::pin::Pin<
        Box<dyn core::future::Future<Output = Result<(), filtrate::EffectSetupError>> + 'a>,
    > {
        Runnable::setup(&mut self.0, ctx)
    }

    fn encode(
        &mut self,
        input: &EffectInput<'_>,
        output: &EffectOutput<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<bool, filtrate::EffectRenderError> {
        Runnable::encode(&mut self.0, input, output, encoder)
    }

    fn set_redraw(&mut self, callback: filtrate::EffectRedrawCallback) {
        Runnable::set_redraw(&mut self.0, callback);
    }

    fn footprint_bound(&mut self) -> Option<filtrate_core::Footprint> {
        Some(<F as cherenkov::BackdropChain<K>>::footprint_bound(
            &self.0.param_bounds(),
        ))
    }
}

/// A registered filter's identity: a layer filter or a backdrop group's
/// capture chain on a surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FilterKey {
    /// A layer's filter (`FilterId`'s raw value).
    Layer(u64),
    /// A backdrop group's capture chain.
    Backdrop {
        /// The surface's raw id.
        surface: u64,
        /// The group's raw id.
        group: u64,
    },
}

pub trait Runnable {
    /// The filter's spatial footprint bound; `None` for effects without
    /// one (a colour filter contributes no reach either way).
    fn footprint_bound(&mut self) -> Option<filtrate_core::Footprint> {
        None
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn setup(&mut self, ctx: &EffectContext<'_>) -> Result<(), filtrate::EffectSetupError>;
    #[cfg(target_arch = "wasm32")]
    fn setup<'a>(
        &'a mut self,
        ctx: &'a EffectContext<'a>,
    ) -> core::pin::Pin<
        Box<dyn core::future::Future<Output = Result<(), filtrate::EffectSetupError>> + 'a>,
    >;
    fn encode(
        &mut self,
        input: &EffectInput<'_>,
        output: &EffectOutput<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<bool, filtrate::EffectRenderError>;
    fn set_redraw(&mut self, callback: filtrate::EffectRedrawCallback);
}

impl<E: Effect> Runnable for E {
    #[cfg(not(target_arch = "wasm32"))]
    fn setup(&mut self, ctx: &EffectContext<'_>) -> Result<(), filtrate::EffectSetupError> {
        pollster::block_on(Effect::setup(self, ctx))
    }

    #[cfg(target_arch = "wasm32")]
    fn setup<'a>(
        &'a mut self,
        ctx: &'a EffectContext<'a>,
    ) -> core::pin::Pin<
        Box<dyn core::future::Future<Output = Result<(), filtrate::EffectSetupError>> + 'a>,
    > {
        Box::pin(Effect::setup(self, ctx))
    }
    fn encode(
        &mut self,
        input: &EffectInput<'_>,
        output: &EffectOutput<'_>,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<bool, filtrate::EffectRenderError> {
        self.encode_render(input, output, encoder)
    }
    fn set_redraw(&mut self, callback: filtrate::EffectRedrawCallback) {
        self.set_redraw_callback(callback);
    }
}

struct Entry {
    effect: Box<dyn Runnable>,
    dirty: Arc<AtomicBool>,
    /// Open while a visible surface's frame runs the filter.
    gate: Arc<WakeGate>,
    again: bool,
    sequence: Option<u64>,
    setup: Option<Result<(), String>>,
    #[cfg(target_arch = "wasm32")]
    setup_pending_frame: bool,
    /// Input/output targets per size, capped; a frame can apply one filter
    /// to several region sizes (#117 sparse backdrop captures).
    io: Vec<((u32, u32), FilterTargets)>,
}

/// The input and output targets of one size.
struct FilterTargets {
    input: (wgpu::Texture, wgpu::TextureView),
    output: (wgpu::Texture, wgpu::TextureView),
    /// The frame sequence that last applied through these targets; a size
    /// unused for a whole frame is dropped rather than retained.
    last_used: u64,
}

impl Entry {
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            clippy::needless_pass_by_ref_mut,
            reason = "wasm32 only reads the pre-prepared setup; native builds need &mut for lazy setup"
        )
    )]
    fn check_setup(
        &mut self,
        id: FilterKey,
        context: &EffectContext<'_>,
        format: wgpu::TextureFormat,
    ) -> Result<(), RenderError> {
        #[cfg(not(target_arch = "wasm32"))]
        let setup = self.setup.get_or_insert_with(|| {
            self.effect
                .setup(&EffectContext {
                    input_format: format,
                    output_format: format,
                    ..*context
                })
                .map_err(|error| error.to_string())
        });
        #[cfg(target_arch = "wasm32")]
        let setup = {
            let _ = (context, format);
            self.setup
                .as_ref()
                .expect("browser prepares active filters before encoding")
        };
        setup
            .as_ref()
            .copied()
            .map_err(|error| RenderError::Render(format!("filter {id:?} setup: {error}")))
    }

    /// The index of `size`'s input/output targets, allocating on first use.
    fn targets_index(
        &mut self,
        device: &wgpu::Device,
        size: (u32, u32),
        format: wgpu::TextureFormat,
        sequence: u64,
    ) -> usize {
        // A used size is live this frame: mark it before stale sizes are
        // dropped so steady state reuses its targets.
        if let Some(index) = self.io.iter().position(|(io_size, _)| *io_size == size) {
            self.io[index].1.last_used = sequence;
        }
        // Stale sizes go first so the lookup below cannot pick them up
        // and the cap stays a bound on live sizes only. A size stays live
        // through the frame after its last use: a frame encoding several
        // region sizes would otherwise drop the earlier ones and
        // reallocate them on the next frame.
        self.io
            .retain(|(_, targets)| targets.last_used.saturating_add(1) >= sequence);
        self.io
            .iter()
            .position(|(io_size, _)| *io_size == size)
            .unwrap_or_else(|| {
                const MAX_FILTER_TARGET_SIZES: usize = 4;
                if self.io.len() == MAX_FILTER_TARGET_SIZES {
                    let evicted = self.io.remove(0).1;
                    for (label, (texture, _)) in [
                        ("filter input", evicted.input),
                        ("filter output", evicted.output),
                    ] {
                        crate::diag::retire(
                            device,
                            crate::diag::RetireArgs {
                                label,
                                class: crate::diag::Class::Target,
                                bytes: u64::from(texture.width())
                                    * u64::from(texture.height())
                                    * super::texel_bytes(texture.format()),
                                used_in_latest_submit: true,
                                reason: "filter size eviction",
                            },
                        );
                    }
                }
                self.io.push((
                    size,
                    FilterTargets {
                        input: super::create_target(
                            device,
                            "filter input",
                            size,
                            super::TARGET_USAGES,
                            format,
                        ),
                        output: super::create_target(
                            device,
                            "filter output",
                            size,
                            super::TARGET_USAGES,
                            format,
                        ),
                        last_used: sequence,
                    },
                ));
                let created = u64::from(size.0) * u64::from(size.1) * super::texel_bytes(format);
                for label in ["filter input", "filter output"] {
                    crate::diag::grow(
                        device,
                        label,
                        crate::diag::Class::Target,
                        0,
                        created,
                        0,
                        true,
                    );
                }
                self.io.len() - 1
            })
    }
}

pub struct Registry {
    entries: FxHashMap<FilterKey, Entry>,
    host: Option<crate::interop::RedrawCallback>,
}

impl Registry {
    pub fn new(host: Option<crate::interop::RedrawCallback>) -> Self {
        Self {
            entries: FxHashMap::default(),
            host,
        }
    }

    pub fn add(&mut self, id: FilterKey, source: Box<dyn Source>) {
        let mut effect = source.build();
        let dirty = Arc::new(AtomicBool::new(false));
        let gate = Arc::new(WakeGate::default());
        let (request, open, host) = (Arc::clone(&dirty), Arc::clone(&gate), self.host.clone());
        effect.set_redraw(Arc::new(move || {
            if !request.swap(true, Ordering::AcqRel)
                && open.is_open()
                && let Some(host) = &host
            {
                host.wake();
            }
        }));
        self.entries.insert(
            id,
            Entry {
                effect,
                dirty,
                gate,
                again: false,
                sequence: None,
                setup: None,
                #[cfg(target_arch = "wasm32")]
                setup_pending_frame: false,
                io: Vec::new(),
            },
        );
    }

    /// Removes an entry; returns the GPU bytes its retired
    /// input/output targets held (for the allocation diagnostic).
    pub fn remove(&mut self, id: FilterKey) -> u64 {
        let Some(entry) = self.entries.remove(&id) else {
            return 0;
        };
        entry.gate.close();
        entry
            .io
            .iter()
            .flat_map(|(_, targets)| [&targets.input.0, &targets.output.0])
            .map(|texture| {
                u64::from(texture.width())
                    * u64::from(texture.height())
                    * super::texel_bytes(texture.format())
            })
            .sum()
    }

    /// Sets each filter's wake gate to the surfaces whose frames run it;
    /// a filter no frame runs wakes nothing.
    pub fn set_surfaces(&self, uses: &FxHashMap<FilterKey, Vec<SurfaceVisibility>>) {
        for (id, entry) in &self.entries {
            entry
                .gate
                .set(uses.get(id).map(Vec::as_slice).unwrap_or_default());
        }
    }

    pub fn wants_redraw(&self, id: FilterKey) -> bool {
        self.entries
            .get(&id)
            .is_some_and(|entry| entry.again || entry.dirty.load(Ordering::Acquire))
    }

    /// Drops every entry's input/output targets — grow-only caches the
    /// renderer releases explicitly outside the hot frame path (#169 A4).
    /// Returns the freed bytes for diagnostics.
    pub(super) fn trim(&mut self) -> u64 {
        let mut bytes = 0;
        for entry in self.entries.values_mut() {
            for (_, targets) in std::mem::take(&mut entry.io) {
                for texture in [targets.input.0, targets.output.0] {
                    bytes += u64::from(texture.width())
                        * u64::from(texture.height())
                        * super::texel_bytes(texture.format());
                }
            }
        }
        bytes
    }

    pub fn gpu_bytes(&self) -> u64 {
        self.entries
            .values()
            .flat_map(|entry| entry.io.iter())
            .flat_map(|(_, targets)| [&targets.input.0, &targets.output.0])
            .map(|texture| {
                u64::from(texture.width())
                    * u64::from(texture.height())
                    * if texture.format() == wgpu::TextureFormat::Rgba16Float {
                        8
                    } else {
                        4
                    }
            })
            .sum()
    }

    /// The registered filter's footprint bound; `None` when `key` is not
    /// registered (an error for callers).
    pub fn footprint_bound(&mut self, key: FilterKey) -> Option<filtrate_core::Footprint> {
        self.entries
            .get_mut(&key)
            .and_then(|entry| entry.effect.footprint_bound())
    }

    pub(super) fn apply(
        &mut self,
        id: FilterKey,
        context: &EffectContext<'_>,
        scratch: &super::ScratchTarget,
        size: (u32, u32),
        timing: EffectFrameTiming,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<(), RenderError> {
        let (device, queue) = (context.device, context.queue);
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or_else(|| RenderError::Render(format!("unregistered filter {id:?}")))?;
        let repeated = entry.sequence == Some(timing.sequence());
        let timing = if repeated {
            EffectFrameTiming::new(
                timing.presentation_time(),
                std::time::Duration::ZERO,
                timing.sequence(),
            )
            .with_discontinuity(timing.is_discontinuity())
        } else {
            timing
        };
        if !repeated {
            #[cfg(not(target_arch = "wasm32"))]
            {
                entry.dirty.swap(false, Ordering::AcqRel);
                entry.again = false;
            }
            #[cfg(target_arch = "wasm32")]
            {
                // Browser setup consumes the initial request before awaiting.
                // A request arriving during setup still needs a second frame.
                let requested = entry.dirty.swap(false, Ordering::AcqRel);
                entry.again = std::mem::take(&mut entry.setup_pending_frame) && requested;
            }
            entry.sequence = Some(timing.sequence());
        }
        let format = scratch.texture.format();
        entry.check_setup(id, context, format)?;
        let targets_index = entry.targets_index(device, size, format, timing.sequence());
        let targets = &entry.io[targets_index].1;
        let (input_texture, input_view) = &targets.input;
        let (output_texture, output_view) = &targets.output;
        let extent = wgpu::Extent3d {
            width: size.0,
            height: size.1,
            depth_or_array_layers: 1,
        };
        encoder.copy_texture_to_texture(
            scratch.texture.as_image_copy(),
            input_texture.as_image_copy(),
            extent,
        );
        let input = EffectInput {
            device,
            queue,
            texture: input_texture,
            view: input_view.clone(),
            format,
            width: size.0,
            height: size.1,
            timing,
            shape: ShapeTextures::default(),
        };
        let output = EffectOutput {
            device,
            queue,
            texture: output_texture,
            view: output_view.clone(),
            format,
            width: size.0,
            height: size.1,
        };
        entry.again |= entry
            .effect
            .encode(&input, &output, encoder)
            .map_err(|error| RenderError::Render(format!("filter {id:?}: {error}")))?;
        encoder.copy_texture_to_texture(
            output_texture.as_image_copy(),
            scratch.texture.as_image_copy(),
            extent,
        );
        Ok(())
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        for entry in self.entries.values() {
            entry.gate.close();
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl Registry {
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    pub(super) async fn prepare(
        &mut self,
        id: FilterKey,
        context: &EffectContext<'_>,
    ) -> Result<(), RenderError> {
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or_else(|| RenderError::Render(format!("unregistered filter {id:?}")))?;
        if entry.setup.is_none() {
            entry.dirty.swap(false, Ordering::AcqRel);
            entry.setup_pending_frame = true;
            entry.setup = Some(
                entry
                    .effect
                    .setup(context)
                    .await
                    .map_err(|error| error.to_string()),
            );
        }
        entry
            .setup
            .as_ref()
            .expect("setup completed")
            .as_ref()
            .copied()
            .map_err(|error| RenderError::Render(format!("filter {id:?} setup: {error}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use filtrate::filters::GaussianBlur;

    /// An adapter plus device, or `None` where no GPU exists.
    fn device_and_queue() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()))
            .into_iter()
            .next()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
    }

    fn entry() -> Entry {
        Entry {
            effect: Box::new(filtrate::Executor::new(GaussianBlur(2.0_f32))),
            dirty: Arc::new(AtomicBool::new(false)),
            gate: Arc::new(WakeGate::default()),
            again: false,
            sequence: None,
            setup: None,
            #[cfg(target_arch = "wasm32")]
            setup_pending_frame: false,
            io: Vec::new(),
        }
    }

    /// A size the previous frame used stays live; a size unused for a
    /// whole frame is dropped. Steady state with two alternating sizes
    /// keeps both sets of targets instead of reallocating one per frame.
    #[test]
    fn targets_index_keeps_the_previous_frame_sizes() {
        let Some((device, _queue)) = device_and_queue() else {
            eprintln!("no GPU adapter; skipping");
            return;
        };
        let format = wgpu::TextureFormat::Rgba16Float;
        let size_a = (64, 64);
        let size_b = (64, 32);
        let mut entry = entry();
        entry.targets_index(&device, size_a, format, 1);
        entry.targets_index(&device, size_b, format, 1);
        entry.targets_index(&device, size_a, format, 2);
        assert!(
            entry.io.iter().any(|(size, _)| *size == size_b),
            "a size the previous frame used is still live"
        );
        let index = entry.targets_index(&device, size_b, format, 2);
        let input_b = entry.io[index].1.input.0.clone();
        assert!(
            input_b
                == entry
                    .io
                    .iter()
                    .find(|(size, _)| *size == size_b)
                    .expect("size B targets")
                    .1
                    .input
                    .0,
            "size B's targets are reused, not reallocated"
        );
        entry.targets_index(&device, size_a, format, 3);
        entry.targets_index(&device, size_a, format, 4);
        assert!(
            !entry.io.iter().any(|(size, _)| *size == size_b),
            "a size unused for a whole frame is dropped"
        );
    }
}
