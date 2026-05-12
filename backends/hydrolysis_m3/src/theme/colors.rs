use material_color_utils::{
    dynamic::{
        color_spec::{Platform, SpecVersion},
        variant::Variant,
    },
    theme_from_color,
    utils::color_utils::Argb,
};
use vello::peniko::Color;
use waterui_graphics::color::{Color as WaterColor, ResolvedColor, Srgb};

const fn role(red: u8, green: u8, blue: u8) -> MaterialRoleColor {
    MaterialRoleColor::new(Argb::from_rgb(red, green, blue))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Light or dark Material color scheme generation mode.
pub enum MaterialColorMode {
    /// Light Material color scheme.
    Light,
    /// Dark Material color scheme.
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// One generated Material system color role.
pub struct MaterialRoleColor(Argb);

impl MaterialRoleColor {
    /// Create a role color from an ARGB value.
    #[must_use]
    pub const fn new(argb: Argb) -> Self {
        Self(argb)
    }

    /// Return the raw ARGB value.
    #[must_use]
    pub const fn argb(self) -> Argb {
        self.0
    }

    /// Convert the role color to the Vello brush color type.
    #[must_use]
    pub fn peniko(self) -> Color {
        let color = self.0;
        Color::new([
            f32::from(color.red()) / 255.0,
            f32::from(color.green()) / 255.0,
            f32::from(color.blue()) / 255.0,
            f32::from(color.alpha()) / 255.0,
        ])
    }

    /// Convert the role color to WaterUI's sRGB color type.
    #[must_use]
    pub fn srgb(self) -> Srgb {
        let color = self.0;
        Srgb::new_u8(color.red(), color.green(), color.blue())
    }

    /// Convert the role color to a resolved WaterUI color.
    #[must_use]
    pub fn resolved(self) -> ResolvedColor {
        ResolvedColor::from(self.srgb())
    }

    /// Convert the role color to a WaterUI view color.
    #[must_use]
    pub fn view_color(self) -> WaterColor {
        WaterColor::from(self.srgb())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Material You system color roles generated from a source color.
pub struct MaterialColorScheme {
    /// The light or dark mode this scheme was generated for.
    pub mode: MaterialColorMode,
    /// The background color role.
    pub background: MaterialRoleColor,
    /// The foreground color used on `background`.
    pub on_background: MaterialRoleColor,
    /// The default surface color role.
    pub surface: MaterialRoleColor,
    /// A dim surface color role.
    pub surface_dim: MaterialRoleColor,
    /// A bright surface color role.
    pub surface_bright: MaterialRoleColor,
    /// The lowest surface container color role.
    pub surface_container_lowest: MaterialRoleColor,
    /// The low surface container color role.
    pub surface_container_low: MaterialRoleColor,
    /// The default surface container color role.
    pub surface_container: MaterialRoleColor,
    /// The high surface container color role.
    pub surface_container_high: MaterialRoleColor,
    /// The highest surface container color role.
    pub surface_container_highest: MaterialRoleColor,
    /// The foreground color used on surface roles.
    pub on_surface: MaterialRoleColor,
    /// The alternate surface color role.
    pub surface_variant: MaterialRoleColor,
    /// The foreground color used on `surface_variant`.
    pub on_surface_variant: MaterialRoleColor,
    /// The inverse surface color role.
    pub inverse_surface: MaterialRoleColor,
    /// The foreground color used on `inverse_surface`.
    pub inverse_on_surface: MaterialRoleColor,
    /// The main outline color role.
    pub outline: MaterialRoleColor,
    /// The subtle outline color role.
    pub outline_variant: MaterialRoleColor,
    /// The shadow color role.
    pub shadow: MaterialRoleColor,
    /// The scrim color role.
    pub scrim: MaterialRoleColor,
    /// The surface tint color role.
    pub surface_tint: MaterialRoleColor,
    /// The primary accent color role.
    pub primary: MaterialRoleColor,
    /// The foreground color used on `primary`.
    pub on_primary: MaterialRoleColor,
    /// The primary container color role.
    pub primary_container: MaterialRoleColor,
    /// The foreground color used on `primary_container`.
    pub on_primary_container: MaterialRoleColor,
    /// The inverse primary color role.
    pub inverse_primary: MaterialRoleColor,
    /// The secondary accent color role.
    pub secondary: MaterialRoleColor,
    /// The foreground color used on `secondary`.
    pub on_secondary: MaterialRoleColor,
    /// The secondary container color role.
    pub secondary_container: MaterialRoleColor,
    /// The foreground color used on `secondary_container`.
    pub on_secondary_container: MaterialRoleColor,
    /// The tertiary accent color role.
    pub tertiary: MaterialRoleColor,
    /// The foreground color used on `tertiary`.
    pub on_tertiary: MaterialRoleColor,
    /// The tertiary container color role.
    pub tertiary_container: MaterialRoleColor,
    /// The foreground color used on `tertiary_container`.
    pub on_tertiary_container: MaterialRoleColor,
    /// The error color role.
    pub error: MaterialRoleColor,
    /// The foreground color used on `error`.
    pub on_error: MaterialRoleColor,
    /// The error container color role.
    pub error_container: MaterialRoleColor,
    /// The foreground color used on `error_container`.
    pub on_error_container: MaterialRoleColor,
    /// The fixed primary color role.
    pub primary_fixed: MaterialRoleColor,
    /// The dim fixed primary color role.
    pub primary_fixed_dim: MaterialRoleColor,
    /// The foreground color used on `primary_fixed`.
    pub on_primary_fixed: MaterialRoleColor,
    /// The variant foreground color used on `primary_fixed`.
    pub on_primary_fixed_variant: MaterialRoleColor,
    /// The fixed secondary color role.
    pub secondary_fixed: MaterialRoleColor,
    /// The dim fixed secondary color role.
    pub secondary_fixed_dim: MaterialRoleColor,
    /// The foreground color used on `secondary_fixed`.
    pub on_secondary_fixed: MaterialRoleColor,
    /// The variant foreground color used on `secondary_fixed`.
    pub on_secondary_fixed_variant: MaterialRoleColor,
    /// The fixed tertiary color role.
    pub tertiary_fixed: MaterialRoleColor,
    /// The dim fixed tertiary color role.
    pub tertiary_fixed_dim: MaterialRoleColor,
    /// The foreground color used on `tertiary_fixed`.
    pub on_tertiary_fixed: MaterialRoleColor,
    /// The variant foreground color used on `tertiary_fixed`.
    pub on_tertiary_fixed_variant: MaterialRoleColor,
}

impl MaterialColorScheme {
    /// Return the standard light Material 3 baseline scheme.
    #[must_use]
    pub const fn baseline_light() -> Self {
        Self {
            mode: MaterialColorMode::Light,
            background: role(0xfe, 0xf7, 0xff),
            on_background: role(0x1d, 0x1b, 0x20),
            surface: role(0xfe, 0xf7, 0xff),
            surface_dim: role(0xde, 0xd8, 0xe1),
            surface_bright: role(0xfe, 0xf7, 0xff),
            surface_container_lowest: role(0xff, 0xff, 0xff),
            surface_container_low: role(0xf7, 0xf2, 0xfa),
            surface_container: role(0xf3, 0xed, 0xf7),
            surface_container_high: role(0xec, 0xe6, 0xf0),
            surface_container_highest: role(0xe6, 0xe0, 0xe9),
            on_surface: role(0x1d, 0x1b, 0x20),
            surface_variant: role(0xe7, 0xe0, 0xec),
            on_surface_variant: role(0x49, 0x45, 0x4f),
            inverse_surface: role(0x32, 0x2f, 0x35),
            inverse_on_surface: role(0xf5, 0xef, 0xf7),
            outline: role(0x79, 0x74, 0x7e),
            outline_variant: role(0xca, 0xc4, 0xd0),
            shadow: role(0x00, 0x00, 0x00),
            scrim: role(0x00, 0x00, 0x00),
            surface_tint: role(0x67, 0x50, 0xa4),
            primary: role(0x67, 0x50, 0xa4),
            on_primary: role(0xff, 0xff, 0xff),
            primary_container: role(0xea, 0xdd, 0xff),
            on_primary_container: role(0x21, 0x00, 0x5d),
            inverse_primary: role(0xd0, 0xbc, 0xff),
            secondary: role(0x62, 0x5b, 0x71),
            on_secondary: role(0xff, 0xff, 0xff),
            secondary_container: role(0xe8, 0xde, 0xf8),
            on_secondary_container: role(0x1d, 0x19, 0x2b),
            tertiary: role(0x7d, 0x52, 0x60),
            on_tertiary: role(0xff, 0xff, 0xff),
            tertiary_container: role(0xff, 0xd8, 0xe4),
            on_tertiary_container: role(0x31, 0x11, 0x1d),
            error: role(0xb3, 0x26, 0x1e),
            on_error: role(0xff, 0xff, 0xff),
            error_container: role(0xf9, 0xde, 0xdc),
            on_error_container: role(0x41, 0x0e, 0x0b),
            primary_fixed: role(0xea, 0xdd, 0xff),
            primary_fixed_dim: role(0xd0, 0xbc, 0xff),
            on_primary_fixed: role(0x21, 0x00, 0x5d),
            on_primary_fixed_variant: role(0x4f, 0x37, 0x8b),
            secondary_fixed: role(0xe8, 0xde, 0xf8),
            secondary_fixed_dim: role(0xcc, 0xc2, 0xdc),
            on_secondary_fixed: role(0x1d, 0x19, 0x2b),
            on_secondary_fixed_variant: role(0x4a, 0x44, 0x58),
            tertiary_fixed: role(0xff, 0xd8, 0xe4),
            tertiary_fixed_dim: role(0xef, 0xb8, 0xc8),
            on_tertiary_fixed: role(0x31, 0x11, 0x1d),
            on_tertiary_fixed_variant: role(0x63, 0x3b, 0x48),
        }
    }

    /// Return the standard dark Material 3 baseline scheme.
    #[must_use]
    pub const fn baseline_dark() -> Self {
        Self {
            mode: MaterialColorMode::Dark,
            background: role(0x14, 0x12, 0x18),
            on_background: role(0xe6, 0xe0, 0xe9),
            surface: role(0x14, 0x12, 0x18),
            surface_dim: role(0x14, 0x12, 0x18),
            surface_bright: role(0x3b, 0x38, 0x3e),
            surface_container_lowest: role(0x0f, 0x0d, 0x13),
            surface_container_low: role(0x1d, 0x1b, 0x20),
            surface_container: role(0x21, 0x1f, 0x26),
            surface_container_high: role(0x2b, 0x29, 0x30),
            surface_container_highest: role(0x36, 0x34, 0x3b),
            on_surface: role(0xe6, 0xe0, 0xe9),
            surface_variant: role(0x49, 0x45, 0x4f),
            on_surface_variant: role(0xca, 0xc4, 0xd0),
            inverse_surface: role(0xe6, 0xe0, 0xe9),
            inverse_on_surface: role(0x32, 0x2f, 0x35),
            outline: role(0x93, 0x8f, 0x99),
            outline_variant: role(0x49, 0x45, 0x4f),
            shadow: role(0x00, 0x00, 0x00),
            scrim: role(0x00, 0x00, 0x00),
            surface_tint: role(0xd0, 0xbc, 0xff),
            primary: role(0xd0, 0xbc, 0xff),
            on_primary: role(0x38, 0x1e, 0x72),
            primary_container: role(0x4f, 0x37, 0x8b),
            on_primary_container: role(0xea, 0xdd, 0xff),
            inverse_primary: role(0x67, 0x50, 0xa4),
            secondary: role(0xcc, 0xc2, 0xdc),
            on_secondary: role(0x33, 0x2d, 0x41),
            secondary_container: role(0x4a, 0x44, 0x58),
            on_secondary_container: role(0xe8, 0xde, 0xf8),
            tertiary: role(0xef, 0xb8, 0xc8),
            on_tertiary: role(0x49, 0x25, 0x32),
            tertiary_container: role(0x63, 0x3b, 0x48),
            on_tertiary_container: role(0xff, 0xd8, 0xe4),
            error: role(0xf2, 0xb8, 0xb5),
            on_error: role(0x60, 0x14, 0x10),
            error_container: role(0x8c, 0x1d, 0x18),
            on_error_container: role(0xf9, 0xde, 0xdc),
            primary_fixed: role(0xea, 0xdd, 0xff),
            primary_fixed_dim: role(0xd0, 0xbc, 0xff),
            on_primary_fixed: role(0x21, 0x00, 0x5d),
            on_primary_fixed_variant: role(0x4f, 0x37, 0x8b),
            secondary_fixed: role(0xe8, 0xde, 0xf8),
            secondary_fixed_dim: role(0xcc, 0xc2, 0xdc),
            on_secondary_fixed: role(0x1d, 0x19, 0x2b),
            on_secondary_fixed_variant: role(0x4a, 0x44, 0x58),
            tertiary_fixed: role(0xff, 0xd8, 0xe4),
            tertiary_fixed_dim: role(0xef, 0xb8, 0xc8),
            on_tertiary_fixed: role(0x31, 0x11, 0x1d),
            on_tertiary_fixed_variant: role(0x63, 0x3b, 0x48),
        }
    }

    /// Generate Material You system roles from a source color.
    #[must_use]
    pub fn from_seed(seed: Argb, mode: MaterialColorMode) -> Self {
        let theme = theme_from_color(seed)
            .variant(Variant::TonalSpot)
            .spec_version(SpecVersion::Spec2021)
            .platform(Platform::Phone)
            .call();
        let scheme = match mode {
            MaterialColorMode::Light => theme.schemes.light,
            MaterialColorMode::Dark => theme.schemes.dark,
        };
        Self {
            mode,
            background: MaterialRoleColor::new(scheme.background),
            on_background: MaterialRoleColor::new(scheme.on_background),
            surface: MaterialRoleColor::new(scheme.surface),
            surface_dim: MaterialRoleColor::new(scheme.surface_dim),
            surface_bright: MaterialRoleColor::new(scheme.surface_bright),
            surface_container_lowest: MaterialRoleColor::new(scheme.surface_container_lowest),
            surface_container_low: MaterialRoleColor::new(scheme.surface_container_low),
            surface_container: MaterialRoleColor::new(scheme.surface_container),
            surface_container_high: MaterialRoleColor::new(scheme.surface_container_high),
            surface_container_highest: MaterialRoleColor::new(scheme.surface_container_highest),
            on_surface: MaterialRoleColor::new(scheme.on_surface),
            surface_variant: MaterialRoleColor::new(scheme.surface_variant),
            on_surface_variant: MaterialRoleColor::new(scheme.on_surface_variant),
            inverse_surface: MaterialRoleColor::new(scheme.inverse_surface),
            inverse_on_surface: MaterialRoleColor::new(scheme.inverse_on_surface),
            outline: MaterialRoleColor::new(scheme.outline),
            outline_variant: MaterialRoleColor::new(scheme.outline_variant),
            shadow: MaterialRoleColor::new(scheme.shadow),
            scrim: MaterialRoleColor::new(scheme.scrim),
            surface_tint: MaterialRoleColor::new(scheme.surface_tint),
            primary: MaterialRoleColor::new(scheme.primary),
            on_primary: MaterialRoleColor::new(scheme.on_primary),
            primary_container: MaterialRoleColor::new(scheme.primary_container),
            on_primary_container: MaterialRoleColor::new(scheme.on_primary_container),
            inverse_primary: MaterialRoleColor::new(scheme.inverse_primary),
            secondary: MaterialRoleColor::new(scheme.secondary),
            on_secondary: MaterialRoleColor::new(scheme.on_secondary),
            secondary_container: MaterialRoleColor::new(scheme.secondary_container),
            on_secondary_container: MaterialRoleColor::new(scheme.on_secondary_container),
            tertiary: MaterialRoleColor::new(scheme.tertiary),
            on_tertiary: MaterialRoleColor::new(scheme.on_tertiary),
            tertiary_container: MaterialRoleColor::new(scheme.tertiary_container),
            on_tertiary_container: MaterialRoleColor::new(scheme.on_tertiary_container),
            error: MaterialRoleColor::new(scheme.error),
            on_error: MaterialRoleColor::new(scheme.on_error),
            error_container: MaterialRoleColor::new(scheme.error_container),
            on_error_container: MaterialRoleColor::new(scheme.on_error_container),
            primary_fixed: MaterialRoleColor::new(scheme.primary_fixed),
            primary_fixed_dim: MaterialRoleColor::new(scheme.primary_fixed_dim),
            on_primary_fixed: MaterialRoleColor::new(scheme.on_primary_fixed),
            on_primary_fixed_variant: MaterialRoleColor::new(scheme.on_primary_fixed_variant),
            secondary_fixed: MaterialRoleColor::new(scheme.secondary_fixed),
            secondary_fixed_dim: MaterialRoleColor::new(scheme.secondary_fixed_dim),
            on_secondary_fixed: MaterialRoleColor::new(scheme.on_secondary_fixed),
            on_secondary_fixed_variant: MaterialRoleColor::new(scheme.on_secondary_fixed_variant),
            tertiary_fixed: MaterialRoleColor::new(scheme.tertiary_fixed),
            tertiary_fixed_dim: MaterialRoleColor::new(scheme.tertiary_fixed_dim),
            on_tertiary_fixed: MaterialRoleColor::new(scheme.on_tertiary_fixed),
            on_tertiary_fixed_variant: MaterialRoleColor::new(scheme.on_tertiary_fixed_variant),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Argb, MaterialColorMode, MaterialColorScheme};

    #[test]
    fn baseline_light_matches_material_web_dynamic_color_tokens() {
        let scheme = MaterialColorScheme::baseline_light();

        assert_eq!(scheme.primary.argb(), Argb::from_rgb(0x67, 0x50, 0xa4));
        assert_eq!(scheme.on_primary.argb(), Argb::from_rgb(0xff, 0xff, 0xff));
        assert_eq!(scheme.surface.argb(), Argb::from_rgb(0xfe, 0xf7, 0xff));
        assert_eq!(scheme.on_surface.argb(), Argb::from_rgb(0x1d, 0x1b, 0x20));
        assert_eq!(scheme.outline.argb(), Argb::from_rgb(0x79, 0x74, 0x7e));
        assert_eq!(
            scheme.surface_variant.argb(),
            Argb::from_rgb(0xe7, 0xe0, 0xec)
        );
    }

    #[test]
    fn source_color_changes_generated_roles() {
        let baseline = MaterialColorScheme::baseline_light();
        let custom = MaterialColorScheme::from_seed(
            Argb::from_rgb(0x00, 0x6a, 0x6a),
            MaterialColorMode::Light,
        );

        assert_ne!(custom.primary.argb(), baseline.primary.argb());
        assert_eq!(custom.on_primary.argb(), Argb::from_rgb(0xff, 0xff, 0xff));
    }
}
