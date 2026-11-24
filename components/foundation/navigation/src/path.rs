//! Reactive navigation path state.

#[cfg(feature = "serde")]
use alloc::collections::BTreeMap;
#[cfg(feature = "serde")]
use alloc::format;
#[cfg(feature = "serde")]
use alloc::string::String;
use alloc::{rc::Rc, vec::Vec};
#[cfg(feature = "serde")]
use core::cell::RefCell;
use core::{
    any::{Any, TypeId, type_name},
    fmt,
    marker::PhantomData,
};

use nami::{Binding, Signal as _};
#[cfg(feature = "serde")]
use serde::de::{DeserializeOwned, DeserializeSeed, Error as DeError, SeqAccess, Visitor};
#[cfg(feature = "serde")]
use serde::ser::{Error as SerError, SerializeTuple as _};
#[cfg(feature = "serde")]
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use waterui_core::contract_violation;

type RouteEquals = fn(&dyn Any, &dyn Any) -> bool;

/// Private route payload used by the heterogeneous specialization of
/// [`NavigationPath`]. It is public only because Rust requires the concrete
/// default-inferred specialization to be nameable across crate boundaries.
#[doc(hidden)]
#[derive(Clone)]
pub struct ErasedNavigationRoute {
    type_id: TypeId,
    type_name: &'static str,
    value: Rc<dyn Any>,
    equals: RouteEquals,
    #[cfg(feature = "serde")]
    restoration: Rc<RefCell<RestorationRegistry>>,
}

impl fmt::Debug for ErasedNavigationRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NavigationRoute")
            .field("type_name", &self.type_name)
            .finish_non_exhaustive()
    }
}

impl ErasedNavigationRoute {
    #[cfg(not(feature = "serde"))]
    pub(crate) fn new<R>(value: R) -> Self
    where
        R: Clone + PartialEq + 'static,
    {
        Self {
            type_id: TypeId::of::<R>(),
            type_name: type_name::<R>(),
            value: Rc::new(value),
            equals: |left, right| {
                let left = left
                    .downcast_ref::<R>()
                    .expect("navigation route type id must match its stored value");
                right.downcast_ref::<R>().is_some_and(|right| left == right)
            },
        }
    }

    #[cfg(feature = "serde")]
    fn new_with_restoration<R>(value: R, restoration: Rc<RefCell<RestorationRegistry>>) -> Self
    where
        R: Clone + PartialEq + 'static,
    {
        let value = Rc::new(value);
        Self {
            type_id: TypeId::of::<R>(),
            type_name: type_name::<R>(),
            value,
            equals: |left, right| {
                let left = left
                    .downcast_ref::<R>()
                    .expect("navigation route type id must match its stored value");
                right.downcast_ref::<R>().is_some_and(|right| left == right)
            },
            restoration,
        }
    }

    pub(crate) const fn type_id(&self) -> TypeId {
        self.type_id
    }

    pub(crate) const fn type_name(&self) -> &'static str {
        self.type_name
    }

    pub(crate) fn downcast_ref<R: 'static>(&self) -> Option<&R> {
        self.value.downcast_ref()
    }

    pub(crate) fn same_route(&self, other: &Self) -> bool {
        self.type_id == other.type_id && (self.equals)(self.value.as_ref(), other.value.as_ref())
    }
}

#[cfg(feature = "serde")]
impl Serialize for ErasedNavigationRoute {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let registry = self.restoration.borrow();
        let entry = registry.routes_by_id.get(&self.type_id).ok_or_else(|| {
            S::Error::custom(format_args!(
                "navigation route type `{}` has no registered restoration key; register it with `restorable_destination` or `register_restoration`",
                self.type_name
            ))
        })?;
        let mut route = serializer.serialize_tuple(2)?;
        route.serialize_element(entry.key)?;
        route.serialize_element((entry.serialize)(self))?;
        route.end()
    }
}

