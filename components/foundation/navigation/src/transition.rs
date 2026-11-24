//! Extensible navigation transitions.

use alloc::{boxed::Box, collections::BTreeMap, rc::Rc, vec::Vec};
use core::{
    any::{Any, TypeId},
    cell::RefCell,
    fmt::{Debug, Formatter, Result as FmtResult},
    hash::{Hash, Hasher},
};

use waterui_core::{
    Environment, Metadata, View, contract_violation, id::Id, metadata::MetadataKey,
};

trait KeyTable: Any {
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

struct TypedKeyTable<K: Hash + Eq + 'static> {
    buckets: BTreeMap<u64, Vec<(K, Id)>>,
}

impl<K: Hash + Eq + 'static> Default for TypedKeyTable<K> {
    fn default() -> Self {
        Self {
            buckets: BTreeMap::new(),
        }
    }
}

impl<K: Hash + Eq + 'static> KeyTable for TypedKeyTable<K> {
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

struct Fnv1a64(u64);

impl Default for Fnv1a64 {
    fn default() -> Self {
        // 64-bit FNV-1a offset basis.
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Hasher for Fnv1a64 {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        // 64-bit FNV-1a prime.
        const PRIME: u64 = 0x0100_0000_01b3;
        for &byte in bytes {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(PRIME);
        }
    }
}

fn hash_key<K: Hash>(key: &K) -> u64 {
    let mut hasher = Fnv1a64::default();
    key.hash(&mut hasher);
    hasher.finish()
}

fn fallback_id<K: Hash>(key: &K) -> Id {
    let raw = (hash_key(key) & 0x7fff_fffe) as i32 + 1;
    Id::try_from(raw).unwrap_or_else(|_| Id::try_from(1).expect("1 is a valid non-zero Id"))
}

struct TransitionKeyRegistryInner {
    tables: BTreeMap<TypeId, Box<dyn KeyTable>>,
    next_id: i32,
}

impl Default for TransitionKeyRegistryInner {
    fn default() -> Self {
        Self {
            tables: BTreeMap::new(),
            next_id: 1,
        }
    }
}

impl TransitionKeyRegistryInner {
    fn intern<K: Hash + Eq + 'static>(&mut self, key: K) -> Id {
        let hash = hash_key(&key);
        let type_id = TypeId::of::<K>();
        let table = self
            .tables
            .entry(type_id)
            .or_insert_with(|| Box::new(TypedKeyTable::<K>::default()));
        let typed = table
            .as_any_mut()
            .downcast_mut::<TypedKeyTable<K>>()
            .expect("TypeId key guarantees exact TypedKeyTable<K>");

        let bucket = typed.buckets.entry(hash).or_default();
        if let Some((_, id)) = bucket.iter().find(|(existing, _)| existing == &key) {
            return *id;
        }

        let raw = self.next_id;
        self.next_id = self.next_id.checked_add(1).unwrap_or_else(|| {
            contract_violation!(
                "matched navigation transition key capacity exhausted; reusing fallback id"
            );
            1
        });
        let id = Id::try_from(raw).unwrap_or_else(|_| fallback_id(&key));
        bucket.push((key, id));
        id
    }
}

/// Per-navigation-container registry erasing typed transition keys into [`Id`]s.
#[derive(Clone, Default)]
pub struct TransitionKeyRegistry(Rc<RefCell<TransitionKeyRegistryInner>>);

impl Debug for TransitionKeyRegistry {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter
            .debug_struct("TransitionKeyRegistry")
            .finish_non_exhaustive()
    }
}

impl TransitionKeyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ensure_in(env: &mut Environment) {
        if env.get::<Self>().is_none() {
            env.insert(Self::new());
        }
    }

    pub fn scoped(env: &Environment) -> Environment {
        let mut scoped = env.clone();
        Self::ensure_in(&mut scoped);
        scoped
    }

    pub fn intern<K: Hash + Eq + 'static>(&self, key: K) -> Id {
        self.0.borrow_mut().intern(key)
    }
}

