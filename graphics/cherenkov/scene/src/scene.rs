use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    BackdropFilter, BackdropGroup, BlendMode, Color, ColorSpace, Draw, Extend, ImageEncoding, Item,
    Layer, Paint, ResourceHash, SceneError, Shape,
};

/// The file name of the serialized scene inside a scene directory.
pub const SCENE_FILE: &str = "scene.json";
/// The name of the content-addressed resource directory inside a scene
/// directory.
pub const RESOURCES_DIR: &str = "resources";

/// A feature of the scene format that an adapter may or may not support.
///
/// A scene's `features` set is computed from its content (see
/// [`Scene::compute_features`]); an adapter maps this set to its capability
/// table and reports the scene as unsupported rather than emulating.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "feature", content = "value", rename_all = "kebab-case")]
pub enum Feature {
    /// `Fill` draw commands.
    Fill,
    /// Even-odd fill rule.
    EvenOdd,
    /// `Stroke` draw commands.
    Stroke,
    /// Dashed strokes.
    StrokeDash,
    /// Arbitrary paths.
    Path,
    /// Continuous (superellipse) corners.
    ContinuousCorners,
    /// Independent paint-coordinate transforms.
    PaintTransform,
    /// Linear gradients.
    LinearGradient,
    /// Two-point radial gradients.
    RadialGradient,
    /// Sweep gradients.
    SweepGradient,
    /// Bilinear mesh-gradient paint.
    MeshGradient,
    /// `Image` draw commands.
    Image,
    /// Image pattern paints.
    ImagePaint,
    /// A non-trivial blend mode (the payload is the mode).
    Blend(BlendMode),
    /// A group compositing in a non-linear space (the payload is the space).
    BlendSpace(crate::BlendSpace),
    /// Layer clips.
    Clip,
    /// A non-zero `scroll_offset` on a layer.
    Scroll,
    /// A `motion` on a layer.
    Animation,
    /// Group opacity below `1.0`.
    Opacity,
    /// A layer filter.
    Filter,
    /// `Shadow` draw commands.
    Shadow,
    /// `Glyphs` draw commands.
    Glyphs,
    /// Glyph runs drawn with a stroke style.
    GlyphStroke,
    /// Per-glyph transforms.
    GlyphTransform,
    /// Variable-font normalized coordinates.
    FontVariations,
    /// Any colour channel above `1.0`.
    HdrColor,
    /// Colours outside the sRGB gamut (P3, Rec. 2020).
    WideGamut,
    /// A gradient or image pattern with `Extend::None` (transparent outside
    /// the domain). Several paint APIs only offer pad/repeat/reflect.
    ExtendNone,
    /// An image resource in `Rgba16F` encoding (half-float texels).
    ImageF16,
    /// An image resource in a non-sRGB colour space (payload is the space).
    ImageColorSpace(crate::ImageColorSpace),
    /// A gradient whose stops are interpolated in the payload space. Engines
    /// that cannot interpolate in a declared space must report it instead of
    /// silently remapping.
    InterpolationSpace(ColorSpace),
    /// A layer sampling a backdrop group.
    Backdrop,
    /// A backdrop group whose capture runs through a Gaussian blur.
    BackdropBlur,
    /// A backdrop group whose capture runs through a colour matrix.
    BackdropColorMatrix,
    /// A member layer carries a per-member backdrop sampling effect.
    BackdropEffect,
    /// A layer with a projective pose.
    Projective,
}

/// The scene's working space. Only linear Display P3 exists today; the enum
/// keeps the contract explicit for adapters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkingSpace {
    /// Linear-light Display P3, premultiplied compositing in `f64` in the
    /// oracle, HDR-capable.
    #[default]
    LinearDisplayP3,
}