#[cfg(feature = "serde")]
type DeserializeRoute = for<'de> fn(
    &mut dyn erased_serde::Deserializer<'de>,
    Rc<RefCell<RestorationRegistry>>,
) -> Result<ErasedNavigationRoute, erased_serde::Error>;

#[cfg(feature = "serde")]
type SerializeRoute = fn(&ErasedNavigationRoute) -> &dyn erased_serde::Serialize;

#[cfg(feature = "serde")]
#[derive(Clone, Copy)]
struct RestorationEntry {
    key: &'static str,
    type_id: TypeId,
    serialize: SerializeRoute,
    deserialize: DeserializeRoute,
}

#[cfg(feature = "serde")]
#[derive(Default)]
struct RestorationRegistry {
    routes_by_key: BTreeMap<&'static str, RestorationEntry>,
    routes_by_id: BTreeMap<TypeId, RestorationEntry>,
}

/// A reactive path representing the destinations above a navigation root.
///
/// Cloning a path shares its state. Use [`Self::snapshot`] and `FromIterator`
/// to create an independent copy.
#[must_use]
pub struct NavigationPath<R: 'static> {
    inner: Binding<Vec<R>>,
    marker: PhantomData<fn() -> R>,
    #[cfg(feature = "serde")]
    restoration: Rc<RefCell<RestorationRegistry>>,
}

impl<R: 'static> Clone for NavigationPath<R> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            marker: PhantomData,
            #[cfg(feature = "serde")]
            restoration: Rc::clone(&self.restoration),
        }
    }
}

impl<R> fmt::Debug for NavigationPath<R>
where
    R: Clone + fmt::Debug + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("NavigationPath")
            .field(&self.inner.snapshot())
            .finish()
    }
}

impl<R: Clone + PartialEq + 'static> NavigationPath<R> {
    /// Creates an empty strongly typed navigation path.
    pub fn new() -> Self {
        core::iter::empty().collect()
    }

    /// Appends a route.
    pub fn push(&self, route: R) {
        self.inner.with_mut(|routes| routes.push(route));
    }

    /// Removes and returns the top route.
    pub fn pop(&self) -> Option<R> {
        self.inner.with_mut(Vec::pop)
    }

    /// Atomically replaces the complete path.
    pub fn replace(&self, routes: impl IntoIterator<Item = R>) {
        self.inner.set(routes.into_iter().collect());
    }
}

impl NavigationPath<ErasedNavigationRoute> {
    /// Creates an empty heterogeneous navigation path.
    pub fn heterogeneous() -> Self {
        Self {
            inner: Binding::container(Vec::new()),
            marker: PhantomData,
            #[cfg(feature = "serde")]
            restoration: Rc::new(RefCell::new(RestorationRegistry::default())),
        }
    }

    /// Appends a route of any concrete type.
    #[cfg(not(feature = "serde"))]
    pub fn push<R>(&self, route: R)
    where
        R: Clone + PartialEq + 'static,
    {
        self.inner
            .with_mut(|routes| routes.push(ErasedNavigationRoute::new(route)));
    }

    /// Appends a route of any concrete type.
    #[cfg(feature = "serde")]
    pub fn push<R>(&self, route: R)
    where
        R: Clone + PartialEq + 'static,
    {
        let route =
            ErasedNavigationRoute::new_with_restoration(route, Rc::clone(&self.restoration));
        self.inner.with_mut(|routes| routes.push(route));
    }

    /// Removes the top route, reporting whether one was present.
    pub fn pop(&self) -> bool {
        self.inner.with_mut(Vec::pop).is_some()
    }

    /// Atomically replaces the path with routes collected by `replacement`.
    pub fn replace(&self, build: impl FnOnce(&mut NavigationPathReplacement)) {
        let mut replacement = NavigationPathReplacement {
            routes: Vec::new(),
            #[cfg(feature = "serde")]
            restoration: Rc::clone(&self.restoration),
        };
        build(&mut replacement);
        self.inner.set(replacement.routes);
    }

