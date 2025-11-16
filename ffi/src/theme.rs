use crate::reactive::WuiComputed;
use crate::{IntoFFI, WuiEnv};
use waterui::SignalExt;
use waterui::theme::color;
use waterui_color::ResolvedColor;
use waterui_text::font::{Body, Caption, Footnote, Headline, ResolvedFont, Subheadline, Title};

macro_rules! theme_color_fn {
    ($name:ident, $token:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(env: *const WuiEnv) -> *mut WuiComputed<ResolvedColor> {
            let env = &*env;
            let computed = <$token>::default().resolve(env).computed();
            computed.into_ffi()
        }
    };
}

macro_rules! theme_font_fn {
    ($name:ident, $token:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(env: *const WuiEnv) -> *mut WuiComputed<ResolvedFont> {
            let env = &*env;
            let token: $token = $token;
            let computed = token.resolve(env).computed();
            computed.into_ffi()
        }
    };
}

theme_color_fn!(waterui_theme_color_background, color::Background);
theme_color_fn!(waterui_theme_color_surface, color::Surface);
theme_color_fn!(waterui_theme_color_surface_variant, color::SurfaceVariant);
theme_color_fn!(waterui_theme_color_border, color::Border);
theme_color_fn!(waterui_theme_color_foreground, color::Foreground);
theme_color_fn!(waterui_theme_color_muted_foreground, color::MutedForeground);
theme_color_fn!(waterui_theme_color_accent, color::Accent);
theme_color_fn!(
    waterui_theme_color_accent_foreground,
    color::AccentForeground
);

theme_font_fn!(waterui_theme_font_body, Body);
theme_font_fn!(waterui_theme_font_title, Title);
theme_font_fn!(waterui_theme_font_headline, Headline);
theme_font_fn!(waterui_theme_font_subheadline, Subheadline);
theme_font_fn!(waterui_theme_font_caption, Caption);
theme_font_fn!(waterui_theme_font_footnote, Footnote);