/// An engine-neutral scene: a pixel size, a clear colour, and a layer tree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    /// Width in physical pixels.
    pub width: u32,
    /// Height in physical pixels.
    pub height: u32,
    /// The working space (always linear Display P3 today).
    pub working_space: WorkingSpace,
    /// The colour the scene is cleared to before drawing.
    pub clear: Color,
    /// The display headroom the scene asks to be presented at — the
    /// `render --present` corpus passes it to the oracle's presentation
    /// functions and announces it on the surface's `Display`. `1.0` (the
    /// default) is SDR and stays out of `scene.json`.
    #[serde(
        default = "Scene::default_present_headroom",
        skip_serializing_if = "Scene::is_default_present_headroom"
    )]
    pub present_headroom: f64,
    /// The features this scene uses.
    pub features: BTreeSet<Feature>,
    /// The backdrop groups member layers sample (`Layer::backdrop`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backdrop_groups: Vec<BackdropGroup>,
    /// The root layer.
    pub root: Layer,
}

impl Scene {
    /// Create an empty scene of `width`×`height` cleared to `clear`.
    #[must_use]
    pub fn new(width: u32, height: u32, clear: Color) -> Self {
        Self {
            width,
            height,
            working_space: WorkingSpace::LinearDisplayP3,
            clear,
            present_headroom: Self::default_present_headroom(),
            features: BTreeSet::new(),
            backdrop_groups: Vec::new(),
            root: Layer::default(),
        }
    }

    /// The default presentation headroom: SDR.
    pub(crate) const fn default_present_headroom() -> f64 {
        1.0
    }

