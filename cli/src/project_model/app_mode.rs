//! Detection of projects created in the removed app mode.
//!
//! App mode recorded its projects in `Water.toml`, so the manifest keys are
//! the reliable signal. The CLI now generates and manages every backend
//! project in the build cache, so such a project is refused with the exact
//! keys to delete — it is never silently reinterpreted.

use std::fmt;

/// `Water.toml` keys only app mode read, as `(table path, key)`; an empty key
/// names the whole table.
const APP_MODE_KEYS: &[(&[&str], &str)] = &[
    (&["package"], "type"),
    // `Water.toml` carries no `[backends]` table at all: the local runtime
    // checkout lives at `waterui_path/backends/apple`, and the Hydrolysis
    // painter is
    // `[hydrolysis]` — the whole table is retired, keys and subtables alike.
    (&["backends"], ""),
];

/// What an app-mode project carries that the CLI no longer reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppModeLeftovers {
    /// Dotted `Water.toml` keys, such as `backends.apple.scheme`.
    pub keys: Vec<String>,
}

impl AppModeLeftovers {
    /// Collect the app-mode leftovers of a project whose `Water.toml` parsed
    /// to `manifest`. `None` when there are none.
    #[must_use]
    pub fn find(manifest: &toml::Table) -> Option<Self> {
        let keys: Vec<String> = APP_MODE_KEYS
            .iter()
            .filter(|(table, key)| {
                // Intermediate path segments must resolve to tables; the
                // last segment need only be present — a scalar
                // `backends = "…"` still names the retired table.
                let Some(value) = manifest.get(table[0]).and_then(|value| {
                    table[1..]
                        .iter()
                        .try_fold(value, |value, name| value.as_table()?.get(*name))
                }) else {
                    return false;
                };
                key.is_empty() || value.as_table().is_some_and(|t| t.contains_key(*key))
            })
            .map(|(table, key)| {
                let mut dotted = table.join(".");
                if !key.is_empty() {
                    dotted.push('.');
                    dotted.push_str(key);
                }
                dotted
            })
            .collect();
        (!keys.is_empty()).then_some(Self { keys })
    }
}

impl fmt::Display for AppModeLeftovers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "This project carries configuration from the removed app and playground \
             package types: the CLI now generates and manages every backend project \
             itself, and `[package] type` no longer selects a mode. \
             Delete the following, then run the command again."
        )?;
        writeln!(f, "Water.toml keys that are no longer read:")?;
        for key in &self.keys {
            writeln!(f, "  - {key}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::AppModeLeftovers;

    fn parse(text: &str) -> toml::Table {
        text.parse().expect("fixture manifest parses")
    }

    #[test]
    fn a_current_manifest_has_no_leftovers() {
        let manifest = parse(
            r#"
                waterui_path = "../waterui"

                [package]
                name = "Demo"
                bundle_identifier = "dev.waterui.demo"

                [hydrolysis]
                painter = "gpu"
            "#,
        );
        assert_eq!(AppModeLeftovers::find(&manifest), None);
    }

    /// Any `[backends]` table is retired configuration — the old
    /// `backend_path` override included — and is named rather than silently
    /// dropped.
    #[test]
    fn a_backends_table_is_named() {
        for manifest in [
            "[backends]",
            "[backends]\npath = \"backends\"",
            "[backends.apple]\nbackend_path = \"../apple-backend\"",
            "[backends.hydrolysis]\npainter = \"gpu\"",
        ] {
            let manifest = parse(&format!(
                "[package]\nname = \"Demo\"\nbundle_identifier = \"dev.waterui.demo\"\n\n{manifest}"
            ));
            let leftovers = AppModeLeftovers::find(&manifest)
                .unwrap_or_else(|| panic!("retired backends table: {manifest}"));
            assert_eq!(leftovers.keys, ["backends"]);
        }
        // A top-level dotted table header or scalar names `backends` the
        // same way — the key is retired whatever shape it takes.
        for manifest in ["backends.apple.scheme = \"demo\"", "backends = \"retired\""] {
            let manifest = parse(&format!(
                "{manifest}\n\n[package]\nname = \"Demo\"\nbundle_identifier = \"dev.waterui.demo\""
            ));
            let leftovers = AppModeLeftovers::find(&manifest)
                .unwrap_or_else(|| panic!("retired backends table: {manifest}"));
            assert_eq!(leftovers.keys, ["backends"]);
        }
    }

    #[test]
    fn app_mode_keys_are_named() {
        let manifest = parse(
            r#"
                [package]
                type = "app"
                name = "Demo"
                bundle_identifier = "dev.waterui.demo"

                [backends]
                path = "backends"

                [backends.apple]
                scheme = "demo"
                backend_path = "../apple-backend"

                [backends.gtk4]
                project_path = "gtk4"
            "#,
        );
        let leftovers = AppModeLeftovers::find(&manifest).expect("app-mode project");
        assert_eq!(leftovers.keys, ["package.type", "backends"]);
        let message = leftovers.to_string();
        assert!(message.contains("backends"));
    }

    /// `type = "playground"` is the removed mode selector too: the key is no
    /// longer read, so it is named rather than silently accepted.
    #[test]
    fn the_playground_type_key_is_named() {
        let manifest = parse(
            r#"
                [package]
                type = "playground"
                name = "Demo"
                bundle_identifier = "dev.waterui.demo"
            "#,
        );
        let leftovers = AppModeLeftovers::find(&manifest).expect("mode key");
        assert_eq!(leftovers.keys, ["package.type"]);
    }
}
