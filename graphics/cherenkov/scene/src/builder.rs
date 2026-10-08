use crate::{
    BackdropFilter, BackdropGroup, BlendMode, Color, Draw, FillRule, GlyphRun, Group, Item, Layer,
    LayerFilter, Motion, Paint, ResourceHash, Sampling, Scene, Shape, StrokeStyle,
};
use kurbo::{Affine, Rect, Vec2};

/// Author a [`Scene`] in Rust. Obtained from [`Scene::builder`].
///
/// Draw methods operate on the scene's root layer; nested layers are built
/// with [`SceneBuilder::layer`], which passes a [`LayerBuilder`] to a closure.
///
/// ```
/// use cherenkov_scene::{Color, Shape};
/// let scene = cherenkov_scene::Scene::builder(64, 64)
///     .clear(Color::srgb(1.0, 1.0, 1.0))
///     .fill(Shape::rect(4.0, 4.0, 56.0, 56.0), Color::srgb(1.0, 0.0, 0.0).into())
///     .build();
/// assert_eq!(scene.root.items.len(), 1);
/// ```
#[derive(Debug)]
pub struct SceneBuilder {
    scene: Scene,
}

impl SceneBuilder {
    /// Start a scene of `width`×`height` pixels.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            scene: Scene::new(width, height, Color::srgb(0.0, 0.0, 0.0)),
        }
    }

    /// Set the clear colour.
    #[must_use]
    pub const fn clear(mut self, color: Color) -> Self {
        self.scene.clear = color;
        self
    }

    /// Set the display headroom the scene asks to be presented at
    /// (`render --present`); `1.0` is the default SDR display.
    #[must_use]
    pub const fn present_headroom(mut self, headroom: f64) -> Self {
        self.scene.present_headroom = headroom;
        self
    }

    /// Declare a backdrop group `id` capturing at `scale` with `filters`,
    /// reduced into a pyramid of `levels` levels (`1` is the single-level
    /// capture); layers sample it via [`LayerBuilder::backdrop`].
    pub fn backdrop_group(
        &mut self,
        id: u32,
        filters: Vec<BackdropFilter>,
        scale: f64,
        levels: u32,
    ) -> &mut Self {
        self.scene.backdrop_groups.push(BackdropGroup {
            id,
            filters,
            scale,
            levels,
            anchor: None,
        });
        self
    }

    /// Declare a backdrop group `id` anchored at the layer carrying
    /// `anchor` as its [`crate::Layer::id`], else as
    /// [`SceneBuilder::backdrop_group`].
    pub fn backdrop_group_anchored(
        &mut self,
        id: u32,
        anchor: u32,
        filters: Vec<BackdropFilter>,
        scale: f64,
        levels: u32,
    ) -> &mut Self {
        self.scene.backdrop_groups.push(BackdropGroup {
            id,
            filters,
            scale,
            levels,
            anchor: Some(anchor),
        });
        self
    }

    /// A [`LayerBuilder`] over the root layer.
    pub const fn root(&mut self) -> LayerBuilder<'_> {
        LayerBuilder {
            layer: &mut self.scene.root,
        }
    }

    /// Finish the scene, computing its feature set.
    #[must_use]
    pub fn build(mut self) -> Scene {
        self.scene.compute_features();
        self.scene
    }
}

macro_rules! root_forward {
    ($( fn $name:ident ( $( $arg:ident : $ty:ty ),* ) ;)*) => {$(
        /// Forward to the same method on the root [`LayerBuilder`].
        #[must_use]
        pub fn $name(mut self, $( $arg : $ty ),*) -> Self {
            self.root().$name($($arg),*);
            self
        }
    )*};
}

// Some forwarded methods (e.g. `clip`) can't be `const` — they assign
// values with drop glue through a mutable reference.
#[allow(clippy::missing_const_for_fn)]
impl SceneBuilder {
    root_forward! {
        fn transform(transform: Affine);
        fn clip(shape: Shape);
        fn opacity(opacity: f64);
        fn blend(blend: BlendMode);
        fn backdrop(group: u32);
        fn backdrop_effect(effect: crate::BackdropEffectSpec);
        fn fill(shape: Shape, paint: Paint);
        fn fill_rule(shape: Shape, rule: FillRule, paint: Paint);
        fn stroke(shape: Shape, stroke: StrokeStyle, paint: Paint);
        fn shadow(shape: Shape, blur_sigma: f64, offset: [f64; 2], color: Color);
        fn glyphs(run: GlyphRun);
        fn image(image: ResourceHash, dst: Rect, sampling: Sampling);
        fn layer(f: impl FnOnce(&mut LayerBuilder));
    }
}

/// A builder for a [`Layer`] and its ordered items.
#[derive(Debug)]
pub struct LayerBuilder<'a> {
    layer: &'a mut Layer,
}

