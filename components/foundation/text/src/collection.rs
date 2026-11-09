//! The one font collection an application draws text with.
//!
//! The font database behind it — every face the process can reach — is
//! expensive to build and to keep: a page with a dozen formulas used to run a
//! dozen independent font enumerations and hold a dozen collections alive,
//! because each component built its own.
//!
//! So the collection is a resource the *host* owns, exactly like the theme's
//! font slots: whoever drives the event loop installs one into the root
//! [`Environment`] at startup, and every component that draws text itself
//! reads it back out with [`FontCollection::from_env`]. A component that
//! finds none installed panics and names the host's responsibility — it does
//! not quietly build a second collection, because that is the defect this
//! module exists to remove.
//!
//! What a component reads is semantic: the faces registered under a family
//! name, each a [`FontFace`] carrying the face's bytes and its index in the
//! font file. Which engine discovered those faces is the host's business —
//! no engine type appears in this API. A self-drawn text engine that shapes
//! with `parley` enables the crate's `engine` feature and reaches the
//! context itself through `FontCollection::from_parley` and
//! `FontCollection::with_parley`.
//!
//! # Ownership
//!
//! `WaterUI` is a main-thread UI framework and the font database behind the
//! collection is not `Sync`, so the collection is shared as single-threaded
//! interior mutability and handed out mutably for the duration of one call.
//! A renderer that shapes text across worker threads keeps its own
//! thread-safe arrangement and installs a collection carrying the same
//! registered faces.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::fmt::{self, Debug, Formatter};

use parley::FontContext;
use parley::fontique::Blob;
#[cfg(any(feature = "engine", feature = "system-fonts"))]
use parley::fontique::{Collection, FamilyId, Script, ScriptExt};
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

/// One face of a family, as the collection registered it.
///
/// Cloning shares the face's bytes with the collection rather than copying
/// them, so a component may keep or hand on a face for as long as it draws.
#[derive(Clone, Debug)]
pub struct FontFace {
    bytes: FontBytes,
    index: u32,
}

impl FontFace {
    /// The face's bytes.
    #[must_use]
    pub fn bytes(&self) -> FontBytes {
        self.bytes.clone()
    }

    /// The face's index inside its font file — `0` unless the file packs
    /// several faces, as a `.ttc` does.
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }
}

/// The bytes of one face, shared with the collection rather than copied.
///
/// The data is reference-counted, so cloning is another handle to the same
/// bytes, and the handle is `Send + Sync`: a component may move it to a
/// worker that rasterizes or subsets the face.
#[derive(Clone)]
pub struct FontBytes(Blob<u8>);

impl Debug for FontBytes {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        // A face's bytes are a font file: printing them would flood the log,
        // so the length is the readable part.
        formatter
            .debug_struct("FontBytes")
            .field("length", &self.0.as_ref().len())
            .finish()
    }
}

impl AsRef<[u8]> for FontBytes {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

/// The faces, in order, that can answer a Common-script symbol on this
/// platform.
///
/// Symbols, arrows and dingbats like `U+2713 CHECK MARK` report the `Zyyy`
/// ("Common") script: they have no script of their own, so one resolves
/// through the fallback list of whatever script its run took. No platform's
/// script-keyed fallback reaches a face that maps them — on macOS the Latin
/// fallback resolves to Helvetica, which has no such glyph — so they drew
/// `.notdef` (water-rs/waterui#1370). Entries are family names; a name that
/// is not installed is skipped, and the platform's last-resort face stays
/// last.
#[cfg(all(
    target_vendor = "apple",
    any(feature = "engine", feature = "system-fonts")
))]
const SYMBOL_FALLBACK_FAMILIES: &[&str] = &[
    "Apple Symbols",
    "Zapf Dingbats",
    "Arial Unicode MS",
    ".LastResort",
];
/// See [`SYMBOL_FALLBACK_FAMILIES`].
#[cfg(all(
    target_os = "windows",
    any(feature = "engine", feature = "system-fonts")
))]
const SYMBOL_FALLBACK_FAMILIES: &[&str] = &["Segoe UI Symbol"];
/// See [`SYMBOL_FALLBACK_FAMILIES`].
#[cfg(all(
    not(any(target_vendor = "apple", target_os = "windows")),
    any(feature = "engine", feature = "system-fonts")
))]
const SYMBOL_FALLBACK_FAMILIES: &[&str] = &["DejaVu Sans", "Noto Sans Symbols 2"];

