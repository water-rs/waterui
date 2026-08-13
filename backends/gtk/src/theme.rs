//! GTK system palette projection into WaterUI theme slots.

use std::rc::Rc;

use gdk4::RGBA;
use gtk4::{Settings, Widget, prelude::*};
use nami::{Binding, SignalExt, binding};
use waterui::theme::{
    ColorScheme,
    color::{
        Accent, AccentContainer, AccentForeground, Background, Border, Foreground, MutedForeground,
        Surface, SurfaceVariant, Tertiary, TertiaryContainer,
    },
    install_color_scheme, install_color_signal, installed_color_scheme, installed_color_signal,
};
use waterui_core::Environment;
use waterui_graphics::color::{ResolvedColor, Srgb};

struct Palette {
    background: Binding<ResolvedColor>,
    surface: Binding<ResolvedColor>,
    surface_variant: Binding<ResolvedColor>,
    border: Binding<ResolvedColor>,
    foreground: Binding<ResolvedColor>,
    muted_foreground: Binding<ResolvedColor>,
    accent: Binding<ResolvedColor>,
    accent_container: Binding<ResolvedColor>,
    accent_foreground: Binding<ResolvedColor>,
    tertiary: Binding<ResolvedColor>,
    tertiary_container: Binding<ResolvedColor>,
}

/// Installs GTK's named system colors and keeps them synchronized with theme changes.
pub(crate) fn install(env: &mut Environment, widget: &Widget) {
    let settings = Settings::default().expect("GTK theme installation requires default settings");
    let palette = Rc::new(Palette::read(widget));
    install_missing::<Background>(env, &palette.background);
    install_missing::<Surface>(env, &palette.surface);
    install_missing::<SurfaceVariant>(env, &palette.surface_variant);
    install_missing::<Border>(env, &palette.border);
    install_missing::<Foreground>(env, &palette.foreground);
    install_missing::<MutedForeground>(env, &palette.muted_foreground);
    install_missing::<Accent>(env, &palette.accent);
    install_missing::<AccentContainer>(env, &palette.accent_container);
    install_missing::<AccentForeground>(env, &palette.accent_foreground);
    install_missing::<Tertiary>(env, &palette.tertiary);
    install_missing::<TertiaryContainer>(env, &palette.tertiary_container);

    if installed_color_scheme(env).is_none() {
        let scheme = binding(system_scheme(&settings));
        install_color_scheme(env, scheme.computed());
        settings.connect_notify_local(
            Some("gtk-application-prefer-dark-theme"),
            move |settings, _| {
                scheme.set(system_scheme(settings));
            },
        );
    }

    let palette_for_appearance = Rc::clone(&palette);
    let widget_for_appearance = widget.clone();
    settings.connect_notify_local(Some("gtk-application-prefer-dark-theme"), move |_, _| {
        palette_for_appearance.update(&widget_for_appearance)
    });
    let widget = widget.clone();
    settings.connect_notify_local(Some("gtk-theme-name"), move |_, _| palette.update(&widget));
}

fn install_missing<T: 'static>(env: &mut Environment, value: &Binding<ResolvedColor>) {
    if installed_color_signal::<T>(env).is_none() {
        install_color_signal::<T>(env, value.clone().computed());
    }
}

impl Palette {
    fn read(widget: &Widget) -> Self {
        let accent = lookup(widget, "theme_selected_bg_color");
        Self {
            background: binding(lookup(widget, "theme_bg_color")),
            surface: binding(lookup(widget, "theme_base_color")),
            surface_variant: binding(lookup(widget, "theme_unfocused_bg_color")),
            border: binding(lookup(widget, "borders")),
            foreground: binding(lookup(widget, "theme_fg_color")),
            muted_foreground: binding(lookup(widget, "theme_unfocused_fg_color")),
            accent: binding(accent),
            accent_container: binding(with_opacity(accent, 0.2)),
            accent_foreground: binding(lookup(widget, "theme_selected_fg_color")),
            tertiary: binding(accent),
            tertiary_container: binding(with_opacity(accent, 0.2)),
        }
    }

    fn update(&self, widget: &Widget) {
        let accent = lookup(widget, "theme_selected_bg_color");
        self.background.set(lookup(widget, "theme_bg_color"));
        self.surface.set(lookup(widget, "theme_base_color"));
        self.surface_variant
            .set(lookup(widget, "theme_unfocused_bg_color"));
        self.border.set(lookup(widget, "borders"));
        self.foreground.set(lookup(widget, "theme_fg_color"));
        self.muted_foreground
            .set(lookup(widget, "theme_unfocused_fg_color"));
        self.accent.set(accent);
        self.accent_container.set(with_opacity(accent, 0.2));
        self.accent_foreground
            .set(lookup(widget, "theme_selected_fg_color"));
        self.tertiary.set(accent);
        self.tertiary_container.set(with_opacity(accent, 0.2));
    }
}

fn lookup(widget: &Widget, name: &str) -> ResolvedColor {
    let rgba = widget
        .style_context()
        .lookup_color(name)
        .unwrap_or_else(|| panic!("GTK theme does not define required color {name}"));
    resolved(rgba)
}

fn resolved(rgba: RGBA) -> ResolvedColor {
    let mut color = ResolvedColor::from_srgb(Srgb::new(rgba.red(), rgba.green(), rgba.blue()));
    color.opacity = rgba.alpha();
    color
}

fn with_opacity(mut color: ResolvedColor, opacity: f32) -> ResolvedColor {
    color.opacity *= opacity;
    color
}

fn system_scheme(settings: &Settings) -> ColorScheme {
    if settings.property::<bool>("gtk-application-prefer-dark-theme") {
        ColorScheme::Dark
    } else {
        ColorScheme::Light
    }
}
