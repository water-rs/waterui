//! Emits `DEP_*_VERSION` environment variables with the exact dependency
//! versions resolved in `Cargo.lock`, for adapter provenance reporting.

use std::path::{Path, PathBuf};

/// Reads `(name, version, git-rev)` tuples out of a `Cargo.lock`; the rev is
/// the `#sha` fragment of a `git+` source, `None` for registry packages.
fn lockfile_packages(text: &str) -> Vec<(String, String, Option<String>)> {
    let mut out = Vec::new();
    let (mut name, mut version, mut rev) = (None::<String>, None::<String>, None);
    let mut flush =
        |name: &mut Option<String>, version: &mut Option<String>, rev: &mut Option<String>| {
            if let (Some(n), Some(v)) = (name.take(), version.take()) {
                out.push((n, v, rev.take()));
            }
        };
    for line in text.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            flush(&mut name, &mut version, &mut rev);
        } else if let Some(v) = line
            .strip_prefix("name = \"")
            .and_then(|s| s.strip_suffix('"'))
        {
            name = Some(v.to_owned());
        } else if let Some(v) = line
            .strip_prefix("version = \"")
            .and_then(|s| s.strip_suffix('"'))
        {
            version = Some(v.to_owned());
        } else if let Some(v) = line.strip_prefix("source = \"git+") {
            rev = v
                .rsplit('#')
                .next()
                .and_then(|s| s.strip_suffix('"'))
                .map(str::to_owned);
        }
    }
    flush(&mut name, &mut version, &mut rev);
    out
}

/// Runs `git <args>` inside `dir` and returns its trimmed stdout. Fails
/// fast: no `git` on PATH, or a nonzero exit (e.g. `dir` is not inside a
/// git checkout), panics instead of falling back to a guessed path.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|err| panic!("`git {}` failed to start: {err}", args.join(" ")));
    assert!(
        output.status.success(),
        "`git {}` failed in {}: {}",
        args.join(" "),
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git stdout is utf-8")
        .trim()
        .to_owned()
}

/// Absolutizes a `git rev-parse` path printed from `manifest`: relative
/// output is relative to that working directory. Canonicalizing also
/// resolves `..` segments; a missing result means the checkout git
/// reported is unusable, so panic.
fn git_path(manifest: &Path, raw: &str) -> PathBuf {
    let raw = Path::new(raw);
    let path = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        manifest.join(raw)
    };
    std::fs::canonicalize(&path)
        .unwrap_or_else(|err| panic!("git path {} does not exist: {err}", path.display()))
}

/// `CHERENKOV_GIT_SHA` is `git rev-parse HEAD`. The bench is a path
/// crate, so Cargo.lock has no revision for it; the external-cost
/// report prints this instead of a hand-typed `--head`.
fn emit_git_sha() {
    let manifest = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).to_path_buf();
    // Ask git where the repository metadata lives instead of assuming a
    // `.git` near the manifest. HEAD is per-worktree and sits in
    // `--git-dir`; loose refs and `packed-refs` are shared and sit in
    // `--git-common-dir`. `--show-toplevel` additionally fails outside a
    // work tree, so a bare repository errors out here.
    let rev_parse = |arg: &str| git(&manifest, &["rev-parse", arg]);
    let gitdir = git_path(&manifest, &rev_parse("--git-dir"));
    let commondir = git_path(&manifest, &rev_parse("--git-common-dir"));
    let _worktree_root = git_path(&manifest, &rev_parse("--show-toplevel"));
    // A `rerun-if-changed` path that does not exist keeps the build
    // script dirty forever, so only watch files that exist right now.
    let track = |path: PathBuf| {
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    };
    let head = gitdir.join("HEAD");
    track(head.clone());
    if let Ok(text) = std::fs::read_to_string(&head)
        && let Some(r) = text.strip_prefix("ref:")
    {
        let branch = commondir.join(r.trim());
        if branch.exists() {
            track(branch);
        } else if let Some(dir) = branch.ancestors().skip(1).find(|d| d.is_dir()) {
            // The ref lives only in `packed-refs`; the next commit
            // recreates it loose. A watched path must exist, so watch
            // the nearest existing ancestor directory — creating the
            // loose file inside it is what Cargo then notices.
            track(dir.to_path_buf());
        }
    }
    track(commondir.join("packed-refs"));
    let sha = git(&manifest, &["rev-parse", "HEAD"]);
    assert!(
        sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()),
        "git rev-parse HEAD returned {sha:?}"
    );
    println!("cargo:rustc-env=CHERENKOV_GIT_SHA={sha}");
}

/// Reads the workspace root `Cargo.lock`: the manifest's nearest ancestor
/// directory that has one. `bench/` sits one level below the root in the
/// standalone repository and three below it in water-rs/waterui's
/// `graphics/` tree.
fn workspace_lock(manifest: &str) -> Option<String> {
    let mut dir = Path::new(manifest);
    loop {
        let lock = dir.join("Cargo.lock");
        if lock.is_file() {
            println!("cargo:rerun-if-changed={}", lock.display());
            return std::fs::read_to_string(&lock).ok();
        }
        dir = dir.parent()?;
    }
}

fn main() {
    emit_git_sha();
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let Some(text) = workspace_lock(&manifest) else {
        return;
    };
    let packages = lockfile_packages(&text);
    for (pkg, env) in [
        ("vello", "DEP_VELLO"),
        ("vello_hybrid", "DEP_VELLO_HYBRID"),
        ("vello_cpu", "DEP_VELLO_CPU"),
        ("skia-safe", "DEP_SKIA_SAFE"),
        ("wgpu", "DEP_WGPU"),
        ("cherenkov-gpu", "DEP_CHERENKOV_GPU"),
        ("cherenkov-cpu", "DEP_CHERENKOV_CPU"),
    ] {
        if let Some((_, v, rev)) = packages.iter().find(|(n, _, _)| n == pkg) {
            println!("cargo:rustc-env={env}_VERSION={v}");
            if let Some(rev) = rev {
                println!("cargo:rustc-env={env}_SOURCE_REV={rev}");
            }
        }
    }
}
