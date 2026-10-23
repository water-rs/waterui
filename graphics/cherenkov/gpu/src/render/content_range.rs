//! Content-derived output range, without inspecting rendered pixels.

use cherenkov::{BlendMode, BlendSpace};
use rustc_hash::FxHashMap;

use super::{
    GpuImage, image_storage, instance,
    prepared::{Op, ResolvedPaint},
    present::ColorRange,
};

/// Working-space colors use straight alpha. Any bounded Display P3 color
/// can use encoded SDR P3 output; values outside that cube need float output.
pub(super) fn working(color: [f32; 4], ceiling: f32) -> ColorRange {
    if color[..3]
        .iter()
        .all(|value| (0.0..=ceiling).contains(value))
    {
        ColorRange::WideGamut
    } else {
        ColorRange::HighDynamicRange
    }
}

fn paint(paint: &ResolvedPaint, images: &FxHashMap<u64, GpuImage>) -> ColorRange {
    match paint {
        ResolvedPaint::Solid(color) => working(*color, 1.0),
        ResolvedPaint::Shader(_) => ColorRange::HighDynamicRange,
        ResolvedPaint::Resources(resources) => {
            let (data, stops) = &**resources;
            if let Some(image) = data.image {
                return image_storage::output_range(images[&image].sampling_flags);
            }
            let stops = &stops[data.first_stop as usize..];
            if data.kind & 0xffff == instance::PAINT_MESH {
                // Two geometry records precede four premultiplied colors per patch.
                return stops
                    .as_chunks::<6>()
                    .0
                    .iter()
                    .flat_map(|patch| &patch[2..])
                    .map(|stop| working(stop.color, stop.color[3]))
                    .max()
                    .unwrap_or_default();
            }
            if (data.packed >> 16) & 0xf == instance::INTERP_SRGB {
                // Encoded interpolation stays in sRGB only when its endpoints do.
                return if stops.iter().all(|stop| {
                    stop.color[..3]
                        .iter()
                        .all(|value| (0.0..=1.0).contains(value))
                }) {
                    ColorRange::Standard
                } else {
                    ColorRange::HighDynamicRange
                };
            }
            stops
                .iter()
                .map(|stop| working(stop.color, 1.0))
                .max()
                .unwrap_or_default()
        }
    }
}

/// Upper bound for the output range of a prepared layer. Unbounded shader,
/// filter and encoded-space blend operations retain extended-range output.
pub(super) fn of_ops(ops: &[Op], images: &FxHashMap<u64, GpuImage>) -> ColorRange {
    ops.iter()
        .map(|op| match op {
            Op::Shaped { paint: source, .. }
            | Op::Path { paint: source, .. }
            | Op::Glyphs { paint: source, .. } => paint(source, images),
            Op::Shadow { color, .. } => working(*color, 1.0),
            Op::BeginIsolate {
                blend,
                space,
                filter,
                ..
            } if filter.is_some()
                || *blend == BlendMode::PlusLighter
                || *space == BlendSpace::SrgbEncoded =>
            {
                ColorRange::HighDynamicRange
            }
            Op::BitmapGlyph { .. }
            | Op::BeginClip { .. }
            | Op::BeginShadow { .. }
            | Op::BeginIsolate { .. }
            | Op::End => ColorRange::Standard,
        })
        .max()
        .unwrap_or_default()
}

