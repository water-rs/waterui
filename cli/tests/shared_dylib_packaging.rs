//! End-to-end check that a packaged binary finds every shared library its
//! dynamic section records.
//!
//! The fixture is the dependency arrangement `water build` produces for a
//! real project: a `dylib` crate pulled in over **git** — so Cargo hashes its
//! `-C metadata` into every `deps/` file name — linked into a binary with
//! `-Cprefer-dynamic`. rustc writes `deps/libwaterui_dylib-<metadata>.so`,
//! Cargo uplifts it to the unhashed `<profile>/libwaterui_dylib.so` the
//! artifact report names, and the binary's `DT_NEEDED` records the hashed
//! name. Staging the unhashed name ships a library the loader never looks
//! for (water-rs/cli#184); packaging must stage each needed library under
//! the name the binary itself records.
//!
//! The second fixture drives the same arrangement through the build path
//! `water run` takes: `RustBuild::build_binary` with
//! [`RustLinkage::SharedRuntime`] — `cargo rustc` on the backend crate's
//! `--bin` unit with `-Cprefer-dynamic`, loader search paths, and the
//! `-Cextra-filename` marker — so the artifact handed to packaging is the
//! marked `deps/` binary a `water run` produces (water-rs/cli#161).
//! Built twice, the second invocation reports the shared dylib unit
//! `fresh`, so the stale check must find the dep-info rustc wrote as
//! `deps/<crate>-<metadata>.d` — the hashed name a git-sourced dylib
//! carries (water-rs/cli#162).

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use target_lexicon::Triple;
use tempfile::{TempDir, tempdir};

use waterui_cli::build::{
    BuiltTarget, RustBuild, RustDynamicLibraries, RustLinkage, SharedExecutable,
    needed_shared_libraries,
};
use waterui_cli::project::{ManagedBackends, Project};
use waterui_cli::toolchain::Host;

/// Write `contents` to `path`, creating parent directories.
fn write(path: &Path, contents: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("create parent dir");
    std::fs::write(path, contents).expect("write file");
}

/// This machine with a fresh `CARGO_HOME` under `root`, so every `cargo`
/// the fixture spawns — directly or inside the CLI, which spawns through
/// the same host — is hermetic with respect to the machine's global Cargo
/// configuration. A `target.<triple|cfg>.rustflags` table in
/// `$CARGO_HOME/config.toml` — e.g. a linker selection like
/// `-C link-arg=-fuse-ld=mold` — outranks the fixture's own
/// `[build] rustflags` (Cargo's rustflags sources are mutually exclusive
/// and `target.*` wins), silently discarding the flags the fixture relies
/// on. The toolchain still resolves through rustup's `RUSTUP_HOME`, which
/// `CARGO_HOME` does not affect.
fn hermetic_host(root: &Path) -> Host {
    let home = root.join("cargo-home");
    std::fs::create_dir_all(&home).expect("create hermetic CARGO_HOME");
    Host::current().with_env("CARGO_HOME", home)
}

