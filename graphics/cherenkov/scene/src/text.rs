//! Text a renderer lays out itself: the parley input of a text layer.
//!
//! A layer with a [`TextSource`] holds, in its `items`, the text's lowering
//! — glyph runs, decoration rectangles, and the groups that isolate a
//! synthetic bold — computed by the corpus
//! generator from the same shaping. The oracle and every renderer without
//! a text-layout API draw those items; the Cherenkov adapters shape the
//! source with [`TextSource::shape`] and record it through the engine's
//! parley adapter instead, so the oracle checks that adapter's lowering.

use kurbo::Point;
use serde::{Deserialize, Serialize};

use crate::{Paint, ResourceHash};

/// Text and its styles, as parley's input.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextSource {
    /// The text.
    pub text: String,
    /// The font stack in fallback order: BLAKE3 hashes of font blobs in
    /// `resources/` (collection index 0). Only these fonts are available.
    pub fonts: Vec<ResourceHash>,
    /// Font size in pixels.
    pub size: f32,
    /// The width lines break at; `None` breaks at hard breaks only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_advance: Option<f32>,
    /// Where the layout's top-left corner lands in the layer.
    pub origin: Point,
    /// The text's brush.
    pub paint: Paint,
    /// Styled byte ranges, applied over the defaults in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<TextSpan>,
}

/// A styled byte range of a [`TextSource`]'s text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextSpan {
    /// The byte range, on character boundaries.
    pub range: [usize; 2],
    /// The range's brush.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paint: Option<Paint>,
    /// Italic style: a font without an italic face or axis is slanted
    /// synthetically.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub italic: bool,
    /// Bold weight: a font without a bold face or weight axis is
    /// emboldened synthetically.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
    /// An underline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub underline: Option<TextDecoration>,
    /// A strikethrough.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strikethrough: Option<TextDecoration>,
}

/// An underline or strikethrough. Unset fields take the text brush and
/// the font's metrics.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TextDecoration {
    /// The decoration's brush.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paint: Option<Paint>,
    /// The top edge's distance above the baseline, in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<f32>,
    /// The thickness, in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
}

#[cfg(feature = "text")]
pub use shaping::{FontResources, ShapedText};

#[cfg(feature = "text")]
mod shaping {
    use std::borrow::Cow;
    use std::sync::Arc;

    use parley::fontique::{Blob, Collection, CollectionOptions, SourceCache};
    use parley::{
        Brush, FontContext, FontFamily, FontFamilyName, FontStyle, FontWeight, Layout,
        LayoutContext, StyleProperty,
    };

    use super::{TextDecoration, TextSource};
    use crate::{Paint, ResourceHash, SceneError};

    /// A [`TextSource`] laid out by parley.
    #[derive(Debug)]
    pub struct ShapedText<B: Brush> {
        /// The layout, with its top-left corner at the origin (the source's
        /// `origin` is not applied).
        pub layout: Layout<B>,
        /// The scene resource behind each font the layout can name.
        pub resources: FontResources,
    }

    /// The scene resource of each font blob a [`ShapedText`] can name, by
    /// parley blob id.
    #[derive(Debug)]
    pub struct FontResources(Vec<(u64, ResourceHash)>);

    impl FontResources {
        /// The scene resource a run's font came from.
        #[must_use]
        pub fn resource(&self, font: &parley::FontData) -> Option<ResourceHash> {
            self.0
                .iter()
                .find(|(id, _)| *id == font.data.id())
                .map(|(_, hash)| *hash)
        }
    }

    impl TextSource {
        /// Lays the text out with parley against its font stack alone (no
        /// system fonts), turning each paint into a brush with `brush`.
        ///
        /// # Errors
        /// [`SceneError::MissingResource`] when `blob` has no bytes for a
        /// font, [`SceneError::InvalidText`] when a font names no family,
        /// and any error `brush` returns.
        pub fn shape<'a, B: Brush, E: From<SceneError>>(
            &self,
            blob: impl Fn(&ResourceHash) -> Option<&'a [u8]>,
            mut brush: impl FnMut(&Paint) -> Result<B, E>,
        ) -> Result<ShapedText<B>, E> {
            let mut fcx = FontContext {
                collection: Collection::new(CollectionOptions {
                    shared: false,
                    system_fonts: false,
                }),
                source_cache: SourceCache::default(),
            };
            let mut families = Vec::with_capacity(self.fonts.len());
            let mut fonts = Vec::with_capacity(self.fonts.len());
            for hash in &self.fonts {
                let bytes = blob(hash).ok_or(SceneError::MissingResource(*hash))?;
                let data = Blob::new(Arc::new(bytes.to_vec()));
                fonts.push((data.id(), *hash));
                let registered = fcx.collection.register_fonts(data, None);
                let family = registered
                    .first()
                    .and_then(|(family, _)| fcx.collection.family_name(*family))
                    .ok_or(SceneError::InvalidText("a text font names no family"))?;
                families.push(FontFamilyName::Named(Cow::Owned(family.to_owned())));
            }
            let mut lcx = LayoutContext::new();
            let mut builder = lcx.ranged_builder(&mut fcx, &self.text, 1.0, false);
            builder.push_default(StyleProperty::FontFamily(FontFamily::List(Cow::Owned(
                families,
            ))));
            builder.push_default(StyleProperty::FontSize(self.size));
            builder.push_default(StyleProperty::Brush(brush(&self.paint)?));
            for span in &self.spans {
                let range = span.range[0]..span.range[1];
                if let Some(paint) = &span.paint {
                    builder.push(StyleProperty::Brush(brush(paint)?), range.clone());
                }
                if span.italic {
                    builder.push(StyleProperty::FontStyle(FontStyle::Italic), range.clone());
                }
                if span.bold {
                    builder.push(StyleProperty::FontWeight(FontWeight::BOLD), range.clone());
                }
                if let Some(underline) = &span.underline {
                    let TextDecoration {
                        paint,
                        offset,
                        size,
                    } = underline;
                    builder.push(StyleProperty::Underline(true), range.clone());
                    builder.push(StyleProperty::UnderlineOffset(*offset), range.clone());
                    builder.push(StyleProperty::UnderlineSize(*size), range.clone());
                    let paint = paint.as_ref().map(&mut brush).transpose()?;
                    builder.push(StyleProperty::UnderlineBrush(paint), range.clone());
                }
                if let Some(strikethrough) = &span.strikethrough {
                    let TextDecoration {
                        paint,
                        offset,
                        size,
                    } = strikethrough;
                    builder.push(StyleProperty::Strikethrough(true), range.clone());
                    builder.push(StyleProperty::StrikethroughOffset(*offset), range.clone());
                    builder.push(StyleProperty::StrikethroughSize(*size), range.clone());
                    let paint = paint.as_ref().map(&mut brush).transpose()?;
                    builder.push(StyleProperty::StrikethroughBrush(paint), range);
                }
            }
            let mut layout = builder.build(&self.text);
            layout.break_all_lines(self.max_advance);
            Ok(ShapedText {
                layout,
                resources: FontResources(fonts),
            })
        }
    }
}
