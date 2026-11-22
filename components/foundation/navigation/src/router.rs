//! URL routing kept separate from path serialization and restoration.

use alloc::{rc::Rc, vec::Vec};

use waterui_core::contract_violation;
use waterui_url::Url;

use crate::NavigationPath;

type RouteResolver<R> = Rc<dyn Fn(&Url) -> Option<Vec<R>>>;

/// Maps external URLs to complete strongly typed navigation paths.
///
/// URL syntax is application-facing and intentionally independent from serde
/// restoration tags. A successful resolution replaces the complete path in one
/// atomic collection update.
#[must_use]
pub struct NavigationRouter<R: 'static> {
    path: NavigationPath<R>,
    resolvers: Vec<RouteResolver<R>>,
}

impl<R: 'static> core::fmt::Debug for NavigationRouter<R> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("NavigationRouter")
            .field("resolver_count", &self.resolvers.len())
            .finish_non_exhaustive()
    }
}

impl<R> NavigationRouter<R>
where
    R: Clone + PartialEq + 'static,
{
    /// Creates a router that writes successful resolutions into `path`.
    pub fn new(path: NavigationPath<R>) -> Self {
        Self {
            path,
            resolvers: Vec::new(),
        }
    }

    /// Adds an application-defined URL resolver.
    ///
    /// Return `None` when the resolver does not own the URL. Return the complete
    /// route sequence when it does.
    pub fn route(mut self, resolver: impl Fn(&Url) -> Option<Vec<R>> + 'static) -> Self {
        self.resolvers.push(Rc::new(resolver));
        self
    }

    /// Resolves a URL and atomically projects it into the path.
    ///
    /// Returns `false` when no registered resolver matches.
    ///
    /// # Contract violation
    ///
    /// More than one resolver claiming the same URL is reported: a debug
    /// build panics; a release build logs it and uses the first match.
    #[must_use]
    pub fn open(&self, url: &Url) -> bool {
        let mut resolved = self.resolvers.iter().filter_map(|resolver| resolver(url));
        let Some(routes) = resolved.next() else {
            return false;
        };
        if resolved.next().is_some() {
            contract_violation!(
                "more than one navigation URL resolver matched `{}`; the first match is used",
                url.as_str()
            );
        }
        self.path.replace(routes);
        true
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;

    #[test]
    fn successful_url_resolution_replaces_the_path_atomically() {
        let path = NavigationPath::from(vec![1_u32]);
        let router = NavigationRouter::new(path.clone())
            .route(|url| (url.path() == "/settings/profile").then_some(vec![2, 3]));

        assert!(router.open(&Url::new("waterui://app/settings/profile")));
        assert_eq!(path.snapshot(), vec![2, 3]);
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "more than one navigation URL resolver matched")
    )]
    fn ambiguous_url_resolution_uses_the_first_match() {
        let path = NavigationPath::from(vec![1_u32]);
        let router = NavigationRouter::new(path.clone())
            .route(|_| Some(vec![2]))
            .route(|_| Some(vec![3]));

        // Release outcome: the first resolver wins.
        assert!(router.open(&Url::new("waterui://app/settings")));
        assert_eq!(path.snapshot(), vec![2]);
    }

    #[test]
    fn unmatched_url_preserves_the_path() {
        let path = NavigationPath::from(vec![1_u32]);
        let router = NavigationRouter::new(path.clone())
            .route(|url| (url.path() == "/settings").then_some(vec![2]));

        assert!(!router.open(&Url::new("waterui://app/library")));
        assert_eq!(path.snapshot(), vec![1]);
    }
}
