//! Gradle plugins a crate declares under `[package.metadata.waterui.android]`,
//! and their staging into the generated Gradle project.
//!
//! A crate whose Android side needs a Gradle plugin applied to the
//! application module — `com.google.gms.google-services`, which turns the
//! Firebase configuration into resources, say — declares it beside its
//! Kotlin sources:
//!
//! ```toml
//! [[package.metadata.waterui.android.gradle-plugin]]
//! id = "com.google.gms.google-services"
//! version = "4.4.2"
//! ```
//!
//! The version lands in the `pluginManagement { plugins {} }` block of the
//! project's `settings.gradle.kts`, the id in the module's `plugins {}`
//! block, so the plugin resolves from the repositories `pluginManagement`
//! lists and applies to the module alone.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::fmt;
use std::path::Path;

use askama::Template;
use eyre::Context;
use serde::Deserialize;
use smol::fs;

/// Markers bracketing the block [`write_gradle_plugins`] maintains inside the
/// `pluginManagement { plugins {} }` block of `settings.gradle.kts`.
pub(super) const SETTINGS_PLUGINS_BEGIN: &str =
    "        // --- begin waterui gradle plugin versions ---";
pub(super) const SETTINGS_PLUGINS_END: &str =
    "        // --- end waterui gradle plugin versions ---";

/// Markers bracketing the block [`write_gradle_plugins`] maintains inside the
/// application module's `plugins {}` block.
pub(super) const MODULE_PLUGINS_BEGIN: &str = "    // --- begin waterui gradle plugins ---";
pub(super) const MODULE_PLUGINS_END: &str = "    // --- end waterui gradle plugins ---";

/// Plugins the generated project already declares itself; a crate declaring
/// one would fight the template over its version.
const TEMPLATE_PLUGINS: &[&str] = &["com.android.application", "com.android.library"];

/// A Gradle plugin id: ASCII alphanumerics, `.`, `_` and `-`, namespaced by
/// at least one `.`, with no empty segment — Gradle's own rule, so a
/// declared id can never break out of the Kotlin string it renders into.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(try_from = "String")]
pub(super) struct GradlePluginId(String);

impl TryFrom<String> for GradlePluginId {
    type Error = String;

    fn try_from(id: String) -> Result<Self, Self::Error> {
        if !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        {
            return Err(format!(
                "Gradle plugin id `{id}` may contain only ASCII letters, digits, `.`, `_` and `-`"
            ));
        }
        if !id.contains('.') || id.split('.').any(str::is_empty) {
            return Err(format!(
                "Gradle plugin id `{id}` must be a namespaced id such as `com.example.plugin`"
            ));
        }
        if id.starts_with("org.gradle.") {
            return Err(format!(
                "Gradle plugin id `{id}` is a core Gradle plugin, which carries no version"
            ));
        }
        Ok(Self(id))
    }
}

impl fmt::Display for GradlePluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A Gradle plugin version: a non-empty run of ASCII alphanumerics, `.`,
/// `_`, `-` and `+`. A plugin request takes a fixed version, so ranges and
/// anything that would need escaping in a Kotlin string are rejected.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub(super) struct GradlePluginVersion(String);

impl TryFrom<String> for GradlePluginVersion {
    type Error = String;

    fn try_from(version: String) -> Result<Self, Self::Error> {
        if version.is_empty()
            || !version
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'))
        {
            return Err(format!(
                "Gradle plugin version `{version}` must be a fixed version of ASCII letters, digits, `.`, `_`, `-` and `+`"
            ));
        }
        Ok(Self(version))
    }
}

impl fmt::Display for GradlePluginVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One `[[package.metadata.waterui.android.gradle-plugin]]` table.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(super) struct GradlePlugin {
    /// The plugin id, as a `plugins { id("...") }` request names it.
    id: GradlePluginId,
    /// The plugin version the project's settings pin.
    version: GradlePluginVersion,
}

/// A merged plugin and the crate that declared it first.
#[derive(Debug)]
struct Declared {
    crate_name: String,
    version: GradlePluginVersion,
}

/// The Gradle plugins the whole dependency graph declares, keyed by id.
///
/// The same plugin at the same version arriving from two crates is applied
/// once; one plugin at two versions cannot resolve and fails the merge with
/// an error naming both crates.
#[derive(Debug, Default)]
pub struct GradlePlugins {
    plugins: BTreeMap<GradlePluginId, Declared>,
}

/// A plugin as the templates render it.
struct RenderedPlugin<'a> {
    id: &'a GradlePluginId,
    version: &'a GradlePluginVersion,
}

