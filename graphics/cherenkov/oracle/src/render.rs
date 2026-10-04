//! The reference renderer: exact-coverage compositing in premultiplied
//! linear Display P3, `f64` throughout, single-threaded.
//!
//! Per the oracle rules:
//!
//! - **Clipping** is the geometric intersection of shape edges and clip
//!   edges ([`crate::clip`]) — never a product of coverages. Nested clips
//!   apply sequentially: each clip is its own edge set.
//! - **Shading** evaluates the paint at the pixel centre and multiplies by
//!   the exact coverage.
//! - **Items** composite source-over in order; a child layer renders into a
//!   fresh canvas (with the layer clip in force), then composites onto the
//!   parent with the layer's opacity and blend mode (W3C Compositing and
//!   Blending Level 1). A destructive Porter-Duff blend applies within the
//!   layer's clip, or over the whole parent when there is no clip.
//! - **Shadows** convolve the shape's exact coverage with a Gaussian in
//!   `f64` and fill it with the colour. Axis-aligned rectangles, rounded
//!   rectangles and circles without clips keep the legacy analytic
//!   reference: the offset shape's unclipped coverage blurred separably
//!   on the viewport grid. Every other shape or placement uses the
//!   general silhouette reference: coverage captured on a canvas padded
//!   by a six-sigma halo — off-viewport coverage still contributes —
//!   blurred with the shape-space covariance, the enclosing clip applied
//!   to the completed shadow. Offsetting the edges before integration is
//!   exact because convolution commutes with translation.
//! - **Strokes** expand with `kurbo`'s stroker; glyph outlines come from
//!   `skrifa`, unhinted ([`crate::glyphs`]).
//!
//! **Backdrop groups** ([`cherenkov_scene::BackdropGroup`]): a group's
//! capture is taken when its first member layer (in paint order: a layer's
//! items in order, depth-first) is reached — a full-canvas copy of the
//! compositing canvas the member is drawn into at that moment, meaning the
//! nearest enclosing layer isolated for opacity `< 1` or a non-Normal blend
//! (clips never isolate). Every child layer composites into a fresh
//! canvas, so a member sitting inside clip-only ancestors sees the
//! semantic level's canvas composited with each ancestor's partial
//! contents in order (see [`flattened`]); a member that is itself
//! isolated sees the parent canvas, since the capture happens before its
//! own isolation begins. The group's filters then run over the copy: `GaussianBlur` is a
//! separable true Gaussian `w(o) = exp(-o² / 2σ²)` normalized over
//! `⌈3σ⌉` taps, clamp-to-edge; `ColorMatrix` applies its three rows to
//! the premultiplied `[r, g, b, a]` pixel, alpha untouched. Every member
//! composites the filtered capture under its own clip's exact coverage,
//! source-over, as its bottom-most content; its items and children draw
//! after. Nested groups follow naturally: an inner group's capture is taken
//! at its first member's paint time and so includes an enclosing member's
//! sample and earlier content. A member without a clip or referencing an
//! undeclared group id is a render error.

use std::collections::HashMap;

use cherenkov_scene::{
    BackdropFilter, BackdropGroup, BlendMode, BlendSpace, Draw, FillRule, GroupItem, Item, Layer,
    LayerFilter, Paint, Scene, Shape,
};
use kurbo::{Affine, Point, Rect, Vec2};

use crate::blend::{blend, src_over};
use crate::clip::{Segment, intersect_edges};
use crate::color::{linear_p3_to_linear_srgb, linear_srgb_to_linear_p3, to_working};
use crate::coverage::Coverage;
use crate::glyphs;
use crate::image::{F32Image, Image};
use crate::paint::{eval_paint, sample_image};
use crate::path::{edges, shape_polylines, stroke_polylines_device};
use crate::resources::Resources;
use crate::shadow::gaussian_blur;

/// The renderer's error type.
#[derive(Debug)]
pub enum RenderError {
    /// Glyph rendering failed.
    Glyphs(glyphs::GlyphError),
    /// A scene resource failed to load or decode.
    Resource(cherenkov_scene::SceneError),
    /// A backdrop-group violation: an undeclared group id or a member
    /// layer without a clip.
    Backdrop(String),
    /// A projective layer the model rejects: an invalid pose, a layer
    /// without a clip, a projective backdrop member, a backdrop group
    /// spanning composition spaces, or no finite density bound.
    Projective(String),
}

impl std::fmt::Display for RenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Glyphs(e) => write!(f, "glyph error: {e}"),
            Self::Resource(e) => write!(f, "resource error: {e}"),
            Self::Backdrop(e) => write!(f, "backdrop error: {e}"),
            Self::Projective(e) => write!(f, "projective error: {e}"),
        }
    }
}

impl std::error::Error for RenderError {}

impl From<glyphs::GlyphError> for RenderError {
    fn from(e: glyphs::GlyphError) -> Self {
        Self::Glyphs(e)
    }
}

impl From<cherenkov_scene::SceneError> for RenderError {
    fn from(e: cherenkov_scene::SceneError) -> Self {
        Self::Resource(e)
    }
}

/// Premultiplied `f64` pixels in the canvas's storage space: premultiplied
/// linear Display P3 unless the level declared sRGB-encoded compositing.
#[derive(Clone)]
struct Canvas {
    pixels: Vec<[f64; 4]>,
    width: usize,
    height: usize,
}

/// `srgb_encode` preserving sign, like the backends' `convert_pixel`.
fn srgb_encode_signed(c: f64) -> f64 {
    if c.abs() <= 0.003_130_8 {
        c * 12.92
    } else {
        (1.055f64.mul_add(c.abs().powf(1.0 / 2.4), -0.055)).copysign(c)
    }
}

/// `srgb_decode` preserving sign, like the backends' `convert_pixel`.
fn srgb_decode_signed(c: f64) -> f64 {
    if c.abs() <= 0.04045 {
        c / 12.92
    } else {
        ((c.abs() + 0.055) / 1.055).powf(2.4).copysign(c)
    }
}

