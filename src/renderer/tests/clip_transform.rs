//! water-rs/hydrolysis#250 — a path drawn after a clip boundary must resolve
//! to its own transform, not to a glyph run's.
//!
//! `encode_transform` dedups against the encoding's last transform, so a fill
//! drawn under the clip layer's transform after `pop_layer` emits no
//! `TRANSFORM` tag of its own. When a glyph run sits between the clip's
//! transform tag and that path, the resolver splices the run's transform
//! entries — and their `TRANSFORM` tags — into the packed stream ahead of the
//! path, and the flatten monoid's `trans_ix = count - 1` lands the path on
//! the run's paint transform.
//!
//! vello already re-arms the force flags after each outline glyph run
//! (vello#424), but that only shields the *immediately following* encode:
//! `Encoding::append` copies the appended encoding's `flags` verbatim,
//! dropping a pending force, and `encode_end_clip` does not restore one. The
//! fix re-arms the flags at every hydrolysis clip boundary and append. This
//! test builds the sequence through [`VelloDrawContext`] — the path every
//! draw call takes — then resolves the packed stream the way `flatten` does
//! and reads the transform the post-clip path lands on.

use std::sync::Arc;

use kurbo::{Affine, Rect};
use peniko::{Blob, Color, Fill, FontData};
use vello_encoding::{PathTag, Resolver, Transform};

use crate::engine::vello_backend::{VelloDrawContext, append_scene};
use crate::engine::{Brush, DrawContext};

/// `trans_ix` the flatten shader computes for the `index`th PATH tag: the
/// number of TRANSFORM tags before it in the packed stream, minus one.
fn path_transform_index(tags: &[PathTag], path_index: usize) -> usize {
    tags.iter()
        .enumerate()
        .filter(|(_, tag)| **tag == PathTag::PATH)
        .nth(path_index)
        .map(|(position, _)| {
            tags[..position]
                .iter()
                .filter(|tag| **tag == PathTag::TRANSFORM)
                .count()
                - 1
        })
        .expect("the path index must exist in the packed tag stream")
}

/// A Roboto glyph with an outline, so `draw_glyphs` takes the outline path
/// that splices transform entries into the packed stream.
const GLYPH_ID: u32 = 36;

fn roboto() -> FontData {
    FontData::new(
        Blob::new(Arc::new(
            include_bytes!("../../../test-fonts/Roboto-Regular.ttf").to_vec(),
        )),
        0,
    )
}

/// The last PATH in the stream is the fill drawn after the clip boundary.
fn last_path_transform(scene: &vello::Scene) -> Transform {
    let mut packed = Vec::new();
    let (layout, _ramps, _images) = Resolver::new().resolve(scene.encoding(), &mut packed);
    let tags = layout.path_tags(&packed);
    let path_count = tags.iter().filter(|tag| **tag == PathTag::PATH).count();
    let trans_ix = path_transform_index(tags, path_count - 1);
    layout.transforms(&packed)[trans_ix]
}

#[test]
fn a_path_after_a_clip_boundary_resolves_to_its_own_transform() {
    let font = roboto();
    let glyph_transform = Affine::translate((500.0, 300.0));
    let clip_transform = Affine::translate((11.0, 7.0));

    let mut scene = vello::Scene::new();
    {
        let mut ctx = VelloDrawContext::with_root_transform(&mut scene, clip_transform);
        ctx.push_layer(1.0, Some(&Rect::new(0.0, 0.0, 100.0, 100.0)));
        ctx.pop_layer();
    }
    // A sibling's glyph run is encoded after the clip's transform tag: at
    // resolve its transform entries are spliced in ahead of the post-clip
    // path — the entries that path's `trans_ix` used to land on.
    scene
        .draw_glyphs(&font)
        .font_size(24.0)
        .transform(glyph_transform)
        .draw(
            Fill::NonZero,
            [vello::Glyph {
                id: GLYPH_ID,
                x: 0.0,
                y: 0.0,
            }]
            .into_iter(),
        );
    // A retained child scene spliced into the frame — `Encoding::append`
    // copies the child's `flags`, dropping the force the glyph run armed.
    append_scene(&mut scene, &vello::Scene::new(), None);
    {
        let mut ctx = VelloDrawContext::with_root_transform(&mut scene, clip_transform);
        // The fill rides the same transform the clip layer encoded: with the
        // flag dropped by the append, `encode_transform` dedups it away.
        ctx.fill_rect(Rect::new(0.0, 0.0, 4.0, 4.0), &Brush::Solid(Color::BLACK));
    }

    assert_eq!(
        last_path_transform(&scene),
        Transform::from_kurbo(&clip_transform),
        "the fill after the clip boundary must resolve to its own transform, \
         not the glyph run's paint transform {glyph_transform:?}"
    );
}
