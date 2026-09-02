//! Parsed SVG documents, ready to draw into a scene.

use waterui_core::layout::Size;

/// A parsed SVG document.
#[derive(Debug)]
pub struct SvgSceneData {
    svg_tree: usvg::Tree,
}

impl SvgSceneData {
    /// Parses SVG content.
    #[must_use]
    pub fn parse(svg_content: &str) -> Self {
        let svg_tree = usvg::Tree::from_str(svg_content, &usvg::Options::default())
            .expect("failed to parse SVG content");
        let svg_size = svg_tree.size();
        assert!(
            svg_size.width().is_finite()
                && svg_size.height().is_finite()
                && svg_size.width() > 0.0
                && svg_size.height() > 0.0,
            "SVG must have positive finite dimensions, got {}x{}",
            svg_size.width(),
            svg_size.height()
        );

        Self { svg_tree }
    }

    /// The document's own size, from its `viewBox` / `width` / `height`.
    ///
    /// This is the size an SVG *is*: `parse` has already established it is finite
    /// and positive, so an icon with no frame around it measures at the size its
    /// author drew, and one given a single axis keeps this document's aspect ratio.
    #[must_use]
    pub fn intrinsic_size(&self) -> Size {
        let size = self.svg_tree.size();
        Size::new(size.width(), size.height())
    }

    /// The transform that fits this document into `width` × `height`.
    ///
    /// Aspect ratio is preserved and the result centred, which is what an icon
    /// in a fixed-size slot wants.
    #[must_use]
    pub fn fitting_transform(&self, width: f32, height: f32) -> kurbo::Affine {
        let size = self.svg_tree.size();
        let scale = (width / size.width()).min(height / size.height());
        let offset_x = f64::from(size.width().mul_add(-scale, width) / 2.0);
        let offset_y = f64::from(size.height().mul_add(-scale, height) / 2.0);
        kurbo::Affine::translate((offset_x, offset_y)) * kurbo::Affine::scale(f64::from(scale))
    }

    /// Draws this document into a scene, whichever engine backs it.
    ///
    /// Going through [`waterui_graphics::Scene2D`] rather than appending a
    /// pre-built `vello::Scene` is what lets an icon render on an engine other
    /// than Vello classic — the simulator has no indirect execution, and an
    /// icon that cannot draw there takes the process down with it.
    pub fn draw(&self, scene: &mut dyn waterui_graphics::Scene2D, width: f32, height: f32) {
        crate::tree_renderer::render_tree(
            scene,
            &self.svg_tree,
            self.fitting_transform(width, height),
        );
    }
}

/// Placeholder token used by reactive SVG templates for tint substitution.
pub const SVG_COLOR_PLACEHOLDER: &str = "__WATERUI_SVG_COLOR__";
