//! Guards `[features]` symmetry across `waterui`, `waterui-internal`, and `waterui-dylib`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

fn manifest_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Strips a TOML `#` comment outside double-quoted string literals.
fn strip_toml_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '#' if !in_string => return &line[..idx],
            _ => {}
        }
    }
    line
}

/// Collects double-quoted string literals from `fragment`, returning whether `]` closed the array.
fn collect_array_items(fragment: &str, items: &mut Vec<String>) -> bool {
    let mut chars = fragment.char_indices().peekable();
    while let Some((_, ch)) = chars.next() {
        match ch {
            '"' => {
                let start = chars.peek().map_or(fragment.len(), |(idx, _)| *idx);
                let mut end = start;
                let mut escaped = false;
                for (idx, inner) in &mut chars {
                    if escaped {
                        escaped = false;
                        continue;
                    }
                    if inner == '\\' {
                        escaped = true;
                    } else if inner == '"' {
                        end = idx;
                        break;
                    }
                }
                items.push(fragment[start..end].to_owned());
            }
            ']' => return true,
            _ => {}
        }
    }
    false
}

/// Parses the `[features]` table from a `Cargo.toml` manifest into ordered feature lists.
fn parse_features_table(path: &Path) -> BTreeMap<String, Vec<String>> {
    let content = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    let mut features = BTreeMap::new();
    let mut in_features = false;
    let mut active: Option<(String, Vec<String>)> = None;

    for (line_no, raw_line) in content.lines().enumerate() {
        let line = strip_toml_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some((key, mut items)) = active.take() {
            if collect_array_items(line, &mut items) {
                assert!(
                    features.insert(key.clone(), items).is_none(),
                    "{}:{}: duplicate feature key `{key}`",
                    path.display(),
                    line_no + 1
                );
            } else {
                active = Some((key, items));
            }
            continue;
        }
        if line.starts_with('[') {
            if in_features {
                break;
            }
            in_features = line == "[features]";
            continue;
        }
        if !in_features {
            continue;
        }
        let (key, rhs) = line.split_once('=').unwrap_or_else(|| {
            panic!(
                "{}:{}: expected `key = [...]` in [features], got `{line}`",
                path.display(),
                line_no + 1
            )
        });
        let key = key.trim().trim_matches('"').to_owned();
        let rhs = rhs.trim();
        let after_open = rhs.strip_prefix('[').unwrap_or_else(|| {
            panic!(
                "{}:{}: expected `[` for feature `{key}`, got `{rhs}`",
                path.display(),
                line_no + 1
            )
        });
        let mut items = Vec::new();
        if collect_array_items(after_open, &mut items) {
            assert!(
                features.insert(key.clone(), items).is_none(),
                "{}:{}: duplicate feature key `{key}`",
                path.display(),
                line_no + 1
            );
        } else {
            active = Some((key, items));
        }
    }

    assert!(
        active.is_none(),
        "{}: unterminated feature array in [features]",
        path.display()
    );
    assert!(
        !features.is_empty(),
        "{}: missing or empty [features] table",
        path.display()
    );
    features
}

fn feature_set<'a>(
    table: &'a BTreeMap<String, Vec<String>>,
    manifest: &str,
    name: &str,
) -> BTreeSet<&'a str> {
    let entries = table
        .get(name)
        .unwrap_or_else(|| panic!("{manifest} is missing `{name}` feature"));
    entries.iter().map(String::as_str).collect()
}

fn intra_crate_implications(entries: &[String]) -> BTreeSet<&str> {
    entries
        .iter()
        .map(String::as_str)
        .filter(|entry| !entry.contains('/') && !entry.contains(':'))
        .collect()
}

#[test]
fn facade_internal_and_dylib_feature_tables_stay_symmetric() {
    let root_dir = manifest_dir();
    let root = parse_features_table(&root_dir.join("Cargo.toml"));
    let internal = parse_features_table(&root_dir.join("src/Cargo.toml"));
    let dylib = parse_features_table(&root_dir.join("utils/dylib/Cargo.toml"));

    assert!(
        !root.contains_key("dynamic_linking"),
        "waterui facade must not expose `dynamic_linking`; generated host crates own shared-runtime linkage"
    );

    let root_keys: BTreeSet<&str> = root.keys().map(String::as_str).collect();
    let internal_keys: BTreeSet<&str> = internal.keys().map(String::as_str).collect();
    let dylib_keys: BTreeSet<&str> = dylib.keys().map(String::as_str).collect();

    assert_eq!(
        root_keys, internal_keys,
        "waterui and waterui-internal [features] keys must match"
    );
    assert_eq!(
        root_keys, dylib_keys,
        "waterui and waterui-dylib [features] keys must match"
    );

    let root_default = feature_set(&root, "Cargo.toml", "default");
    let internal_default = feature_set(&internal, "src/Cargo.toml", "default");
    let dylib_default = feature_set(&dylib, "utils/dylib/Cargo.toml", "default");
    assert_eq!(
        root_default, internal_default,
        "waterui and waterui-internal `default` features must match"
    );
    assert_eq!(
        root_default, dylib_default,
        "waterui and waterui-dylib `default` features must match"
    );

    let root_all = feature_set(&root, "Cargo.toml", "all");
    let internal_all = feature_set(&internal, "src/Cargo.toml", "all");
    let dylib_all = feature_set(&dylib, "utils/dylib/Cargo.toml", "all");
    assert!(
        !dylib_all.contains("waterui-internal/all"),
        "waterui-dylib `all` must enumerate own features instead of forwarding waterui-internal/all"
    );
    assert_eq!(
        root_all, internal_all,
        "waterui and waterui-internal `all` features must match"
    );
    assert_eq!(
        root_all, dylib_all,
        "waterui and waterui-dylib `all` features must match"
    );

    for feat in &root_keys {
        if matches!(*feat, "default" | "all") {
            continue;
        }
        let root_entries = &root[*feat];
        let dylib_entries = &dylib[*feat];

        let expected_internal = format!("waterui-internal/{feat}");
        assert!(
            root_entries.contains(&expected_internal),
            "Cargo.toml feature `{feat}` must forward `{expected_internal}`"
        );
        assert!(
            !root_entries
                .iter()
                .any(|entry| entry.contains("waterui-dylib")),
            "Cargo.toml feature `{feat}` must not forward to `waterui-dylib`: {root_entries:?}"
        );
        assert!(
            dylib_entries.contains(&expected_internal),
            "utils/dylib/Cargo.toml feature `{feat}` must forward `{expected_internal}`"
        );

        assert_eq!(
            intra_crate_implications(root_entries),
            intra_crate_implications(dylib_entries),
            "feature `{feat}` intra-crate implications differ between Cargo.toml and utils/dylib/Cargo.toml"
        );
    }
}