    /// Serde helper: the default headroom is left out of `scene.json`.
    #[cfg_attr(
        not(target_arch = "wasm32"),
        expect(
            clippy::trivially_copy_pass_by_ref,
            reason = "serde's skip_serializing_if takes a reference"
        )
    )]
    #[expect(clippy::float_cmp, reason = "the default test is exact equality")]
    pub(crate) fn is_default_present_headroom(headroom: &f64) -> bool {
        *headroom == Self::default_present_headroom()
    }

    /// Recompute `self.features` from the layer tree. Called by builders;
    /// call again after mutating a scene by hand.
    pub fn compute_features(&mut self) {
        let mut f = BTreeSet::new();
        if self.clear.is_hdr() {
            f.insert(Feature::HdrColor);
        }
        if self.clear.is_wide_gamut() {
            f.insert(Feature::WideGamut);
        }
        for group in &self.backdrop_groups {
            for filter in &group.filters {
                match filter {
                    BackdropFilter::GaussianBlur { .. } => {
                        f.insert(Feature::BackdropBlur);
                    }
                    BackdropFilter::ColorMatrix { .. } => {
                        f.insert(Feature::BackdropColorMatrix);
                    }
                }
            }
        }
        collect_layer_features(&self.root, &mut f);
        self.features = f;
    }

    /// Load `scene.json` from a scene directory.
    ///
    /// # Errors
    /// Returns [`SceneError`] on I/O or JSON failures.
    pub fn load(dir: &Path) -> Result<Self, SceneError> {
        let text = std::fs::read_to_string(dir.join(SCENE_FILE))?;
        let mut scene: Self = serde_json::from_str(&text)?;
        // The stored `features` set is advisory input, not truth: recompute
        // it from the layer tree and reject scenes that lie about what they
        // use (a stale or hand-edited file would otherwise bypass an
        // adapter's capability check).
        scene.validate_image_encodings()?;
        let declared = std::mem::take(&mut scene.features);
        scene.compute_features();
        if scene.features != declared {
            return Err(SceneError::FeatureMismatch {
                declared: declared.into_iter().collect(),
                computed: scene.features.iter().cloned().collect(),
            });
        }
        scene.validate_backdrops()?;
        scene.validate_projections()?;
        scene.validate_text()?;
        Ok(scene)
    }

    /// A text layer's source must name fonts and character-boundary
    /// ranges, and its items must be a text lowering: glyph runs, non-zero
    /// rectangle fills and plain groups of glyph runs, with no live items
    /// or paint motion.
    fn validate_text(&self) -> Result<(), SceneError> {
        fn source(text: &crate::TextSource) -> Result<(), SceneError> {
            if text.fonts.is_empty() {
                return Err(SceneError::InvalidText("the font stack is empty"));
            }
            for span in &text.spans {
                let [start, end] = span.range;
                if start > end
                    || !text.text.is_char_boundary(start)
                    || !text.text.is_char_boundary(end)
                {
                    return Err(SceneError::InvalidText(
                        "a span range is not a character-boundary range of the text",
                    ));
                }
            }
            Ok(())
        }
        /// A group a synthetic bold isolates its glyph runs in: opaque,
        /// normally blended, in linear space.
        #[expect(
            clippy::float_cmp,
            reason = "only the exact default opacity leaves the group plain"
        )]
        fn plain_glyph_group(group: &crate::Group) -> bool {
            group.opacity == 1.0
                && group.blend == crate::BlendMode::Normal
                && group.blend_space == crate::BlendSpace::Linear
                && group
                    .items
                    .iter()
                    .all(|item| matches!(item, crate::GroupItem::Draw(Draw::Glyphs(_))))
        }
        fn walk(layer: &Layer) -> Result<(), SceneError> {
            if let Some(text) = &layer.text {
                source(text)?;
                if !layer.live.is_empty()
                    || matches!(layer.motion, Some(crate::Motion::Paint { .. }))
                {
                    return Err(SceneError::InvalidText(
                        "a text layer's items are not live or animated",
                    ));
                }
                let lowered = layer.items.iter().all(|item| match item {
                    Item::Draw(
                        Draw::Glyphs(_)
                        | Draw::Fill {
                            shape: crate::Shape::Rect(_),
                            rule: crate::FillRule::NonZero,
                            ..
                        },
                    ) => true,
                    Item::Group(group) => plain_glyph_group(group),
                    Item::Draw(_) | Item::Layer(_) => false,
                });
                if !lowered {
                    return Err(SceneError::InvalidText(
                        "a text layer's items are its glyph runs, decoration rectangles \
                         and plain groups of glyph runs",
                    ));
                }
            }
            for item in &layer.items {
                if let Item::Layer(l) = item {
                    walk(l)?;
                }
            }
            Ok(())
        }
        walk(&self.root)
    }

    /// The scene root is never projective (it is the surface), a tilt
    /// motion needs a projection to animate, and a rotation motion's
    /// affine base recovery does not apply to a projective pose.
    fn validate_projections(&self) -> Result<(), SceneError> {
        fn walk(layer: &Layer) -> Result<(), SceneError> {
            match (&layer.motion, &layer.projection) {
                (Some(crate::Motion::Tilt { .. }), None) => {
                    return Err(SceneError::InvalidProjection(
                        "a tilt motion needs a projection",
                    ));
                }
                (Some(crate::Motion::Rotation { .. }), Some(_)) => {
                    return Err(SceneError::InvalidProjection(
                        "a rotation motion on a projective layer",
                    ));
                }
                _ => {}
            }
            for item in &layer.items {
                if let Item::Layer(l) = item {
                    walk(l)?;
                }
            }
            Ok(())
        }
        if self.root.projection.is_some() {
            return Err(SceneError::InvalidProjection(
                "the scene root is projective",
            ));
        }
        walk(&self.root)
    }

    /// Every layer's `backdrop` must name a declared group and carry a clip.
    fn validate_backdrops(&self) -> Result<(), SceneError> {
        fn walk(layer: &Layer, groups: &[BackdropGroup]) -> Result<(), SceneError> {
            if let Some(id) = layer.backdrop {
                if layer.clip.is_none() {
                    return Err(SceneError::BackdropMemberUnclipped(id));
                }
                if !groups.iter().any(|g| g.id == id) {
                    return Err(SceneError::UnknownBackdropGroup(id));
                }
            }
            if let Some(effect) = &layer.backdrop_effect {
                if layer.backdrop.is_none() {
                    return Err(SceneError::BackdropEffectWithoutGroup);
                }
                Scene::validate_effect(effect)?;
            }
            for item in &layer.items {
                if let Item::Layer(l) = item {
                    walk(l, groups)?;
                }
            }
            Ok(())
        }
        walk(&self.root, &self.backdrop_groups)
    }

    /// A `backdrop_effect`'s parameters must be finite and in range.
    fn validate_effect(effect: &crate::BackdropEffectSpec) -> Result<(), SceneError> {
        use crate::BackdropEffectSpec as E;
        match effect {
            E::ColorMatrix { matrix } if matrix.iter().all(|v| v.is_finite()) => Ok(()),
            E::ColorMatrix { .. } => {
                Err(SceneError::InvalidBackdropEffect("non-finite matrix entry"))
            }
            E::Refraction { depth, .. } if !(depth.is_finite() && *depth > 0.0) => {
                Err(SceneError::InvalidBackdropEffect(
                    "refraction depth must be a finite positive number",
                ))
            }
            E::Refraction { strength, .. } if !(strength.is_finite() && *strength >= 0.0) => {
                Err(SceneError::InvalidBackdropEffect(
                    "refraction strength must be a finite non-negative number",
                ))
            }
            E::Refraction { .. } => Ok(()),
            E::RimLight { width, .. } if !(width.is_finite() && *width > 0.0) => {
                Err(SceneError::InvalidBackdropEffect(
                    "rim-light width must be a finite positive number",
                ))
            }
            E::RimLight { color, gain, .. } => {
                if color.iter().all(|v| v.is_finite()) && gain.is_finite() && *gain >= 0.0 {
                    Ok(())
                } else {
                    Err(SceneError::InvalidBackdropEffect(
                        "rim-light colour and gain must be finite, gain non-negative",
                    ))
                }
            }
        }
    }

    /// Write `scene.json` into `dir` (creating it), without touching
    /// `resources/`.
    ///
    /// # Errors
    /// Returns [`SceneError`] on I/O or JSON failures.
    pub fn save(&self, dir: &Path) -> Result<(), SceneError> {
        std::fs::create_dir_all(dir)?;
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        std::fs::write(dir.join(SCENE_FILE), text)?;
        Ok(())
    }

    /// The `resources/` directory of a scene directory.
    #[must_use]
    pub fn resources_dir(dir: &Path) -> PathBuf {
        dir.join(RESOURCES_DIR)
    }

    /// Insert `bytes` into `dir`'s resource store and return the hash.
    ///
    /// # Errors
    /// Returns [`SceneError`] on I/O failure.
    pub fn store_resource(dir: &Path, bytes: &[u8]) -> Result<ResourceHash, SceneError> {
        let hash = ResourceHash::of(bytes);
        let resources = Self::resources_dir(dir);
        std::fs::create_dir_all(&resources)?;
        std::fs::write(resources.join(hash.file_name()), bytes)?;
        Ok(hash)
    }

    /// Read the resource blob `hash` from `dir`.
    ///
    /// # Errors
    /// [`SceneError::MissingResource`] if the blob is absent, [`SceneError::Io`]
    /// on read failure.
    pub fn resource(dir: &Path, hash: ResourceHash) -> Result<Vec<u8>, SceneError> {
        let path = Self::resources_dir(dir).join(hash.file_name());
        match std::fs::read(&path) {
            Ok(b) => Ok(b),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(SceneError::MissingResource(hash))
            }
            Err(e) => Err(SceneError::Io(e)),
        }
    }

    /// All resource hashes referenced by the scene.
    #[must_use]
    pub fn resource_refs(&self) -> Vec<ResourceHash> {
        let mut out = Vec::new();
        collect_resource_refs(&self.root, &mut out);
        out
    }
}

