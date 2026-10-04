//! The project's Cargo configuration, carried into out-of-tree cargo spawns.
//!
//! Cargo discovers its configuration by walking the working directory's
//! ancestor chain: `<dir>/.cargo/config.toml` for each ancestor, shallowest
//! lowest precedence, then `$CARGO_HOME/config.toml` below all of them. The
//! CLI runs some cargo builds for a project with a different working
//! directory — managed backend crates scaffolded under
//! `~/.water/build_cache/<project>/` — so that discovery never reaches
//! `<project>/.cargo/config.toml` and the project's `rustflags`, `-L` linker
//! paths and `[env]` entries are silently dropped.
//!
//! [`cargo_config_args`] reproduces the project's hierarchy through
//! `--config` arguments. Cargo applies the same resolution rules to a config
//! file passed via `--config <path>` as to a discovered one: a file named
//! `config.toml` living inside a `.cargo` directory resolves its relative
//! path values (`[env]` `relative = true`, `paths`, `source.*.directory`,
//! `build.target-dir`, …) against the `.cargo` directory's parent, so
//! passing the discovered files verbatim preserves those semantics exactly.
//! `--config` files outrank every auto-discovered file — including the
//! managed crate's own `.cargo/config.toml`, when the scaffold writes one —
//! so that file is re-listed last to keep its precedence.
//!
//! One setting survives as a literal string instead: `-L`/`--extern` paths
//! inside `rustflags`/`rustdocflags` are never interpreted by Cargo — rustc
//! resolves them against the working directory of the invocation, which for
//! a managed crate is the build cache, not the project. [`cargo_config_args`]
//! therefore also writes a generated config carrying only those entries with
//! the paths rebased to absolute ones against the directory that declared
//! them; array values concatenate across `--config` sources, so the absolute
//! `-L` joins the search path while the leftover relative one simply misses.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use color_eyre::eyre::{Context as _, Result};

/// `--config` argument pairs making a `cargo` invocation whose working
/// directory is `build_dir` see the same Cargo configuration as a `cargo`
/// invocation made inside `project_root`.
///
/// # Errors
/// Fails when a config file exists but cannot be read or parsed — Cargo
/// would fail the same way inside the project.
pub fn cargo_config_args(project_root: &Path, build_dir: &Path) -> Result<Vec<OsString>> {
    Ok(cargo_config_files(project_root, build_dir)?
        .into_iter()
        .flat_map(|path| [OsString::from("--config"), path.into_os_string()])
        .collect())
}

/// The Cargo configuration files [`cargo_config_args`] passes via
/// `--config`, in the same ascending precedence order.
///
/// The list is empty when `build_dir` sits inside `project_root`: discovery
/// already reaches the project files. Otherwise it contains every
/// `<ancestor>/.cargo/config.toml` (or the legacy `.cargo/config`) on
/// `project_root`'s chain shallow→deep, then a generated file carrying the
/// cwd-relative `-L`/`--extern` paths rebased to absolute, then the
/// `build_dir` crate's own `.cargo/config.toml` when one exists so a
/// generated crate-level configuration keeps the highest precedence.
///
/// `$CARGO_HOME/config.toml` needs no entry: Cargo reads it for every
/// invocation regardless of working directory.
///
/// Resolvers that mirror Cargo's layering — like
/// [`crate::toolchain::cargo_rustflags`] — need this file list itself, not
/// just the argument pairs.
///
/// # Errors
/// Fails when a config file exists but cannot be read or parsed — Cargo
/// would fail the same way inside the project.
pub fn cargo_config_files(project_root: &Path, build_dir: &Path) -> Result<Vec<PathBuf>> {
    let project_root = &dunce::canonicalize(project_root)
        .wrap_err_with(|| format!("cannot resolve project root {}", project_root.display()))?;
    let build_dir = &dunce::canonicalize(build_dir)
        .wrap_err_with(|| format!("cannot resolve build dir {}", build_dir.display()))?;
    if build_dir == project_root || build_dir.starts_with(project_root) {
        return Ok(Vec::new());
    }

    let mut ancestors: Vec<PathBuf> = project_root.ancestors().map(Path::to_path_buf).collect();
    ancestors.reverse(); // shallowest first: lower precedence, project last.

    // The flag file rebases `-L`/`--extern` entries that rustc resolves
    // against the managed crate's directory instead of the project.
    let mut flag_sections: Vec<(Vec<String>, Vec<String>)> = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();
    for dir in &ancestors {
        let Some(path) = discovered_config_file(dir) else {
            continue;
        };
        let table = read_config(&path)?;
        collect_flag_paths(&table, dir, &mut flag_sections);
        files.push(path);
    }

    if flag_sections.iter().any(|(_, flags)| !flags.is_empty()) {
        files.push(write_flag_config(
            &build_dir.join(".water-cargo-config"),
            &flag_sections,
        )?);
    }

    // A config file the generated crate itself ships keeps precedence over
    // everything imported from the project's hierarchy.
    if let Some(path) = discovered_config_file(build_dir) {
        files.push(path);
    }

    Ok(files)
}