/// Convert a premultiplied pixel between linear Display P3 and encoded
/// sRGB: `cpu/src/render/paint.rs::convert_pixel` in `f64`.
fn convert_pixel(pixel: [f64; 4], encode: bool) -> [f64; 4] {
    let alpha = pixel[3];
    if alpha == 0.0 {
        return [0.0; 4];
    }
    let straight = [pixel[0] / alpha, pixel[1] / alpha, pixel[2] / alpha];
    let converted = if encode {
        linear_p3_to_linear_srgb(straight).map(srgb_encode_signed)
    } else {
        linear_srgb_to_linear_p3(straight.map(srgb_decode_signed))
    };
    [
        converted[0] * alpha,
        converted[1] * alpha,
        converted[2] * alpha,
        alpha,
    ]
}

/// Move a premultiplied pixel between storage spaces.
fn move_space(pixel: [f64; 4], from: BlendSpace, to: BlendSpace) -> [f64; 4] {
    if from == to {
        pixel
    } else {
        convert_pixel(pixel, to == BlendSpace::SrgbEncoded)
    }
}

/// Composite `src` onto `dst` blending in `blend` space: both operands are
/// converted from their storage spaces into `blend`, the blend applies,
/// and the result lands back in the destination's space. Matches
/// `cpu/src/render/blend.rs::in_space` generalized to stored-space levels.
fn composite_pixel(
    mode: BlendMode,
    blend_space: BlendSpace,
    dst_space: BlendSpace,
    src_space: BlendSpace,
    dst: [f64; 4],
    src: [f64; 4],
) -> [f64; 4] {
    let b = move_space(dst, dst_space, blend_space);
    let s = move_space(src, src_space, blend_space);
    move_space(blend(mode, b, s), blend_space, dst_space)
}

impl Canvas {
    fn new(width: usize, height: usize, clear: [f64; 4]) -> Self {
        Self {
            pixels: vec![clear; width * height],
            width,
            height,
        }
    }
}

/// One compositing level: a canvas plus the opacity and blend mode it
/// composites into the level below it with. `semantic` marks the canvases
/// a backdrop capture sees as its compositing target: the surface canvas
/// and every layer isolated for a filter, `opacity < 1` or a non-Normal
/// blend. `space` is the canvas's storage space — an isolated level's own
/// `blend_space`, or the enclosing level's for a transparent (clip-only)
/// level, which inherits the space it composites into.
struct Level {
    canvas: Canvas,
    opacity: f64,
    blend: BlendMode,
    space: BlendSpace,
    semantic: bool,
}

/// The canvas at the top of `chain`.
const fn top(chain: &mut [Level]) -> &mut Canvas {
    &mut chain.last_mut().expect("a canvas is always pushed").canvas
}

/// What has been painted so far into the top level's compositing target:
/// the nearest semantic level's canvas, composited with the partial
/// contents of every clip-only level above it in order — exactly what the
/// chain would produce if every pending level composited right now.
fn flattened(chain: &[Level]) -> Canvas {
    let sem = chain
        .iter()
        .rposition(|level| level.semantic)
        .expect("the root level is semantic");
    let mut acc = chain[sem].canvas.clone();
    for level in &chain[sem + 1..] {
        for (dst, &src) in acc.pixels.iter_mut().zip(&level.canvas.pixels) {
            let s = src.map(|v| v * level.opacity);
            *dst = if level.blend == BlendMode::Normal {
                src_over(*dst, s)
            } else {
                blend(level.blend, *dst, s)
            };
        }
    }
    acc
}

/// Backdrop-group render state: the scene's declared groups plus each
/// group's filtered capture — the canvas's pixels and the space they were
/// captured in — taken at its first member's paint point.
struct Backdrops<'a> {
    groups: &'a [BackdropGroup],
    captures: HashMap<u32, (BlendSpace, Canvas)>,
    /// The composition space being painted: 0 for the surface, a fresh
    /// number for each projective layer's local image.
    space: usize,
    /// The last composition space number handed out.
    spaces: usize,
    /// The composition space each group's first member was found in.
    member_space: HashMap<u32, usize>,
}

impl Backdrops<'_> {
    /// The declared group `id`, or the unknown-group render error.
    fn group(&self, id: u32) -> Result<&BackdropGroup, RenderError> {
        self.groups.iter().find(|g| g.id == id).ok_or_else(|| {
            RenderError::Backdrop(format!("layer samples unknown backdrop group {id}"))
        })
    }
}

/// The oracle renderer.
#[derive(Debug)]
pub struct Renderer {
    width: usize,
    height: usize,
    scene_rect: Rect,
    reconstruction: crate::projective::Reconstruction,
}