    /// Registers route type `R` for serde restoration under the stable
    /// persistent `key`.
    ///
    /// Serialized paths store `key` rather than `core::any::type_name::<R>()`,
    /// so persisted state stays valid across module moves, renames, and
    /// compiler upgrades.
    ///
    /// # Contract violations
    ///
    /// `key` must be non-empty, each `key` must map to at most one concrete
    /// route type `R`, and each concrete route type `R` must map to at most
    /// one `key`. Re-registering the same `(key, R)` pair is idempotent.
    #[cfg(feature = "serde")]
    pub fn register_restoration<R>(&self, key: &'static str)
    where
        R: Clone + PartialEq + Serialize + DeserializeOwned + 'static,
    {
        fn deserialize_route<R>(
            deserializer: &mut dyn erased_serde::Deserializer<'_>,
            restoration: Rc<RefCell<RestorationRegistry>>,
        ) -> Result<ErasedNavigationRoute, erased_serde::Error>
        where
            R: Clone + PartialEq + Serialize + DeserializeOwned + 'static,
        {
            erased_serde::deserialize::<R>(deserializer)
                .map(|route| ErasedNavigationRoute::new_with_restoration(route, restoration))
        }

        fn serialize_route<R>(route: &ErasedNavigationRoute) -> &dyn erased_serde::Serialize
        where
            R: Serialize + 'static,
        {
            route
                .downcast_ref::<R>()
                .expect("registered navigation serializer type must match route type")
        }

        let route_type = type_name::<R>();
        if key.is_empty() {
            contract_violation!(
                "navigation restoration key must not be empty; `{route_type}` is not registered for restoration"
            );
            return;
        }
        let type_id = TypeId::of::<R>();
        let entry = RestorationEntry {
            key,
            type_id,
            serialize: serialize_route::<R>,
            deserialize: deserialize_route::<R>,
        };
        let mut registry = self.restoration.borrow_mut();
        match (
            registry.routes_by_key.get(key),
            registry.routes_by_id.get(&type_id),
        ) {
            (None, None) => {
                registry.routes_by_key.insert(key, entry);
                registry.routes_by_id.insert(type_id, entry);
            }
            (Some(by_key), Some(by_id)) if by_key.type_id == type_id && by_id.key == key => {}
            (Some(_), _) => {
                contract_violation!(
                    "navigation restoration key `{key}` identifies two route types; `{route_type}` is not registered for restoration"
                );
            }
            (None, Some(by_id)) => {
                let existing_key = by_id.key;
                contract_violation!(
                    "navigation route type `{route_type}` is already registered under restoration key `{existing_key}`; ignoring `{key}`"
                );
            }
        }
    }

    /// Returns a serde seed that atomically restores this heterogeneous path.
    #[cfg(feature = "serde")]
    #[must_use]
    pub fn restoration(&self) -> impl for<'de> DeserializeSeed<'de, Value = ()> + '_ {
        NavigationPathRestoration { path: self }
    }
}

/// Route collector inferred by heterogeneous [`NavigationPath::replace`].
#[doc(hidden)]
pub struct NavigationPathReplacement {
    routes: Vec<ErasedNavigationRoute>,
    #[cfg(feature = "serde")]
    restoration: Rc<RefCell<RestorationRegistry>>,
}

impl fmt::Debug for NavigationPathReplacement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NavigationPathReplacement")
            .field("len", &self.routes.len())
            .finish_non_exhaustive()
    }
}

impl NavigationPathReplacement {
    /// Appends one concrete route to the pending atomic replacement.
    pub fn push<R>(&mut self, route: R)
    where
        R: Clone + PartialEq + 'static,
    {
        #[cfg(feature = "serde")]
        self.routes
            .push(ErasedNavigationRoute::new_with_restoration(
                route,
                Rc::clone(&self.restoration),
            ));
        #[cfg(not(feature = "serde"))]
        self.routes.push(ErasedNavigationRoute::new(route));
    }
}