pub fn resolve_transition_key<K: Hash + Eq + 'static>(env: &Environment, key: K) -> Id {
    if let Some(registry) = env.get::<TransitionKeyRegistry>() {
        registry.intern(key)
    } else {
        contract_violation!(
            "matched navigation transition keys must be resolved inside a NavigationStack or NavigationSplitView"
        );
        fallback_id(&key)
    }
}

/// Marks the source geometry for a matched navigation transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavigationTransitionSource(pub(crate) Id);

impl MetadataKey for NavigationTransitionSource {}

/// Marks the destination geometry for a matched navigation transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavigationTransitionDestination(pub(crate) Id);

impl MetadataKey for NavigationTransitionDestination {}

/// A view marked as the source geometry for a matched navigation transition keyed by `K`.
#[derive(Debug)]
pub struct KeyedNavigationTransitionSource<V, K> {
    content: V,
    key: K,
}

impl<V: View, K: Hash + Eq + 'static> View for KeyedNavigationTransitionSource<V, K> {
    fn body(self, env: &Environment) -> impl View {
        let id = resolve_transition_key(env, self.key);
        Metadata::new(self.content, NavigationTransitionSource(id))
    }
}

/// A view marked as the destination geometry for a matched navigation transition keyed by `K`.
#[derive(Debug)]
pub struct KeyedNavigationTransitionDestination<V, K> {
    content: V,
    key: K,
}

impl<V: View, K: Hash + Eq + 'static> View for KeyedNavigationTransitionDestination<V, K> {
    fn body(self, env: &Environment) -> impl View {
        let id = resolve_transition_key(env, self.key);
        Metadata::new(self.content, NavigationTransitionDestination(id))
    }
}

/// Navigation transition metadata available on every view.
pub trait NavigationTransitionViewExt: View + Sized {
    /// Marks this view as the source geometry for a matched transition.
    fn navigation_transition_source<K: Hash + Eq + 'static>(
        self,
        key: K,
    ) -> KeyedNavigationTransitionSource<Self, K> {
        KeyedNavigationTransitionSource { content: self, key }
    }

    /// Marks this view as the destination geometry for a matched transition.
    fn navigation_transition_destination<K: Hash + Eq + 'static>(
        self,
        key: K,
    ) -> KeyedNavigationTransitionDestination<Self, K> {
        KeyedNavigationTransitionDestination { content: self, key }
    }
}

impl<V: View> NavigationTransitionViewExt for V {}

impl NavigationTransitionSource {
    /// Returns the transition identity.
    #[doc(hidden)]
    #[must_use]
    pub const fn id(self) -> Id {
        self.0
    }
}

impl NavigationTransitionDestination {
    /// Returns the transition identity.
    #[doc(hidden)]
    #[must_use]
    pub const fn id(self) -> Id {
        self.0
    }
}

/// Direction of a navigation transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "closed: a navigation transition pushes or pops"
)]
pub enum NavigationTransitionDirection {
    /// A destination is entering above the current page.
    Push,
    /// The current destination is leaving toward its parent.
    Pop,
}

/// Per-layer values resolved by a custom navigation transition.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct NavigationTransitionLayer {
    /// Horizontal offset as a fraction of the navigation viewport width.
    offset_x: f32,
    /// Vertical offset as a fraction of the navigation viewport height.
    offset_y: f32,
    /// Uniform scale.
    scale: f32,
    /// Layer opacity.
    opacity: f32,
}

impl NavigationTransitionLayer {
    /// Horizontal offset as a fraction of the navigation viewport width.
    #[must_use]
    pub const fn offset_x(&self) -> f32 {
        self.offset_x
    }

    /// Vertical offset as a fraction of the navigation viewport height.
    #[must_use]
    pub const fn offset_y(&self) -> f32 {
        self.offset_y
    }

    /// Uniform scale.
    #[must_use]
    pub const fn scale(&self) -> f32 {
        self.scale
    }

