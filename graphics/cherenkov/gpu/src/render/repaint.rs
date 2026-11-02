//! Restrict raster work without changing the viewport or paint coordinates.

use cherenkov::damage::Region;

pub fn intersection(a: [u32; 4], b: [u32; 4]) -> Option<[u32; 4]> {
    let x = a[0].max(b[0]);
    let y = a[1].max(b[1]);
    let right = (a[0] + a[2]).min(b[0] + b[2]);
    let bottom = (a[1] + a[3]).min(b[1] + b[3]);
    (x < right && y < bottom).then(|| [x, y, right - x, bottom - y])
}

#[derive(Debug)]
pub struct Scissors<'a> {
    region: &'a Region,
    target: [u32; 4],
    clip: [u32; 4],
}

impl<'a> Scissors<'a> {
    pub const fn new(region: &'a Region, target: [u32; 4]) -> Self {
        Self {
            region,
            target,
            clip: target,
        }
    }

    #[cfg(target_vendor = "apple")]
    pub const fn within(mut self, clip: [u32; 4]) -> Self {
        self.clip = clip;
        self
    }

    pub fn rectangles(&self) -> impl Iterator<Item = [u32; 4]> + '_ {
        self.region.rectangles().iter().filter_map(move |rect| {
            let visible = intersection(rect.xywh().into(), self.target)?;
            let [x, y, w, h] = intersection(visible, self.clip)?;
            Some([x - self.target[0], y - self.target[1], w, h])
        })
    }

    pub fn draw_count(&self) -> u32 {
        u32::try_from(self.rectangles().count()).expect("damage rectangle count")
    }

    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        mut draw: impl FnMut(&mut wgpu::RenderPass<'_>),
    ) {
        for [x, y, width, height] in self.rectangles() {
            pass.set_scissor_rect(x, y, width, height);
            draw(pass);
        }
    }
}

/// Clear quads are upload data, not scene commands or retained geometry.
#[derive(Default)]
pub struct Clears {
    instances: Vec<Option<u32>>,
}

impl Clears {
    pub const fn bytes(&self) -> u64 {
        (self.instances.capacity() * size_of::<Option<u32>>()) as u64
    }

    pub fn instance(&self, pass: usize) -> Option<u32> {
        self.instances[pass]
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "device pixel coordinates fit f32"
    )]
    pub fn prepare(
        &mut self,
        frame: &mut super::lower::Frame,
        demand: &super::composite::demand::Demand,
    ) {
        self.instances.clear();
        for (index, pass) in frame.passes.iter().enumerate() {
            let [x, y, w, h] = pass.region;
            let partial = !demand.pass(index).is_empty()
                && demand.pass(index).pixel_area() < u64::from(w) * u64::from(h);
            let clear = pass.clear.filter(|_| {
                partial
                    && matches!(
                        pass.target,
                        super::lower::Target::Part(_)
                            | super::lower::Target::Plane(_)
                            | super::lower::Target::Projected(_)
                    )
            });
            self.instances.push(clear.map(|[r, g, b, a]| {
                let instance =
                    u32::try_from(frame.instances.len()).expect("instance count fits u32");
                let mut quad = super::instance::Instance::new(super::instance::KIND_SPAN);
                quad.bounds = [x as f32, y as f32, (x + w) as f32, (y + h) as f32];
                // Pass clears are premultiplied; the simple shader premultiplies
                // its input exactly once. Transparent clears carry zero RGB.
                quad.color = if a == 0.0 {
                    [0.0; 4]
                } else {
                    [r / a, g / a, b / a, a]
                };
                frame.instances.push(quad);
                instance
            }));
        }
    }
}

pub struct ClearDraw<'a> {
    pub timestamps: Option<wgpu::RenderPassTimestampWrites<'a>>,
    pub pipeline: &'a wgpu::RenderPipeline,
    pub bind0: &'a wgpu::BindGroup,
    pub bind1: &'a wgpu::BindGroup,
    pub offset: u32,
    pub instance: u32,
}

impl ClearDraw<'_> {
    #[expect(
        clippy::cast_precision_loss,
        reason = "device pixel coordinates fit f32"
    )]
    pub fn encode(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        scissors: &Scissors<'_>,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("repaint clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: self.timestamps.clone(),
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(
            0.0,
            0.0,
            scissors.target[2] as f32,
            scissors.target[3] as f32,
            0.0,
            1.0,
        );
        pass.set_pipeline(self.pipeline);
        pass.set_bind_group(0, self.bind0, &[self.offset]);
        pass.set_bind_group(1, self.bind1, &[]);
        scissors.draw(&mut pass, |pass| {
            pass.draw(0..6, self.instance..self.instance + 1);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disjoint_damage_keeps_holes_and_target_coordinates() {
        let mut region = Region::new((100, 100));
        region.add(10, 20, 15, 10);
        region.add(40, 25, 20, 20);
        let scissors = Scissors::new(&region, [20, 20, 30, 20]);
        assert_eq!(
            scissors.rectangles().collect::<Vec<_>>(),
            [[0, 0, 5, 10], [20, 5, 10, 15]]
        );
        assert_eq!(
            Scissors::new(&region, [60, 60, 10, 10])
                .rectangles()
                .count(),
            0
        );
    }
}

#[cfg(test)]
#[path = "repaint_tests.rs"]
mod render_tests;
