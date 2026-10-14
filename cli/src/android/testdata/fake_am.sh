#!/bin/sh
# Fake `am` inside the fake device: logs argv, exits under the staged status.
printf '%s\n' "$*" >> "$WATERUI_AM_LOG"
exit "${WATERUI_FAKE_AM_STATUS:-0}"
