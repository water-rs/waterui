#!/usr/bin/env bash
# Runs the dew demo firmware on the QEMU-emulated ESP32-S3 and reports the
# serial sentinel lines (DEW_FIRST_FRAME_US / DEW_REBUILD_*_US / DEW_QEMU_OK).
set -euo pipefail
cd "$(dirname "$0")"

ELF=target/xtensa-esp32s3-espidf/debug/dew-esp32s3-demo
BIN="${TMPDIR:-/tmp}/dew-esp32s3-flash.bin"
LOG="${TMPDIR:-/tmp}/dew-qemu-serial.log"
QEMU="$HOME/.local/esp-qemu/qemu/bin/qemu-system-xtensa"

espflash save-image --chip esp32s3 --merge --flash-size 8mb \
    --partition-table partitions.csv "$ELF" "$BIN" >/dev/null

rm -f "$LOG"
EFUSE="${TMPDIR:-/tmp}/dew-qemu-efuse.bin"
# eFuse image: BLK2 word4 bits[0:2] = ADC calibration version 1, so startup
# reads (zero) calibration codes from eFuse instead of running hardware
# self-calibration, which hangs under QEMU's unemulated ADC.
python3 - "$EFUSE" <<'PYEOF'
import sys
data = bytearray(1024)
data[64] = 0x01
open(sys.argv[1], 'wb').write(bytes(data))
PYEOF

"$QEMU" -nographic -machine esp32s3 \
    -drive file="$BIN",if=mtd,format=raw \
    -drive file="$EFUSE",if=none,format=raw,id=efuse \
    -global driver=nvram.esp32s3.efuse,property=drive,value=efuse \
    >"$LOG" 2>&1 &
QEMU_PID=$!

for _ in $(seq 1 120); do
    if grep -qE "DEW_QEMU_OK|panicked|abort\(\)|Guru Meditation" "$LOG"; then
        break
    fi
    sleep 1
done
kill "$QEMU_PID" 2>/dev/null || true

grep -E "DEW_|dew:|panicked|abort|Guru Meditation|Backtrace" "$LOG" | head -25
grep -q "DEW_QEMU_OK" "$LOG"
