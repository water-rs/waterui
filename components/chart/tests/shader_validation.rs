use std::fs;
use std::path::Path;

use waterui_chart::renderer::base::shader_with_common;

#[test]
fn validate_all_wgsl_shaders() {
    let shaders_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/shaders");
    let mut errors = Vec::new();

    for entry in fs::read_dir(&shaders_dir).expect("Failed to read shaders directory") {
        let entry = entry.expect("Failed to read shader entry");
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("wgsl") {
            continue;
        }

        let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or_default();
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("Failed to read shader source: {file_name}"));

        let full_source = if file_name == "common.wgsl" {
            source
        } else {
            shader_with_common(&source)
        };

        let module = match wgpu::naga::front::wgsl::parse_str(&full_source) {
            Ok(module) => module,
            Err(err) => {
                errors.push(format!("{file_name}: WGSL parse error: {err}"));
                continue;
            }
        };

        let mut validator = wgpu::naga::valid::Validator::new(
            wgpu::naga::valid::ValidationFlags::all(),
            wgpu::naga::valid::Capabilities::all(),
        );

        if let Err(err) = validator.validate(&module) {
            errors.push(format!("{file_name}: WGSL validation error: {err}"));
        }
    }

    assert!(
        errors.is_empty(),
        "WGSL validation failed:\n{}",
        errors.join("\n")
    );
}
