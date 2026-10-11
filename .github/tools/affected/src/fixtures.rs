//! `affected nextest-fixtures [--root <dir>]`: checks the nextest setup
//! script rules in `<root>/.config/nextest.toml` against the package graph.
//!
//! The `test-fonts` setup script installs the font families that the
//! styled suites resolve through system font discovery. Those suites are
//! the test binaries that link `waterui-testing`, so the packages the
//! script must serve are a fact of the package graph: every workspace
//! member whose test build reaches `waterui-testing`. nextest's filterset
//! language cannot state that set exactly — `rdeps(waterui-testing)`
//! follows dev-dependency edges at every hop, so it also takes in each
//! package that merely depends on a crate whose own tests use
//! `waterui-testing`, which is nearly the whole workspace. The rule
//! therefore names its packages, and this check fails whenever the names
//! drift from the graph, listing the packages missing from the rule and
//! the packages it serves for no reason.
//!
//! The filter is evaluated with nextest's own filterset engine, once per
//! test binary of every workspace member, so the check sees exactly what
//! nextest selects whatever shape the expression takes. Every profile is
//! checked with the rules nextest applies to it: its own, followed by the
//! default profile's.

use std::collections::{BTreeMap, BTreeSet};

use camino::Utf8Path;
use guppy::graph::{
    cargo::BuildPlatform, BuildTargetId, DependencyDirection, PackageGraph, PackageMetadata,
};
use nextest_filtering::{
    BinaryQuery, EvalContext, Filterset, FiltersetKind, KnownGroups, ParseContext,
};
use nextest_metadata::{RustBinaryId, RustTestBinaryKind};
use serde::Deserialize;

/// The setup script whose package set the graph determines.
const TEST_FONTS: &str = "test-fonts";

/// The crate whose linkage defines a font consumer.
const TESTING_CRATE: &str = "waterui-testing";

#[derive(Deserialize)]
struct NextestConfig {
    #[serde(default)]
    profile: BTreeMap<String, Profile>,
}

#[derive(Deserialize)]
struct Profile {
    #[serde(default)]
    scripts: Vec<ScriptRule>,
}

