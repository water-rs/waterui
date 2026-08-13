use waterui::reactive::{Computed, Signal};
use waterui::text::font::{
    Body, Caption, Font, FontWeight, Footnote, Headline, ResolvedFont, Subheadline, Title,
};
use waterui::theme::install_font_signal;
use waterui_core::{Environment, resolve::Resolvable};

const MATERIAL_TYPEFACE: &str = "Roboto, sans-serif";

const fn font(
    size: f32,
    weight: FontWeight,
    line_height: f32,
    letter_spacing: f32,
) -> ResolvedFont {
    ResolvedFont::with_static_family(size, weight, MATERIAL_TYPEFACE)
        .with_typography_metrics(line_height, letter_spacing)
}

fn install_default<T: 'static>(env: &mut Environment, value: ResolvedFont) {
    if env.query::<T, Computed<ResolvedFont>>().is_none() {
        install_font_signal::<T>(env, Computed::constant(value));
    }
}

pub fn install_defaults(env: &mut Environment) {
    install_default::<Body>(env, font(16.0, FontWeight::Normal, 24.0, 0.15));
    install_default::<Title>(env, font(22.0, FontWeight::Normal, 28.0, 0.0));
    install_default::<Headline>(env, font(24.0, FontWeight::Normal, 32.0, 0.0));
    install_default::<Subheadline>(env, font(16.0, FontWeight::Medium, 24.0, 0.15));
    install_default::<Caption>(env, font(12.0, FontWeight::Normal, 16.0, 0.4));
    install_default::<Footnote>(env, font(11.0, FontWeight::Medium, 16.0, 0.5));
}

#[derive(Debug, Clone, Copy)]
pub struct LabelLarge;

impl Resolvable for LabelLarge {
    type Resolved = ResolvedFont;

    fn resolve(&self, _env: &Environment) -> impl Signal<Output = Self::Resolved> {
        Computed::constant(font(14.0, FontWeight::Medium, 20.0, 0.1))
    }
}

pub fn label_large() -> Font {
    Font::new(LabelLarge)
}

#[derive(Debug, Clone, Copy)]
pub struct LabelMedium;

impl Resolvable for LabelMedium {
    type Resolved = ResolvedFont;

    fn resolve(&self, _env: &Environment) -> impl Signal<Output = Self::Resolved> {
        Computed::constant(font(12.0, FontWeight::Medium, 16.0, 0.5))
    }
}

pub fn label_medium() -> Font {
    Font::new(LabelMedium)
}

#[derive(Debug, Clone, Copy)]
pub struct LabelSmall;

impl Resolvable for LabelSmall {
    type Resolved = ResolvedFont;

    fn resolve(&self, _env: &Environment) -> impl Signal<Output = Self::Resolved> {
        Computed::constant(font(11.0, FontWeight::Medium, 16.0, 0.5))
    }
}

pub fn label_small() -> Font {
    Font::new(LabelSmall)
}

#[derive(Debug, Clone, Copy)]
pub struct BodyLarge;

impl Resolvable for BodyLarge {
    type Resolved = ResolvedFont;

    fn resolve(&self, _env: &Environment) -> impl Signal<Output = Self::Resolved> {
        Computed::constant(font(16.0, FontWeight::Normal, 24.0, 0.15))
    }
}

pub fn body_large() -> Font {
    Font::new(BodyLarge)
}

#[derive(Debug, Clone, Copy)]
pub struct BodyMedium;

impl Resolvable for BodyMedium {
    type Resolved = ResolvedFont;

    fn resolve(&self, _env: &Environment) -> impl Signal<Output = Self::Resolved> {
        Computed::constant(font(14.0, FontWeight::Normal, 20.0, 0.25))
    }
}

pub fn body_medium() -> Font {
    Font::new(BodyMedium)
}

#[derive(Debug, Clone, Copy)]
pub struct BodySmall;

impl Resolvable for BodySmall {
    type Resolved = ResolvedFont;

    fn resolve(&self, _env: &Environment) -> impl Signal<Output = Self::Resolved> {
        Computed::constant(font(12.0, FontWeight::Normal, 16.0, 0.4))
    }
}

pub fn body_small() -> Font {
    Font::new(BodySmall)
}

#[derive(Debug, Clone, Copy)]
pub struct TitleSmall;

impl Resolvable for TitleSmall {
    type Resolved = ResolvedFont;

    fn resolve(&self, _env: &Environment) -> impl Signal<Output = Self::Resolved> {
        Computed::constant(font(14.0, FontWeight::Medium, 20.0, 0.1))
    }
}

pub fn title_small() -> Font {
    Font::new(TitleSmall)
}

#[derive(Debug, Clone, Copy)]
pub struct HeadlineSmall;

impl Resolvable for HeadlineSmall {
    type Resolved = ResolvedFont;

    fn resolve(&self, _env: &Environment) -> impl Signal<Output = Self::Resolved> {
        Computed::constant(font(24.0, FontWeight::Normal, 32.0, 0.0))
    }
}

pub fn headline_small() -> Font {
    Font::new(HeadlineSmall)
}