/// Appends the platform's symbol faces to the tail of every script-keyed
/// fallback list.
///
/// fontique keys a fallback query to the script of the run that needed it,
/// and `Script::sample` has no `Zyyy` entry, so a Common-script symbol only
/// ever reaches whatever its run's script resolves — a script-keyed list —
/// then the Han list fontique appends to every query for common
/// punctuation. Giving every script's list the same tail covers the symbol
/// no matter which script the run reports, and the `Zyyy` key itself for a
/// consumer that queries it directly.
///
/// Each script's own platform fallback is resolved first, exactly as an
/// unconfigured collection resolves it lazily, so the symbol faces sit
/// behind the script's real coverage and ahead of the last-resort face
/// rather than replacing either.
#[cfg(any(feature = "engine", feature = "system-fonts"))]
fn configure_fallbacks(collection: &mut Collection) {
    let families: Vec<FamilyId> = SYMBOL_FALLBACK_FAMILIES
        .iter()
        .filter_map(|name| collection.family_id(name))
        .collect();
    if families.is_empty() {
        return;
    }
    for &(script, _) in Script::all_samples() {
        // Reading the key once lets the platform install its own fallback
        // for the script before the symbols are appended behind it: writing
        // an entry first would make fontique skip that lookup entirely.
        let _ = collection.fallback_families(script);
        collection.append_fallbacks(script, families.iter().copied());
    }
    collection.append_fallbacks(Script::from_bytes(*b"Zyyy"), families.iter().copied());
}

impl FontCollection {
    /// Shares a `parley` font context as the application's collection.
    ///
    /// The platform's symbol faces are appended to the collection's
    /// script-keyed fallback lists before it is shared, so a Common-script
    /// symbol resolves to a face that maps it on every host that installs
    /// its fonts through this type.
    ///
    /// This is the engine seam: a host whose text engine is `parley` — the
    /// self-drawn backend — has a [`FontContext`] to share, and only it
    /// enables the `engine` feature this method sits behind.
    #[cfg(feature = "engine")]
    #[must_use]
    pub fn from_parley(fonts: FontContext) -> Self {
        Self::from_font_context(fonts)
    }

    /// The system's fonts, discovered now.
    ///
    /// This is the collection a host with no font stack of its own installs —
    /// the native Apple, Android and GTK backends draw text with the platform's
    /// own text engine and own no font collection of their own, so the one a
    /// self-drawn component reads is discovered here, once per application.
    #[cfg(feature = "system-fonts")]
    #[must_use]
    pub fn system() -> Self {
        Self::from_font_context(FontContext::new())
    }

    /// Shares `fonts` as the application's collection: the one constructor,
    /// behind whichever feature has a [`FontContext`] to offer — `engine`
    /// receives the host's own, `system-fonts` discovers the platform's.
    ///
    /// The platform's symbol faces are appended to the collection's
    /// script-keyed fallback lists before it is shared, so a Common-script
    /// symbol resolves to a face that maps it on every host that installs
    /// its fonts through this type.
    #[cfg(any(feature = "engine", feature = "system-fonts"))]
    fn from_font_context(fonts: FontContext) -> Self {
        let mut fonts = fonts;
        configure_fallbacks(&mut fonts.collection);
        Self(Rc::new(RefCell::new(fonts)))
    }

    /// Installs this collection as the one the application draws with.
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

    /// The faces registered under `family`, in the collection's order.
    ///
    /// Each [`FontFace`] shares its bytes with the collection and carries the
    /// face's index in its font file — the semantic read a self-drawing
    /// component takes, with no engine type in it. A family the collection
    /// does not know answers an empty `Vec`; a face whose source cannot be
    /// loaded is omitted and reported.
    ///
    /// # Panics
    ///
    /// Panics when called while the collection is mutably borrowed — inside a
    /// `with_parley` call or another `faces` call on the same collection: the
    /// collection is handed out exclusively, and drawing that re-enters the
    /// collection is a bug in the caller.
    #[must_use]
    pub fn faces(&self, family: &str) -> Vec<FontFace> {
        let mut fonts = self.0.borrow_mut();
        let Some(info) = fonts.collection.family_by_name(family) else {
            return Vec::new();
        };
        info.fonts()
            .iter()
            .enumerate()
            .filter_map(|(position, font)| {
                let bytes = fonts.source_cache.get(font.source());
                if bytes.is_none() {
                    tracing::warn!(
                        family,
                        position,
                        "omitting a font face whose source could not be loaded"
                    );
                }
                bytes.map(|bytes| FontFace {
                    bytes: FontBytes(bytes),
                    index: font.index(),
                })
            })
            .collect()
    }