    /// Layer opacity.
    #[must_use]
    pub const fn opacity(&self) -> f32 {
        self.opacity
    }
}

impl NavigationTransitionLayer {
    /// An unchanged, fully visible layer.
    pub const IDENTITY: Self = Self::new(0.0, 0.0, 1.0, 1.0);

    /// A layer offset by `offset_x` and `offset_y` viewport fractions,
    /// scaled by `scale`, at `opacity`.
    #[must_use]
    pub const fn new(offset_x: f32, offset_y: f32, scale: f32, opacity: f32) -> Self {
        Self {
            offset_x,
            offset_y,
            scale,
            opacity,
        }
    }
}

/// One resolved frame of a navigation transition.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct NavigationTransitionFrame {
    /// Transform and opacity applied to the page that was active.
    outgoing: NavigationTransitionLayer,
    /// Transform and opacity applied to the page becoming active.
    incoming: NavigationTransitionLayer,
}

impl NavigationTransitionFrame {
    /// Transform and opacity applied to the page that was active.
    #[must_use]
    pub const fn outgoing(&self) -> NavigationTransitionLayer {
        self.outgoing
    }

    /// Transform and opacity applied to the page becoming active.
    #[must_use]
    pub const fn incoming(&self) -> NavigationTransitionLayer {
        self.incoming
    }
}

impl NavigationTransitionFrame {
    /// A frame with no visual transition.
    pub const IDENTITY: Self = Self::new(
        NavigationTransitionLayer::IDENTITY,
        NavigationTransitionLayer::IDENTITY,
    );

    /// A frame applying `outgoing` to the page that was active and `incoming`
    /// to the page becoming active.
    #[must_use]
    pub const fn new(
        outgoing: NavigationTransitionLayer,
        incoming: NavigationTransitionLayer,
    ) -> Self {
        Self { outgoing, incoming }
    }
}

/// How a retained renderer executes one transition.
///
/// This is deliberately independent of [`NavigationTransition::native`]: a
/// custom transition is free to report a platform-native projection for Apple
/// and Android while still resolving its own frames on a retained renderer.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each retained transition variant by variant; a new retained transition must fail their build instead of falling into a wildcard arm"
)]
pub enum RetainedNavigationTransition {
    /// Let the renderer apply its own themed platform-default motion.
    PlatformDefault,
    /// Animate geometry matched by this identity across the two pages.
    MatchedGeometry(Id),
    /// Sample [`NavigationTransition::frame`] for every step.
    Frames,
    /// Apply the transaction with no animation at all.
    None,
}

/// A navigation transition executable by retained renderers.
///
/// Apple and Android negotiate [`Self::native`] first. A transition that does
/// not expose a native representation is reported as custom; native backends
/// log a warning and apply the transaction without animation.
///
/// Retained renderers such as Hydrolysis dispatch on [`Self::retained`], which
/// defaults to [`RetainedNavigationTransition::Frames`] — so a custom
/// transition's [`Self::frame`] is always what runs unless it explicitly asks
/// to be rendered some other way.
pub trait NavigationTransition: Debug + 'static {
    /// Resolves a normalized transition frame.
    ///
    /// `progress` is in the inclusive range `0.0..=1.0`. The default is the
    /// identity frame, which is only ever sampled by transitions that opt out
    /// of [`RetainedNavigationTransition::Frames`].
    fn frame(
        &self,
        progress: f32,
        direction: NavigationTransitionDirection,
    ) -> NavigationTransitionFrame {
        let _ = (progress, direction);
        NavigationTransitionFrame::IDENTITY
    }

    /// Returns a platform-native representation when one exists.
    #[doc(hidden)]
    fn native(&self) -> Option<NativeNavigationTransition> {
        None
    }

    /// Returns how a retained renderer should execute this transition.
    #[doc(hidden)]
    fn retained(&self) -> RetainedNavigationTransition {
        RetainedNavigationTransition::Frames
    }

    /// Resolves any container-scoped transition keys against `env`.
    #[doc(hidden)]
    fn resolve(&self, _env: &Environment) -> Option<AnyNavigationTransition> {
        None
    }
}