impl<R: Clone + 'static> NavigationPath<R> {
    /// Returns the number of routes in the path.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.with_ref(Vec::len)
    }

    /// Removes exactly `count` routes from the top.
    ///
    /// # Contract violation
    ///
    /// A `count` beyond the current path length is reported: a debug build
    /// panics; a release build logs it and removes every route.
    pub fn pop_n(&self, count: usize) {
        let length = self.len();
        if count > length {
            contract_violation!(
                "cannot pop {count} routes from a navigation path of length {length}; every route is removed"
            );
        }
        self.truncate(length.saturating_sub(count));
    }

    /// Truncates the path to at most `length` routes.
    pub fn truncate(&self, length: usize) {
        self.inner.with_mut(|routes| routes.truncate(length));
    }

    /// Removes every route.
    pub fn clear(&self) {
        self.inner.set(Vec::new());
    }

    /// Returns whether the path contains no routes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns an independent snapshot of the current routes.
    #[must_use]
    pub fn snapshot(&self) -> Vec<R> {
        self.inner.snapshot()
    }

    /// Returns an iterator over an independent snapshot.
    pub fn iter(&self) -> impl Iterator<Item = R> {
        self.snapshot().into_iter()
    }

    /// Calls `watcher` with the routes after every change; registering
    /// delivers nothing, so a subscriber reads [`Self::snapshot`] first.
    pub(crate) fn watch(
        &self,
        watcher: impl Fn(Vec<R>) + 'static,
    ) -> impl nami::watcher::WatcherGuard {
        self.inner
            .watch(move |context| watcher(context.into_value()))
    }
}

impl<R: Clone + PartialEq + 'static> Default for NavigationPath<R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: Clone + PartialEq + 'static> From<Vec<R>> for NavigationPath<R> {
    fn from(routes: Vec<R>) -> Self {
        Self {
            inner: Binding::container(routes),
            marker: PhantomData,
            #[cfg(feature = "serde")]
            restoration: Rc::new(RefCell::new(RestorationRegistry::default())),
        }
    }
}

#[cfg(feature = "serde")]
struct NavigationPathRestoration<'a> {
    path: &'a NavigationPath<ErasedNavigationRoute>,
}

#[cfg(feature = "serde")]
impl fmt::Debug for NavigationPathRestoration<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NavigationPathRestoration")
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::de::DeserializeSeed<'de> for NavigationPathRestoration<'_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct PathVisitor<'a> {
            path: &'a NavigationPath<ErasedNavigationRoute>,
        }

        impl<'de> serde::de::Visitor<'de> for PathVisitor<'_> {
            type Value = ();

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a heterogeneous navigation path")
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let mut routes = Vec::new();
                while let Some(route) = sequence.next_element_seed(RouteSeed {
                    registry: &self.path.restoration,
                })? {
                    routes.push(route);
                }
                self.path.inner.set(routes);
                Ok(())
            }
        }

        deserializer.deserialize_seq(PathVisitor { path: self.path })
    }
}

#[cfg(feature = "serde")]
struct RouteSeed<'a> {
    registry: &'a Rc<RefCell<RestorationRegistry>>,
}

#[cfg(feature = "serde")]
impl<'de> serde::de::DeserializeSeed<'de> for RouteSeed<'_> {
    type Value = ErasedNavigationRoute;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct RouteVisitor<'a> {
            registry: &'a Rc<RefCell<RestorationRegistry>>,
        }

        impl<'de> Visitor<'de> for RouteVisitor<'_> {
            type Value = ErasedNavigationRoute;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a navigation route key followed by its serde payload")
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let key: String = sequence
                    .next_element()?
                    .ok_or_else(|| A::Error::custom("navigation route is missing its key"))?;
                let factory = self
                    .registry
                    .borrow()
                    .routes_by_key
                    .get(key.as_str())
                    .map(|entry| entry.deserialize)
                    .ok_or_else(|| {
                        A::Error::custom(format!("navigation route key `{key}` is not registered"))
                    })?;
                sequence
                    .next_element_seed(RoutePayloadSeed {
                        factory,
                        restoration: Rc::clone(self.registry),
                    })?
                    .ok_or_else(|| {
                        A::Error::custom(format!("navigation route `{key}` is missing its payload"))
                    })
            }
        }

        deserializer.deserialize_tuple(
            2,
            RouteVisitor {
                registry: self.registry,
            },
        )
    }
}

