use alloc::boxed::Box;

use crate::{IntoFFI, WuiEnv, reactive::WuiComputed};
use nami::SignalExt;
use waterui::theme::{self, color, install_color_signal_for, install_typography_signal_for};
use waterui_color::ResolvedColor;
use waterui_core::resolve::Resolvable;
use waterui_text::font::{Body, Caption, Footnote, Headline, ResolvedFont, Subheadline, Title};

fn take_computed<T>(ptr: *mut WuiComputed<T>) -> Option<waterui::Computed<T>> {
    unsafe { ptr.as_mut().map(|_| Box::from_raw(ptr).0) }
}

fn install_color_token<T>(env: &mut waterui::Environment, ptr: *mut WuiComputed<ResolvedColor>)
where
    T: theme::ThemeColorKey,
{
    if let Some(computed) = take_computed(ptr) {
        install_color_signal_for::<T>(env, computed);
    }
}

fn install_font_token<T>(env: &mut waterui::Environment, ptr: *mut WuiComputed<ResolvedFont>)
where
    T: 'static,
{
    if let Some(computed) = take_computed(ptr) {
        install_typography_signal_for::<T>(env, computed);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_env_install_theme(
    env: *mut WuiEnv,
    background: *mut WuiComputed<ResolvedColor>,
    surface: *mut WuiComputed<ResolvedColor>,
    surface_variant: *mut WuiComputed<ResolvedColor>,
    border: *mut WuiComputed<ResolvedColor>,
    foreground: *mut WuiComputed<ResolvedColor>,
    muted_foreground: *mut WuiComputed<ResolvedColor>,
    accent: *mut WuiComputed<ResolvedColor>,
    accent_foreground: *mut WuiComputed<ResolvedColor>,
    body: *mut WuiComputed<ResolvedFont>,
    title: *mut WuiComputed<ResolvedFont>,
    headline: *mut WuiComputed<ResolvedFont>,
    subheadline: *mut WuiComputed<ResolvedFont>,
    caption: *mut WuiComputed<ResolvedFont>,
) {
    let env = unsafe { &mut *env };
    // Store a Theme baseline so queries like `theme(env)` still succeed.
    env.insert(theme::Theme::light());
    install_color_token::<color::Background>(env, background);
    install_color_token::<color::Surface>(env, surface);
    install_color_token::<color::SurfaceVariant>(env, surface_variant);
    install_color_token::<color::Border>(env, border);
    install_color_token::<color::Foreground>(env, foreground);
    install_color_token::<color::MutedForeground>(env, muted_foreground);
    install_color_token::<color::Accent>(env, accent);
    install_color_token::<color::AccentForeground>(env, accent_foreground);
    install_font_token::<Body>(env, body);
    install_font_token::<Title>(env, title);
    install_font_token::<Headline>(env, headline);
    install_font_token::<Subheadline>(env, subheadline);
    install_font_token::<Caption>(env, caption);
}

macro_rules! theme_color_fn {
    ($fn_name:ident, $token:path) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $fn_name(env: *const WuiEnv) -> *mut WuiComputed<ResolvedColor> {
            let env = unsafe { &*env };
            let computed = $token.resolve(env).computed();
            computed.into_ffi()
        }
    };
}

macro_rules! theme_font_fn {
    ($fn_name:ident, $token:path) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $fn_name(env: *const WuiEnv) -> *mut WuiComputed<ResolvedFont> {
            let env = unsafe { &*env };
            let computed = $token.resolve(env).computed();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WuiEnv;

    #[test]
    fn background_color_computed_is_readable() {
        let env = WuiEnv(waterui::Environment::new());
        let ptr = unsafe { waterui_theme_color_background(&env) };
        assert!(!ptr.is_null());
        let value = unsafe { crate::color::waterui_read_computed_resolved_color(ptr) };
        assert!(value.opacity >= 0.0);
        unsafe {
            crate::color::waterui_drop_computed_resolved_color(ptr);
        }
    }
}