/// Type-erased navigation transition retained by a stack.
#[derive(Clone)]
pub struct AnyNavigationTransition(alloc::rc::Rc<dyn NavigationTransition>);

impl Debug for AnyNavigationTransition {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl AnyNavigationTransition {
    /// Erases a concrete transition.
    #[must_use]
    pub fn new(transition: impl NavigationTransition) -> Self {
        Self(alloc::rc::Rc::new(transition))
    }

    /// Resolves a frame for a retained renderer.
    ///
    /// # Contract violation
    ///
    /// `progress` must be within `0.0..=1.0`. Otherwise a debug build
    /// panics; a release build logs it and clamps the progress into the
    /// range (`NaN` becomes `0.0`).
    #[must_use]
    pub fn frame(
        &self,
        progress: f32,
        direction: NavigationTransitionDirection,
    ) -> NavigationTransitionFrame {
        let progress = if (0.0..=1.0).contains(&progress) {
            progress
        } else {
            contract_violation!(
                "navigation transition progress must be in 0.0..=1.0; the progress is clamped into the range"
            );
            if progress.is_nan() {
                0.0
            } else {
                progress.clamp(0.0, 1.0)
            }
        };
        self.0.frame(progress, direction)
    }

    /// Resolves the native capability projection.
    #[doc(hidden)]
    #[must_use]
    pub fn native(&self) -> NativeNavigationTransition {
        self.0
            .native()
            .unwrap_or(NativeNavigationTransition::Custom)
    }

    /// Resolves how a retained renderer should execute this transition.
    #[doc(hidden)]
    #[must_use]
    pub fn retained(&self) -> RetainedNavigationTransition {
        self.0.retained()
    }

    /// Resolves any container-scoped transition keys against `env`.
    #[doc(hidden)]
    #[must_use]
    pub fn resolve(self, env: &Environment) -> Self {
        self.0.resolve(env).unwrap_or(self)
    }
}

impl NavigationTransition for AnyNavigationTransition {
    fn frame(
        &self,
        progress: f32,
        direction: NavigationTransitionDirection,
    ) -> NavigationTransitionFrame {
        self.0.frame(progress, direction)
    }

    fn native(&self) -> Option<NativeNavigationTransition> {
        self.0.native()
    }

    fn retained(&self) -> RetainedNavigationTransition {
        self.0.retained()
    }
}

/// Transition representation understood by native backends.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeNavigationTransition {
    /// Platform-default navigation motion.
    Automatic,
    /// Cross-fade.
    Fade,
    /// Platform-native zoom/shared-element transition.
    Zoom(Id),
    /// No animation.
    None,
    /// A retained-renderer-only custom transition.
    Custom,
}

/// Built-in navigation transition constructors.
pub mod navigation_transition {
    use alloc::rc::Rc;
    use core::{
        any::type_name,
        cell::RefCell,
        fmt::{Debug, Formatter, Result as FmtResult},
        hash::Hash,
    };

    use super::{
        AnyNavigationTransition, NativeNavigationTransition, NavigationTransition,
        NavigationTransitionDirection, NavigationTransitionFrame, NavigationTransitionLayer,
        RetainedNavigationTransition, resolve_transition_key,
    };
    use waterui_core::{Environment, contract_violation, id::Id};

    /// Platform-default navigation motion.
    #[derive(Debug, Clone, Copy, Default)]
    pub struct Automatic;

    /// Cross-fade navigation motion.
    #[derive(Debug, Clone, Copy, Default)]
    pub struct Fade;

    enum ZoomState<K> {
        Unresolved(Option<K>),
        Resolved(Id),
    }

    /// Platform-native zoom/shared-element navigation motion keyed by `K`.
    pub struct Zoom<K>(Rc<RefCell<ZoomState<K>>>);