    /// Runs `with_parley` against the collection's `parley` context.
    ///
    /// This is the engine seam: only a self-drawn text engine whose shaper is
    /// `parley` needs the context itself, which is why the method sits behind
    /// the `engine` feature.
    ///
    /// # Panics
    ///
    /// Panics when called from inside another `with_parley`: the collection is
    /// handed out exclusively, and shaping that re-enters shaping is a bug in
    /// the caller.
    #[cfg(feature = "engine")]
    pub fn with_parley<R>(&self, with_parley: impl FnOnce(&mut FontContext) -> R) -> R {
        with_parley(&mut self.0.borrow_mut())
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
    use alloc::{borrow::ToOwned, rc::Rc};
    use core::cell::RefCell;

    use waterui_core::Environment;

    use super::{FontBytes, FontCollection, NOT_INSTALLED};

    /// An empty collection: these tests are about sharing, not about faces, and
    /// enumerating the runner's fonts would make them slow and host-dependent.
    /// Built straight from the handle's field, so the tests that need no faces
    /// compile under `font-collection` alone, without either constructor.
    fn empty() -> FontCollection {
        FontCollection(Rc::new(RefCell::new(parley::FontContext {
            collection: parley::fontique::Collection::new(parley::fontique::CollectionOptions {
                system_fonts: false,
                ..parley::fontique::CollectionOptions::default()
            }),
            source_cache: parley::fontique::SourceCache::default(),
        })))
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
    #[cfg(feature = "engine")]
    #[test]
    fn the_loan_reaches_the_installed_collection() {
        let collection = empty();
        let first = collection.with_parley(|fonts| fonts.collection.family_names().count());
        let second = collection.with_parley(|fonts| fonts.collection.family_names().count());

        assert_eq!(first, second);
    }

    /// The faces a family answers carry its bytes and its index — the whole
    /// of the semantic read a self-drawing component takes. The family is
    /// the first the system enumerates: the test cannot name one a runner
    /// may not have installed.
    #[cfg(feature = "system-fonts")]
    #[test]
    fn a_family_answers_its_registered_faces() {
        let collection = FontCollection::system();
        let family = {
            let mut fonts = collection.0.borrow_mut();
            fonts
                .collection
                .family_names()
                .next()
                .expect("the system collection enumerates no families")
                .to_owned()
        };

        let faces = collection.faces(&family);

        assert!(
            !faces.is_empty(),
            "an installed family must answer at least one face"
        );
        for face in &faces {
            assert!(
                !face.bytes().as_ref().is_empty(),
                "a registered face carries its font's bytes"
            );
        }
    }

    /// A family the collection does not know is not an error: it answers an
    /// empty `Vec`.
    #[test]
    fn an_unknown_family_answers_no_faces() {
        assert!(empty().faces("WaterUI test: no such family").is_empty());
    }

    /// `FontBytes` crosses threads: a component hands a face's bytes to a
    /// worker that rasterizes or subsets it. The assertion is at compile
    /// time — the test fails to build, not to run, if the bound ever breaks.
    #[test]
    fn font_bytes_moves_between_threads() {
        fn assert_send_and_sync<T: Send + Sync>() {}
        assert_send_and_sync::<FontBytes>();
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

    /// `U+2713 CHECK MARK` reports the `Zyyy` ("Common") script, so it reaches
    /// fonts only through the fallback tail of whatever script its run took —
    /// `Latn` for a bare `text("✓")`, which is the key this query uses. The
    /// collection must answer it with a face that maps the codepoint, and not
    /// the last-resort placeholder face, or it draws as tofu
    /// (water-rs/waterui#1370).
    #[cfg(all(feature = "system-fonts", feature = "engine"))]
    #[test]
    fn a_common_script_symbol_resolves_to_a_face_that_maps_it() {
        use parley::fontique::{QueryStatus, Script};

        let collection = FontCollection::system();
        collection.with_parley(|fonts| {
            let mut covered = None;
            {
                let mut query = fonts.collection.query(&mut fonts.source_cache);
                query.set_fallbacks(Script::from_bytes(*b"Latn"));
                query.matches_with(|font| {
                    if font.charmap().is_some_and(|charmap| {
                        charmap.map('\u{2713}').is_some_and(|glyph| glyph != 0)
                    }) {
                        covered = Some(font.family.0);
                        QueryStatus::Stop
                    } else {
                        QueryStatus::Continue
                    }
                });
            }
            let covered = covered.expect("no face in the collection maps U+2713 CHECK MARK");
            let name = fonts.collection.family_name(covered).unwrap_or_default();
            assert_ne!(
                name, ".LastResort",
                "U+2713 resolved to the last-resort placeholder face: no real face answered it"
            );
        });
    }
}
