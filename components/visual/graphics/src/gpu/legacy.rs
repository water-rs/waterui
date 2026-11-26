//! Temporary compatibility shim for split crates (`waterui-particle`,
//! `waterui-map-gpu`, `waterui-video-gpu`) while transitioning `#4419` to the
//! associated-type [`crate::GpuContent`]. Removed once all three split crates
//! migrate to `waterui_graphics::{GpuContent, GpuView}`.

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::any::type_name;
use core::fmt;

use nami::Computed;
use waterui_core::layout::{ProposalSize, Size, ViewDimensions};
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{Environment, Native, Str, View};

use super::content::ErasedGpuContent;
use super::leaf::GpuContentLeaf;
use super::{Context, Frame, SurfaceHooks, measure_by_intrinsic_size};
use crate::draw::kurbo;
use crate::input::SurfaceInputEvent;

#[doc(hidden)]
pub trait GpuContent: Send + 'static {
    fn setup(&mut self, gpu: &Context<'_>);

    fn render(&mut self, frame: &mut Frame<'_>);

    fn is_opaque(&self) -> bool {
        false
    }

    fn intrinsic_size(&self) -> Option<Size> {
        None
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        measure_by_intrinsic_size(self.intrinsic_size(), proposal)
    }

    fn preferred_surface_hdr(&self) -> Option<bool> {
        None
    }
}

struct EraseLegacy<T>(T);

impl<T: GpuContent> ErasedGpuContent for EraseLegacy<T> {
    fn setup(&mut self, gpu: &Context<'_>) {
        self.0.setup(gpu);
    }

    fn render(&mut self, frame: &mut Frame<'_>) {
        self.0.render(frame);
    }

    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.0.measure(proposal)
    }

    fn preferred_surface_hdr(&self) -> Option<bool> {
        self.0.preferred_surface_hdr()
    }
}

#[doc(hidden)]
pub struct GpuView<T: GpuContent> {
    content: T,
    intrinsic_size: Option<Size>,
    opaque: bool,
    surface_prefers_hdr: Option<bool>,
    hooks: SurfaceHooks,
    label: Computed<Option<Str>>,
    value: Computed<Option<Str>>,
}

impl<T: GpuContent> fmt::Debug for GpuView<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GpuView")
            .field("content", &type_name::<T>())
            .field("intrinsic_size", &self.intrinsic_size)
            .field("opaque", &self.opaque)
            .finish_non_exhaustive()
    }
}

impl<T: GpuContent> GpuView<T> {
    #[must_use]
    pub fn new(content: T) -> Self {
        Self {
            intrinsic_size: content.intrinsic_size(),
            opaque: content.is_opaque(),
            content,
            surface_prefers_hdr: None,
            hooks: SurfaceHooks::default(),
            label: Computed::constant(None),
            value: Computed::constant(None),
        }
    }

    #[must_use]
    pub fn on_input(mut self, handler: impl Fn(&SurfaceInputEvent) + 'static) -> Self {
        self.hooks.input = Some(Rc::new(handler));
        self
    }

    #[must_use]
    pub fn on_frame(mut self, hook: impl Fn() + 'static) -> Self {
        self.hooks.frame = Some(Rc::new(hook));
        self
    }

    #[must_use]
    pub fn on_ime_caret(mut self, query: impl Fn() -> Option<kurbo::Rect> + 'static) -> Self {
        self.hooks.caret = Some(Rc::new(query));
        self
    }

    #[must_use]
    pub fn offered_label(mut self, label: impl IntoComputed<Option<Str>>) -> Self {
        self.label = label.into_computed();
        self
    }

    #[must_use]
    pub fn offered_value(mut self, value: impl IntoComputed<Option<Str>>) -> Self {
        self.value = value.into_computed();
        self
    }

    #[must_use]
    pub const fn prefer_hdr_surface(mut self) -> Self {
        self.surface_prefers_hdr = Some(true);
        self
    }

    #[must_use]
    pub const fn prefer_sdr_surface(mut self) -> Self {
        self.surface_prefers_hdr = Some(false);
        self
    }

    pub(crate) fn into_leaf(self) -> GpuContentLeaf {
        GpuContentLeaf::from_parts(
            Box::new(EraseLegacy(self.content)),
            self.intrinsic_size,
            self.opaque,
            self.surface_prefers_hdr,
            self.hooks,
            self.label,
            self.value,
        )
    }
}

impl<T: GpuContent> From<GpuView<T>> for GpuContentLeaf {
    fn from(view: GpuView<T>) -> Self {
        view.into_leaf()
    }
}

impl<T: GpuContent> View for GpuView<T> {
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self.into_leaf())
    }
}