/// Run a fixture command to success or fail the test with its output.
fn run(command: &mut Command, what: &str) -> Output {
    let output = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|error| panic!("failed to spawn {what}: {error}"));
    assert!(
        output.status.success(),
        "{what} failed with {}:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// The executable Cargo reported building for `app`'s `--bin` unit.
fn built_executable(host: &Host, app_dir: &Path) -> PathBuf {
    let mut child = host
        .std_command("cargo")
        .args(["build", "--message-format=json"])
        .current_dir(app_dir)
        .env("CARGO_TERM_COLOR", "never")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn cargo build");
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .expect("piped stdout")
        .read_to_string(&mut stdout)
        .expect("read cargo messages");
    let status = child.wait().expect("wait on cargo build");
    assert!(status.success(), "fixture `cargo build` failed: {status}");

    for line in stdout.lines() {
        let Ok(message) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if message["reason"] != "compiler-artifact" {
            continue;
        }
        let kinds = message["target"]["kind"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if !kinds.iter().any(|kind| kind == "bin") {
            continue;
        }
        if let Some(executable) = message["executable"].as_str() {
            return PathBuf::from(executable);
        }
    }
    panic!("cargo reported no binary artifact for the fixture")
}

/// Whether `name` — a recorded dynamic dependency — is supplied by the
/// platform's own runtime rather than the packaged dist directory.
fn is_system_library(name: &str) -> bool {
    let base = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .to_lowercase();
    // ELF system libraries the platform loader resolves.
    for prefix in [
        "linux-vdso",
        "linux-gate",
        "ld-linux",
        "ld-musl",
        "ld64",
        "libc.so",
        "libc.musl",
        "libm.so",
        "libdl.so",
        "librt.so",
        "libutil.so",
        "libresolv.so",
        "libpthread.so",
        "libthread_db.so",
        "libgcc_s",
        "libgcc",
        "libatomic.so",
        "libstdc++",
        "libasan",
        "libubsan",
        "libnsl.so",
    ] {
        if base.starts_with(prefix) {
            return true;
        }
    }
    // PE imports Windows itself supplies.
    if cfg!(windows)
        && matches!(
            base.split('.').next().unwrap_or(""),
            "kernel32"
                | "ntdll"
                | "msvcrt"
                | "ucrtbase"
                | "vcruntime140"
                | "vcruntime140_1"
                | "msvcp140"
                | "concrt140"
                | "advapi32"
                | "user32"
                | "gdi32"
                | "shell32"
                | "ole32"
                | "oleaut32"
                | "ws2_32"
                | "wship6"
                | "iphlpapi"
                | "dnsapi"
                | "bcrypt"
                | "crypt32"
                | "secur32"
                | "sspicli"
                | "rpcrt4"
                | "netapi32"
                | "winmm"
                | "imm32"
                | "version"
                | "psapi"
                | "dbghelp"
                | "shlwapi"
                | "comctl32"
                | "comdlg32"
                | "setupapi"
                | "cfgmgr32"
                | "powrprof"
                | "userenv"
                | "kernel.appcore"
                | "normaliz"
                | "winspool"
                | "gdiplus"
                | "dwmapi"
                | "uxtheme"
                | "msimg32"
                | "authz"
                | "fwpuclnt"
                | "oleacc"
                | "winhttp"
                | "wininet"
        )
    {
        return true;
    }
    if base.starts_with("api-ms-") {
        return true;
    }
    // Mach-O install names the OS supplies.
    name.starts_with("/usr/lib/")
        || name.starts_with("/System/Library/")
        || name.starts_with("@rpath/libswift")
}

/// The file name the fixture's `waterui_dylib` crate uplifts to for this
/// target — the name `BuiltTarget::shared_runtime` carries.
fn shared_runtime_name(triple: &Triple) -> String {
    if triple.operating_system == target_lexicon::OperatingSystem::Windows {
        "waterui_dylib.dll".to_owned()
    } else if triple.operating_system == target_lexicon::OperatingSystem::Darwin(None) {
        "libwaterui_dylib.dylib".to_owned()
    } else {
        "libwaterui_dylib.so".to_owned()
    }
}

/// Vendor the fixture `waterui-dylib` crate into a git repository under
/// `root` — so Cargo hashes the source into every `deps/` file name — and
/// return its `file://` URL.
fn scaffold_dylib(host: &Host, root: &Path) -> String {
    let dylib_dir = root.join("waterui-dylib");
    write(
        &dylib_dir.join("Cargo.toml"),
        "[package]\nname = \"waterui-dylib\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\nname = \"waterui_dylib\"\ncrate-type = [\"dylib\", \"rlib\"]\n",
    );
    write(
        &dylib_dir.join("src/lib.rs"),
        "/// Marker the fixture binary calls so the linker keeps the dependency.\npub extern \"C\" fn fixture_marker() -> u8 {\n    42\n}\n",
    );
    run(
        host.std_command("git")
            .args(["init", "-q"])
            .current_dir(&dylib_dir),
        "git init",
    );
    run(
        host.std_command("git")
            .args(["add", "-A"])
            .current_dir(&dylib_dir),
        "git add",
    );
    run(
        host.std_command("git")
            .args([
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "user.name=fixture",
            ])
            .args(["commit", "-qm", "fixture"])
            .current_dir(&dylib_dir),
        "git commit",
    );

    url::Url::from_directory_path(&dylib_dir)
        .expect("dylib directory URL")
        .to_string()
}

/// Scaffold the fixture app under `root` and return its directory: a `dylib`
/// crate vendored in a local git repository — so Cargo hashes the source
/// into the dylib's `deps/` names — and a binary linking it dynamically.
fn scaffold_fixture(host: &Host, root: &Path) -> PathBuf {
    let dylib_url = scaffold_dylib(host, root);
    let app_dir = root.join("app");
    write(
        &app_dir.join("Cargo.toml"),
        &format!(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nwaterui-dylib = {{ git = \"{dylib_url}\" }}\n"
        ),
    );
    write(
        &app_dir.join("src/main.rs"),
        "fn main() {\n    // A referenced symbol keeps the `DT_NEEDED` the link would\n    // otherwise drop under `--as-needed`.\n    assert_eq!(waterui_dylib::fixture_marker(), 42);\n}\n",
    );
    write(
        &app_dir.join(".cargo/config.toml"),
        "[build]\nrustflags = [\"-Cprefer-dynamic\"]\n",
    );
    // The CLI opens a Water project; the resolve consults it for the
    // project's toolchain (which the fallback libstd lookup needs).
    write(
        &app_dir.join("Water.toml"),
        "[package]\nname = \"app\"\nbundle_identifier = \"dev.waterui.fixture\"\n",
    );
    app_dir
}

/// Scaffold the `water run` arrangement under `root` and return the `(app,
/// backend)` directories: an `app` library crate carrying the git-sourced
/// `waterui-dylib` dependency and the `dev` feature `with_linkage` enables,
/// and a standalone-workspace `backend` crate whose `--bin` calls into it —
/// the shape `build_hydrolysis` compiles for a project.
fn scaffold_run_fixture(host: &Host, root: &Path) -> (PathBuf, PathBuf) {
    let dylib_url = scaffold_dylib(host, root);

    let app_dir = root.join("app");
    write(
        &app_dir.join("Cargo.toml"),
        &format!(
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[features]\ndev = []\n\n[dependencies]\nwaterui-dylib = {{ git = \"{dylib_url}\" }}\n"
        ),
    );
    write(
        &app_dir.join("src/lib.rs"),
        "/// Entry point the generated backend's `main` calls.\npub fn run() {\n    // A referenced symbol keeps the `DT_NEEDED` the link would\n    // otherwise drop under `--as-needed`.\n    assert_eq!(waterui_dylib::fixture_marker(), 42);\n}\n",
    );
    write(
        &app_dir.join("Water.toml"),
        "[package]\nname = \"app\"\nbundle_identifier = \"dev.waterui.fixture\"\n",
    );

    let backend_dir = root.join("backend");
    write(
        &backend_dir.join("Cargo.toml"),
        "[package]\nname = \"backend\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\napp = { path = \"../app\" }\n\n[workspace]\n",
    );
    write(
        &backend_dir.join("src/main.rs"),
        "fn main() {\n    app::run();\n}\n",
    );
    (app_dir, backend_dir)
}

/// Assert every non-system dynamic dependency `executable` records exists in
/// `dist` under exactly the recorded file name.
fn assert_dist_satisfies_needed(executable: &Path, dist: &Path) {
    let needed = needed_shared_libraries(executable).expect("read dynamic dependencies");
    assert!(
        needed.iter().any(|name| name.contains("waterui_dylib")),
        "fixture binary records no waterui_dylib dependency: {needed:?}"
    );
    for name in &needed {
        if is_system_library(name) {
            continue;
        }
        let file_name = name.rsplit(['/', '\\']).next().unwrap_or(name);
        assert!(
            dist.join(file_name).is_file(),
            "dist is missing {file_name} which {} records as needed\nstaged: {:?}",
            executable.display(),
            std::fs::read_dir(dist)
                .map(|entries| entries
                    .flatten()
                    .map(|entry| entry.file_name())
                    .collect::<Vec<_>>())
                .unwrap_or_default(),
        );
    }
}

/// A command that runs `executable` on `host` with `library_dir` on the
/// platform loader's search path.
fn command_with_library_dir(host: &Host, executable: &Path, library_dir: &Path) -> Command {
    let mut command = host.std_command(executable);
    if cfg!(windows) {
        let mut paths = vec![library_dir.to_path_buf()];
        paths.extend(host.path_entries());
        command.env("PATH", std::env::join_paths(paths).expect("join PATH"));
    } else if cfg!(target_os = "macos") {
        command.env("DYLD_FALLBACK_LIBRARY_PATH", library_dir);
    } else {
        command.env("LD_LIBRARY_PATH", library_dir);
    }
    command
}

/// Run the staged `executable` with `dist` on the library search path and
/// assert it exits successfully.
fn assert_staged_binary_runs(host: &Host, executable: &Path, dist: &Path) {
    let staged_exe = dist.join(executable.file_name().expect("exe name"));
    std::fs::copy(executable, &staged_exe).expect("stage executable");
    let output = command_with_library_dir(host, &staged_exe, dist)
        .output()
        .expect("launch staged binary");
    assert!(
        output.status.success(),
        "staged binary failed: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Run `executable` where it already lives, with `runtime_dir` — the profile
/// directory it sits in — on the loader search path, the way `water mcp`
/// spawns the binary it just staged.
fn assert_binary_runs_in_place(host: &Host, executable: &Path, runtime_dir: &Path) {
    let output = command_with_library_dir(host, executable, runtime_dir)
        .output()
        .expect("launch rebuilt binary");
    assert!(
        output.status.success(),
        "rebuilt binary failed against the restaged runtime: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Build a binary that links a git-sourced `dylib` crate dynamically, run
/// `RustDynamicLibraries::resolve` + `stage` the way platform packaging
/// does, and assert the dist directory contains every non-system library the
/// binary's own dynamic records name — then run the staged binary.
#[test]
fn packaged_binary_finds_every_shared_library_it_records() {
    let temporary: TempDir = tempdir().expect("tempdir");
    let host = hermetic_host(temporary.path());
    smol::block_on(async {
        let root = temporary.path();
        let app_dir = scaffold_fixture(&host, root);

        let executable = built_executable(&host, &app_dir);
        let profile_dir = app_dir.join("target/debug");
        let triple = Triple::host();
        let shared_runtime = profile_dir.join(shared_runtime_name(&triple));
        assert!(
            shared_runtime.is_file(),
            "cargo did not produce the shared runtime {}",
            shared_runtime.display()
        );

        let built = BuiltTarget {
            profile_dir: profile_dir.clone(),
            artifact: executable.clone(),
            executable: Some(SharedExecutable::unlocked(executable.clone())),
            entry_binary: None,
            shared_runtime: Some(shared_runtime),
            app_library: None,
            cef_helper: None,
        };
        let project = Project::open(&host, &app_dir, ManagedBackends::NONE)
            .await
            .expect("open fixture project");

        let libraries = RustDynamicLibraries::resolve(&built, &triple, &project)
            .await
            .expect("resolve shared libraries");
        let dist = root.join("dist");
        libraries
            .stage(&dist)
            .await
            .expect("stage shared libraries");

        assert_dist_satisfies_needed(&executable, &dist);
        assert_staged_binary_runs(&host, &executable, &dist);
    });
}

/// Run the binary a `water run` build produces — `RustBuild::build_binary`
/// under [`RustLinkage::SharedRuntime`], the `cargo rustc` invocation whose
/// `water_build_marker_*` `--cfg` lands the artifact at
/// `deps/<name>-<marker>` — through the
/// packaging resolve + stage, and assert the dist directory satisfies every
/// shared library the binary's own dynamic section records, then run the
/// staged binary (water-rs/cli#161).
#[test]
fn run_built_binary_finds_every_shared_library_it_records() {
    let temporary: TempDir = tempdir().expect("tempdir");
    let host = hermetic_host(temporary.path());
    smol::block_on(async {
        let root = temporary.path();
        let (app_dir, backend_dir) = scaffold_run_fixture(&host, root);

        let triple = Triple::host();
        let built = RustBuild::new(&host, &backend_dir, triple.clone())
            .with_target_dir(root.join("target"))
            .with_linkage(RustLinkage::SharedRuntime, "app/dev", &["$ORIGIN"])
            .build_binary("backend", false)
            .await
            .expect("build the fixture backend binary");
        let project = Project::open(&host, &app_dir, ManagedBackends::NONE)
            .await
            .expect("open fixture project");

        let libraries = RustDynamicLibraries::resolve(&built, &triple, &project)
            .await
            .expect("resolve shared libraries");
        let dist = root.join("dist");
        libraries
            .stage(&dist)
            .await
            .expect("stage shared libraries");

        assert_dist_satisfies_needed(&built.artifact, &dist);
        assert_staged_binary_runs(&host, &built.artifact, &dist);
    });
}

/// Build the binary a `water run` produces — `RustBuild::build_binary` under
/// [`RustLinkage::SharedRuntime`] — twice on one target directory. The
/// second `cargo rustc` invocation reports the shared dylib unit `fresh`,
/// so the stale check must locate the dep-info rustc wrote as
/// `deps/<crate>-<metadata>.d`: the recorded name the dylib's own dynamic
/// section carries when the dependency comes from git (water-rs/cli#162).
#[test]
fn a_second_shared_runtime_build_finds_the_dylib_dep_info() {
    let temporary: TempDir = tempdir().expect("tempdir");
    let host = hermetic_host(temporary.path());
    smol::block_on(async {
        let root = temporary.path();
        let (_app_dir, backend_dir) = scaffold_run_fixture(&host, root);

        let build = RustBuild::new(&host, &backend_dir, Triple::host())
            .with_target_dir(root.join("target"))
            .with_linkage(RustLinkage::SharedRuntime, "app/dev", &["$ORIGIN"]);
        build
            .build_binary("backend", false)
            .await
            .expect("build the fixture backend binary");
        build
            .build_binary("backend", false)
            .await
            .expect("rebuild the fixture backend binary on the warm cache");
    });
}

/// `cargo rustc --bin` on unchanged inputs reports the unit `fresh` and
/// emits nothing: the second build's artifact is the first's — the same
/// inode, the same mtime — proving neither the compile nor the marked
/// relink touched the file a `rerun-if-changed` consumer would notice
/// (#2073).
#[test]
fn an_unchanged_build_leaves_the_binary_artifact_untouched() {
    let temporary: TempDir = tempdir().expect("tempdir");
    let host = hermetic_host(temporary.path());
    smol::block_on(async {
        let root = temporary.path();
        let (_app_dir, backend_dir) = scaffold_run_fixture(&host, root);
        let build = RustBuild::new(&host, &backend_dir, Triple::host())
            .with_target_dir(root.join("target"))
            .with_linkage(RustLinkage::SharedRuntime, "app/dev", &["$ORIGIN"]);

        let first = build
            .build_binary("backend", false)
            .await
            .expect("first fixture build");
        let first_metadata = std::fs::metadata(&first.artifact).expect("first artifact metadata");
        // `first` still holds the artifact lock the next `build_binary`
        // waits on; the assertions only need its captured metadata.
        let first_artifact = first.artifact.clone();
        drop(first);
        let second = build
            .build_binary("backend", false)
            .await
            .expect("second fixture build on the warm cache");
        let second_metadata =
            std::fs::metadata(&second.artifact).expect("second artifact metadata");

        assert_eq!(
            first_artifact, second.artifact,
            "an unchanged build reports the same marked artifact path"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            assert_eq!(
                first_metadata.ino(),
                second_metadata.ino(),
                "a fresh unit must not re-emit the artifact: the inode changed"
            );
        }
        assert_eq!(
            first_metadata.modified().expect("first mtime"),
            second_metadata.modified().expect("second mtime"),
            "a fresh unit must not re-emit the artifact: the mtime changed"
        );
    });
}

/// Re-resolve the vendored `waterui-dylib` at a bumped version: a new
/// package id hashes into a new `-C metadata` suffix on every `deps/` name,
/// the way a framework update does between two `water mcp` runs sharing one
/// target directory.
fn bump_vendored_dylib(host: &Host, root: &Path, backend_dir: &Path) {
    let dylib_dir = root.join("waterui-dylib");
    write(
        &dylib_dir.join("Cargo.toml"),
        "[package]\nname = \"waterui-dylib\"\nversion = \"0.2.0\"\nedition = \"2021\"\n\n[lib]\nname = \"waterui_dylib\"\ncrate-type = [\"dylib\", \"rlib\"]\n",
    );
    run(
        host.std_command("git")
            .args(["add", "-A"])
            .current_dir(&dylib_dir),
        "git add",
    );
    run(
        host.std_command("git")
            .args([
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "user.name=fixture",
            ])
            .args(["commit", "-qm", "bump"])
            .current_dir(&dylib_dir),
        "git commit",
    );
    run(
        host.std_command("cargo")
            .args(["update", "-p", "waterui-dylib"])
            .current_dir(backend_dir),
        "cargo update waterui-dylib",
    );
}

/// The file name part of a recorded dynamic dependency.
fn needed_name_file(recorded_name: &str) -> String {
    recorded_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(recorded_name)
        .to_owned()
}

/// The shared libraries a directory holds for `waterui_dylib`, sorted — the
/// platform's shared-library prefix and suffix, so Cargo's import library,
/// export file and PDB (`waterui_dylib.dll.lib`, `.dll.exp`, `.pdb` on
/// Windows) are never mistaken for a staged runtime.
fn staged_waterui_names(dir: &Path) -> Vec<String> {
    let prefix = format!("{}waterui_dylib", std::env::consts::DLL_PREFIX);
    let suffix = std::env::consts::DLL_SUFFIX;
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("read staged dir")
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            (name.starts_with(&prefix) && name.ends_with(suffix)).then_some(name)
        })
        .collect();
    names.sort_unstable();
    names
}

/// Stage into the Cargo profile directory twice across a dylib `-C
/// metadata` change — the arrangement `water mcp` uses, where the staged
/// runtime lives beside the `<profile>/<name>` uplift the launch path
/// executes — and assert the second stage lands the newly needed
/// `libwaterui_dylib-<hash>.so` and removes the stale hash the first stage
/// left, then run the uplift itself (water-rs/cli#176).
#[test]
fn restaging_replaces_a_stale_hashed_shared_runtime() {
    let temporary: TempDir = tempdir().expect("tempdir");
    let host = hermetic_host(temporary.path());
    smol::block_on(async {
        let root = temporary.path();
        let (app_dir, backend_dir) = scaffold_run_fixture(&host, root);
        let triple = Triple::host();
        let target_dir = root.join("target");
        let project = Project::open(&host, &app_dir, ManagedBackends::NONE)
            .await
            .expect("open fixture project");

        let built = RustBuild::new(&host, &backend_dir, triple.clone())
            .with_target_dir(target_dir.clone())
            .with_linkage(RustLinkage::SharedRuntime, "app/dev", &["$ORIGIN"])
            .build_binary("backend", false)
            .await
            .expect("first fixture build");
        let runtime_dir = built.profile_dir.clone();
        RustDynamicLibraries::resolve(&built, &triple, &project)
            .await
            .expect("resolve first shared libraries")
            .stage(&runtime_dir)
            .await
            .expect("stage first runtime");
        let first_waterui: Vec<String> = needed_shared_libraries(&built.artifact)
            .expect("first needed")
            .into_iter()
            .filter(|name| name.contains("waterui_dylib"))
            .collect();
        assert_eq!(
            staged_waterui_names(&runtime_dir).as_slice(),
            first_waterui
                .iter()
                .map(|name| needed_name_file(name))
                .collect::<Vec<_>>()
                .as_slice(),
            "first stage must leave exactly the recorded runtime name"
        );

        bump_vendored_dylib(&host, root, &backend_dir);

        // `built`'s artifact lock must close before the rebuild waits on it.
        drop(built);
        let rebuilt = RustBuild::new(&host, &backend_dir, triple.clone())
            .with_target_dir(target_dir)
            .with_linkage(RustLinkage::SharedRuntime, "app/dev", &["$ORIGIN"])
            .build_binary("backend", false)
            .await
            .expect("rebuild after dylib bump");
        let rebuilt_needed: Vec<String> = needed_shared_libraries(&rebuilt.artifact)
            .expect("rebuilt needed")
            .into_iter()
            .filter(|name| name.contains("waterui_dylib"))
            .collect();
        assert_ne!(
            first_waterui, rebuilt_needed,
            "the fixture must change the recorded dylib name"
        );
        RustDynamicLibraries::resolve(&rebuilt, &triple, &project)
            .await
            .expect("resolve rebuilt shared libraries")
            .stage(&runtime_dir)
            .await
            .expect("restage runtime");

        assert_eq!(
            staged_waterui_names(&runtime_dir).as_slice(),
            rebuilt_needed
                .iter()
                .map(|name| needed_name_file(name))
                .collect::<Vec<_>>()
                .as_slice(),
            "restaging must replace the stale hashed runtime with the recorded one"
        );
        assert_binary_runs_in_place(
            &host,
            rebuilt
                .executable()
                .expect("the rebuilt unit reports an executable"),
            &runtime_dir,
        );
    });
}
