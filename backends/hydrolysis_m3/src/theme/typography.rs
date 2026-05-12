use waterui::text::font::{FontWeight, ResolvedFont};
use waterui::theme::FontSettings;

const MATERIAL_TYPEFACE: &str = "Roboto";

const fn font(size: f32, weight: FontWeight) -> ResolvedFont {
    ResolvedFont::with_static_family(size, weight, MATERIAL_TYPEFACE)
}

pub(crate) fn settings() -> FontSettings {
    FontSettings::new()
        .body(font(16.0, FontWeight::Normal))
        .title(font(22.0, FontWeight::Normal))
        .headline(font(24.0, FontWeight::Normal))
        .subheadline(font(16.0, FontWeight::Medium))
        .caption(font(12.0, FontWeight::Normal))
        .footnote(font(11.0, FontWeight::Medium))
}

#[cfg(test)]
mod tests {
    use super::{MATERIAL_TYPEFACE, settings};
    use waterui::Plugin as _;
    use waterui::env::Environment;
    use waterui::reactive::Signal as _;
    use waterui::text::font::{Body, Caption, FontWeight, Footnote, Headline, Subheadline, Title};
    use waterui_core::resolve::Resolvable as _;

    fn assert_material_font(
        font: waterui::text::font::ResolvedFont,
        expected_size: f32,
        expected_weight: FontWeight,
    ) {
        assert_eq!(font.size, expected_size);
        assert_eq!(font.weight, expected_weight);
        assert_eq!(font.family.as_deref(), Some(MATERIAL_TYPEFACE));
    }

    #[test]
    fn font_slots_match_material_web_v0_192_type_scale() {
        let mut env = Environment::new();
        waterui::theme::Theme::new()
            .fonts(settings())
            .install(&mut env);

        assert_material_font(Body.resolve(&env).get(), 16.0, FontWeight::Normal);
        assert_material_font(Title.resolve(&env).get(), 22.0, FontWeight::Normal);
        assert_material_font(Headline.resolve(&env).get(), 24.0, FontWeight::Normal);
        assert_material_font(Subheadline.resolve(&env).get(), 16.0, FontWeight::Medium);
        assert_material_font(Caption.resolve(&env).get(), 12.0, FontWeight::Normal);
        assert_material_font(Footnote.resolve(&env).get(), 11.0, FontWeight::Medium);
    }
}
