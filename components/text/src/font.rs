use core::fmt::Debug;

use nami::{impl_constant, Computed, Signal};
use waterui_core::{resolve::{self, AnyResolvable, Resolvable}, Environment};

/// Font configuration for text rendering.
///
/// This struct defines all the visual properties that can be applied to text,
/// including size, styling, and decorations.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Font(AnyResolvable<ResolvedFont>);

impl Default for Font {
    fn default() -> Self {
        Self::new(Body)
    }
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ResolvedFont {
    pub size: f32,
    pub weight:FontWeight,
}

impl ResolvedFont{
    #[must_use]
    pub const fn new(size:f32,weight:FontWeight) -> Self{
        Self{size,weight}
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash,Default)]
pub enum FontWeight{
    Thin,
    UltraLight,
    Light,
    #[default]
    Normal,
    Medium,
    SemiBold,
    Bold,
    UltraBold,
    Black,
}

impl_constant!(FontWeight);

pub trait CustomFont:Debug+Clone{
    fn resolve(&self,env:&Environment) -> ResolvedFont;
}

trait CustomFontImpl {
    fn resolve(&self, env: &Environment) -> ResolvedFont;
    fn box_clone(&self) -> Box<dyn CustomFontImpl>;
}

#[derive(Debug, Clone,Copy)]
pub struct Title;


impl Resolvable for Title {
    type Resolved = ResolvedFont;
    fn resolve(&self, env: &Environment) -> impl Signal<Output=Self::Resolved> {
        env.query::<Self,Computed<Self::Resolved>>().cloned().unwrap_or_else(|| Computed::constant(ResolvedFont::new(24.0,FontWeight::Bold)))
    }
}

#[derive(Debug, Clone,Copy)]
pub struct Body;

impl Resolvable for Body {
    type Resolved = ResolvedFont;
    fn resolve(&self, env: &Environment) -> impl Signal<Output=Self::Resolved> {
        env.query::<Self,Computed<Self::Resolved>>().cloned().unwrap_or_else(|| Computed::constant(ResolvedFont::new(16.0,FontWeight::Normal)))
    }
}

impl Font{
    pub fn new(font: impl Resolvable<Resolved = ResolvedFont> + 'static) -> Self {
        Self(AnyResolvable::new(font))
    }

    pub fn weight(self, weight: FontWeight) -> Self {
        Self::new(resolve::Map::new(self.0, move |font| {
            ResolvedFont {
                size: font.size,
                weight,
            }
        }))
    }

    pub fn size(self, size: f32) -> Self {
        Self::new(resolve::Map::new(self.0, move |font| {
            ResolvedFont {
                size,
                weight: font.weight,
            }
        }))
    }


    /// Sets the font to bold weight.
    /// Equal to calling `font.weight(FontWeight::Bold)`.
    #[must_use]
    pub fn bold(self) -> Self {
        self.weight(FontWeight::Bold)
    }

    pub fn resolve(&self,env:&Environment) -> Computed<ResolvedFont>{
        self.0.resolve(env)
    }
}