//! Picture-local command identity survives recording replacement and tiling.

use crate::render::prepared::{ClipShape, GlyphSource, Op, Outline, ResolvedPaint};
use cherenkov::kurbo::Affine;
use std::sync::Arc;

#[derive(Default)]
pub(super) struct Versions {
    ops: Vec<Op>,
    revisions: Arc<[u64]>,
    generation: u64,
}

impl Versions {
    pub fn update(&mut self, ops: &[Op], inverse: Affine, first: usize) {
        self.generation += 1;
        let mut revisions = Vec::with_capacity(ops.len());
        let mut scopes: Vec<(u32, u64)> = Vec::new();
        for (index, original) in ops.iter().enumerate() {
            let mut op = original.clone();
            match &mut op {
                Op::Shaped { local, ambient, .. } | Op::Shadow { local, ambient, .. } => {
                    *local = inverse * *local;
                    *ambient = inverse * *ambient;
                }
                Op::Path { local, .. }
                | Op::Glyphs { local, .. }
                | Op::BitmapGlyph { local, .. }
                | Op::BeginClip { local, .. }
                | Op::BeginPicture { local, .. } => *local = inverse * *local,
                Op::BeginShadow { parameters, .. } => {
                    parameters.transform = inverse * parameters.transform;
                }
                Op::BeginIsolate { .. } | Op::End => {}
            }
            let end = cherenkov::lowering::Operation::end_mut(&mut op).map(|end| {
                *end -= u32::try_from(first).expect("operation index fits u32");
                *end
            });
            let own = if self.ops.get(index).is_some_and(|previous| *previous == op) {
                self.revisions[index]
            } else {
                self.generation
            };
            let revision = own.max(scopes.last().map_or(0, |(_, revision)| *revision));
            if let Some(end) = end {
                scopes.push((end, revision));
            }
            if scopes.last().is_some_and(|(end, _)| *end as usize == index) {
                scopes.pop();
            }
            revisions.push(revision);
            if let Some(previous) = self.ops.get_mut(index) {
                *previous = op;
            } else {
                self.ops.push(op);
            }
        }
        self.ops.truncate(ops.len());
        self.revisions = revisions.into();
    }

    pub fn revisions(&self) -> Arc<[u64]> {
        Arc::clone(&self.revisions)
    }

    pub fn bytes(&self) -> u64 {
        (self.ops.capacity() * size_of::<Op>()
            + self.revisions.len() * size_of::<u64>()
            + 2 * size_of::<usize>()) as u64
            + self.ops.iter().map(op_bytes).sum::<u64>()
    }
}

fn op_bytes(op: &Op) -> u64 {
    use cherenkov::lowering::memory::{glyph_run_bytes, shape_bytes};
    let (paint, geometry) = match op {
        Op::Shaped { paint, .. } => (Some(paint), 0),
        Op::Path { paint, outline, .. } => (
            Some(paint),
            match outline {
                Outline::Fill { elements, .. } => std::mem::size_of_val(elements.as_ref()) as u64,
                Outline::Stroke { shape, stroke } => {
                    shape_bytes(shape) + (stroke.dash_pattern.len() * size_of::<f64>()) as u64
                }
                Outline::Source { .. } => 0,
            },
        ),
        Op::Glyphs { paint, run, .. } => (
            Some(paint),
            match run {
                GlyphSource::Run(run) => glyph_run_bytes(run),
                GlyphSource::Command { .. } => 0,
            },
        ),
        Op::BeginClip {
            shape: ClipShape::Path { elements, .. },
            ..
        } => (None, std::mem::size_of_val(elements.as_ref()) as u64),
        _ => (None, 0),
    };
    geometry
        + match paint {
            Some(ResolvedPaint::Resources(data)) => {
                (size_of_val(data.as_ref())
                    + data.1.capacity() * size_of::<crate::render::instance::Stop>())
                    as u64
            }
            Some(ResolvedPaint::Shader(shader)) => {
                (size_of_val(shader.as_ref())
                    + shader.source.uniforms.capacity() * size_of::<f32>()) as u64
            }
            _ => 0,
        }
}
