[package]
name = "{{ package_name }}"
version = "0.1.0"
edition = "2024"
publish = false

[workspace]

[[bin]]
name = "{{ package_name }}"
path = "src/main.rs"

[dependencies]
esp-idf-svc = "0.52"
{{ app_crate_name }} = { path = "{{ app_crate_path }}" }
{%- if let Some(dew_path) = dew_path %}
waterui-dew = { path = "{{ dew_path }}", default-features = false, features = ["espidf", "progress"] }
{%- else %}
waterui-dew = { version = "{{ dew_version }}", default-features = false, features = ["espidf", "progress"] }
{%- endif %}
{%- if let Some(core_path) = core_path %}
waterui-core = { path = "{{ core_path }}" }
{%- else %}
waterui-core = "{{ waterui_version }}"
{%- endif %}

[build-dependencies]
embuild = "0.33"

# Codegen optimization level for the firmware. On Xtensa the upstream LLVM
# backend miscompiles the CPU rasterization stack at higher levels (corrupted
# strip indices, LoadProhibited crashes), so those chips size-optimize ("s");
# RISC-V chips use the mainline backend and build at full optimization ("2").
[profile.dev]
opt-level = "{{ opt_level }}"

[profile.release]
opt-level = "{{ opt_level }}"