impl GradlePlugins {
    /// Merges `crate_name`'s declarations.
    ///
    /// # Errors
    ///
    /// Returns an error when a crate declares a plugin the generated project
    /// already applies, or a plugin another crate declared at a different
    /// version.
    pub(super) fn merge(
        &mut self,
        crate_name: &str,
        declared: Vec<GradlePlugin>,
    ) -> eyre::Result<()> {
        for plugin in declared {
            eyre::ensure!(
                !TEMPLATE_PLUGINS.contains(&plugin.id.0.as_str()),
                "{crate_name} declares Gradle plugin `{}`, which the generated project applies itself",
                plugin.id
            );
            match self.plugins.entry(plugin.id) {
                Entry::Vacant(entry) => {
                    entry.insert(Declared {
                        crate_name: crate_name.to_owned(),
                        version: plugin.version,
                    });
                }
                Entry::Occupied(entry) => {
                    let existing = entry.get();
                    eyre::ensure!(
                        existing.version == plugin.version,
                        "crates `{}` and `{crate_name}` declare Gradle plugin `{}` at versions `{}` and `{}`; the project can apply only one",
                        existing.crate_name,
                        entry.key(),
                        existing.version,
                        plugin.version
                    );
                }
            }
        }
        Ok(())
    }

    fn rendered(&self) -> Vec<RenderedPlugin<'_>> {
        self.plugins
            .iter()
            .map(|(id, declared)| RenderedPlugin {
                id,
                version: &declared.version,
            })
            .collect()
    }

    /// Renders the managed `pluginManagement { plugins {} }` block of
    /// `settings.gradle.kts`, markers included.
    pub(super) fn render_settings_block(&self) -> eyre::Result<String> {
        let block = SettingsPluginsTemplate {
            begin: SETTINGS_PLUGINS_BEGIN,
            end: SETTINGS_PLUGINS_END,
            plugins: self.rendered(),
        }
        .render()
        .wrap_err("rendering the Gradle plugin versions block")?;
        Ok(block.trim_end().to_owned())
    }

    /// Renders the managed module `plugins {}` block, markers included.
    pub(super) fn render_module_block(&self) -> eyre::Result<String> {
        let block = ModulePluginsTemplate {
            begin: MODULE_PLUGINS_BEGIN,
            end: MODULE_PLUGINS_END,
            plugins: self.rendered(),
        }
        .render()
        .wrap_err("rendering the Gradle module plugins block")?;
        Ok(block.trim_end().to_owned())
    }
}

/// The managed block of `pluginManagement { plugins {} }`. Ids and versions
/// are validated types that cannot contain a quote, so no escaping applies.
#[derive(Template)]
#[template(
    path = "src/templates/android_gradle/settings_plugins.kts.tpl",
    escape = "none"
)]
struct SettingsPluginsTemplate<'a> {
    begin: &'static str,
    end: &'static str,
    plugins: Vec<RenderedPlugin<'a>>,
}

/// The managed block of the module's `plugins {}`.
#[derive(Template)]
#[template(
    path = "src/templates/android_gradle/module_plugins.kts.tpl",
    escape = "none"
)]
struct ModulePluginsTemplate<'a> {
    begin: &'static str,
    end: &'static str,
    plugins: Vec<RenderedPlugin<'a>>,
}

/// Rewrites the managed plugin blocks of an application module: the version
/// pins inside `pluginManagement { plugins {} }` of the project's
/// `settings.gradle.kts`, and the ids inside the `plugins {}` block of
/// `module_dir/build.gradle.kts`. The templates ship both marker pairs; a
/// file missing one was hand-edited or generated before this mechanism and
/// is an error. An empty set leaves the pairs empty, so a plugin whose crate
/// left the graph stops applying.
pub(super) async fn write_gradle_plugins(
    project_dir: &Path,
    module_dir: &Path,
    plugins: &GradlePlugins,
) -> eyre::Result<()> {
    rewrite_block(
        &project_dir.join("settings.gradle.kts"),
        SETTINGS_PLUGINS_BEGIN,
        SETTINGS_PLUGINS_END,
        "`pluginManagement { plugins {} }`",
        &plugins.render_settings_block()?,
    )
    .await?;
    rewrite_block(
        &module_dir.join("build.gradle.kts"),
        MODULE_PLUGINS_BEGIN,
        MODULE_PLUGINS_END,
        "`plugins {}`",
        &plugins.render_module_block()?,
    )
    .await
}

async fn rewrite_block(
    path: &Path,
    begin_marker: &str,
    end_marker: &str,
    location: &str,
    block: &str,
) -> eyre::Result<()> {
    let existing = fs::read_to_string(path)
        .await
        .wrap_err_with(|| format!("reading Gradle script {}", path.display()))?;
    let body = super::splice_managed_block(&existing, path, begin_marker, end_marker, block)?
        .ok_or_else(|| {
            eyre::eyre!(
                "{} has no managed Gradle plugins block ({begin_marker} / {end_marker}); the template must emit the marker pair inside {location}",
                path.display()
            )
        })?;
    super::super::templates::write_file_if_changed(path, body.as_bytes())
        .await
        .map_err(Into::into)
}