/// Lay the `--config` layers [`cargo_config_files`] returns down as a chain
/// of `.cargo/config.toml` files under `build_dir`.
///
/// Returns the directory to resolve config from to see the files at Cargo's
/// `--config` precedence. Cargo's `--config` arguments sit above every auto-discovered file —
/// including the crate's own `.cargo/config.toml` — and merge with the same
/// rules. Resolvers that discover config hierarchically (like
/// `cargo_config2::Config::load_with_options`) have no `--config` channel, so
/// the files are materialized where discovery reaches them: `files[i]` is
/// copied to `config-chain/d/…/d/.cargo/config.toml`, `i` directories deep,
/// preserving the ascending precedence of the input list. Anything deeper
/// than `build_dir`'s own hierarchy outranks it, matching `--config`.
///
/// The copies live under `.water-cargo-config`, the generated directory the
/// `-L` rebase already uses; contents are rewritten on every call so a stale
/// chain never lingers.
///
/// # Errors
/// Fails when a directory cannot be created or a file cannot be copied —
/// the build would fail resolving configuration anyway.
pub fn config_chain_dir(build_dir: &Path, files: &[PathBuf]) -> Result<PathBuf> {
    let chain = build_dir.join(".water-cargo-config").join("config-chain");
    if chain.is_dir() {
        std::fs::remove_dir_all(&chain)
            .wrap_err_with(|| format!("cannot refresh config chain {}", chain.display()))?;
    }
    let mut dir = chain;
    for file in files {
        dir = dir.join("d");
        let cargo_dir = dir.join(".cargo");
        std::fs::create_dir_all(&cargo_dir)
            .wrap_err_with(|| format!("cannot create config chain dir {}", cargo_dir.display()))?;
        std::fs::copy(file, cargo_dir.join("config.toml"))
            .wrap_err_with(|| format!("cannot copy {} into the config chain", file.display()))?;
    }
    Ok(dir)
}

/// The config file Cargo would read inside `dir`: `.cargo/config.toml`, or
/// the legacy `.cargo/config` when no `.toml` file exists.
fn discovered_config_file(dir: &Path) -> Option<PathBuf> {
    let toml = dir.join(".cargo").join("config.toml");
    if toml.is_file() {
        return Some(toml);
    }
    let legacy = dir.join(".cargo").join("config");
    legacy.is_file().then_some(legacy)
}

/// Read and parse a Cargo config file.
fn read_config(path: &Path) -> Result<toml::Table> {
    let text = std::fs::read_to_string(path)
        .wrap_err_with(|| format!("failed to read {}", path.display()))?;
    text.parse::<toml::Table>()
        .wrap_err_with(|| format!("failed to parse {}", path.display()))
}

/// Collect the cwd-relative `-L`/`--extern` paths from every
/// `rustflags`/`rustdocflags` list in `table` — `build.*` and
/// `target.<triple-or-cfg>.*` — rebased to absolute ones against `root`, the
/// config file's discovery directory. Each entry pushed onto `sections` is
/// the section's key path (`["build", "rustflags"]`,
/// `["target", "<key>", "rustflags"]`) with the rebased flag tokens.
fn collect_flag_paths(
    table: &toml::Table,
    root: &Path,
    sections: &mut Vec<(Vec<String>, Vec<String>)>,
) {
    let flags_in = |section: &toml::Table,
                    key_path: &[String],
                    sections: &mut Vec<(Vec<String>, Vec<String>)>| {
        for key in ["rustflags", "rustdocflags"] {
            let Some(value) = section.get(key) else {
                continue;
            };
            let tokens: Vec<String> = match value {
                toml::Value::String(flags) => flags.split_whitespace().map(String::from).collect(),
                toml::Value::Array(flags) => flags
                    .iter()
                    .filter_map(|flag| flag.as_str().map(String::from))
                    .collect(),
                _ => continue,
            };
            let mut rebased = Vec::new();
            let mut tokens = tokens.iter();
            while let Some(token) = tokens.next() {
                let Some(tail) = flag_tail(token) else {
                    continue;
                };
                if tail.is_empty() {
                    // A bare `-L`/`--extern`: the next token is its path
                    // (or `kind=path`) argument.
                    if let Some(next) = tokens.next()
                        && let Some(argument) = rebased_path_argument(next, root)
                    {
                        rebased.push(token.clone());
                        rebased.push(argument);
                    }
                } else {
                    // A glued `-Lpath` / `-Lkind=path` / `--extern=name=path`:
                    // rebuild the flag with the path rebased.
                    let prefix = &token[..token.len() - tail.len()];
                    if let Some(argument) = rebased_path_argument(tail, root) {
                        rebased.push(format!("{prefix}{argument}"));
                    }
                }
            }
            let mut path = key_path.to_vec();
            path.push((*key).to_string());
            sections.push((path, rebased));
        }
    };
    if let Some(build) = table.get("build").and_then(toml::Value::as_table) {
        flags_in(build, &["build".to_string()], sections);
    }
    if let Some(target) = table.get("target").and_then(toml::Value::as_table) {
        for (name, section) in target {
            if let Some(section) = section.as_table() {
                flags_in(section, &["target".to_string(), name.clone()], sections);
            }
        }
    }
}

