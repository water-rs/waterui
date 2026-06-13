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
waterui-dew = { path = "{{ dew_path }}", default-features = false, features = ["espidf"] }
{%- else %}
waterui-dew = { version = "{{ dew_version }}", default-features = false, features = ["espidf"] }
{%- endif %}
{%- if let Some(core_path) = core_path %}
waterui-core = { path = "{{ core_path }}" }
{%- else %}
waterui-core = "{{ waterui_version }}"
{%- endif %}

[build-dependencies]
embuild = "0.33"

# The upstream Xtensa LLVM backend currently miscompiles the CPU
# rasterization stack at higher optimization levels (corrupted strip
# indices, LoadProhibited crashes). Size-optimize everything until the
# upstream backend is fixed.
[profile.dev]
opt-level = "s"

[profile.release]
opt-level = "s"