/// Asserts a scaffolded `settings.gradle.kts` carries the empty plugin
/// versions marker pair inside `pluginManagement { plugins {} }`.
#[cfg(test)]
#[track_caller]
pub fn assert_settings_plugin_markers(settings: &str) {
    let block =
        format!("    plugins {{\n{SETTINGS_PLUGINS_BEGIN}\n{SETTINGS_PLUGINS_END}\n    }}\n}}");
    let management = settings
        .find("pluginManagement {")
        .unwrap_or_else(|| panic!("settings lack `pluginManagement`: {settings}"));
    let markers = settings
        .find(&block)
        .unwrap_or_else(|| panic!("settings lack the plugin versions block: {settings}"));
    assert!(
        management < markers && !settings[management..markers].contains("\n}"),
        "the plugin versions markers must close `pluginManagement`: {settings}"
    );
}

/// Asserts a scaffolded module build script carries the empty plugin marker
/// pair inside its leading `plugins {}` block.
#[cfg(test)]
#[track_caller]
pub fn assert_module_plugin_markers(build: &str) {
    let block = format!("{MODULE_PLUGINS_BEGIN}\n{MODULE_PLUGINS_END}\n}}");
    assert!(build.starts_with("plugins {\n"), "{build}");
    let markers = build
        .find(&block)
        .unwrap_or_else(|| panic!("module build script lacks the plugin block: {build}"));
    assert!(
        !build[..markers].contains("\n}"),
        "the plugin markers must sit inside the leading `plugins {{}}`: {build}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[derive(Deserialize)]
    struct Table {
        #[serde(rename = "gradle-plugin")]
        gradle_plugin: Vec<GradlePlugin>,
    }

    fn declared(toml_text: &str) -> Vec<GradlePlugin> {
        toml::from_str::<Table>(toml_text)
            .expect("plugin table parses")
            .gradle_plugin
    }

    const GOOGLE_SERVICES: &str = r#"
        [[gradle-plugin]]
        id = "com.google.gms.google-services"
        version = "4.4.2"
    "#;

    fn merged(crates: &[(&str, &str)]) -> eyre::Result<GradlePlugins> {
        let mut plugins = GradlePlugins::default();
        for (name, table) in crates {
            plugins.merge(name, declared(table))?;
        }
        Ok(plugins)
    }

    #[test]
    fn plugins_render_into_both_managed_blocks() {
        let plugins = merged(&[
            ("waterkit-push", GOOGLE_SERVICES),
            (
                "waterkit-other",
                "[[gradle-plugin]]\nid = \"com.example.alpha\"\nversion = \"1.0.0-rc.1\"\n",
            ),
        ])
        .expect("merge");
        assert_eq!(
            plugins.render_settings_block().expect("render settings"),
            format!(
                "{SETTINGS_PLUGINS_BEGIN}\n        id(\"com.example.alpha\") version \"1.0.0-rc.1\"\n        id(\"com.google.gms.google-services\") version \"4.4.2\"\n{SETTINGS_PLUGINS_END}"
            )
        );
        assert_eq!(
            plugins.render_module_block().expect("render module"),
            format!(
                "{MODULE_PLUGINS_BEGIN}\n    id(\"com.example.alpha\")\n    id(\"com.google.gms.google-services\")\n{MODULE_PLUGINS_END}"
            )
        );
    }

    #[test]
    fn no_plugins_render_empty_blocks() {
        let plugins = GradlePlugins::default();
        assert_eq!(
            plugins.render_settings_block().expect("render"),
            format!("{SETTINGS_PLUGINS_BEGIN}\n{SETTINGS_PLUGINS_END}")
        );
        assert_eq!(
            plugins.render_module_block().expect("render"),
            format!("{MODULE_PLUGINS_BEGIN}\n{MODULE_PLUGINS_END}")
        );
    }

    #[test]
    fn identical_declarations_merge_into_one() {
        let plugins = merged(&[("a", GOOGLE_SERVICES), ("b", GOOGLE_SERVICES)]).expect("merge");
        assert_eq!(plugins.plugins.len(), 1);
    }

    #[test]
    fn conflicting_versions_name_both_crates() {
        let error = merged(&[
            ("waterkit-push", GOOGLE_SERVICES),
            ("waterkit-auth", &GOOGLE_SERVICES.replace("4.4.2", "4.3.0")),
        ])
        .expect_err("two versions of one plugin must fail");
        let message = error.to_string();
        assert!(message.contains("`waterkit-push`"), "{message}");
        assert!(message.contains("`waterkit-auth`"), "{message}");
        assert!(
            message.contains("4.4.2") && message.contains("4.3.0"),
            "{message}"
        );
    }

    #[test]
    fn a_plugin_the_template_applies_fails() {
        let error = merged(&[(
            "rogue",
            "[[gradle-plugin]]\nid = \"com.android.application\"\nversion = \"8.0.0\"\n",
        )])
        .expect_err("the template's own plugin must fail");
        assert!(
            error.to_string().contains("com.android.application"),
            "{error}"
        );
    }

    #[test]
    fn malformed_tables_fail_to_parse() {
        for table in [
            // An unknown key.
            "[[gradle-plugin]]\nid = \"com.example.a\"\nversion = \"1\"\napply = false\n",
            // A missing version.
            "[[gradle-plugin]]\nid = \"com.example.a\"\n",
            // An id without a namespace.
            "[[gradle-plugin]]\nid = \"plugin\"\nversion = \"1\"\n",
            // An id with an empty segment.
            "[[gradle-plugin]]\nid = \"com..a\"\nversion = \"1\"\n",
            // An id that would break out of its Kotlin string.
            "[[gradle-plugin]]\nid = \"com.example.a\\\")\"\nversion = \"1\"\n",
            // A core plugin.
            "[[gradle-plugin]]\nid = \"org.gradle.maven-publish\"\nversion = \"1\"\n",
            // A version range.
            "[[gradle-plugin]]\nid = \"com.example.a\"\nversion = \"[1.0,2.0)\"\n",
            // An empty version.
            "[[gradle-plugin]]\nid = \"com.example.a\"\nversion = \"\"\n",
        ] {
            assert!(
                toml::from_str::<Table>(table).is_err(),
                "must reject: {table}"
            );
        }
    }

    const SETTINGS: &str = "pluginManagement {\n    repositories {\n        google()\n    }\n    plugins {\n        // --- begin waterui gradle plugin versions ---\n        // --- end waterui gradle plugin versions ---\n    }\n}\n\nrootProject.name = \"demo\"\n";
    const MODULE: &str = "plugins {\n    id(\"com.android.application\")\n    // --- begin waterui gradle plugins ---\n    // --- end waterui gradle plugins ---\n}\n\nandroid {}\n";

    #[test]
    fn staging_rewrites_both_blocks() {
        let root = tempdir().expect("temp root");
        let module_dir = root.path().join("app");
        std::fs::create_dir_all(&module_dir).expect("module dir");
        std::fs::write(root.path().join("settings.gradle.kts"), SETTINGS).expect("settings");
        std::fs::write(module_dir.join("build.gradle.kts"), MODULE).expect("module");

        let plugins = merged(&[("waterkit-push", GOOGLE_SERVICES)]).expect("merge");
        smol::block_on(write_gradle_plugins(root.path(), &module_dir, &plugins)).expect("stage");
        let settings = std::fs::read_to_string(root.path().join("settings.gradle.kts"))
            .expect("read settings");
        assert!(
            settings.contains("        id(\"com.google.gms.google-services\") version \"4.4.2\"\n"),
            "{settings}"
        );
        let module = std::fs::read_to_string(module_dir.join("build.gradle.kts")).expect("module");
        assert!(
            module.starts_with("plugins {\n    id(\"com.android.application\")\n    // --- begin waterui gradle plugins ---\n    id(\"com.google.gms.google-services\")\n"),
            "{module}"
        );

        // A plugin whose crate left the graph stops applying.
        smol::block_on(write_gradle_plugins(
            root.path(),
            &module_dir,
            &GradlePlugins::default(),
        ))
        .expect("restage");
        assert_eq!(
            std::fs::read_to_string(root.path().join("settings.gradle.kts")).expect("settings"),
            SETTINGS
        );
        assert_eq!(
            std::fs::read_to_string(module_dir.join("build.gradle.kts")).expect("module"),
            MODULE
        );
    }

    #[test]
    fn a_script_without_the_markers_fails() {
        let root = tempdir().expect("temp root");
        let module_dir = root.path().join("app");
        std::fs::create_dir_all(&module_dir).expect("module dir");
        std::fs::write(
            root.path().join("settings.gradle.kts"),
            "rootProject.name = \"x\"\n",
        )
        .expect("settings");
        std::fs::write(module_dir.join("build.gradle.kts"), MODULE).expect("module");
        let error = smol::block_on(write_gradle_plugins(
            root.path(),
            &module_dir,
            &GradlePlugins::default(),
        ))
        .expect_err("missing markers must fail");
        assert!(error.to_string().contains("settings.gradle.kts"), "{error}");
    }
}