/// The flag-carrying tail of a rustc flag token: `Some("-L")`→ bare flag
/// (`""`), `Some("native=./libs")`→ the argument part. `None` when the token
/// is neither `-L` nor `--extern`.
fn flag_tail(token: &str) -> Option<&str> {
    if token == "-L" || token == "--extern" {
        return Some("");
    }
    token
        .strip_prefix("-L")
        .or_else(|| token.strip_prefix("--extern="))
}

/// Split a `-L`/`--extern` argument (`path` or `kind=path`) and rebase a
/// relative `path` against the config file's discovery directory. `kind`
/// (`native`, `dependency`, `framework`, a crate name) is kept verbatim.
/// Returns `None` for absolute paths, which need no rebase.
fn rebased_path_argument(argument: &str, root: &Path) -> Option<String> {
    let (kind, path) = match argument.split_once('=') {
        Some((kind, path)) => (format!("{kind}="), path),
        None => (String::new(), argument),
    };
    let path = Path::new(path.strip_prefix("./").unwrap_or(path));
    path.is_relative()
        .then(|| format!("{kind}{}", root.join(path).display()))
}

/// Write the generated config carrying the rebased `-L`/`--extern` flags to
/// `<out_dir>/project-flags.toml` and return its path.
fn write_flag_config(out_dir: &Path, sections: &[(Vec<String>, Vec<String>)]) -> Result<PathBuf> {
    let mut root = toml::Table::new();
    for (key_path, flags) in sections {
        if flags.is_empty() {
            continue;
        }
        // Navigate `build` / `target.<name>` down to the flags key.
        let (last, parents) = key_path.split_last().expect("key path is non-empty");
        let mut section = &mut root;
        for key in parents {
            section = section
                .entry(key.clone())
                .or_insert_with(|| toml::Value::Table(toml::Table::new()))
                .as_table_mut()
                .expect("config section is a table");
        }
        section.insert(
            last.clone(),
            toml::Value::Array(flags.iter().cloned().map(toml::Value::String).collect()),
        );
    }
    std::fs::create_dir_all(out_dir)
        .wrap_err_with(|| format!("failed to create {}", out_dir.display()))?;
    let path = out_dir.join("project-flags.toml");
    std::fs::write(
        &path,
        toml::to_string(&root).expect("config table serializes"),
    )
    .wrap_err_with(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temporary directory and its canonical path. The implementation
    /// canonicalizes the project and build directories, so expected paths
    /// must be built from the same form: macOS temp dirs live behind the
    /// `/var` → `/private/var` symlink and Windows hands out 8.3 short names.
    fn temp_root() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = dunce::canonicalize(temp.path()).unwrap();
        (temp, root)
    }

    fn write(path: &Path, contents: &str) {
        std::fs::create_dir_all(path.parent().expect("file has a parent")).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    /// The probe crate whose build fails unless the `--cfg
    /// water_config_probe` marker reaches rustc.
    fn probe_crate(dir: &Path) {
        write(
            &dir.join("Cargo.toml"),
            "[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        write(
            &dir.join("src/lib.rs"),
            "#[cfg(not(water_config_probe))]\ncompile_error!(\"the project's cargo config did not reach this build\");\n",
        );
    }

    #[test]
    fn empty_when_the_build_runs_inside_the_project() {
        let (_temp, root) = temp_root();
        let project = root.join("proj");
        write(
            &project.join(".cargo").join("config.toml"),
            "[build]\nrustflags = [\"--cfg\", \"water_config_probe\"]\n",
        );
        // The managed crate living under the project keeps pure discovery.
        let nested = project.join("tools/backend");
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(
            cargo_config_args(&project, &nested).unwrap(),
            [] as [std::ffi::OsString; 0]
        );
        assert_eq!(
            cargo_config_args(&project, &project).unwrap(),
            [] as [std::ffi::OsString; 0]
        );
    }

    #[test]
    fn lists_the_whole_hierarchy_in_precedence_order() {
        let (_temp, root) = temp_root();
        let project = root.join("outer").join("inner").join("proj");
        write(
            &project.join(".cargo").join("config.toml"),
            "[build]\nrustflags = [\"--cfg\", \"water_config_probe\"]\n",
        );
        write(
            &root.join("outer").join(".cargo").join("config.toml"),
            "[build]\nrustflags = [\"--cfg\", \"outer_marker\"]\n",
        );
        let build = root.join("cache").join("backend");
        std::fs::create_dir_all(&build).unwrap();

        let args = cargo_config_args(&project, &build).unwrap();
        let flags: Vec<String> = args
            .iter()
            .filter_map(|a| a.clone().into_string().ok())
            .collect();
        let outer = root.join("outer").join(".cargo").join("config.toml");
        let own = project.join(".cargo").join("config.toml");
        let outer_pos = flags.iter().position(|f| Path::new(f) == outer).unwrap();
        let own_pos = flags.iter().position(|f| Path::new(f) == own).unwrap();
        // Ancestors are listed first; the project's own file is last = highest
        // precedence.
        assert!(outer_pos < own_pos);
        // No `-L` paths → no generated flag file.
        assert!(
            !build
                .join(".water-cargo-config")
                .join("project-flags.toml")
                .exists()
        );
    }

    #[test]
    fn build_cache_config_keeps_precedence() {
        let (_temp, root) = temp_root();
        let project = root.join("proj");
        write(
            &project.join(".cargo").join("config.toml"),
            "[build]\nrustflags = []\n",
        );
        let build = root.join("cache").join("backend");
        write(
            &build.join(".cargo").join("config.toml"),
            "[build]\nrustflags = [\"--cfg\", \"harness_marker\"]\n",
        );

        let args = cargo_config_args(&project, &build).unwrap();
        let own = build.join(".cargo").join("config.toml");
        // The managed crate's config is the final --config = top precedence.
        assert_eq!(args.last().map(Path::new), Some(own.as_path()));
    }

    #[test]
    fn cwd_relative_flag_paths_are_rebased_to_absolute() {
        let (_temp, root) = temp_root();
        let project = root.join("proj");
        write(
            &project.join(".cargo").join("config.toml"),
            "[build]\nrustflags = [\"-L\", \"native=./libs\", \"-Lframework=./fw\", \"--extern\", \"helper=./rlib/libhelper.rlib\"]\n",
        );
        let build = root.join("cache").join("backend");
        std::fs::create_dir_all(&build).unwrap();

        let args = cargo_config_args(&project, &build).unwrap();
        let generated = build.join(".water-cargo-config").join("project-flags.toml");
        assert!(args.iter().any(|a| Path::new(a) == generated));
        let contents = std::fs::read_to_string(&generated).unwrap();
        for relative in ["libs", "fw", "rlib/libhelper.rlib"] {
            let absolute = project.join(relative);
            assert!(
                contents.contains(&absolute.display().to_string()),
                "{contents}"
            );
        }
    }

    /// The contract: a `cargo` run in the managed crate's directory receives
    /// the project's `[build] rustflags` exactly as a run inside the project
    /// would.
    #[test]
    fn managed_build_receives_the_project_rustflags() {
        let (_temp, root) = temp_root();
        let project = root.join("proj");
        write(
            &project.join(".cargo").join("config.toml"),
            "[build]\nrustflags = [\"--cfg\", \"water_config_probe\"]\n",
        );
        let build = root.join("build_cache").join("backend");
        probe_crate(&build);

        let args = cargo_config_args(&project, &build).unwrap();
        let status = std::process::Command::new("cargo")
            .current_dir(&build)
            .arg("build")
            .args(&args)
            .status()
            .expect("cargo runs");
        assert!(
            status.success(),
            "managed build with the project config fails"
        );

        // Sanity: the same build without the args fails — the probe crate
        // cannot compile without the `--cfg water_config_probe` marker.
        std::process::Command::new("cargo")
            .current_dir(&build)
            .arg("clean")
            .status()
            .unwrap();
        let status = std::process::Command::new("cargo")
            .current_dir(&build)
            .arg("build")
            .status()
            .expect("cargo runs");
        assert!(!status.success(), "probe crate compiled without the marker");
    }
}
