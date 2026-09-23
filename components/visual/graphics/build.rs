//! Build-time WGSL validation and `WaterUI` graphics metadata emission.

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

fn template_default_for_line_prefix(prefix: &str) -> &'static str {
    let line_prefix = prefix
        .rsplit_once('\n')
        .map_or(prefix, |(_, line)| line)
        .trim_end();
    if line_prefix.ends_with(": bool =") {
        return "false";
    }
    if line_prefix.ends_with(": u32 =") {
        return "0u";
    }
    if line_prefix.ends_with(": i32 =") {
        return "0i";
    }
    "0.0"
}

fn materialize_wgsl_template(source: &str) -> String {
    let mut materialized = String::with_capacity(source.len());
    let mut cursor = 0usize;

    while let Some(offset) = source[cursor..].find("__") {
        let start = cursor + offset;
        let search_from = start + 2;
        let Some(end_offset) = source[search_from..].find("__") else {
            break;
        };
        let end = search_from + end_offset + 2;
        let token = &source[start..end];
        let inner = &token[2..token.len() - 2];
        if inner.is_empty()
            || !inner
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        {
            materialized.push_str(&source[cursor..end]);
            cursor = end;
            continue;
        }

        materialized.push_str(&source[cursor..start]);
        materialized.push_str(template_default_for_line_prefix(&source[..start]));
        cursor = end;
    }

    materialized.push_str(&source[cursor..]);
    materialized
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
        let source = materialize_wgsl_template(&raw_source);

        if is_fragment_only_shader(&raw_source) {
            let source = format!("{prelude_source}{source}");
            validate_wgsl_source(&path, &source);
        } else {
            validate_wgsl_source(&path, &source);
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
    if env::var_os("CARGO_FEATURE_GPU").is_some() {
        let shader_root = manifest_dir.join("src/shaders");
        waterui_build_support::shader::compile_wgsl_shader(
            shader_root.join("animated_mesh_gradient.wgsl"),
            "animated_mesh_gradient",
        );
        waterui_build_support::shader::compile_wgsl_shader(
            shader_root.join("mesh_gradient.wgsl"),
            "mesh_gradient",
        );
        waterui_build_support::shader::compile_wgsl_shader(
            shader_root.join("image_generator.wgsl"),
            "image_generator",
        );
        waterui_build_support::shader::compile_wgsl_shader(shader_root.join("blit.wgsl"), "blit");
        let flowing_gradient_source = format!(
            "{}{}",
            fs::read_to_string(shader_root.join("prelude.wgsl"))
                .expect("failed to read ShaderSurface prelude"),
            fs::read_to_string(shader_root.join("flowing_gradient.wgsl"))
                .expect("failed to read flowing gradient fragment")
        );
        waterui_build_support::shader::compile_wgsl_source(
            "shaders/flowing_gradient.wgsl#shader_surface",
            &flowing_gradient_source,
            "flowing_gradient",
        );
    }

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