/// Classify the same content-first tree order that opens engine parts during
/// lowering. Promoted content is excluded; its children paint in the next part.
pub(super) fn parts(
    tree: &cherenkov::SurfaceTree,
    layers: &mut FxHashMap<cherenkov::LayerId, super::lower::ContentData>,
    images: &FxHashMap<u64, GpuImage>,
    bindings: &FxHashMap<cherenkov::LayerId, super::gpu_content::Binding>,
    plan: &super::planes::Plan,
    clear: cherenkov::WorkingColor,
    output: &mut Vec<ColorRange>,
) {
    #[expect(
        clippy::too_many_arguments,
        reason = "paint-order walk with retained content and image resources"
    )]
    fn visit(
        id: cherenkov::LayerId,
        tree: &cherenkov::SurfaceTree,
        layers: &mut FxHashMap<cherenkov::LayerId, super::lower::ContentData>,
        images: &FxHashMap<u64, GpuImage>,
        bindings: &FxHashMap<cherenkov::LayerId, super::gpu_content::Binding>,
        plan: &super::planes::Plan,
        part: &mut usize,
        output: &mut [ColorRange],
    ) {
        if plan.opens_part().any(|layer| layer == id) {
            *part += 1;
        }
        let node = tree.layer(id);
        if !plan.planes.iter().any(|plane| plane.layer == id) {
            let range = if node.filter.is_some()
                || node.backdrop.is_some()
                || node.blend == BlendMode::PlusLighter
                || bindings.contains_key(&id)
            {
                ColorRange::HighDynamicRange
            } else {
                layers.get_mut(&id).map_or(ColorRange::Standard, |content| {
                    *content.range.get_or_insert_with(|| {
                        let (ops, _) = content
                            .retained
                            .current()
                            .expect("content prepared before classification");
                        of_ops(ops, images)
                    })
                })
            };
            output[*part] = output[*part].max(range);
        }
        for &child in &node.children {
            visit(child, tree, layers, images, bindings, plan, part, output);
        }
    }
    output.clear();
    output.resize(plan.parts(), ColorRange::Standard);
    output[0] = working(clear.components, 1.0);
    let mut part = 0;
    visit(
        tree.root(),
        tree,
        layers,
        images,
        bindings,
        plan,
        &mut part,
        output,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_colors_preserve_extended_range_and_premultiplied_mesh_colors() {
        assert_eq!(working([0.9, 0.2, 0.1, 0.3], 1.0), ColorRange::WideGamut);
        assert_eq!(
            working([1.5, 0.2, 0.1, 1.0], 1.0),
            ColorRange::HighDynamicRange
        );
        assert_eq!(
            working([-0.2, 0.2, 0.1, 1.0], 1.0),
            ColorRange::HighDynamicRange
        );
        assert_eq!(working([0.2, 0.1, 0.0, 0.25], 0.25), ColorRange::WideGamut);
        assert_eq!(
            working([0.4, 0.1, 0.0, 0.25], 0.25),
            ColorRange::HighDynamicRange
        );
    }

    #[test]
    fn transformed_gradient_range_comes_from_colors_not_coordinates() {
        use super::super::prepared::Lowerer;
        use cherenkov::lowering::Compiler;
        use cherenkov::{ColorStop, Command, LinearGradient, Paint, ShapeData, WorkingColor};
        use kurbo::{Affine, Rect};

        let mut gradient = LinearGradient::new((0.0, 0.0), (20.0, 0.0));
        gradient.stops = vec![
            ColorStop {
                offset: 0.0,
                color: WorkingColor::BLACK,
            },
            ColorStop {
                offset: 1.0,
                color: WorkingColor::WHITE,
            },
        ];
        let images = FxHashMap::default();
        let fonts = FxHashMap::default();
        let mut pending = Vec::new();
        let mut lowerer = Lowerer {
            pending: &mut pending,
            fonts: &fonts,
            images: &images,
        };
        for (red, expected) in [
            (1.0, ColorRange::WideGamut),
            (2.0, ColorRange::HighDynamicRange),
        ] {
            gradient.stops[1].color.components[0] = red;
            let command = Command::Fill {
                shape: ShapeData::Rect(Rect::new(0.0, 0.0, 100.0, 100.0)),
                paint: Paint::Linear(gradient.clone())
                    .transformed(Affine::translate((40.0, -30.0)) * Affine::scale(0.5)),
            };
            let mut ops = Vec::new();
            lowerer
                .draw(&command, Affine::IDENTITY, &mut ops)
                .expect("prepared gradient");
            assert_eq!(of_ops(&ops, &images), expected);
        }
    }
}
