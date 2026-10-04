[build]
target = "{{ ctx.esp32.resolved_target_triple() }}"

[target.{{ ctx.esp32.resolved_target_triple() }}]
linker = "ldproxy"
rustflags = ["--cfg", "espidf_time64"]

[unstable]
build-std = ["std", "panic_abort"]

[env]
MCU = "{{ ctx.esp32.chip }}"
ESP_IDF_VERSION = "v5.3.3"
ESP_IDF_TOOLS_INSTALL_DIR = "global"