/// One `[[profile.<name>.scripts]]` entry. Wrapper-script keys are not
/// modelled: a rule that names no setup script is not this check's
/// concern.
#[derive(Deserialize)]
struct ScriptRule {
    filter: Option<String>,
    platform: Option<toml::Value>,
    setup: Option<Setup>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Setup {
    One(String),
    Many(Vec<String>),
}

impl Setup {
    fn names(&self, name: &str) -> bool {
        match self {
            Self::One(one) => one == name,
            Self::Many(many) => many.iter().any(|entry| entry == name),
        }
    }
}

/// The workspace members whose test binaries link `waterui-testing`: the
/// member itself and its dev-dependencies seed a closure over normal and
/// build links, because cargo does not make dev-dependencies transitive.
pub fn font_consumers(graph: &PackageGraph) -> BTreeSet<String> {
    let members: Vec<PackageMetadata<'_>> = graph
        .resolve_workspace()
        .packages(DependencyDirection::Forward)
        .collect();
    let testing = members
        .iter()
        .find(|package| package.name() == TESTING_CRATE)
        .unwrap_or_else(|| panic!("{TESTING_CRATE} is not a workspace member"))
        .id();
    let mut consumers = BTreeSet::new();
    for member in &members {
        let mut stack: Vec<_> = member
            .direct_links()
            .filter(|link| link.dev().is_present())
            .map(|link| link.to().id())
            .chain([member.id()])
            .collect();
        let mut reached: BTreeSet<_> = stack.iter().copied().collect();
        while let Some(id) = stack.pop() {
            let package = graph
                .metadata(id)
                .expect("a reached package is in the graph");
            for link in package.direct_links() {
                if (link.normal().is_present() || link.build().is_present())
                    && reached.insert(link.to().id())
                {
                    stack.push(link.to().id());
                }
            }
        }
        if reached.contains(testing) {
            consumers.insert(member.name().to_string());
        }
    }
    consumers
}

/// The nextest binary kind of a build target, as nextest derives it;
/// build scripts carry no tests.
fn binary_kind(
    package: &PackageMetadata<'_>,
    id: &BuildTargetId<'_>,
) -> Option<RustTestBinaryKind> {
    match id {
        BuildTargetId::Library if package.is_proc_macro() => Some(RustTestBinaryKind::PROC_MACRO),
        BuildTargetId::Library => Some(RustTestBinaryKind::LIB),
        BuildTargetId::Binary(_) => Some(RustTestBinaryKind::BIN),
        BuildTargetId::Test(_) => Some(RustTestBinaryKind::TEST),
        BuildTargetId::Benchmark(_) => Some(RustTestBinaryKind::BENCH),
        BuildTargetId::Example(_) => Some(RustTestBinaryKind::EXAMPLE),
        BuildTargetId::BuildScript => None,
        other => panic!("unknown build target kind: {other:?}"),
    }
}

/// The workspace members the given filters select, evaluated per test
/// binary. A filter that needs test names to decide, or that selects only
/// some of a package's binaries, does not describe a package set and is
/// rejected.
fn selected_packages(graph: &PackageGraph, filters: &[&str]) -> BTreeSet<String> {
    let cx = ParseContext::new(graph);
    let parse = |input: &str, kind| {
        Filterset::parse(input.to_owned(), &cx, kind, &KnownGroups::Unavailable)
            .unwrap_or_else(|errors| panic!("filter `{input}` did not parse: {errors:?}"))
    };
    let default_filter = parse("all()", FiltersetKind::DefaultFilter).compiled;
    let eval = EvalContext {
        default_filter: &default_filter,
    };
    let filters: Vec<Filterset> = filters
        .iter()
        .map(|filter| parse(filter, FiltersetKind::OverrideFilter))
        .collect();

    let mut selected = BTreeSet::new();
    for package in graph
        .resolve_workspace()
        .packages(DependencyDirection::Forward)
    {
        let mut verdicts = BTreeSet::new();
        for target in package.build_targets() {
            let Some(kind) = binary_kind(&package, &target.id()) else {
                continue;
            };
            let binary_id = RustBinaryId::from_parts(package.name(), &kind, target.name());
            let query = BinaryQuery {
                package_id: package.id(),
                binary_id: &binary_id,
                binary_name: target.name(),
                kind: &kind,
                platform: BuildPlatform::Target,
            };
            let mut matched = false;
            for filter in &filters {
                matched |= filter.matches_binary(&query, &eval).unwrap_or_else(|| {
                    panic!(
                        "filter `{}` needs test names to decide binary `{binary_id}`; \
                         a {TEST_FONTS} rule must select whole packages",
                        filter.input
                    )
                });
            }
            verdicts.insert(matched);
        }
        match verdicts.len() {
            0 => {}
            1 => {
                if verdicts.contains(&true) {
                    selected.insert(package.name().to_string());
                }
            }
            _ => panic!(
                "the {TEST_FONTS} rules select only some of {}'s test binaries; \
                 a {TEST_FONTS} rule must select whole packages",
                package.name()
            ),
        }
    }
    selected
}

/// The filters of a profile's own rules that run the `test-fonts` script.
fn own_test_font_filters(profile: &Profile) -> Vec<&str> {
    profile
        .scripts
        .iter()
        .filter(|rule| {
            rule.setup
                .as_ref()
                .is_some_and(|setup| setup.names(TEST_FONTS))
        })
        .map(|rule| {
            assert!(
                rule.platform.is_none(),
                "a platform-scoped {TEST_FONTS} rule is not a package set; \
                 the fonts serve every platform the styled suites run on"
            );
            rule.filter
                .as_deref()
                .unwrap_or_else(|| panic!("a {TEST_FONTS} rule has no filter"))
        })
        .collect()
}

/// The `test-fonts` filters nextest applies under each profile: the
/// profile's own rules, then the default profile's.
fn test_font_filters(config: &NextestConfig) -> BTreeMap<&str, Vec<&str>> {
    let default = config
        .profile
        .get("default")
        .map(own_test_font_filters)
        .unwrap_or_default();
    let mut filters: BTreeMap<&str, Vec<&str>> = config
        .profile
        .iter()
        .map(|(name, profile)| {
            let mut filters = own_test_font_filters(profile);
            if name != "default" {
                filters.extend(default.iter().copied());
            }
            (name.as_str(), filters)
        })
        .collect();
    filters.entry("default").or_default();
    filters
}

/// Runs the check against the workspace at `root`; returns whether every
/// profile's `test-fonts` rules select exactly the graph's consumers.
pub fn check(root: &Utf8Path, graph: &PackageGraph) -> bool {
    let path = root.join(".config/nextest.toml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {path}: {error}"));
    let config: NextestConfig =
        toml::from_str(&text).unwrap_or_else(|error| panic!("{path} did not parse: {error}"));

    let consumers = font_consumers(graph);
    let mut consistent = true;
    for (profile, filters) in test_font_filters(&config) {
        let selected = selected_packages(graph, &filters);
        let missing: Vec<_> = consumers.difference(&selected).collect();
        let extra: Vec<_> = selected.difference(&consumers).collect();
        if missing.is_empty() && extra.is_empty() {
            continue;
        }
        consistent = false;
        eprintln!(
            "{path}: under profile `{profile}`, the {TEST_FONTS} setup script \
             does not serve exactly the packages whose tests link {TESTING_CRATE}"
        );
        if !missing.is_empty() {
            eprintln!("  missing (their tests link {TESTING_CRATE}): {missing:?}");
        }
        if !extra.is_empty() {
            eprintln!("  extra (their tests do not link {TESTING_CRATE}): {extra:?}");
        }
    }
    consistent
}