#[cfg(test)]
mod tests {
    use super::{
        MATERIAL_TYPEFACE, body_large, body_medium, body_small, headline_small, install_defaults,
        label_large, label_medium, label_small, title_small,
    };
    use waterui::Plugin as _;
    use waterui::env::Environment;
    use waterui::reactive::Signal as _;
    use waterui::text::font::{
        Body, Caption, FontWeight, Footnote, Headline, ResolvedFont, Subheadline, Title,
    };
    use waterui_core::resolve::Resolvable as _;

    #[allow(
        clippy::needless_pass_by_value,
        reason = "test helper; passing assertion inputs by value reads clearest"
    )]
    fn assert_material_font(
        font: waterui::text::font::ResolvedFont,
        expected_size: f32,
        expected_weight: FontWeight,
        expected_line_height: f32,
        expected_letter_spacing: f32,
    ) {
        assert_eq!(font.size, expected_size);
        assert_eq!(font.weight, expected_weight);
        assert_eq!(font.line_height, Some(expected_line_height));
        assert_eq!(font.letter_spacing, expected_letter_spacing);
        assert_eq!(font.family.as_deref(), Some(MATERIAL_TYPEFACE));
    }

    #[test]
    fn material_typeface_uses_roboto_then_script_aware_sans_fallbacks() {
        assert_eq!(MATERIAL_TYPEFACE, "Roboto, sans-serif");
    }

    #[test]
    fn font_slots_match_mdui_2_1_5_type_scale() {
        let mut env = Environment::new();
        install_defaults(&mut env);

        assert_material_font(
            Body.resolve(&env).get(),
            16.0,
            FontWeight::Normal,
            24.0,
            0.15,
        );
        assert_material_font(
            Title.resolve(&env).get(),
            22.0,
            FontWeight::Normal,
            28.0,
            0.0,
        );
        assert_material_font(
            Headline.resolve(&env).get(),
            24.0,
            FontWeight::Normal,
            32.0,
            0.0,
        );
        assert_material_font(
            Subheadline.resolve(&env).get(),
            16.0,
            FontWeight::Medium,
            24.0,
            0.15,
        );
        assert_material_font(
            Caption.resolve(&env).get(),
            12.0,
            FontWeight::Normal,
            16.0,
            0.4,
        );
        assert_material_font(
            Footnote.resolve(&env).get(),
            11.0,
            FontWeight::Medium,
            16.0,
            0.5,
        );
    }

    #[test]
    fn material_defaults_preserve_app_font_overrides() {
        let app_body = ResolvedFont::new(27.0, FontWeight::Bold);
        let mut env = Environment::new();
        waterui::theme::Theme::new()
            .fonts(waterui::theme::FontSettings::new().body(app_body.clone()))
            .install(&mut env);

        install_defaults(&mut env);

        let resolved_body = Body.resolve(&env).get();
        assert_eq!(resolved_body.size, app_body.size);
        assert_eq!(resolved_body.weight, app_body.weight);
        assert_eq!(resolved_body.family, app_body.family);
        assert_material_font(
            Title.resolve(&env).get(),
            22.0,
            FontWeight::Normal,
            28.0,
            0.0,
        );
    }

    #[test]
    fn label_large_matches_mdui_2_1_5_label_large() {
        let env = Environment::new();

        assert_material_font(
            label_large().resolve(&env).get(),
            14.0,
            FontWeight::Medium,
            20.0,
            0.1,
        );
    }

    #[test]
    fn label_medium_matches_mdui_2_1_5_label_medium() {
        let env = Environment::new();

        assert_material_font(
            label_medium().resolve(&env).get(),
            12.0,
            FontWeight::Medium,
            16.0,
            0.5,
        );
    }

    #[test]
    fn label_small_matches_mdui_2_1_5_label_small() {
        let env = Environment::new();

        assert_material_font(
            label_small().resolve(&env).get(),
            11.0,
            FontWeight::Medium,
            16.0,
            0.5,
        );
    }

    #[test]
    fn body_medium_matches_mdui_2_1_5_body_medium() {
        let env = Environment::new();

        assert_material_font(
            body_medium().resolve(&env).get(),
            14.0,
            FontWeight::Normal,
            20.0,
            0.25,
        );
    }

    #[test]
    fn body_large_matches_mdui_2_1_5_body_large() {
        let env = Environment::new();

        assert_material_font(
            body_large().resolve(&env).get(),
            16.0,
            FontWeight::Normal,
            24.0,
            0.15,
        );
    }

    #[test]
    fn body_small_matches_mdui_2_1_5_body_small() {
        let env = Environment::new();

        assert_material_font(
            body_small().resolve(&env).get(),
            12.0,
            FontWeight::Normal,
            16.0,
            0.4,
        );
    }

    #[test]
    fn title_small_matches_mdui_2_1_5_title_small() {
        let env = Environment::new();

        assert_material_font(
            title_small().resolve(&env).get(),
            14.0,
            FontWeight::Medium,
            20.0,
            0.1,
        );
    }

    #[test]
    fn headline_small_matches_mdui_2_1_5_headline_small() {
        let env = Environment::new();

        assert_material_font(
            headline_small().resolve(&env).get(),
            24.0,
            FontWeight::Normal,
            32.0,
            0.0,
        );
    }
}
