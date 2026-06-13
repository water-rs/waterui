#!/usr/bin/env bash
# Runs the dew demo firmware on the QEMU-emulated ESP32-C3 (RISC-V) and
# reports the serial sentinels (DEW_C3_*_US / DEW_C3_OK).
set -euo pipefail
cd "$(dirname "$0")"

ELF=target/riscv32imc-esp-espidf/debug/dew-esp32c3-demo
BIN="${TMPDIR:-/tmp}/dew-esp32c3-flash.bin"
LOG="${TMPDIR:-/tmp}/dew-c3-qemu-serial.log"
QEMU="$HOME/.local/esp-qemu/qemu/bin/qemu-system-riscv32"

espflash save-image --chip esp32c3 --merge --flash-size 4mb \
    --partition-table partitions.csv "$ELF" "$BIN" >/dev/null

rm -f "$LOG"
"$QEMU" -nographic -machine esp32c3 \
    -drive file="$BIN",if=mtd,format=raw \
    >"$LOG" 2>&1 &
QEMU_PID=$!

for _ in $(seq 1 120); do
    if grep -qE "DEW_C3_OK|panicked|abort\(\)|Guru Meditation" "$LOG"; then
        break
    fi
    sleep 1
done
kill "$QEMU_PID" 2>/dev/null || true

grep -E "DEW_|dew\[c3\]|panicked|abort|Guru Meditation|Backtrace" "$LOG" | head -25
grep -q "DEW_C3_OK" "$LOG"