impl Renderer {
    /// A renderer for `width`×`height` output pixels.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel dimensions are far below 2^53"
    )]
    pub const fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            scene_rect: Rect::new(0.0, 0.0, width as f64, height as f64),
            reconstruction: crate::projective::Reconstruction::Model,
        }
    }

    /// The renderer reconstructing projective layers with `reconstruction`
    /// (the specified model by default; see
    /// [`crate::projective::Reconstruction`]).
    #[must_use]
    pub const fn with_reconstruction(
        mut self,
        reconstruction: crate::projective::Reconstruction,
    ) -> Self {
        self.reconstruction = reconstruction;
        self
    }

    /// Render `scene` into a premultiplied linear-P3 `f32` image.
    /// `scene_dir` locates the `resources/` directory.
    ///
    /// # Errors
    /// `RenderError` on glyph failures or missing resources.
    ///
    /// # Panics
    /// If the canvas chain is corrupted (internal invariant).
    pub fn render(
        &self,
        scene: &Scene,
        scene_dir: &std::path::Path,
    ) -> Result<F32Image, RenderError> {
        self.render_image(scene, scene_dir)
            .map(|image| F32Image::from_f64(&image))
    }

    /// Render `scene` into a premultiplied linear-P3 `f64` image — the
    /// full-precision reference the presentation functions
    /// ([`crate::present`]) consume.
    ///
    /// # Errors
    /// `RenderError` on glyph failures or missing resources.
    ///
    /// # Panics
    /// Never; the root canvas is always pushed, so the final `pop` cannot
    /// fail.
    pub fn render_image(
        &self,
        scene: &Scene,
        scene_dir: &std::path::Path,
    ) -> Result<Image, RenderError> {
        let mut resources = Resources::new(scene_dir.to_path_buf());
        let clear = to_working(&scene.clear);
        let mut chain = vec![Level {
            canvas: Canvas::new(self.width, self.height, clear),
            opacity: 1.0,
            blend: BlendMode::Normal,
            space: BlendSpace::Linear,
            semantic: true,
        }];
        let mut backdrops = Backdrops {
            groups: &scene.backdrop_groups,
            captures: HashMap::new(),
            space: 0,
            spaces: 0,
            member_space: HashMap::new(),
        };
        let root_tf = scene.root.transform
            * Affine::translate((-scene.root.scroll_offset.x, -scene.root.scroll_offset.y));
        self.render_items(
            &scene.root.items,
            root_tf,
            &scene_clip_stack(&scene.root, scene.root.transform, self.width, self.height),
            &mut chain,
            &mut resources,
            &mut backdrops,
        )?;
        let Level { canvas, .. } = chain.pop().expect("the root canvas is always pushed");
        Ok(Image {
            width: self.width,
            height: self.height,
            pixels: canvas.pixels,
        })
    }

    /// Render `items` (a layer's contents) into the canvas on top of
    /// `chain`.
    ///
    /// `tf` maps the items' user space to scene space; `clips` is the stack
    /// of active clip boundary edge sets, already in scene space.
    fn render_items(
        &self,
        items: &[Item],
        tf: Affine,
        clips: &[Vec<Segment>],
        chain: &mut Vec<Level>,
        resources: &mut Resources,
        backdrops: &mut Backdrops<'_>,
    ) -> Result<(), RenderError> {
        for item in items {
            match item {
                Item::Draw(draw) => {
                    self.render_draw(draw, tf, clips, chain, resources, backdrops)?;
                }
                Item::Layer(child) => {
                    if let Some(gid) = child.backdrop {
                        // One capture per group: its members share one
                        // composition space.
                        let space = backdrops.space;
                        if *backdrops.member_space.entry(gid).or_insert(space) != space {
                            return Err(RenderError::Projective(
                                "projective-backdrop-cross-space".into(),
                            ));
                        }
                        if child.projection.is_some() {
                            return Err(RenderError::Projective(
                                "projective-backdrop-member".into(),
                            ));
                        }
                        // The group's one capture point is this position in
                        // painter order: what has been painted so far into
                        // the member's compositing canvas — the nearest
                        // semantic level's canvas plus, in order, the
                        // partial contents of every clip-only level the
                        // member sits inside — filtered once and shared by
                        // all members.
                        let group = backdrops.group(gid)?;
                        let space = chain
                            .iter()
                            .rposition(|level| level.semantic)
                            .map_or(BlendSpace::Linear, |i| chain[i].space);
                        let mut capture = flattened(chain);
                        for filter in &group.filters {
                            apply_backdrop_filter(&mut capture, filter);
                        }
                        backdrops.captures.entry(gid).or_insert((space, capture));
                    }
                    self.render_child_layer(child, tf, clips, chain, resources, backdrops)?;
                }
                Item::Group(group) => {
                    self.render_group(group, tf, clips, chain, resources, backdrops)?;
                }
            }
        }
        Ok(())
    }

    /// Composite a child layer. Every child renders into a fresh canvas
    /// under the accumulated clips plus the child's own clip, then
    /// composites with opacity and blend mode: a `Normal`-blend child sees
    /// (and blends against) only what painted into its own canvas, never
    /// the parent's. Clip-only levels are not semantic isolations — a
    /// backdrop capture looks through them to the nearest `opacity < 1` or
    /// `blend != Normal` level (see `flattened`).
    #[expect(
        clippy::many_single_char_names,
        reason = "w/h/dst/s/b/c name geometry and pixel values"
    )]
    fn render_child_layer(
        &self,
        child: &Layer,
        parent_tf: Affine,
        clips: &[Vec<Segment>],
        chain: &mut Vec<Level>,
        resources: &mut Resources,
        backdrops: &mut Backdrops<'_>,
    ) -> Result<(), RenderError> {
        if child.projection.is_some() {
            return self.render_projective(child, parent_tf, clips, chain, resources, backdrops);
        }
        let tf = parent_tf * child.transform;
        let mut child_clips = clips.to_vec();
        if let Some(clip) = &child.clip {
            child_clips.push(shape_edges(clip, tf));
        }
        // Content and children draw translated by -scroll_offset inside
        // the clip; `motion` is ignored — the oracle renders the settled
        // scene.
        let content_tf = tf * Affine::translate((-child.scroll_offset.x, -child.scroll_offset.y));

        let (w, h) = (top(chain).width, top(chain).height);
        // A member that is itself filtered draws its backdrop sample into
        // the enclosing canvas first: `layer` in gpu/src/render/lower.rs
        // emits the sample to the current target before and outside the
        // layer's isolation, so the layer's filter covers the member's
        // items but never the sample.
        if child.filter.is_some() && child.backdrop.is_some() {
            let space = chain.last().map_or(BlendSpace::Linear, |l| l.space);
            self.backdrop_sample(child, tf, clips, top(chain), space, backdrops)?;
        }
        // A filtered layer is a semantic isolation too: a backdrop capture
        // inside it reads this canvas, matching `isolate` in
        // gpu/src/render/lower.rs.
        let semantic =
            child.filter.is_some() || child.opacity < 1.0 || child.blend != BlendMode::Normal;
        // A transparent level shares the space it composites into; an
        // isolated level stores its declared space (layers always linear).
        let parent_space = chain.last().map_or(BlendSpace::Linear, |l| l.space);
        chain.push(Level {
            canvas: Canvas::new(w, h, [0.0; 4]),
            opacity: child.opacity,
            blend: child.blend,
            space: if semantic {
                BlendSpace::Linear
            } else {
                parent_space
            },
            semantic,
        });
        self.render_layer_body(
            child,
            tf,
            clips,
            content_tf,
            &child_clips,
            chain,
            resources,
            backdrops,
        )?;
        let Level {
            canvas: mut sub,
            opacity,
            blend: mode,
            space,
            ..
        } = chain.pop().expect("the child level is pushed above");
        if let Some(filter) = child.filter.as_deref() {
            let texels = match filter {
                LayerFilter::BlendImage { image, .. } => {
                    Some(resources.texels(*image).map_err(RenderError::Resource)?)
                }
                _ => None,
            };
            crate::filter::apply(filter, texels, &mut sub.pixels, sub.width, sub.height);
            // The layer clip masks the filter's output, not only its input.
            if !child_clips.is_empty() {
                let mask = self.shape_coverage(
                    &Shape::Rect(self.scene_rect),
                    FillRule::NonZero,
                    Affine::IDENTITY,
                    &child_clips,
                );
                for (px, m) in sub.pixels.iter_mut().zip(mask) {
                    *px = px.map(|v| v * m);
                }
            }
        }

        // A destructive operator is bounded by the effective clip: outside
        // it the destination is untouched, and the clip edge is antialiased
        // between the backdrop and the blended result. Unclipped it covers
        // the whole parent.
        let clip_cov: Option<Vec<f64>> = if Self::is_destructive(mode) && !child_clips.is_empty() {
            let mut segs = child_clips[0].clone();
            for c in &child_clips[1..] {
                segs = intersect_edges(&segs, FillRule::NonZero, c);
            }
            let mut cov = Coverage::new(self.width, self.height);
            for &s in &segs {
                cov.add_line(s.0, s.1, s.2, s.3);
            }
            Some(cov.finish(FillRule::NonZero))
        } else {
            None
        };

        let dst_space = chain.last().map_or(BlendSpace::Linear, |l| l.space);
        for (i, (dst, &src)) in top(chain).pixels.iter_mut().zip(&sub.pixels).enumerate() {
            let s = src.map(|v| v * opacity);
            *dst = if mode == BlendMode::Normal && space == dst_space {
                src_over(*dst, s)
            } else {
                let b = composite_pixel(mode, space, dst_space, space, *dst, s);
                match clip_cov.as_ref().map(|v| v[i]) {
                    Some(c) if c >= 1.0 => b,
                    Some(c) if c <= 0.0 => *dst,
                    Some(c) => std::array::from_fn(|ch| c.mul_add(b[ch] - dst[ch], dst[ch])),
                    None => b,
                }
            };
        }
        Ok(())
    }

    /// A projective child (see [`crate::projective`]): its content, clip,
    /// filter and children render into a fresh local image — a semantic
    /// isolation of its own composition space — which is mipmapped and
    /// reconstructed at every parent pixel. The ancestor clips' exact
    /// coverage and the layer's opacity scale the sample, and the blend
    /// mode composites it. A destructive operator's domain is the
    /// projected layer clip geometrically intersected with the ancestor
    /// clips.
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel indices are far below 2^53"
    )]
    fn render_projective(
        &self,
        child: &Layer,
        parent_tf: Affine,
        clips: &[Vec<Segment>],
        chain: &mut [Level],
        resources: &mut Resources,
        backdrops: &mut Backdrops<'_>,
    ) -> Result<(), RenderError> {
        use crate::projective::{
            Mipmapped, Reconstruction, domain, place, pose, project_edges, to_raster,
        };
        let projection = child.projection.as_ref().expect("callers check projection");
        let clip = child
            .clip
            .as_ref()
            .ok_or_else(|| RenderError::Projective("projective-unclipped".into()))?;
        let pose = pose(projection, child.transform).map_err(RenderError::Projective)?;
        let homography = to_raster(&pose, parent_tf);
        let Some(placement) = place(
            &homography,
            domain(clip),
            (self.width, self.height),
            self.reconstruction.refine(),
        )
        .map_err(RenderError::Projective)?
        else {
            return Ok(());
        };
        let image = Mipmapped::new(
            self.render_local(child, clip, &placement, resources, backdrops)?,
            placement.width,
            placement.height,
        );
        let ancestors = (!clips.is_empty()).then(|| {
            self.shape_coverage(
                &Shape::Rect(self.scene_rect),
                FillRule::NonZero,
                Affine::IDENTITY,
                clips,
            )
        });
        let domain = Self::is_destructive(child.blend).then(|| {
            let outline = crate::path::shape_polylines(clip, placement.local_to_texel);
            let mut segs = project_edges(&placement.forward, &outline, (self.width, self.height));
            for c in clips {
                segs = intersect_edges(&segs, FillRule::NonZero, c);
            }
            let mut cov = Coverage::new(self.width, self.height);
            for &s in &segs {
                cov.add_line(s.0, s.1, s.2, s.3);
            }
            cov.finish(FillRule::NonZero)
        });
        let dst_space = chain.last().map_or(BlendSpace::Linear, |l| l.space);
        let mode = child.blend;
        for (i, dst) in top(chain).pixels.iter_mut().enumerate() {
            let at = [(i % self.width) as f64 + 0.5, (i / self.width) as f64 + 0.5];
            let inside = ancestors.as_ref().map_or(1.0, |cov| cov[i]);
            let sample = match self.reconstruction {
                Reconstruction::Model => image.sample(&placement.inverse, at),
                Reconstruction::Supersampled { grid, .. } => {
                    image.box_sample(&placement.inverse, [at[0] - 0.5, at[1] - 0.5], grid)
                }
            };
            let src = sample.map(|v| v * child.opacity * inside);
            *dst = if mode == BlendMode::Normal && dst_space == BlendSpace::Linear {
                src_over(*dst, src)
            } else {
                let blended = composite_pixel(
                    mode,
                    BlendSpace::Linear,
                    dst_space,
                    BlendSpace::Linear,
                    *dst,
                    src,
                );
                match domain.as_ref().map(|cov| cov[i]) {
                    Some(c) if c >= 1.0 => blended,
                    Some(c) if c <= 0.0 => *dst,
                    Some(c) => std::array::from_fn(|ch| c.mul_add(blended[ch] - dst[ch], dst[ch])),
                    None => blended,
                }
            };
        }
        Ok(())
    }

    /// A projective layer's completed local image: its content and
    /// children under its clip in texel space, then its filter, masked
    /// by the clip. The image is its own semantic level and composition
    /// space.
    fn render_local(
        &self,
        child: &Layer,
        clip: &Shape,
        placement: &crate::projective::Placement,
        resources: &mut Resources,
        backdrops: &mut Backdrops<'_>,
    ) -> Result<Vec<[f64; 4]>, RenderError> {
        let (width, height) = (placement.width, placement.height);
        let local = Self::new(width, height).with_reconstruction(self.reconstruction);
        let mut local_chain = vec![Level {
            canvas: Canvas::new(width, height, [0.0; 4]),
            opacity: 1.0,
            blend: BlendMode::Normal,
            space: BlendSpace::Linear,
            semantic: true,
        }];
        let local_clips = vec![shape_edges(clip, placement.local_to_texel)];
        let content_tf = placement.local_to_texel
            * Affine::translate((-child.scroll_offset.x, -child.scroll_offset.y));
        let outer = backdrops.space;
        backdrops.spaces += 1;
        backdrops.space = backdrops.spaces;
        local.render_items(
            &child.items,
            content_tf,
            &local_clips,
            &mut local_chain,
            resources,
            backdrops,
        )?;
        backdrops.space = outer;
        let Level {
            canvas: mut sub, ..
        } = local_chain.pop().expect("the local root is pushed");
        if let Some(filter) = child.filter.as_deref() {
            if self.reconstruction != crate::projective::Reconstruction::Model {
                // Filter parameters are in local raster pixels, which the
                // refinement would shrink: a different image, not a finer
                // reconstruction of the same one.
                return Err(RenderError::Projective(
                    "the quality reference does not refine a filtered projective layer".into(),
                ));
            }
            let texels = match filter {
                LayerFilter::BlendImage { image, .. } => {
                    Some(resources.texels(*image).map_err(RenderError::Resource)?)
                }
                _ => None,
            };
            crate::filter::apply(filter, texels, &mut sub.pixels, width, height);
            let mask = local.shape_coverage(
                &Shape::Rect(local.scene_rect),
                FillRule::NonZero,
                Affine::IDENTITY,
                &local_clips,
            );
            for (px, m) in sub.pixels.iter_mut().zip(mask) {
                *px = px.map(|v| v * m);
            }
        }
        Ok(sub.pixels)
    }

    /// A display-list group: members composite with each other in the
    /// group's `blend_space` (the level's canvas stores premultiplied
    /// values in that space), then the group composites onto the level
    /// below with `opacity` and `blend`. A fully transparent group
    /// (`opacity` 1, `Normal`, `Linear`) shares the enclosing level's
    /// space and passes through.
    fn render_group(
        &self,
        group: &cherenkov_scene::Group,
        tf: Affine,
        clips: &[Vec<Segment>],
        chain: &mut Vec<Level>,
        resources: &mut Resources,
        backdrops: &mut Backdrops<'_>,
    ) -> Result<(), RenderError> {
        let parent_space = chain.last().map_or(BlendSpace::Linear, |l| l.space);
        let semantic = group.opacity < 1.0
            || group.blend != BlendMode::Normal
            || group.blend_space != BlendSpace::Linear;
        let space = if semantic {
            group.blend_space
        } else {
            parent_space
        };
        let (w, h) = (top(chain).width, top(chain).height);
        chain.push(Level {
            canvas: Canvas::new(w, h, [0.0; 4]),
            opacity: group.opacity,
            blend: group.blend,
            space,
            semantic,
        });
        for item in &group.items {
            match item {
                GroupItem::Draw(draw) => {
                    self.render_draw(draw, tf, clips, chain, resources, backdrops)?;
                }
                GroupItem::Group(inner) => {
                    self.render_group(inner, tf, clips, chain, resources, backdrops)?;
                }
            }
        }
        let Level {
            canvas: sub,
            opacity,
            blend: mode,
            space,
            ..
        } = chain.pop().expect("the group level is pushed above");
        let dst_space = chain.last().map_or(BlendSpace::Linear, |l| l.space);
        for (dst, &src) in top(chain).pixels.iter_mut().zip(&sub.pixels) {
            let s = src.map(|v| v * opacity);
            *dst = if mode == BlendMode::Normal && space == dst_space {
                src_over(*dst, s)
            } else {
                composite_pixel(mode, space, dst_space, space, *dst, s)
            };
        }
        Ok(())
    }

    /// Porter-Duff operators where a transparent source changes the
    /// destination: the composite is bounded by the effective clip (or the
    /// whole parent when unclipped).
    const fn is_destructive(blend: BlendMode) -> bool {
        matches!(
            blend,
            BlendMode::Clear
                | BlendMode::Src
                | BlendMode::SrcIn
                | BlendMode::SrcOut
                | BlendMode::DestIn
                | BlendMode::DestAtop
        )
    }

    /// Draw the member's shared group capture into `canvas` under the
    /// member clip's exact coverage, source-over. `clips` is the enclosing
    /// clip stack; the member's own clip is the sampled shape.
    ///
    /// # Errors
    /// `RenderError::Backdrop` when the member has no clip.
    fn backdrop_sample(
        &self,
        child: &Layer,
        tf: Affine,
        clips: &[Vec<Segment>],
        canvas: &mut Canvas,
        space: BlendSpace,
        backdrops: &Backdrops<'_>,
    ) -> Result<(), RenderError> {
        let gid = child.backdrop.expect("callers check backdrop membership");
        let clip = child.clip.as_ref().ok_or_else(|| {
            RenderError::Backdrop(format!("backdrop group {gid} member layer has no clip"))
        })?;
        let coverage = self.shape_coverage(clip, FillRule::NonZero, tf, clips);
        if let Some((cap_space, capture)) = backdrops.captures.get(&gid) {
            // SDF effects need the member clip's analytic box (the GPU
            // errors the same name for a mask or path clip).
            let sdf_clip = match &child.backdrop_effect {
                None | Some(cherenkov_scene::BackdropEffectSpec::ColorMatrix { .. }) => None,
                Some(_) => Some(
                    crate::sdf::box_params(clip)
                        .map(|(shape, extra)| (shape, tf * extra))
                        .ok_or_else(|| {
                            RenderError::Backdrop("backdrop-effect-sdf-path".to_string())
                        })?,
                ),
            };
            let (w, h) = (capture.width, capture.height);
            #[expect(
                clippy::cast_precision_loss,
                reason = "pixel indices are far below 2^53"
            )]
            for (i, dst) in canvas.pixels.iter_mut().enumerate() {
                let c = coverage[i];
                if c == 0.0 {
                    continue;
                }
                let src = sample_backdrop(
                    child,
                    &capture.pixels,
                    w,
                    h,
                    [(i % w) as f64 + 0.5, (i / w) as f64 + 0.5],
                    sdf_clip.as_ref(),
                );
                *dst = src_over(*dst, move_space(src.map(|v| v * c), *cap_space, space));
            }
        }
        Ok(())
    }

    /// A child's body: the backdrop sample under the member's clip (its
    /// bottom-most content), then its items. `tf` is the child's transform
    /// in scene space, `clips` the enclosing clip stack without the child's
    /// own clip, `content_tf` the items' transform, `child_clips` the full
    /// stack including the child's own clip.
    #[expect(
        clippy::too_many_arguments,
        reason = "the four transform/clip parameters are each needed"
    )]
    fn render_layer_body(
        &self,
        child: &Layer,
        tf: Affine,
        clips: &[Vec<Segment>],
        content_tf: Affine,
        child_clips: &[Vec<Segment>],
        target: &mut Vec<Level>,
        resources: &mut Resources,
        backdrops: &mut Backdrops<'_>,
    ) -> Result<(), RenderError> {
        if let Some(gid) = child.backdrop {
            // The capture was taken when this member (or an earlier one)
            // was reached; sample it under the member clip's coverage as
            // the layer's bottom-most content. A filtered member already
            // drew its sample into the enclosing canvas — its isolation
            // does not cover the sample — so only the clip check applies.
            if child.filter.is_none() {
                let space = target.last().map_or(BlendSpace::Linear, |l| l.space);
                self.backdrop_sample(child, tf, clips, top(target), space, backdrops)?;
            } else if child.clip.is_none() {
                return Err(RenderError::Backdrop(format!(
                    "backdrop group {gid} member layer has no clip"
                )));
            }
        }
        self.render_items(
            &child.items,
            content_tf,
            child_clips,
            target,
            resources,
            backdrops,
        )
    }

    /// Exact coverage of `shape` under `tf`, clipped by every clip set.
    fn shape_coverage(
        &self,
        shape: &Shape,
        rule: FillRule,
        tf: Affine,
        clips: &[Vec<Segment>],
    ) -> Vec<f64> {
        let mut segs = edges(&shape_polylines(shape, tf));
        for clip in clips {
            segs = intersect_edges(&segs, rule, clip);
        }
        let mut cov = Coverage::new(self.width, self.height);
        for &s in &segs {
            cov.add_line(s.0, s.1, s.2, s.3);
        }
        cov.finish(rule)
    }

    /// The general silhouette reference: capture outside the viewport, blur
    /// with shape-space covariance, and apply the enclosing clip afterwards.
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "corpus shapes and surface dimensions are finite and small"
    )]
    fn silhouette_coverage(
        &self,
        shape: &Shape,
        sigma: f64,
        offset: [f64; 2],
        tf: Affine,
        clips: &[Vec<Segment>],
    ) -> Vec<f64> {
        let [a, b, c, d, _, _] = tf.as_coeffs();
        let px = (6.0 * sigma).mul_add(a.hypot(c), 2.0).ceil().max(0.0) as usize;
        let py = (6.0 * sigma).mul_add(b.hypot(d), 2.0).ceil().max(0.0) as usize;
        let (width, height) = (self.width + 2 * px, self.height + 2 * py);
        let renderer = Self::new(width, height);
        let place = Affine::translate((px as f64, py as f64))
            * tf
            * Affine::translate(Vec2::new(offset[0], offset[1]));
        let coverage = renderer.shape_coverage(shape, FillRule::NonZero, place, &[]);
        let blurred = crate::shadow::affine_blur(&coverage, width, height, sigma, [a, b, c, d]);
        let clip = (!clips.is_empty()).then(|| {
            self.shape_coverage(
                &Shape::Rect(self.scene_rect),
                FillRule::NonZero,
                Affine::IDENTITY,
                clips,
            )
        });
        let mut out = Vec::with_capacity(self.width * self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                let value = blurred[(y + py) * width + x + px]
                    * clip.as_ref().map_or(1.0, |c| c[y * self.width + x]);
                out.push(value);
            }
        }
        out
    }

    /// Composite `paint` over `canvas`, multiplied by `coverage`; the paint
    /// is sampled at pixel centres in the items' user space (`inv_tf`).
    /// # Errors
    /// `RenderError` on missing resources.
    fn composite_paint(
        canvas: &mut Canvas,
        space: BlendSpace,
        coverage: &[f64],
        paint: &Paint,
        inv_tf: Affine,
        resources: &mut Resources,
    ) -> Result<(), RenderError> {
        let (w, h) = (canvas.width, canvas.height);
        #[expect(
            clippy::cast_precision_loss,
            reason = "pixel indices are far below 2^53"
        )]
        for py in 0..h {
            for px in 0..w {
                let c = coverage[py * w + px];
                if c <= 0.0 {
                    continue;
                }
                let p = inv_tf * Point::new(px as f64 + 0.5, py as f64 + 0.5);
                // A member draw lands in the level's storage space.
                let src = move_space(
                    eval_paint(paint, p, resources)?.map(|v| v * c),
                    BlendSpace::Linear,
                    space,
                );
                let idx = py * w + px;
                canvas.pixels[idx] = src_over(canvas.pixels[idx], src);
            }
        }
        Ok(())
    }

    #[allow(clippy::many_single_char_names)] // u/v/w/h/x/y are the natural names
    #[expect(
        clippy::cast_precision_loss,
        reason = "pixel and image indices are far below 2^53"
    )]
    fn render_draw(
        &self,
        draw: &Draw,
        tf: Affine,
        clips: &[Vec<Segment>],
        chain: &mut Vec<Level>,
        resources: &mut Resources,
        backdrops: &mut Backdrops<'_>,
    ) -> Result<(), RenderError> {
        let inv_tf = tf.inverse();
        match draw {
            Draw::Fill { shape, rule, paint } => {
                let coverage = self.shape_coverage(shape, *rule, tf, clips);
                let space = chain.last().map_or(BlendSpace::Linear, |l| l.space);
                Self::composite_paint(top(chain), space, &coverage, paint, inv_tf, resources)?;
            }
            Draw::Stroke {
                shape,
                stroke,
                paint,
            } => {
                let mut segs = edges(&stroke_polylines_device(shape, stroke, tf));
                for clip in clips {
                    segs = intersect_edges(&segs, FillRule::NonZero, clip);
                }
                let mut cov = Coverage::new(self.width, self.height);
                for &s in &segs {
                    cov.add_line(s.0, s.1, s.2, s.3);
                }
                let coverage = cov.finish(FillRule::NonZero);
                let space = chain.last().map_or(BlendSpace::Linear, |l| l.space);
                Self::composite_paint(top(chain), space, &coverage, paint, inv_tf, resources)?;
            }
            Draw::Shadow {
                shape,
                blur_sigma,
                offset,
                color,
            } => {
                // Exact: shift the shape edges by the offset before coverage,
                // then blur (convolution commutes with translation).
                let tf_off = tf * Affine::translate(Vec2::new(offset[0], offset[1]));
                let blurred = if matches!(
                    shape,
                    Shape::Rect(_) | Shape::RoundedRect(_) | Shape::Circle(_)
                ) && tf.as_coeffs()[..4] == [1.0, 0.0, 0.0, 1.0]
                    && clips.is_empty()
                {
                    // Preserve the established separable reference's exact arithmetic.
                    let coverage = self.shape_coverage(shape, FillRule::NonZero, tf_off, clips);
                    gaussian_blur(&coverage, self.width, self.height, *blur_sigma)
                } else {
                    self.silhouette_coverage(shape, *blur_sigma, *offset, tf, clips)
                };
                let src = to_working(color);
                let space = chain.last().map_or(BlendSpace::Linear, |l| l.space);
                for (px, &c) in top(chain).pixels.iter_mut().zip(&blurred) {
                    if c > 0.0 {
                        *px = src_over(
                            *px,
                            move_space(src.map(|v| v * c), BlendSpace::Linear, space),
                        );
                    }
                }
            }
            Draw::Glyphs(run) => {
                let items = glyphs::items_for_glyph_run(run, tf, resources, self.scene_rect)?;
                self.render_items(&items, tf, clips, chain, resources, backdrops)?;
            }
            Draw::Image {
                image,
                encoding,
                dst,
                sampling,
            } => {
                let (dw, dh) = (dst.x1 - dst.x0, dst.y1 - dst.y0);
                let coverage =
                    self.shape_coverage(&Shape::Rect(*dst), FillRule::NonZero, tf, clips);
                let img = resources.image(*image, *encoding)?.clone();
                let (w, h) = (top(chain).width, top(chain).height);
                for py in 0..h {
                    for px in 0..w {
                        let c = coverage[py * w + px];
                        if c <= 0.0 {
                            continue;
                        }
                        let p = inv_tf * Point::new(px as f64 + 0.5, py as f64 + 0.5);
                        // sample_image takes image-space coordinates: the
                        // image spans [0, w] × [0, h] over `dst`.
                        let u = (p.x - dst.x0) / dw * img.width as f64;
                        let v = (p.y - dst.y0) / dh * img.height as f64;
                        let src = sample_image(&img, u, v, *sampling).map(|x| x * c);
                        let idx = py * w + px;
                        let space = chain.last().map_or(BlendSpace::Linear, |l| l.space);
                        top(chain).pixels[idx] = src_over(
                            top(chain).pixels[idx],
                            move_space(src, BlendSpace::Linear, space),
                        );
                    }
                }
            }
        }
        Ok(())
    }
}

