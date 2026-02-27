use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn collect_wgsl_files(root: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root)
        .unwrap_or_else(|error| panic!("failed to read directory {}: {error}", root.display()))
    {
        let entry = entry.unwrap_or_else(|error| {
            panic!(
                "failed to read directory entry in {}: {error}",
                root.display()
            )
        });
        let path = entry.path();
        if path.is_dir() {
            collect_wgsl_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("wgsl") {
            out.push(path);
        }
    }
}

fn is_fragment_only_shader(source: &str) -> bool {
    source.contains("@fragment") && !source.contains("@vertex")
}

fn validate_wgsl_source(path: &Path, source: &str) {
    let flags = naga::valid::ValidationFlags::all();
    let capabilities = naga::valid::Capabilities::all();
    let module = naga::front::wgsl::parse_str(source)
        .unwrap_or_else(|error| panic!("WGSL parse error in {}: {error}", path.display()));

    let mut validator = naga::valid::Validator::new(flags, capabilities);
    validator
        .validate(&module)
        .unwrap_or_else(|error| panic!("WGSL validation error in {}: {error}", path.display()));
}

fn validate_wgsl_shaders(manifest_dir: &Path) {
    let shader_root = manifest_dir.join("src/shaders");
    let prelude_path = shader_root.join("prelude.wgsl");
    let prelude_source = fs::read_to_string(&prelude_path).unwrap_or_else(|error| {
        panic!("failed to read shader {}: {error}", prelude_path.display())
    });

    let mut files = Vec::new();
    collect_wgsl_files(&shader_root, &mut files);
    files.sort();

    for path in files {
        println!("cargo:rerun-if-changed={}", path.display());
        let raw_source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read shader {}: {error}", path.display()));

        if is_fragment_only_shader(&raw_source) {
            let source = format!("{prelude_source}{raw_source}");
            validate_wgsl_source(&path, &source);
        } else {
            validate_wgsl_source(&path, &raw_source);
        }
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=WATERUI_GRAPHICS_COMMIT");

    let manifest_dir = PathBuf::from(
        env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set for build script"),
    );
    validate_wgsl_shaders(&manifest_dir);

    if let Ok(commit) = env::var("WATERUI_GRAPHICS_COMMIT")
        && !commit.trim().is_empty()
    {
        println!("cargo:rustc-env=WATERUI_GRAPHICS_COMMIT={}", commit.trim());
        return;
    }

    let commit = Command::new("git")
        .args(["rev-parse", "--short=12", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_owned());

    println!("cargo:rustc-env=WATERUI_GRAPHICS_COMMIT={commit}");
}
