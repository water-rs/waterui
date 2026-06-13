[build]
target = "xtensa-{{ ctx.esp32.chip }}-espidf"

[target.xtensa-{{ ctx.esp32.chip }}-espidf]
linker = "ldproxy"
rustflags = ["--cfg", "espidf_time64"]

[unstable]
build-std = ["std", "panic_abort"]

[env]
MCU = "{{ ctx.esp32.chip }}"
ESP_IDF_VERSION = "v5.3.3"
ESP_IDF_TOOLS_INSTALL_DIR = "global"