impl LayerBuilder<'_> {
    /// Set the layer transform.
    pub const fn transform(&mut self, transform: Affine) -> &mut Self {
        self.layer.transform = transform;
        self
    }

    /// Set the layer clip shape.
    pub fn clip(&mut self, shape: Shape) -> &mut Self {
        self.layer.clip = Some(shape);
        self
    }

    /// Set the group opacity.
    pub const fn opacity(&mut self, opacity: f64) -> &mut Self {
        self.layer.opacity = opacity;
        self
    }

    /// Set the blend mode.
    pub const fn blend(&mut self, blend: BlendMode) -> &mut Self {
        self.layer.blend = blend;
        self
    }

    /// Make the layer a member of backdrop group `group`: the group's
    /// filtered capture is drawn as the bottom-most content inside the
    /// layer's clip.
    pub const fn backdrop(&mut self, group: u32) -> &mut Self {
        self.layer.backdrop = Some(group);
        self
    }

    /// Set the filter applied to the layer's isolated content.
    pub fn filter(&mut self, filter: LayerFilter) -> &mut Self {
        self.layer.filter = Some(Box::new(filter));
        self
    }

    /// Set the layer's per-member [`crate::BackdropEffectSpec`] on its
    /// backdrop composite.
    pub const fn backdrop_effect(&mut self, effect: crate::BackdropEffectSpec) -> &mut Self {
        self.layer.backdrop_effect = Some(effect);
        self
    }

    /// Set the scroll offset: content and children draw translated by
    /// `-offset` inside the layer's clip.
    pub const fn scroll_offset(&mut self, offset: Vec2) -> &mut Self {
        self.layer.scroll_offset = offset;
        self
    }

    /// Make the layer projective (see [`crate::Projection`]).
    pub fn projection(&mut self, projection: crate::Projection) -> &mut Self {
        self.layer.projection = Some(Box::new(projection));
        self
    }

    /// Adds a per-frame live item (see [`Layer::live`]).
    pub fn live(&mut self, live: crate::Live) -> &mut Self {
        self.layer.live.push(live);
        self
    }

    /// Make the layer a text layer laid out from `source`; its items must
    /// be the text's lowering (see [`Layer::text`]).
    pub fn text(&mut self, source: crate::TextSource) -> &mut Self {
        self.layer.text = Some(Box::new(source));
        self
    }

    /// Set the layer's one-time motion.
    // Not `const`: `Motion::Paint` holds gradient stop `Vec`s, so
    // `Option<Motion>` drops.
    pub fn motion(&mut self, motion: Motion) -> &mut Self {
        self.layer.motion = Some(motion);
        self
    }

    /// Give the layer the `id` a [`crate::BackdropGroup::anchor`] can
    /// name: the group's capture is taken beneath this layer.
    pub const fn id(&mut self, id: u32) -> &mut Self {
        self.layer.id = Some(id);
        self
    }

    /// Add a child layer and build it in `f`.
    pub fn layer(&mut self, f: impl FnOnce(&mut LayerBuilder)) -> &mut Self {
        let mut layer = Layer::default();
        f(&mut LayerBuilder { layer: &mut layer });
        self.layer.items.push(Item::Layer(layer));
        self
    }

    /// The number of items pushed so far — the index the next item gets.
    #[must_use]
    pub const fn item_count(&self) -> usize {
        self.layer.items.len()
    }

    /// Push a [`Draw::Fill`] item.
    pub fn fill(&mut self, shape: Shape, paint: Paint) -> &mut Self {
        self.push(Draw::Fill {
            shape,
            rule: FillRule::NonZero,
            paint,
        })
    }

    /// Push a [`Draw::Fill`] item with an explicit fill rule.
    pub fn fill_rule(&mut self, shape: Shape, rule: FillRule, paint: Paint) -> &mut Self {
        self.push(Draw::Fill { shape, rule, paint })
    }

    /// Push a [`Draw::Stroke`] item.
    pub fn stroke(&mut self, shape: Shape, stroke: StrokeStyle, paint: Paint) -> &mut Self {
        self.push(Draw::Stroke {
            shape,
            stroke,
            paint,
        })
    }

    /// Push a [`Draw::Shadow`] item.
    pub fn shadow(
        &mut self,
        shape: Shape,
        blur_sigma: f64,
        offset: [f64; 2],
        color: Color,
    ) -> &mut Self {
        self.push(Draw::Shadow {
            shape,
            blur_sigma,
            offset,
            color,
        })
    }

    /// Push a [`Draw::Glyphs`] item.
    pub fn glyphs(&mut self, run: GlyphRun) -> &mut Self {
        self.push(Draw::Glyphs(run))
    }

    /// Push a [`Draw::Image`] item (8-bit sRGB PNG).
    pub fn image(&mut self, image: ResourceHash, dst: Rect, sampling: Sampling) -> &mut Self {
        self.image_encoded(image, crate::ImageEncoding::default(), dst, sampling)
    }

    /// Push a [`Draw::Image`] item with an explicit encoding.
    pub fn image_encoded(
        &mut self,
        image: ResourceHash,
        encoding: crate::ImageEncoding,
        dst: Rect,
        sampling: Sampling,
    ) -> &mut Self {
        self.push(Draw::Image {
            image,
            encoding,
            dst,
            sampling,
        })
    }

    /// Push a raw draw command.
    pub fn push(&mut self, draw: Draw) -> &mut Self {
        self.layer.items.push(Item::Draw(draw));
        self
    }

    /// Push an [`Item::Group`].
    pub fn group(&mut self, group: Group) -> &mut Self {
        self.layer.items.push(Item::Group(group));
        self
    }
}

impl Scene {
    /// Start building a scene of `width`×`height`.
    #[must_use]
    pub fn builder(width: u32, height: u32) -> SceneBuilder {
        SceneBuilder::new(width, height)
    }
}
