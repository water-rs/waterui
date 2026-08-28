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
opt-level = {{ opt_level_literal }}

# Release firmware is flash-budgeted: fat LTO plus a single codegen unit lets
# the linker see the whole program (dead-stripping the text-shaping and
# rasterization paths an app never reaches), and stripping drops symbol
# tables espflash does not need. Measured on the reference esp32c3 app,
# whole-program optimization cut the non-font flash image by ~15%
# (3.06 MB to 2.61 MB); together with font subsetting the whole image
# dropped 30% (3.83 MB to 2.70 MB).
[profile.release]
opt-level = {{ opt_level_literal }}
lto = "fat"
codegen-units = 1
strip = "symbols"