fn collect_draw_resource_refs(draw: &Draw, out: &mut Vec<ResourceHash>) {
    match draw {
        Draw::Glyphs(run) => {
            out.push(run.font);
            collect_paint_resources(&run.paint, out);
        }
        Draw::Image { image, .. } => out.push(*image),
        Draw::Fill { paint, .. } | Draw::Stroke { paint, .. } => {
            collect_paint_resources(paint, out);
        }
        Draw::Shadow { .. } => {}
    }
}

fn collect_group_resource_refs(group: &crate::Group, out: &mut Vec<ResourceHash>) {
    for item in &group.items {
        match item {
            crate::GroupItem::Draw(d) => collect_draw_resource_refs(d, out),
            crate::GroupItem::Group(g) => collect_group_resource_refs(g, out),
        }
    }
}

fn collect_resource_refs(layer: &Layer, out: &mut Vec<ResourceHash>) {
    if let Some(crate::LayerFilter::BlendImage { image, .. }) = layer.filter.as_deref() {
        out.push(*image);
    }
    if let Some(text) = &layer.text {
        out.extend(&text.fonts);
    }
    for item in &layer.items {
        match item {
            Item::Layer(l) => collect_resource_refs(l, out),
            Item::Draw(d) => collect_draw_resource_refs(d, out),
            Item::Group(g) => collect_group_resource_refs(g, out),
        }
    }
}