/// The member's backdrop composite at device pixel centre `p`: the
/// per-member effect's sample of the filtered capture, before coverage and
/// opacity. `sdf_clip` is the member clip's box shape and box →
/// box-local inverse, `Some` whenever the effect reads the clip's SDF.
#[allow(clippy::many_single_char_names)] // p/q/c/t/d/n name points and pixel values
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "pixel-centre coordinates index the capture after clamping"
)]
fn sample_backdrop(
    layer: &Layer,
    capture: &[[f64; 4]],
    width: usize,
    height: usize,
    p: [f64; 2],
    sdf_clip: Option<&(crate::sdf::BoxShape, Affine)>,
) -> [f64; 4] {
    use cherenkov_scene::BackdropEffectSpec as E;
    match &layer.backdrop_effect {
        None => {
            // `bilinear` at a texel centre is the texel: keep the exact
            // pre-effect read.
            capture[usize::min(p[1] as usize, height - 1) * width
                + usize::min(p[0] as usize, width - 1)]
        }
        Some(E::ColorMatrix { matrix }) => {
            // 3x4 on the premultiplied sample, filtrate layout: the fourth
            // column is a bias that scales with alpha; alpha passes through.
            let c = crate::sdf::bilinear(capture, width, height, p);
            [
                matrix[0].mul_add(
                    c[0],
                    matrix[1].mul_add(c[1], matrix[2].mul_add(c[2], matrix[3] * c[3])),
                ),
                matrix[4].mul_add(
                    c[0],
                    matrix[5].mul_add(c[1], matrix[6].mul_add(c[2], matrix[7] * c[3])),
                ),
                matrix[8].mul_add(
                    c[0],
                    matrix[9].mul_add(c[1], matrix[10].mul_add(c[2], matrix[11] * c[3])),
                ),
                c[3],
            ]
        }
        Some(E::Refraction { depth, strength }) => {
            let (shape, clip_tf) = sdf_clip.expect("SDF effects carry a box clip");
            let (d, n) = crate::sdf::distance_and_normal(shape, clip_tf, p);
            let t = (1.0 + d / depth).clamp(0.0, 1.0);
            let q = [
                (-n[0] * strength).mul_add(t * t, p[0]),
                (-n[1] * strength).mul_add(t * t, p[1]),
            ];
            crate::sdf::bilinear(capture, width, height, q)
        }
        Some(E::RimLight {
            width: rim_w,
            color,
            gain,
        }) => {
            let (shape, clip_tf) = sdf_clip.expect("SDF effects carry a box clip");
            let (d, _) = crate::sdf::distance_and_normal(shape, clip_tf, p);
            let t = (1.0 + d / rim_w).clamp(0.0, 1.0);
            let mut c = crate::sdf::bilinear(capture, width, height, p);
            let k = color[3] * gain * t * t;
            c[0] = color[0].mul_add(k, c[0]);
            c[1] = color[1].mul_add(k, c[1]);
            c[2] = color[2].mul_add(k, c[2]);
            c
        }
    }
}

