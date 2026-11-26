//! The backend leaf [`GpuContentLeaf`] that [`GpuView`](super::GpuView) lowers into.

use alloc::boxed::Box;
use alloc::string::String;
use core::fmt;

use cherenkov_gpu::interop::GpuContentBox;
use nami::{Computed, Signal};
use waterui_core::layout::{ProposalSize, Size, StretchAxis, ViewDimensions};
use waterui_core::{Environment, Native, NativeView, Str, View, contract_violation};

use super::content::{Erase, ErasedGpuContent};
use super::{Context, Frame, GpuContentHandle, SurfaceHooks, measure_by_intrinsic_size};
use crate::draw::kurbo;
use crate::input::SurfaceInputEvent;
use crate::{GpuContent, GpuView};

/// The backend leaf produced by [`GpuView`].
///
/// Backends mount this type to consume the one-shot [`GpuContent`] producer
/// alongside its UI-thread hooks (input, per-frame pump, IME caret, live
/// measurement, HDR preference, and accessibility metadata). Application code
/// constructs [`GpuView`] instead.
pub struct GpuContentLeaf {
    content: Option<Box<dyn ErasedGpuContent>>,
    content_handle: Option<GpuContentHandle>,
    intrinsic_size: Option<Size>,
    opaque: bool,
    surface_prefers_hdr: Option<bool>,
    hooks: SurfaceHooks,
    label: Computed<Option<Str>>,
    value: Computed<Option<Str>>,
}

impl fmt::Debug for GpuContentLeaf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GpuContentLeaf")
            .field("intrinsic_size", &self.intrinsic_size)
            .field("opaque", &self.opaque)
            .finish_non_exhaustive()
    }
}

impl GpuContentLeaf {
    /// Creates a leaf directly from `content`, for backend hosts and offscreen
    /// renderers that bypass the authoring [`GpuView`] builder.
    #[must_use]
    pub fn new(content: impl GpuContent<Params = ()>) -> Self {
        GpuView::new(content).into_leaf()
    }

    pub(super) const fn from_parts(
        content: Box<dyn ErasedGpuContent>,
        intrinsic_size: Option<Size>,
        opaque: bool,
        surface_prefers_hdr: Option<bool>,
        hooks: SurfaceHooks,
        label: Computed<Option<Str>>,
        value: Computed<Option<Str>>,
    ) -> Self {
        Self {
            content: Some(content),
            content_handle: None,
            intrinsic_size,
            opaque,
            surface_prefers_hdr,
            hooks,
            label,
            value,
        }
    }

    /// The content's text caret, if it has one right now.
    #[must_use]
    pub fn ime_caret(&self) -> Option<kurbo::Rect> {
        self.hooks.ime_caret()
    }

    /// Runs the per-frame UI hook, if any.
    pub fn frame(&self) {
        self.hooks.frame();
    }

    /// The reactive accessibility label signal.
    #[must_use]
    pub const fn label(&self) -> &Computed<Option<Str>> {
        &self.label
    }

    /// The reactive accessibility value signal.
    #[must_use]
    pub const fn value(&self) -> &Computed<Option<Str>> {
        &self.value
    }

    /// Whether this leaf takes input events.
    #[must_use]
    pub const fn wants_input_events(&self) -> bool {
        self.hooks.wants_input_events()
    }

    /// Routes an input event to the content's handler.
    pub fn input(&self, event: &SurfaceInputEvent) {
        self.hooks.input(event);
    }

    /// The natural size of the content, if it has one.
    #[must_use]
    pub const fn intrinsic_size(&self) -> Option<Size> {
        self.intrinsic_size
    }

    /// Measures this leaf's content against a layout proposal.
    ///
    /// Unlike [`intrinsic_size`](Self::intrinsic_size) — the snapshot taken at
    /// [`GpuView::new`](super::GpuView::new) — this asks the live content, so a
    /// source whose size resolves asynchronously reports it once it has one.
    #[must_use]
    pub fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.content_handle.as_ref().map_or_else(
            || {
                self.content.as_ref().map_or_else(
                    || measure_by_intrinsic_size(None, proposal),
                    |content| content.measure(proposal),
                )
            },
            |handle| handle.measure(proposal),
        )
    }

    /// Whether the content is opaque.
    #[must_use]
    pub const fn is_opaque(&self) -> bool {
        self.opaque
    }

    /// Resolves this leaf's explicit or content-provided HDR preference.
    ///
    /// `None` means follow the surrounding platform style.
    #[must_use]
    pub fn resolved_hdr_preference(&self) -> Option<bool> {
        self.surface_prefers_hdr.or_else(|| {
            self.content_handle
                .as_ref()
                .and_then(GpuContentHandle::preferred_surface_hdr)
                .or_else(|| {
                    self.content
                        .as_ref()
                        .and_then(|content| content.preferred_surface_hdr())
                })
        })
    }

    /// The accessibility name.
    #[must_use]
    pub fn accessibility_label(&self) -> Option<String> {
        self.label.snapshot().map(|label| label.as_str().to_owned())
    }

    /// The accessibility value.
    #[must_use]
    pub fn accessibility_value(&self) -> Option<String> {
        self.value.snapshot().map(|value| value.as_str().to_owned())
    }

    /// Transfers this producer to a Cherenkov GPU layer.
    ///
    /// Asynchronous producer work wakes the host through the wake of each
    /// engine surface that draws the layer. The UI hooks and input handlers
    /// stay in this leaf.
    #[must_use]
    pub fn take_engine_content(&mut self) -> GpuContentBox {
        GpuContentBox::new(self.engine_content())
    }

    /// A shareable handle to this leaf's content for the engine.
    ///
    /// Every call answers a handle to the *same* content object: a host that
    /// rebuilds its engine layer after device loss re-installs a fresh handle
    /// so the engine's setup runs the one content on the new device, keeping
    /// the state the content had accumulated.
    pub fn engine_content(&mut self) -> GpuContentHandle {
        let content = &mut self.content;
        self.content_handle
            .get_or_insert_with(|| {
                GpuContentHandle::new(content.take().unwrap_or_else(|| {
                    contract_violation!(
                        "GpuContentLeaf content missing before engine mount; the layer draws nothing"
                    );
                    Box::new(Erase(Vacant))
                }))
            })
            .clone()
    }
}

/// Content that draws nothing: fallback if a leaf somehow lost its content.
struct Vacant;

impl GpuContent for Vacant {
    type Params = ();

    fn setup(&mut self, _gpu: &Context<'_>) {}

    fn render(&mut self, _frame: &mut Frame<'_>) {}
}

impl NativeView for GpuContentLeaf {
    fn stretch_axis(&self) -> StretchAxis {
        if self.intrinsic_size.is_some() {
            StretchAxis::None
        } else {
            StretchAxis::Both
        }
    }
}

impl View for GpuContentLeaf {
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self)
    }
}
