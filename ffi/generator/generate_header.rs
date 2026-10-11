//! Regenerates `ffi/waterui.h` from the `waterui-ffi` crate via cbindgen and
//! propagates the header to the native backend submodules.
//!
//! The generator lives in its own crate so that building it does not build the
//! framework: its dependencies are cbindgen and the `syn`/`prettyplease`
//! machinery the merge in `merge.rs` needs.
//!
//! `ffi/waterui.h` is one C interface shared by every native backend, so it
//! must carry the union of the exports across every target the backends ship.
//! cbindgen expands macros through `cargo rustc -Zunpretty=expanded` for the
//! host target only — a header generated on Linux drops every Apple-gated
//! export, and one generated on macOS drops the Linux and Android ones. This
//! generator instead expands the crate once per shipped target with an
//! explicit `--target`, unions the expanded syntax trees in a stable order,
//! and feeds the merged source to cbindgen for parsing and emission.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use cbindgen::{Builder, Config};

mod merge;

/// One `cargo rustc -Zunpretty=expanded` invocation the header is unioned
/// over. `features` are passed verbatim as the `--features` argument.
struct ExpansionTarget {
    /// Rust target triple passed to `--target`.
    triple: &'static str,
    /// Whether default features are enabled for this expansion.
    default_features: bool,
    /// Feature names joined into the `--features` argument.
    features: &'static str,
}

/// The targets the checked-in header is the union of, in expansion order.
///
/// `ffi` source gates only on `target_os` (`macos`, `ios`, `android`),
/// `target_vendor = "apple"`, and features, so one triple per distinct cfg
/// outcome covers every shipped target: `aarch64-apple-ios-sim` and the other
/// Android ABIs evaluate identically to the triples listed here.
///
/// Feature sets mirror what each backend build selects — Android builds
/// `android-jni` (mutually exclusive with `c-api`) plus every capability
/// feature, and the Apple iOS expansion omits the `header` aggregate because
/// its CEF dependency only compiles for macOS.
const EXPANSION_TARGETS: &[ExpansionTarget] = &[
    // `header` aggregates every optional C surface; see `ffi/Cargo.toml`.
    ExpansionTarget {
        triple: "aarch64-apple-darwin",
        default_features: true,
        features: "header,video",
    },
    ExpansionTarget {
        triple: "aarch64-apple-ios",
        default_features: true,
        features: "map,video",
    },
    ExpansionTarget {
        triple: "aarch64-linux-android",
        default_features: false,
        features: "std,android-jni,gpu,map,media,webview,video",
    },
    ExpansionTarget {
        triple: "x86_64-unknown-linux-gnu",
        default_features: false,
        features: "std,c-api,gpu,map,media,webview,video",
    },
];

fn main() -> ExitCode {
    // The expansions run `cargo rustc -- -Zunpretty=expanded`, which prints
    // expanded source instead of writing the declared artifacts.
    // Artifact-caching rustc wrappers (sccache) fail while collecting those
    // missing outputs, so the expansion subprocesses must run unwrapped. They
    // produce nothing cacheable; every real build keeps its wrapper.
    // SAFETY: called at the start of `main`, before any other thread exists.
    unsafe { env::set_var("RUSTC_WRAPPER", "") };
    let crate_dir = ffi_crate_dir();
    let toolchain = toolchain_name();

    let mut expansions = Vec::with_capacity(EXPANSION_TARGETS.len());
    for target in EXPANSION_TARGETS {
        match expand(&crate_dir, target, &toolchain) {
            Ok(source) => expansions.push(source),
            Err(error) => {
                eprintln!("{error}");
                return ExitCode::FAILURE;
            }
        }
    }

    let mut merged = syn::parse_file(&expansions[0])
        .expect("the first expansion is produced by rustc and must parse");
    for source in &expansions[1..] {
        let file = syn::parse_file(source).expect("an expansion produced by rustc must parse");
        merge::merge_items(&mut merged.items, file.items);
    }
    let merged_source = prettyplease::unparse(&merged);

    // The merged expansion is the union surface of `waterui-ffi`: feed it to
    // cbindgen as the crate root while the crate's own `src/lib.rs` is
    // temporarily swapped out, so dependency crates still resolve through the
    // normal manifest and source walk. The file is restored before exit.
    let lib_rs = crate_dir.join("src/lib.rs");
    let original = fs::read(&lib_rs).expect("failed to read the FFI crate root");
    let _restore = RestoreOnDrop {
        path: lib_rs.clone(),
        original,
    };
    fs::write(&lib_rs, merged_source).expect("failed to write the merged crate root");

    let mut config =
        Config::from_file(crate_dir.join("cbindgen.toml")).expect("failed to load cbindgen.toml");
    // The crate root on disk is already the merged expansion; cbindgen must
    // not expand again, or it would redo a host-only expansion and lose the
    // union.
    config.parse.expand.crates.clear();
    config.parse.expand.features = None;
    let bindings = Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
        .expect("Unable to generate bindings");
    let mut header_bytes = Vec::new();
    bindings.write(&mut header_bytes);
    let header_path = crate_dir.join("waterui.h");
    // Every native backend syncs its copy from `ffi/waterui.h` in its own
    // CI; none rides a gitlink in this tree any more.
    fs::write(&header_path, header_bytes).expect("failed to write generated header");
    ExitCode::SUCCESS
}

