//! Reactive navigation path state.

#[cfg(feature = "serde")]
use alloc::collections::BTreeMap;
#[cfg(feature = "serde")]
use alloc::string::String;
use alloc::{rc::Rc, vec::Vec};
#[cfg(feature = "serde")]
use core::cell::RefCell;
use core::{any::Any, any::TypeId, fmt, marker::PhantomData};

use nami::collection::{Collection as _, List};

/// Internal bound used by heterogeneous destination registration.
#[doc(hidden)]
#[cfg(feature = "serde")]
pub trait RestorableNavigationRoute:
    Clone + PartialEq + serde::Serialize + serde::de::DeserializeOwned + 'static
{
}

#[cfg(feature = "serde")]
impl<T> RestorableNavigationRoute for T where
    T: Clone + PartialEq + serde::Serialize + serde::de::DeserializeOwned + 'static
{
}

/// Internal bound used by heterogeneous destination registration.
#[doc(hidden)]
#[cfg(not(feature = "serde"))]
pub trait RestorableNavigationRoute: Clone + PartialEq + 'static {}

#[cfg(not(feature = "serde"))]
impl<T> RestorableNavigationRoute for T where T: Clone + PartialEq + 'static {}

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
            type_name: core::any::type_name::<R>(),
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
            type_name: core::any::type_name::<R>(),
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
impl serde::Serialize for ErasedNavigationRoute {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeTuple as _;

        let registry = self.restoration.borrow();
        let entry = registry.routes_by_id.get(&self.type_id).unwrap_or_else(|| {
            panic!(
                "navigation route type `{}` has no registered serde destination",
                self.type_name
            )
        });
        let mut route = serializer.serialize_tuple(2)?;
        route.serialize_element(self.type_name)?;
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
    type_id: TypeId,
    serialize: SerializeRoute,
    deserialize: DeserializeRoute,
}

#[cfg(feature = "serde")]
#[derive(Default)]
struct RestorationRegistry {
    routes_by_name: BTreeMap<&'static str, RestorationEntry>,
    routes_by_id: BTreeMap<TypeId, RestorationEntry>,
}

/// A reactive path representing the destinations above a navigation root.
///
/// Cloning a path shares its state. Use [`Self::snapshot`] and `FromIterator`
/// to create an independent copy.
#[must_use]
pub struct NavigationPath<R: 'static> {
    inner: List<R>,
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
        self.inner.push(route);
    }

    /// Removes and returns the top route.
    pub fn pop(&self) -> Option<R> {
        self.inner.pop()
    }

    /// Atomically replaces the complete path.
    pub fn replace(&self, routes: impl IntoIterator<Item = R>) {
        let _ = self.inner.replace(routes.into_iter().collect());
    }
}

impl NavigationPath<ErasedNavigationRoute> {
    /// Creates an empty heterogeneous navigation path.
    pub fn heterogeneous() -> Self {
        Self {
            inner: List::new(),
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
        self.inner.push(ErasedNavigationRoute::new(route));
    }

    /// Appends a route of any concrete type.
    #[cfg(feature = "serde")]
    pub fn push<R>(&self, route: R)
    where
        R: Clone + PartialEq + 'static,
    {
        self.inner.push(ErasedNavigationRoute::new_with_restoration(
            route,
            Rc::clone(&self.restoration),
        ));
    }

    /// Removes the top route, reporting whether one was present.
    pub fn pop(&self) -> bool {
        self.inner.pop().is_some()
    }

    /// Atomically replaces the path with routes collected by `replacement`.
    pub fn replace(&self, build: impl FnOnce(&mut NavigationPathReplacement)) {
        let mut replacement = NavigationPathReplacement {
            routes: Vec::new(),
            #[cfg(feature = "serde")]
            restoration: Rc::clone(&self.restoration),
        };
        build(&mut replacement);
        let _ = self.inner.replace(replacement.routes);
    }

    #[cfg(feature = "serde")]
    pub(crate) fn register_restoration<R>(&self)
    where
        R: Clone + PartialEq + serde::Serialize + serde::de::DeserializeOwned + 'static,
    {
        fn deserialize_route<R>(
            deserializer: &mut dyn erased_serde::Deserializer<'_>,
            restoration: Rc<RefCell<RestorationRegistry>>,
        ) -> Result<ErasedNavigationRoute, erased_serde::Error>
        where
            R: Clone + PartialEq + serde::Serialize + serde::de::DeserializeOwned + 'static,
        {
            erased_serde::deserialize::<R>(deserializer)
                .map(|route| ErasedNavigationRoute::new_with_restoration(route, restoration))
        }

        fn serialize_route<R>(route: &ErasedNavigationRoute) -> &dyn erased_serde::Serialize
        where
            R: serde::Serialize + 'static,
        {
            route
                .downcast_ref::<R>()
                .expect("registered navigation serializer type must match route type")
        }

        let type_name = core::any::type_name::<R>();
        let type_id = TypeId::of::<R>();
        let entry = RestorationEntry {
            type_id,
            serialize: serialize_route::<R>,
            deserialize: deserialize_route::<R>,
        };
        let mut registry = self.restoration.borrow_mut();
        match (
            registry.routes_by_name.get(type_name),
            registry.routes_by_id.get(&type_id),
        ) {
            (None, None) => {
                registry.routes_by_name.insert(type_name, entry);
                registry.routes_by_id.insert(type_id, entry);
            }
            (Some(by_name), Some(by_id)) => {
                assert_eq!(
                    by_name.type_id, type_id,
                    "navigation restoration type name `{type_name}` identifies two route types"
                );
                assert_eq!(
                    by_id.type_id, type_id,
                    "navigation restoration registry is inconsistent for `{type_name}`"
                );
            }
            _ => panic!("navigation restoration registry is inconsistent for `{type_name}`"),
        }
    }

    /// Returns a serde seed that atomically restores this heterogeneous path.
    #[cfg(feature = "serde")]
    #[must_use]
    pub fn restoration(&self) -> impl for<'de> serde::de::DeserializeSeed<'de, Value = ()> + '_ {
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
        self.inner.len()
    }

