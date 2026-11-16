//! Runs a [`Effect`] as a `WaterKit` video frame processor.
//!
//! Frames carry explicit media timing, so animated effects follow presentation
//! time and stay deterministic through pause, seek, offline export, frame
//! stepping and playback-rate changes.

use waterkit_video_core::{Error, FrameTiming};
use waterkit_video_processing::{FrameProcessor, GpuTextureFrame, GpuTexturePool, TimedFrame};

use crate::{
    Effect, EffectContext, EffectFrameTiming, EffectInput, EffectOutput, EffectRedrawCallback,
    PipelineCache,
};

/// A Filtrate effect adapted to `WaterKit`'s typed video processing pipeline.
///
/// The processor submits GPU commands without waiting for completion; wgpu
/// queue ordering keeps the returned texture usable by subsequent GPU stages.
/// Its output allocations are recycled after the corresponding frame drops.
pub struct GpuEffectProcessor<E: Effect> {
    effect: E,
    device: wgpu::Device,
    queue: wgpu::Queue,
    input_format: wgpu::TextureFormat,
    output_format: wgpu::TextureFormat,
    texture_pool: GpuTexturePool,
    needs_redraw: bool,
}

impl<E: Effect> std::fmt::Debug for GpuEffectProcessor<E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GpuEffectProcessor")
            .field("input_format", &self.input_format)
            .field("output_format", &self.output_format)
            .field("needs_redraw", &self.needs_redraw())
            .finish_non_exhaustive()
    }
}

impl<E: Effect> GpuEffectProcessor<E> {
    /// Initializes an effect for a device and explicit input/output formats.
    ///
    /// `redraw_callback` lets an interactive host wake when a reactive filter
    /// parameter changes. Offline and continuous video pipelines can pass
    /// `None` because the next input frame already drives rendering.
    ///
    /// # Errors
    ///
    /// Returns a processing error when effect setup fails.
    #[expect(
        clippy::future_not_send,
        reason = "effect setup borrows the single-threaded `PipelineCache` across its await, as the GPU host thread does everywhere else"
    )]
    pub async fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        mut effect: E,
        input_format: wgpu::TextureFormat,
        output_format: wgpu::TextureFormat,
        redraw_callback: Option<EffectRedrawCallback>,
    ) -> Result<Self, Error> {
        if let Some(callback) = redraw_callback {
            effect.set_redraw_callback(callback);
        }
        let pipelines = PipelineCache::default();
        effect
            .setup(&EffectContext {
                device: &device,
                queue: &queue,
                input_format,
                output_format,
                pipelines: &pipelines,
            })
            .await
            .map_err(|error| Error::Processing(error.to_string()))?;

        Ok(Self {
            effect,
            device,
            queue,
            input_format,
            output_format,
            texture_pool: GpuTexturePool::new(),
            needs_redraw: false,
        })
    }

    /// Borrows the configured effect.
    #[must_use]
    pub const fn effect(&self) -> &E {
        &self.effect
    }

    /// Mutably borrows the configured effect.
    pub const fn effect_mut(&mut self) -> &mut E {
        &mut self.effect
    }

    /// Returns whether animation or a parameter update requires another frame.
    #[must_use]
    pub fn needs_redraw(&self) -> bool {
        self.needs_redraw || self.effect.redraw_hint()
    }
}

impl<E: Effect> FrameProcessor<GpuTextureFrame> for GpuEffectProcessor<E> {
    type Output = GpuTextureFrame;

    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            clippy::future_not_send,
            reason = "on wasm32 wgpu devices, queues and textures are not `Send`, so neither is a future holding them"
        )
    )]
    fn process(
        &mut self,
        input: TimedFrame<GpuTextureFrame>,
    ) -> impl Future<Output = Result<TimedFrame<Self::Output>, Error>> {
        core::future::ready(self.encode_frame(input))
    }
}

impl<E: Effect> GpuEffectProcessor<E> {
    fn encode_frame(
        &mut self,
        input: TimedFrame<GpuTextureFrame>,
    ) -> Result<TimedFrame<GpuTextureFrame>, Error> {
        let (input, timing) = input.into_parts();
        if input.format() != self.input_format {
            return Err(Error::Processing(format!(
                "Filtrate processor expected {:?} input but received {:?}",
                self.input_format,
                input.format()
            )));
        }

        let (width, height) = self.effect.output_size(input.width(), input.height());
        let output = self
            .texture_pool
            .acquire(&self.device, width, height, self.output_format);
        let effect_input = EffectInput::new(
            &self.device,
            &self.queue,
            input.texture(),
            effect_timing(timing),
            // Video frames are not laid out in points.
            1.0,
        );
        let effect_output = EffectOutput::new(&self.device, &self.queue, output.texture());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("waterkit video Filtrate frame"),
            });
        self.needs_redraw = self
            .effect
            .encode_render(&effect_input, &effect_output, &mut encoder)
            .map_err(|error| Error::Processing(error.to_string()))?;
        self.queue.submit([encoder.finish()]);
        Ok(TimedFrame::new(output, timing))
    }
}

const fn effect_timing(timing: FrameTiming) -> EffectFrameTiming {
    EffectFrameTiming::new(
        timing.presentation_time(),
        timing.duration(),
        timing.sequence(),
    )
    .with_discontinuity(timing.is_discontinuity())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use waterkit_video_core::FrameTiming;
    use waterkit_video_processing::{FrameProcessor, GpuTextureFrame, TimedFrame};

    use super::{GpuEffectProcessor, effect_timing};
    use crate::{Executor, filters::Brightness};

    #[test]
    fn filtrate_timing_uses_media_time_without_wall_clock_sampling() {
        let media = FrameTiming::new(Duration::from_secs(7), Duration::from_millis(16), 420)
            .with_discontinuity(true);
        let effect = effect_timing(media);

        assert_eq!(effect.presentation_time(), Duration::from_secs(7));
        assert_eq!(effect.delta(), Duration::from_millis(16));
        assert_eq!(effect.sequence(), 420);
        assert!(effect.is_discontinuity());
    }

    #[test]
    fn gpu_effect_processor_preserves_frame_dimensions_and_timing() {
        pollster::block_on(async {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: None,
                    force_fallback_adapter: false,
                    apply_limit_buckets: false,
                })
                .await
                .expect("WaterKit Filtrate test requires a GPU adapter");
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default())
                .await
                .expect("WaterKit Filtrate test requires a working GPU device");
            let format = wgpu::TextureFormat::Rgba8Unorm;
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("WaterKit Filtrate test input"),
                size: wgpu::Extent3d {
                    width: 8,
                    height: 6,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let timing = FrameTiming::new(Duration::from_secs(2), Duration::from_millis(16), 120);
            let mut processor = GpuEffectProcessor::new(
                device.clone(),
                queue,
                Executor::new(Brightness(0.25_f32)),
                format,
                format,
                None,
            )
            .await
            .expect("Filtrate processor setup must succeed");
            let output = processor
                .process(TimedFrame::new(GpuTextureFrame::new(texture), timing))
                .await
                .expect("Filtrate frame processing must succeed");

            assert_eq!(output.frame().width(), 8);
            assert_eq!(output.frame().height(), 6);
            assert_eq!(output.frame().format(), format);
            assert_eq!(output.timing(), timing);
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
        });
    }
}
