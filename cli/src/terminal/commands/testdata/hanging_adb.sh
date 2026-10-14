#!/bin/sh
printf '%s\n' "$*" >> "$WATERUI_FAKE_ADB_LOG"
exec /bin/sleep 10
