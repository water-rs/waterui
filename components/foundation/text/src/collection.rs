//! The one font collection an application shapes and typesets with.
//!
//! A [`parley::FontContext`] is a database of every face the process can reach.
//! Building one discovers the system's fonts, which is neither cheap nor
//! something a view should be doing: a page with a dozen formulas used to run a
//! dozen independent font enumerations and hold a dozen collections alive,
//! because each component built its own.
//!
//! So the collection is a resource the *host* owns, exactly like the theme's
//! font slots: whoever drives the event loop installs one into the root
//! [`Environment`] at startup, and every component that has to shape or
//! typeset something reads it back out with [`FontCollection::from_env`]. A
//! component that finds none installed panics and names the host's
//! responsibility — it does not quietly build a second collection, because
//! that is the defect this module exists to remove.
//!
//! # Ownership
//!
//! `WaterUI` is a main-thread UI framework and `parley`'s contexts are not
//! `Sync`, so the collection is shared as single-threaded interior mutability
//! and handed out mutably for the duration of one call. A renderer that shapes
//! text across worker threads keeps its own thread-safe arrangement and
//! installs a collection carrying the same registered faces.

use alloc::rc::Rc;
use core::cell::RefCell;
use core::fmt::{self, Debug, Formatter};

use parley::FontContext;
use waterui_core::Environment;

/// The message a component reports when the host installed no collection.
///
/// It names the host rather than the component, because a missing collection is
/// never something the view could have done differently.
const NOT_INSTALLED: &str = "no font collection is installed in the environment: \
     the host must install exactly one at startup with `FontCollection::install`, \
     before it hands the environment to the application";

/// The application's shared font collection.
///
/// Cloning it shares the one collection rather than copying it, so a component
/// may keep a handle for as long as it draws.
#[derive(Clone)]
pub struct FontCollection(Rc<RefCell<FontContext>>);

impl Debug for FontCollection {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        // `parley::FontContext` is not `Debug`, and its contents — every face
        // the process can reach — would not be readable if it were. The
        // identity is the useful part: it is what tells two handles apart.
        formatter
            .debug_struct("FontCollection")
            .field("identity", &self.identity())
            .finish()
    }
}

impl FontCollection {
    /// Shares `fonts` as the application's collection.
    #[must_use]
    pub fn new(fonts: FontContext) -> Self {
        Self(Rc::new(RefCell::new(fonts)))
    }

    /// The system's fonts, discovered now.
    ///
    /// This is the collection a host with no font stack of its own installs —
    /// the native Apple, Android and GTK backends draw text with the platform's
    /// own text engine and own no `parley` collection, so the one a self-drawn
    /// component reads is discovered here, once per application.
    #[cfg(feature = "system-fonts")]
    #[must_use]
    pub fn system() -> Self {
        Self::new(FontContext::new())
    }

    /// Installs this collection as the one the application shapes with.
    ///
    /// Called by the host on the root environment, before the application's own
    /// views are built. Installing twice replaces the first, which is a host
    /// bug rather than a supported way to swap collections mid-flight.
    pub fn install(self, env: &mut Environment) {
        env.insert(self);
    }

    /// The collection installed in `env`.
    ///
    /// # Panics
    ///
    /// Panics when the host installed none. There is deliberately no fallback:
    /// building one here would be the per-component enumeration this type
    /// exists to replace, and it would shape against the system's faces while
    /// the rest of the window shapes against the host's registered ones.
    #[must_use]
    pub fn from_env(env: &Environment) -> Self {
        env.get::<Self>().cloned().expect(NOT_INSTALLED)
    }

    /// Runs `use_fonts` against the collection.
    ///
    /// # Panics
    ///
    /// Panics when called from inside another `use_fonts`: the collection is
    /// handed out exclusively, and shaping that re-enters shaping is a bug in
    /// the caller.
    pub fn use_fonts<R>(&self, use_fonts: impl FnOnce(&mut FontContext) -> R) -> R {
        use_fonts(&mut self.0.borrow_mut())
    }

    /// A stable identity for the collection behind this handle.
    ///
    /// Two handles onto the same collection report the same value; handles onto
    /// two different collections never do.
    #[must_use]
    pub fn identity(&self) -> usize {
        Rc::as_ptr(&self.0) as usize
    }
}

#[cfg(test)]
mod tests {
    use waterui_core::Environment;

    use super::{FontCollection, NOT_INSTALLED};

    /// An empty collection: these tests are about sharing, not about faces, and
    /// enumerating the runner's fonts would make them slow and host-dependent.
    fn empty() -> FontCollection {
        FontCollection::new(parley::FontContext {
            collection: parley::fontique::Collection::new(parley::fontique::CollectionOptions {
                system_fonts: false,
                ..parley::fontique::CollectionOptions::default()
            }),
            source_cache: parley::fontique::SourceCache::default(),
        })
    }

    /// Two components reading the environment get the same collection, not two
    /// copies of one. This is the whole point of the type: the defect it
    /// replaces was every component holding a collection of its own.
    #[test]
    fn two_components_share_one_collection() {
        let mut env = Environment::new();
        empty().install(&mut env);

        let first = FontCollection::from_env(&env);
        let second = FontCollection::from_env(&env);

        assert_eq!(
            first.identity(),
            second.identity(),
            "every component must read back the one collection the host installed"
        );
    }

    /// A child scope sees the collection its parent installed.
    #[test]
    fn an_extended_environment_keeps_the_collection() {
        let mut env = Environment::new();
        let installed = empty();
        let identity = installed.identity();
        installed.install(&mut env);

        let child = env.extending(7_u32);

        assert_eq!(FontCollection::from_env(&child).identity(), identity);
    }

    /// Two collections are two collections. Without this, the sharing test
    /// above would pass against a type that handed out a fresh one every time.
    #[test]
    fn separate_collections_are_told_apart() {
        assert_ne!(empty().identity(), empty().identity());
    }

    /// Nothing installed is a host bug, and it is reported as one rather than
    /// papered over with a collection built on the spot.
    #[test]
    #[should_panic(expected = "no font collection is installed in the environment")]
    fn reading_an_uninstalled_collection_names_the_host() {
        let _ = FontCollection::from_env(&Environment::new());
    }

    /// The mutable loan is what a shaping call takes, and it is the same
    /// collection every time.
    #[test]
    fn the_loan_reaches_the_installed_collection() {
        let collection = empty();
        let first = collection.use_fonts(|fonts| fonts.collection.family_names().count());
        let second = collection.use_fonts(|fonts| fonts.collection.family_names().count());

        assert_eq!(first, second);
    }

    /// The message a component panics with has to say whose job the collection
    /// is, since the view that noticed could never have supplied it.
    #[test]
    fn the_panic_names_the_hosts_responsibility() {
        assert!(NOT_INSTALLED.contains("the host must install"));
    }

    /// A host with no font stack of its own gets one from the installer.
    #[test]
    fn the_installer_gives_a_bare_host_a_collection() {
        let mut env = Environment::new();
        crate::install_system_font_collection(&mut env);

        let installed = FontCollection::from_env(&env).identity();
        assert_eq!(FontCollection::from_env(&env).identity(), installed);
    }

    /// A host that owns its own font stack installs that one, and the installer
    /// leaves it alone rather than enumerating the system's fonts over the top.
    #[test]
    fn the_installer_yields_to_a_collection_the_host_already_owns() {
        let mut env = Environment::new();
        let owned = empty();
        let identity = owned.identity();
        owned.install(&mut env);

        crate::install_system_font_collection(&mut env);

        assert_eq!(FontCollection::from_env(&env).identity(), identity);
    }
}