    impl<K> Clone for Zoom<K> {
        fn clone(&self) -> Self {
            Self(Rc::clone(&self.0))
        }
    }

    impl<K> Debug for Zoom<K> {
        fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
            let id = match &*self.0.borrow() {
                ZoomState::Unresolved(_) => Option::None,
                ZoomState::Resolved(id) => Some(*id),
            };
            formatter
                .debug_struct("Zoom")
                .field("key_type", &type_name::<K>())
                .field("resolved_id", &id)
                .finish()
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct ResolvedZoom(Id);

    /// Navigation with animation disabled.
    #[derive(Debug, Clone, Copy, Default)]
    pub struct None;

    /// Uses the platform-default navigation motion.
    #[must_use]
    pub const fn automatic() -> Automatic {
        Automatic
    }

    /// Cross-fades between pages.
    #[must_use]
    pub const fn fade() -> Fade {
        Fade
    }

    /// Uses a platform-native zoom/shared-element transition matching `source`.
    #[must_use]
    pub fn zoom<K: Hash + Eq + 'static>(source: K) -> Zoom<K> {
        Zoom(Rc::new(RefCell::new(ZoomState::Unresolved(Some(source)))))
    }

    /// Disables navigation animation.
    #[must_use]
    pub const fn none() -> None {
        None
    }

    // `Automatic` deliberately resolves no frames of its own: every backend
    // supplies the motion its platform expects, which for a retained renderer
    // means the themed navigation motion rather than a curve hardcoded here.
    impl NavigationTransition for Automatic {
        fn native(&self) -> Option<NativeNavigationTransition> {
            Some(NativeNavigationTransition::Automatic)
        }

        fn retained(&self) -> RetainedNavigationTransition {
            RetainedNavigationTransition::PlatformDefault
        }
    }

    impl NavigationTransition for Fade {
        fn frame(
            &self,
            progress: f32,
            _direction: NavigationTransitionDirection,
        ) -> NavigationTransitionFrame {
            NavigationTransitionFrame {
                outgoing: NavigationTransitionLayer {
                    opacity: 1.0 - progress,
                    ..NavigationTransitionLayer::IDENTITY
                },
                incoming: NavigationTransitionLayer {
                    opacity: progress,
                    ..NavigationTransitionLayer::IDENTITY
                },
            }
        }

        fn native(&self) -> Option<NativeNavigationTransition> {
            Some(NativeNavigationTransition::Fade)
        }
    }

    fn zoom_frame(progress: f32) -> NavigationTransitionFrame {
        NavigationTransitionFrame {
            outgoing: NavigationTransitionLayer {
                scale: 1.0 + 0.05 * progress,
                opacity: 1.0 - progress,
                ..NavigationTransitionLayer::IDENTITY
            },
            incoming: NavigationTransitionLayer {
                scale: 0.85 + 0.15 * progress,
                opacity: progress,
                ..NavigationTransitionLayer::IDENTITY
            },
        }
    }