impl Scene {
    /// Validates every `Draw::Image`/`Paint::Image` encoding declaration.
    ///
    /// # Errors
    /// [`SceneError::InvalidImageEncoding`] on the first bad declaration.
    fn validate_image_encodings(&self) -> Result<(), SceneError> {
        fn paint_encoding(paint: &Paint) -> Option<&ImageEncoding> {
            match paint {
                Paint::Transformed { paint, .. } => paint_encoding(paint),
                Paint::Image(ip) => Some(&ip.encoding),
                _ => None,
            }
        }
        fn draw_encoding(draw: &Draw) -> Result<(), SceneError> {
            match draw {
                Draw::Image { encoding, .. } => encoding.validate(),
                Draw::Fill { paint, .. }
                | Draw::Stroke { paint, .. }
                | Draw::Glyphs(crate::GlyphRun { paint, .. }) => {
                    paint_encoding(paint).map_or(Ok(()), super::draw::ImageEncoding::validate)
                }
                Draw::Shadow { .. } => Ok(()),
            }
        }
        fn visit_group(item_group: &crate::Group) -> Result<(), SceneError> {
            for item in &item_group.items {
                match item {
                    crate::GroupItem::Draw(d) => draw_encoding(d)?,
                    crate::GroupItem::Group(g) => visit_group(g)?,
                }
            }
            Ok(())
        }
        fn visit(layer: &Layer) -> Result<(), SceneError> {
            for item in &layer.items {
                match item {
                    Item::Layer(l) => visit(l)?,
                    Item::Draw(d) => draw_encoding(d)?,
                    Item::Group(g) => visit_group(g)?,
                }
            }
            Ok(())
        }
        visit(&self.root)
    }
}

fn collect_paint_resources(paint: &Paint, out: &mut Vec<ResourceHash>) {
    match paint {
        Paint::Transformed { paint, .. } => collect_paint_resources(paint, out),
        Paint::Image(image) => out.push(image.image),
        _ => {}
    }
}

fn collect_paint_features(paint: &Paint, f: &mut BTreeSet<Feature>) {
    match paint {
        Paint::Transformed { paint, .. } => {
            f.insert(Feature::PaintTransform);
            collect_paint_features(paint, f);
        }
        Paint::Solid(c) => collect_color_features(c, f),
        Paint::Linear(g) => {
            f.insert(Feature::LinearGradient);
            f.insert(Feature::InterpolationSpace(g.interpolation));
            if g.extend == Extend::None {
                f.insert(Feature::ExtendNone);
            }
            collect_stops(&g.stops, f);
        }
        Paint::Radial(g) => {
            f.insert(Feature::RadialGradient);
            f.insert(Feature::InterpolationSpace(g.interpolation));
            if g.extend == Extend::None {
                f.insert(Feature::ExtendNone);
            }
            collect_stops(&g.stops, f);
        }
        Paint::Sweep(g) => {
            f.insert(Feature::SweepGradient);
            f.insert(Feature::InterpolationSpace(g.interpolation));
            if g.extend == Extend::None {
                f.insert(Feature::ExtendNone);
            }
            collect_stops(&g.stops, f);
        }
        Paint::Mesh(mesh) => {
            f.insert(Feature::MeshGradient);
            for color in mesh.colors() {
                collect_color_features(color, f);
            }
        }
        Paint::Image(ip) => {
            f.insert(Feature::ImagePaint);
            collect_encoding_features(&ip.encoding, f);
            if ip.extend_x == Extend::None || ip.extend_y == Extend::None {
                f.insert(Feature::ExtendNone);
            }
        }
    }
}

fn collect_encoding_features(encoding: &ImageEncoding, f: &mut BTreeSet<Feature>) {
    match encoding {
        ImageEncoding::Png { color_space } => {
            if *color_space != crate::ImageColorSpace::Srgb {
                f.insert(Feature::ImageColorSpace(*color_space));
            }
        }
        ImageEncoding::Rgba16F { color_space, .. } => {
            f.insert(Feature::ImageF16);
            f.insert(Feature::ImageColorSpace(*color_space));
        }
    }
}