    /// Removes exactly `count` routes from the top.
    ///
    /// # Panics
    ///
    /// Panics when `count` exceeds the current path length.
    pub fn pop_n(&self, count: usize) {
        let routes = self.inner.snapshot();
        let length = routes.len();
        assert!(
            count <= length,
            "cannot pop {count} routes from a navigation path of length {length}"
        );
        self.truncate(length - count);
    }

    /// Truncates the path to at most `length` routes.
    pub fn truncate(&self, length: usize) {
        let mut routes = self.inner.snapshot();
        routes.truncate(length);
        let _ = self.inner.replace(routes);
    }

    /// Removes every route.
    pub fn clear(&self) {
        let _ = self.inner.replace(Vec::new());
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

    pub(crate) fn watch(
        &self,
        watcher: impl Fn(Vec<R>) + 'static,
    ) -> impl nami::watcher::WatcherGuard {
        self.inner
            .watch(.., move |context| watcher(context.into_value().to_vec()))
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
            inner: List::from(routes),
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
                let _ = self.path.inner.replace(routes);
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

        impl<'de> serde::de::Visitor<'de> for RouteVisitor<'_> {
            type Value = ErasedNavigationRoute;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a navigation route type name followed by its serde payload")
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                use serde::de::Error as _;

                let type_name: String = sequence
                    .next_element()?
                    .ok_or_else(|| A::Error::custom("navigation route is missing its type name"))?;
                let factory = self
                    .registry
                    .borrow()
                    .routes_by_name
                    .get(type_name.as_str())
                    .map(|entry| entry.deserialize)
                    .ok_or_else(|| {
                        A::Error::custom(alloc::format!(
                            "navigation route type `{type_name}` is not registered"
                        ))
                    })?;
                sequence
                    .next_element_seed(RoutePayloadSeed {
                        factory,
                        restoration: Rc::clone(self.registry),
                    })?
                    .ok_or_else(|| {
                        A::Error::custom(alloc::format!(
                            "navigation route `{type_name}` is missing its payload"
                        ))
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
impl<R> serde::Serialize for NavigationPath<R>
where
    R: Clone + serde::Serialize + 'static,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.snapshot().serialize(serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de, R> serde::Deserialize<'de> for NavigationPath<R>
where
    R: Clone + PartialEq + serde::Deserialize<'de> + 'static,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Vec::<R>::deserialize(deserializer).map(Self::from)
    }
}

#[cfg(test)]
mod tests {
    use alloc::{rc::Rc, string::String, vec};

    use super::NavigationPath;

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

        assert_eq!(notifications.get(), 2, "watch includes its initial value");
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

        assert_eq!(notifications.get(), 2, "watch includes its initial value");
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
        let encoded = serde_json::to_string(&path).expect("typed path must serialize");
        let restored: NavigationPath<u32> =
            serde_json::from_str(&encoded).expect("typed path must deserialize");

        assert_eq!(restored.snapshot(), vec![1, 2, 3]);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn heterogeneous_path_restores_with_automatic_type_names() {
        use serde::de::DeserializeSeed as _;

        let path = NavigationPath::heterogeneous();
        path.register_restoration::<u32>();
        path.register_restoration::<String>();
        path.push(7_u32);
        path.push(String::from("settings"));
        let encoded = serde_json::to_string(&path).expect("heterogeneous path must serialize");

        let restored = NavigationPath::heterogeneous();
        restored.register_restoration::<u32>();
        restored.register_restoration::<String>();
        let mut deserializer = serde_json::Deserializer::from_str(&encoded);
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
    fn heterogeneous_restoration_registration_is_idempotent() {
        let path = NavigationPath::heterogeneous();

        path.register_restoration::<u32>();
        path.register_restoration::<u32>();
        path.push(7_u32);

        assert_eq!(
            serde_json::to_string(&path).expect("registered route must serialize"),
            alloc::format!("[[\"{}\",7]]", core::any::type_name::<u32>())
        );
    }
}