    impl<K: Hash + Eq + 'static> Zoom<K> {
        fn resolved_id(&self) -> Option<Id> {
            match &*self.0.borrow() {
                ZoomState::Resolved(id) => Some(*id),
                ZoomState::Unresolved(_) => Option::None,
            }
        }
    }

    impl<K: Hash + Eq + 'static> NavigationTransition for Zoom<K> {
        fn frame(
            &self,
            progress: f32,
            _direction: NavigationTransitionDirection,
        ) -> NavigationTransitionFrame {
            zoom_frame(progress)
        }

        fn native(&self) -> Option<NativeNavigationTransition> {
            self.resolved_id().map_or_else(
                || {
                    contract_violation!(
                        "matched zoom transition must be resolved against a NavigationStack environment before querying native projection"
                    );
                    Some(NativeNavigationTransition::Automatic)
                },
                |id| Some(NativeNavigationTransition::Zoom(id)),
            )
        }

        fn retained(&self) -> RetainedNavigationTransition {
            self.resolved_id().map_or_else(
                || {
                    contract_violation!(
                        "matched zoom transition must be resolved against a NavigationStack environment before querying retained execution"
                    );
                    RetainedNavigationTransition::PlatformDefault
                },
                RetainedNavigationTransition::MatchedGeometry,
            )
        }

        fn resolve(&self, env: &Environment) -> Option<AnyNavigationTransition> {
            let mut state = self.0.borrow_mut();
            let id = match &mut *state {
                ZoomState::Resolved(id) => *id,
                ZoomState::Unresolved(key) => {
                    let key = key
                        .take()
                        .expect("unresolved zoom transition must hold a key");
                    let id = resolve_transition_key(env, key);
                    *state = ZoomState::Resolved(id);
                    id
                }
            };
            Some(AnyNavigationTransition::new(ResolvedZoom(id)))
        }
    }

    impl NavigationTransition for ResolvedZoom {
        fn frame(
            &self,
            progress: f32,
            _direction: NavigationTransitionDirection,
        ) -> NavigationTransitionFrame {
            zoom_frame(progress)
        }

        fn native(&self) -> Option<NativeNavigationTransition> {
            Some(NativeNavigationTransition::Zoom(self.0))
        }

        fn retained(&self) -> RetainedNavigationTransition {
            RetainedNavigationTransition::MatchedGeometry(self.0)
        }
    }

    impl NavigationTransition for None {
        fn frame(
            &self,
            _progress: f32,
            _direction: NavigationTransitionDirection,
        ) -> NavigationTransitionFrame {
            NavigationTransitionFrame::IDENTITY
        }

        fn native(&self) -> Option<NativeNavigationTransition> {
            Some(NativeNavigationTransition::None)
        }

        fn retained(&self) -> RetainedNavigationTransition {
            RetainedNavigationTransition::None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AnyNavigationTransition, NativeNavigationTransition, NavigationTransition,
        NavigationTransitionDirection, NavigationTransitionFrame, NavigationTransitionLayer,
        NavigationTransitionViewExt as _, RetainedNavigationTransition, TransitionKeyRegistry,
        navigation_transition, resolve_transition_key,
    };
    use crate::{IntoNavigationStackLayout as _, NavigationStack, NavigationView};
    use waterui_core::{AnyView, Environment};

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "navigation transition progress must be in 0.0..=1.0")
    )]
    fn out_of_range_progress_is_clamped() {
        let transition = AnyNavigationTransition::new(navigation_transition::fade());
        let direction = NavigationTransitionDirection::Push;

        // Release outcome: the progress is clamped into the range.
        assert_eq!(
            transition.frame(2.0, direction),
            transition.frame(1.0, direction)
        );
        assert_eq!(
            transition.frame(f32::NAN, direction),
            transition.frame(0.0, direction)
        );
    }

    /// A transition that wants the platform's own motion on Apple and Android
    /// but draws its own curve everywhere else — the combination that used to
    /// lose its frames on a retained renderer.
    #[derive(Debug)]
    struct NativelyAutomaticButCustomFrames;

    impl NavigationTransition for NativelyAutomaticButCustomFrames {
        fn frame(
            &self,
            progress: f32,
            _direction: NavigationTransitionDirection,
        ) -> NavigationTransitionFrame {
            NavigationTransitionFrame {
                outgoing: NavigationTransitionLayer {
                    scale: 1.0 - progress,
                    ..NavigationTransitionLayer::IDENTITY
                },
                incoming: NavigationTransitionLayer::IDENTITY,
            }
        }

        fn native(&self) -> Option<NativeNavigationTransition> {
            Some(NativeNavigationTransition::Automatic)
        }
    }

    /// Requesting a platform-native projection must not cost a transition its
    /// own frames: the two capabilities are negotiated independently.
    #[test]
    fn a_native_projection_does_not_suppress_custom_frames() {
        let transition = AnyNavigationTransition::new(NativelyAutomaticButCustomFrames);

        assert_eq!(transition.native(), NativeNavigationTransition::Automatic);
        assert_eq!(transition.retained(), RetainedNavigationTransition::Frames);
        let scale = transition
            .frame(1.0, NavigationTransitionDirection::Push)
            .outgoing
            .scale;
        assert!(
            scale.abs() <= f32::EPSILON,
            "the retained renderer must sample this transition's own curve, got {scale}"
        );
    }

    /// The built-in styles each declare how a retained renderer runs them, so
    /// none of them depend on being recognised by native identity.
    #[test]
    fn built_in_styles_declare_their_retained_execution() {
        #[derive(Hash, PartialEq, Eq)]
        struct PhotoKey(&'static str);

        assert_eq!(
            AnyNavigationTransition::new(navigation_transition::automatic()).retained(),
            RetainedNavigationTransition::PlatformDefault
        );
        assert_eq!(
            AnyNavigationTransition::new(navigation_transition::fade()).retained(),
            RetainedNavigationTransition::Frames
        );
        assert_eq!(
            AnyNavigationTransition::new(navigation_transition::none()).retained(),
            RetainedNavigationTransition::None
        );

        let registry = TransitionKeyRegistry::new();
        let mut env = Environment::new();
        env.insert(registry);

        let expected = resolve_transition_key(&env, PhotoKey("hero"));
        let zoom = AnyNavigationTransition::new(navigation_transition::zoom(PhotoKey("hero")))
            .resolve(&env);
        assert_eq!(
            zoom.retained(),
            RetainedNavigationTransition::MatchedGeometry(expected)
        );
        assert_eq!(zoom.native(), NativeNavigationTransition::Zoom(expected));
    }

    #[test]
    fn distinct_key_types_with_same_value_do_not_collide() {
        let mut env = Environment::new();
        env.insert(TransitionKeyRegistry::new());

        let u32_id = resolve_transition_key(&env, 0_u32);
        let usize_id = resolve_transition_key(&env, 0_usize);
        let zero_again = resolve_transition_key(&env, 0_u32);

        assert_ne!(u32_id, usize_id);
        assert_eq!(u32_id, zero_again);
    }

    #[test]
    fn navigation_stack_and_view_resolve_matching_transition_keys() {
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        struct PhotoRoute(usize);

        let stack_env = TransitionKeyRegistry::scoped(&Environment::new());
        let mut stack_layout = NavigationStack::new(NavigationView::new(
            "Gallery",
            ().navigation_transition_source(PhotoRoute(3)),
        ))
        .transition(navigation_transition::zoom(PhotoRoute(3)))
        .into_layout::<AnyView>();
        stack_layout.transition = stack_layout.transition.resolve(&stack_env);

        let stack_zoom_id = match stack_layout.transition.native() {
            NativeNavigationTransition::Zoom(id) => id,
            other => panic!("expected native zoom transition, got {other:?}"),
        };
        let root = stack_layout.resolve(&stack_env);
        assert!(root.transition.is_none());

        let mut resolved_page =
            NavigationView::new("Photo", ().navigation_transition_destination(PhotoRoute(3)))
                .transition(navigation_transition::zoom(PhotoRoute(3)))
                .into_resolved::<AnyView>();
        resolved_page.resolve_native_fields(&stack_env);
        let page_zoom_id = match resolved_page
            .transition
            .expect("page transition should stay present")
            .native()
        {
            NativeNavigationTransition::Zoom(id) => id,
            other => panic!("expected page native zoom transition, got {other:?}"),
        };

        let resolved_key_id = resolve_transition_key(&stack_env, PhotoRoute(3));
        let other_key_id = resolve_transition_key(&stack_env, PhotoRoute(4));

        assert_eq!(stack_zoom_id, page_zoom_id);
        assert_eq!(stack_zoom_id, resolved_key_id);
        assert_ne!(other_key_id, resolved_key_id);
    }
}