fn collect_stops(stops: &[crate::draw::GradientStop], f: &mut BTreeSet<Feature>) {
    for s in stops {
        collect_color_features(&s.color, f);
    }
}

fn collect_color_features(c: &Color, f: &mut BTreeSet<Feature>) {
    if c.is_hdr() {
        f.insert(Feature::HdrColor);
    }
    if c.is_wide_gamut() {
        f.insert(Feature::WideGamut);
    }
}

fn collect_shape_features(shape: &Shape, f: &mut BTreeSet<Feature>) {
    match shape {
        Shape::Continuous(_) => {
            f.insert(Feature::ContinuousCorners);
        }
        Shape::Path { .. } => {
            f.insert(Feature::Path);
        }
        _ => {}
    }
}

fn collect_draw_features(draw: &Draw, f: &mut BTreeSet<Feature>) {
    match draw {
        Draw::Fill { shape, rule, paint } => {
            f.insert(Feature::Fill);
            if *rule == crate::FillRule::EvenOdd {
                f.insert(Feature::EvenOdd);
            }
            collect_shape_features(shape, f);
            collect_paint_features(paint, f);
        }
        Draw::Stroke {
            shape,
            stroke,
            paint,
        } => {
            f.insert(Feature::Stroke);
            if !stroke.dash_pattern.is_empty() {
                f.insert(Feature::StrokeDash);
            }
            collect_shape_features(shape, f);
            collect_paint_features(paint, f);
        }
        Draw::Shadow { shape, color, .. } => {
            f.insert(Feature::Shadow);
            collect_shape_features(shape, f);
            collect_color_features(color, f);
        }
        Draw::Glyphs(run) => {
            f.insert(Feature::Glyphs);
            if let Some(stroke) = &run.stroke {
                f.insert(Feature::GlyphStroke);
                if !stroke.dash_pattern.is_empty() {
                    f.insert(Feature::StrokeDash);
                }
            }
            if run.glyphs.iter().any(|g| g.transform.is_some()) {
                f.insert(Feature::GlyphTransform);
            }
            if !run.normalized_coords.is_empty() {
                f.insert(Feature::FontVariations);
            }
            collect_paint_features(&run.paint, f);
        }
        Draw::Image { encoding, .. } => {
            f.insert(Feature::Image);
            collect_encoding_features(encoding, f);
        }
    }
}

fn collect_group_features(group: &crate::Group, f: &mut BTreeSet<Feature>) {
    if group.opacity < 1.0 {
        f.insert(Feature::Opacity);
    }
    if group.blend != BlendMode::Normal {
        f.insert(Feature::Blend(group.blend));
    }
    if group.blend_space != crate::BlendSpace::Linear {
        f.insert(Feature::BlendSpace(group.blend_space));
    }
    for item in &group.items {
        match item {
            crate::GroupItem::Draw(d) => collect_draw_features(d, f),
            crate::GroupItem::Group(g) => collect_group_features(g, f),
        }
    }
}

fn collect_layer_features(layer: &Layer, f: &mut BTreeSet<Feature>) {
    if layer.clip.is_some() {
        f.insert(Feature::Clip);
    }
    if layer.opacity < 1.0 {
        f.insert(Feature::Opacity);
    }
    if layer.filter.is_some() {
        f.insert(Feature::Filter);
    }
    if layer.blend != BlendMode::Normal {
        f.insert(Feature::Blend(layer.blend));
    }
    if layer.scroll_offset != kurbo::Vec2::ZERO {
        f.insert(Feature::Scroll);
    }
    if layer.motion.is_some() {
        f.insert(Feature::Animation);
    }
    if layer.backdrop.is_some() {
        f.insert(Feature::Backdrop);
    }
    if layer.backdrop_effect.is_some() {
        f.insert(Feature::BackdropEffect);
    }
    if layer.projection.is_some() {
        f.insert(Feature::Projective);
    }
    for item in &layer.items {
        match item {
            Item::Layer(l) => collect_layer_features(l, f),
            Item::Draw(d) => collect_draw_features(d, f),
            Item::Group(g) => collect_group_features(g, f),
        }
    }
}
