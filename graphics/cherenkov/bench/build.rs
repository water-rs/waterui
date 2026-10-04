//! Emits `DEP_*_VERSION` environment variables with the exact dependency
//! versions resolved in `Cargo.lock`, for adapter provenance reporting.

use std::path::Path;

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

/// `CHERENKOV_GIT_SHA` is `git rev-parse HEAD`. The bench is a path
/// crate, so Cargo.lock has no revision for it; the external-cost
/// report prints this instead of a hand-typed `--head`.
fn emit_git_sha() {
    let manifest = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).to_path_buf();
    let repo = manifest.join("..");
    let dotgit = repo.join(".git");
    println!("cargo:rerun-if-changed={}", dotgit.display());
    let gitdir = if dotgit.is_file() {
        std::fs::read_to_string(&dotgit).ok().and_then(|text| {
            text.strip_prefix("gitdir:").map(|dir| {
                let dir = Path::new(dir.trim());
                if dir.is_absolute() {
                    dir.to_path_buf()
                } else {
                    repo.join(dir)
                }
            })
        })
    } else if dotgit.is_dir() {
        Some(dotgit)
    } else {
        None
    };
    if let Some(gitdir) = gitdir {
        let head = gitdir.join("HEAD");
        println!("cargo:rerun-if-changed={}", head.display());
        if let Ok(text) = std::fs::read_to_string(&head)
            && let Some(r) = text.strip_prefix("ref:")
        {
            println!("cargo:rerun-if-changed={}", gitdir.join(r.trim()).display());
        }
    }
    let output = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(&repo)
        .output()
        .unwrap_or_else(|err| panic!("git rev-parse HEAD failed to start: {err}"));
    assert!(
        output.status.success(),
        "git rev-parse HEAD failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let sha = String::from_utf8(output.stdout).expect("git sha is utf-8");
    let sha = sha.trim();
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
