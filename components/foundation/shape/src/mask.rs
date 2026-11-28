//! Alpha-mask metadata and static-dispatch view wrapper.
//!
//! [`Mask`] masks a view with the alpha channel of another view laid out in
//! the receiver's bounds.

use waterui_core::{
    AnyView, Environment, Metadata, View,
    backend::{Backend, ChildSlot, MapSlots},
    metadata::MetadataKey,
};

/// Metadata payload carrying the mask view whose alpha channel masks the
/// wrapped content.
///
/// The wrapped content determines layout size, stretch axis, and priority;
/// the mask view is measured against the content's bounds and centered within
/// them.
#[derive(Debug)]
pub struct Mask<S = AnyView> {
    mask: S,
}

impl<S> Mask<S> {
    /// Creates alpha-mask metadata from `mask`.
    #[must_use]
    pub const fn new(mask: S) -> Self {
        Self { mask }
    }

    /// Consumes the metadata and returns the mask view slot.
    #[must_use]
    pub fn into_mask(self) -> S {
        self.mask
    }

    /// Borrows the mask view slot.
    #[must_use]
    pub const fn mask(&self) -> &S {
        &self.mask
    }

    /// Maps the mask view slot through `f`.
    #[must_use]
    pub fn map_slots<T>(self, f: impl FnOnce(S) -> T) -> Mask<T> {
        Mask { mask: f(self.mask) }
    }
}

impl<S: 'static> MetadataKey for Mask<S> {}

impl<S: 'static> MapSlots<S> for Mask<S> {
    type With<T: 'static> = Mask<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> Mask<T> {
        Mask { mask: f(self.mask) }
    }
}

/// View wrapper that masks `content` with the alpha channel of `mask` while
/// projecting both views into the backend's child slot without `AnyView`
/// erasure.
#[derive(Debug)]
#[must_use]
pub struct MaskView<Content, M> {
    content: Content,
    mask: Mask<M>,
}

impl<Content, M> MaskView<Content, M> {
    /// Creates a masked view pairing `content` with `mask`.
    pub const fn new(content: Content, mask: Mask<M>) -> Self {
        Self { content, mask }
    }
}

impl<Content: View, M: View> View for MaskView<Content, M> {
    fn body(self, _env: &Environment) -> impl View {
        Metadata::new(self.content, self.mask.map_slots(AnyView::new))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.metadata(
            Metadata::new(
                B::Slot::from_view(self.content),
                self.mask.map_slots(B::Slot::from_view),
            ),
            env,
            cx,
        )
    }
}

#[cfg(test)]
mod tests {
    use core::any::TypeId;

    use waterui_core::backend::{Backend, ChildSlot, cast};
    use waterui_core::metadata::MetadataKey;
    use waterui_core::view::hook::{HookList, HookableView};
    use waterui_core::{AnyView, Environment, Metadata, NativeView, View};

    use super::{Mask, MaskView};

    struct ContentProbe;
    impl View for ContentProbe {
        fn body(self, _env: &Environment) -> impl View {}
    }

    struct MaskProbe;
    impl View for MaskProbe {
        fn body(self, _env: &Environment) -> impl View {}
    }

    #[test]
    fn mask_view_dispatches_statically_without_anyview() {
        struct RecordedSlot(TypeId);

        impl ChildSlot for RecordedSlot {
            fn from_view<V: View>(_view: V) -> Self {
                Self(TypeId::of::<V>())
            }
        }

        struct CapturingBackend {
            seen: Option<(TypeId, TypeId)>,
        }

        impl Backend for CapturingBackend {
            type Output = ();
            type Ctx = ();
            type Slot = RecordedSlot;

            fn native<T: NativeView + 'static>(&mut self, _native: T, _env: &Environment, (): ()) {}

            fn metadata<K: MetadataKey>(
                &mut self,
                metadata: Metadata<K, Self::Slot>,
                _env: &Environment,
                (): (),
            ) {
                if let Ok(mask) = cast::<K, Mask<RecordedSlot>>(metadata.value) {
                    self.seen = Some((metadata.content.0, mask.into_mask().0));
                }
            }

            fn erased(&mut self, _view: AnyView, _env: &Environment, (): ()) {
                panic!("MaskView must dispatch statically without AnyView");
            }

            fn scoped<H: HookList, V: View>(
                &mut self,
                _hooks: H,
                view: V,
                env: &Environment,
                (): (),
            ) {
                view.dispatch(self, env, ());
            }

            fn hookable<T: HookableView>(&mut self, view: T, env: &Environment, (): ()) {
                self.native(view, env, ());
            }
        }

        let view = MaskView::new(ContentProbe, Mask::new(MaskProbe));
        let mut backend = CapturingBackend { seen: None };
        view.dispatch(&mut backend, &Environment::new(), ());
        assert_eq!(
            backend.seen,
            Some((TypeId::of::<ContentProbe>(), TypeId::of::<MaskProbe>()))
        );
    }
}