/// Apply one backdrop-group filter to a captured canvas, in place.
fn apply_backdrop_filter(canvas: &mut Canvas, filter: &BackdropFilter) {
    match filter {
        BackdropFilter::GaussianBlur { sigma } => {
            canvas.pixels = backdrop_blur(&canvas.pixels, canvas.width, canvas.height, *sigma);
        }
        BackdropFilter::ColorMatrix { matrix } => {
            for px in &mut canvas.pixels {
                let c = *px;
                *px = [
                    matrix[0].mul_add(
                        c[0],
                        matrix[1].mul_add(c[1], matrix[2].mul_add(c[2], matrix[3] * c[3])),
                    ),
                    matrix[4].mul_add(
                        c[0],
                        matrix[5].mul_add(c[1], matrix[6].mul_add(c[2], matrix[7] * c[3])),
                    ),
                    matrix[8].mul_add(
                        c[0],
                        matrix[9].mul_add(c[1], matrix[10].mul_add(c[2], matrix[11] * c[3])),
                    ),
                    c[3],
                ];
            }
        }
    }
}

/// Separable true-Gaussian blur of premultiplied pixels in `f64`:
/// `w(o) = exp(-o² / 2σ²)` normalized over `⌈3σ⌉` taps each side,
/// clamp-to-edge at the canvas boundary.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    reason = "kernel radius and pixel indices are small non-negative values"
)]
fn backdrop_blur(src: &[[f64; 4]], width: usize, height: usize, sigma: f64) -> Vec<[f64; 4]> {
    if sigma <= 1e-9 || src.is_empty() {
        return src.to_vec();
    }
    let radius = (3.0 * sigma).ceil() as usize;
    let mut kernel: Vec<f64> = (0..=2 * radius)
        .map(|i| {
            let d = i as f64 - radius as f64;
            (-d * d / (2.0 * sigma * sigma)).exp()
        })
        .collect();
    let sum: f64 = kernel.iter().sum();
    for w in &mut kernel {
        *w /= sum;
    }
    let mut tmp = vec![[0.0; 4]; src.len()];
    for y in 0..height {
        for x in 0..width {
            let mut acc = [0.0; 4];
            for (i, &w) in kernel.iter().enumerate() {
                let xx = (x as i64 + i as i64 - radius as i64).clamp(0, width as i64 - 1) as usize;
                let s = src[y * width + xx];
                for (a, &c) in acc.iter_mut().zip(&s) {
                    *a = w.mul_add(c, *a);
                }
            }
            tmp[y * width + x] = acc;
        }
    }
    let mut out = vec![[0.0; 4]; src.len()];
    for y in 0..height {
        for x in 0..width {
            let mut acc = [0.0; 4];
            for (i, &w) in kernel.iter().enumerate() {
                let yy = (y as i64 + i as i64 - radius as i64).clamp(0, height as i64 - 1) as usize;
                let s = tmp[yy * width + x];
                for (a, &c) in acc.iter_mut().zip(&s) {
                    *a = w.mul_add(c, *a);
                }
            }
            out[y * width + x] = acc;
        }
    }
    out
}

/// Edges of `shape` under `tf` (used for clip boundaries), flattened in
/// device space.
fn shape_edges(shape: &Shape, tf: Affine) -> Vec<Segment> {
    edges(&shape_polylines(shape, tf))
}

/// The clip stack for `layer`: its own clip (in scene space), empty if none.
fn scene_clip_stack(layer: &Layer, tf: Affine, _w: usize, _h: usize) -> Vec<Vec<Segment>> {
    layer
        .clip
        .as_ref()
        .map(|c| vec![shape_edges(c, tf)])
        .unwrap_or_default()
}