#[cfg(feature = "serde")]
struct RoutePayloadSeed {
    factory: DeserializeRoute,
    restoration: Rc<RefCell<RestorationRegistry>>,
}

#[cfg(feature = "serde")]
impl<'de> serde::de::DeserializeSeed<'de> for RoutePayloadSeed {
    type Value = ErasedNavigationRoute;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;

        let mut deserializer = <dyn erased_serde::Deserializer>::erase(deserializer);
        (self.factory)(&mut deserializer, self.restoration).map_err(D::Error::custom)
    }
}

impl<R: Clone + PartialEq + 'static> FromIterator<R> for NavigationPath<R> {
    fn from_iter<I: IntoIterator<Item = R>>(routes: I) -> Self {
        Self::from(routes.into_iter().collect::<Vec<_>>())
    }
}

#[cfg(feature = "serde")]
impl<R> Serialize for NavigationPath<R>
where
    R: Clone + Serialize + 'static,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.snapshot().serialize(serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de, R> Deserialize<'de> for NavigationPath<R>
where
    R: Clone + PartialEq + Deserialize<'de> + 'static,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Vec::<R>::deserialize(deserializer).map(Self::from)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "serde")]
    use alloc::string::ToString as _;
    use alloc::{rc::Rc, string::String, vec};
    #[cfg(feature = "serde")]
    use serde::de::DeserializeSeed as _;
    #[cfg(feature = "serde")]
    use serde_json::{Deserializer as JsonDeserializer, from_str, to_string};

    use super::NavigationPath;

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "cannot pop 3 routes from a navigation path of length 2")
    )]
    fn pop_n_beyond_the_length_removes_every_route() {
        let path = NavigationPath::from(vec![1, 2]);

        path.pop_n(3);

        // Release outcome: every route is removed.
        assert_eq!(path.snapshot(), vec![0; 0]);
    }

    #[test]
    fn clone_shares_state_and_snapshot_is_independent() {
        let path = NavigationPath::from(vec![1]);
        let shared = path.clone();
        let snapshot = path.snapshot();

        shared.push(2);

        assert_eq!(path.snapshot(), vec![1, 2]);
        assert_eq!(snapshot, vec![1]);
    }

    #[test]
    fn replace_notifies_once() {
        let path = NavigationPath::from(vec![1]);
        let notifications = Rc::new(core::cell::Cell::new(0));
        let observed = Rc::clone(&notifications);
        let _guard = path.watch(move |_| observed.set(observed.get() + 1));

        path.replace([2, 3, 4]);

        assert_eq!(notifications.get(), 1, "registering delivers nothing");
        assert_eq!(path.snapshot(), vec![2, 3, 4]);
    }

    #[test]
    fn heterogeneous_path_accepts_distinct_route_types() {
        let path = NavigationPath::heterogeneous();
        path.push(7_u32);
        path.push(String::from("settings"));

        let routes = path.snapshot();
        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].downcast_ref::<u32>(), Some(&7));
        assert_eq!(
            routes[1].downcast_ref::<String>().map(String::as_str),
            Some("settings")
        );
    }

    #[test]
    fn heterogeneous_replace_is_atomic_and_accepts_distinct_types() {
        let path = NavigationPath::heterogeneous();
        let notifications = Rc::new(core::cell::Cell::new(0));
        let observed = Rc::clone(&notifications);
        let _guard = path.watch(move |_| observed.set(observed.get() + 1));

        path.replace(|routes| {
            routes.push(7_u32);
            routes.push(String::from("settings"));
        });

        assert_eq!(notifications.get(), 1, "registering delivers nothing");
        let routes = path.snapshot();
        assert_eq!(routes[0].downcast_ref::<u32>(), Some(&7));
        assert_eq!(
            routes[1].downcast_ref::<String>().map(String::as_str),
            Some("settings")
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn typed_path_round_trips_through_serde() {
        let path = NavigationPath::from(vec![1_u32, 2, 3]);
        let encoded = to_string(&path).expect("typed path must serialize");
        let restored: NavigationPath<u32> =
            from_str(&encoded).expect("typed path must deserialize");

        assert_eq!(restored.snapshot(), vec![1, 2, 3]);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn heterogeneous_path_restores_with_explicit_keys() {
        let path = NavigationPath::heterogeneous();
        path.register_restoration::<u32>("id");
        path.register_restoration::<String>("section");
        path.push(7_u32);
        path.push(String::from("settings"));
        let encoded = to_string(&path).expect("heterogeneous path must serialize");
        assert_eq!(encoded, r#"[["id",7],["section","settings"]]"#);

        let restored = NavigationPath::heterogeneous();
        restored.register_restoration::<u32>("id");
        restored.register_restoration::<String>("section");
        let mut deserializer = JsonDeserializer::from_str(&encoded);
        restored
            .restoration()
            .deserialize(&mut deserializer)
            .expect("registered heterogeneous path must restore");

        let routes = restored.snapshot();
        assert_eq!(routes[0].downcast_ref::<u32>(), Some(&7));
        assert_eq!(
            routes[1].downcast_ref::<String>().map(String::as_str),
            Some("settings")
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn restoration_key_survives_module_move() {
        mod v1 {
            use serde::{Deserialize, Serialize};

            #[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
            pub struct Settings(pub u32);
        }
        mod v2 {
            use serde::{Deserialize, Serialize};

            #[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
            pub struct Settings(pub u32);
        }

        let path = NavigationPath::heterogeneous();
        path.register_restoration::<v1::Settings>("settings");
        path.push(v1::Settings(42));
        let encoded = to_string(&path).expect("registered route must serialize");

        assert!(
            !encoded.contains("::tests::") && !encoded.contains("v1"),
            "persisted route key must not embed module path: {encoded}"
        );
        assert_eq!(encoded, r#"[["settings",42]]"#);

        let restored = NavigationPath::heterogeneous();
        restored.register_restoration::<v2::Settings>("settings");
        let mut deserializer = JsonDeserializer::from_str(&encoded);
        restored
            .restoration()
            .deserialize(&mut deserializer)
            .expect("saved route must deserialize into moved route type under same key");

        let routes = restored.snapshot();
        assert_eq!(routes.len(), 1);
        assert!(routes[0].downcast_ref::<v2::Settings>() == Some(&v2::Settings(42)));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn heterogeneous_restoration_registration_is_idempotent() {
        let path = NavigationPath::heterogeneous();

        path.register_restoration::<u32>("id");
        path.register_restoration::<u32>("id");
        path.push(7_u32);

        assert_eq!(
            to_string(&path).expect("registered route must serialize"),
            r#"[["id",7]]"#
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn unregistered_heterogeneous_route_fails_to_serialize() {
        let path = NavigationPath::heterogeneous();
        path.push(7_u32);

        let error =
            to_string(&path).expect_err("unregistered heterogeneous route must fail to serialize");
        assert!(
            error
                .to_string()
                .contains("has no registered restoration key"),
            "unexpected error: {error}"
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "navigation restoration key `dup` identifies two route types")
    )]
    fn conflicting_key_for_two_route_types_is_rejected() {
        let path = NavigationPath::heterogeneous();
        path.register_restoration::<u32>("dup");
        path.register_restoration::<String>("dup");
    }

    #[cfg(feature = "serde")]
    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "is already registered under restoration key `first`")
    )]
    fn conflicting_keys_for_same_route_type_are_rejected() {
        let path = NavigationPath::heterogeneous();
        path.register_restoration::<u32>("first");
        path.register_restoration::<u32>("second");
    }

    #[cfg(feature = "serde")]
    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "navigation restoration key must not be empty")
    )]
    fn empty_restoration_key_is_rejected() {
        let path = NavigationPath::heterogeneous();
        path.register_restoration::<u32>("");
    }
}
