# dew on ESP32-S3

Firmware demo running WaterUI's dew backend on ESP32-S3 (std on ESP-IDF),
plus a QEMU flow that needs no hardware.

## Build

Requires the espup Xtensa toolchain (`espup install --targets esp32s3`)
and `ldproxy`/`espflash` (`cargo install ldproxy espflash`).

```bash
source ~/export-esp.sh
cargo build                  # main demo
cargo build --bin hello      # minimal boot-verification binary
```

## Run in QEMU (no hardware)

```bash
./qemu-run.sh                # builds flash image, boots, asserts serial sentinels
```

Two QEMU-specific pieces of lore this script encodes:

- **eFuse image**: the `esp_adc` component registers a global constructor
  that runs ADC self-calibration at startup; QEMU does not emulate the
  ADC, so it hangs forever. Writing ADC calibration version 1 into the
  emulated eFuse (BLK2 word 4, bits 0–2) routes startup through the
  eFuse-read path instead. Without this the boot stalls right after the
  `eFuse: calibration efuse version does not match` line.
- `CONFIG_ESP_SYSTEM_MEMPROT_FEATURE=n`: QEMU does not emulate the
  ESP32-S3 permission-control hardware.

## Run on hardware

Verified on a Waveshare ESP32-S3-Touch-AMOLED-2.06 over its USB-C port
(native USB-Serial-JTAG; `CONFIG_ESP_CONSOLE_USB_SERIAL_JTAG=y` routes
the log console there):

```bash
espflash flash --partition-table partitions.csv \
    target/xtensa-esp32s3-espidf/debug/dew-esp32s3-demo
espflash monitor
```

## Current blocker

The esp-rs Xtensa backend miscompiles `vello_cpu`'s rasterization: any
anti-aliased fill crashes (on hardware and QEMU alike; toolchains
1.95/1.96; every opt level; both pixel pipelines; vello_cpu 0.0.9 and git
main). The identical code passes on wasm32-wasip1 and on desktop with the
scalar-fallback SIMD level forced, so the trigger is the code generator,
not dew or vello_cpu. `XTENSA_MISCOMPILE_ISSUE.md` is a ready-to-file
upstream report. Until it is fixed upstream, develop against the desktop
panel simulator (`cargo run -p waterui-dew --example watch_sim --features
simulator`) — it exercises every layer of the embedded flow except the
final flush sink.