/// Expands `waterui-ffi` for `target` and returns the expanded source, or an
/// error naming the install command when the target's std is missing.
fn expand(crate_dir: &Path, target: &ExpansionTarget, toolchain: &str) -> Result<String, String> {
    assert_target_installed(target.triple, toolchain)?;

    let mut command = Command::new(env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned()));
    command
        .arg("rustc")
        .arg("--lib")
        .arg("--profile")
        .arg("check")
        .arg("--manifest-path")
        .arg(crate_dir.join("Cargo.toml"))
        .arg("--target")
        .arg(target.triple)
        .arg("-p")
        .arg("waterui-ffi");
    if !target.default_features {
        command.arg("--no-default-features");
    }
    if !target.features.is_empty() {
        command.arg("--features").arg(target.features);
    }
    command.arg("--").arg("-Zunpretty=expanded");
    command.env("CARGO_TERM_COLOR", "never");

    let output = command.output().map_err(|error| {
        format!(
            "error: failed to run `{command:?}` for `{}`: {error}",
            target.triple
        )
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: String = stderr
            .lines()
            .rev()
            .take(20)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        let hint = if stderr.contains("can't find crate for")
            || stderr.contains("target may not be installed")
        {
            format!(
                "\n       install it with: rustup target add {} --toolchain {toolchain}",
                target.triple
            )
        } else {
            String::new()
        };
        return Err(format!(
            "error: expansion for `{}` failed:{hint}\n{tail}",
            target.triple
        ));
    }
    let source = String::from_utf8(output.stdout).map_err(|error| {
        format!(
            "error: expansion for `{}` is not valid UTF-8: {error}",
            target.triple
        )
    })?;
    if source.trim().is_empty() {
        return Err(format!(
            "error: expansion for `{}` produced no output",
            target.triple
        ));
    }
    Ok(source)
}

/// Fails the run when `triple`'s standard library is not installed in the
/// active toolchain, naming the `rustup target add` command that fixes it.
fn assert_target_installed(triple: &str, toolchain: &str) -> Result<(), String> {
    let sysroot = Command::new(env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned()))
        .args(["--print", "sysroot"])
        .output()
        .map_err(|error| format!("error: failed to query the Rust sysroot: {error}"))?;
    let sysroot = String::from_utf8_lossy(&sysroot.stdout).trim().to_owned();
    if sysroot.is_empty() {
        return Err("error: `rustc --print sysroot` produced no output".to_owned());
    }
    if !Path::new(&sysroot)
        .join("lib/rustlib")
        .join(triple)
        .is_dir()
    {
        return Err(format!(
            "error: Rust target `{triple}` is not installed for toolchain `{toolchain}`.\n       install it with: rustup target add {triple} --toolchain {toolchain}"
        ));
    }
    Ok(())
}

/// Name of the rustup toolchain that runs this generator, derived from the
/// sysroot so the error hints name the toolchain `cargo +<name>` selected.
fn toolchain_name() -> String {
    Command::new(env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned()))
        .args(["--print", "sysroot"])
        .output()
        .ok()
        .and_then(|output| {
            PathBuf::from(String::from_utf8_lossy(&output.stdout).trim().to_owned())
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "<toolchain>".to_owned())
}

/// Directory of the `waterui-ffi` crate this generator binds, which is the
/// parent of this crate's own directory.
fn ffi_crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("failed to determine the FFI crate directory from the generator manifest path")
        .to_path_buf()
}

/// Restores the crate root it replaced when the run ends, however it ends.
struct RestoreOnDrop {
    path: PathBuf,
    original: Vec<u8>,
}

impl Drop for RestoreOnDrop {
    fn drop(&mut self) {
        if let Err(error) = fs::write(&self.path, &self.original) {
            eprintln!("error: failed to restore {}: {error}", self.path.display());
        }
    }
}
