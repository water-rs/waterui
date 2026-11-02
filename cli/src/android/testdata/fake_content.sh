#!/bin/sh
# Fake `content` provider CLI inside the fake device: logs argv, feeds
# `write` stdin to the capture file, and honors per-verb failure env.
printf '%s\n' "$*" >> "$WATERUI_CONTENT_LOG"
if [ -n "${WATERUI_FAKE_CONTENT_HANG-}" ]; then
    exec sleep "$WATERUI_FAKE_CONTENT_HANG"
fi
case "$1" in
write)
    cat > "$WATERUI_CONTENT_STDIN"
    # The SDK contract: provider errors are stderr text with a zero exit.
    if [ -n "${WATERUI_FAKE_CONTENT_STDERR-}" ]; then
        printf '%s\n' "$WATERUI_FAKE_CONTENT_STDERR" >&2
    fi
    exit "${WATERUI_FAKE_CONTENT_STATUS:-0}"
    ;;
delete)
    if [ -n "${WATERUI_FAKE_CONTENT_DELETE_STDERR-}" ]; then
        printf '%s\n' "$WATERUI_FAKE_CONTENT_DELETE_STDERR" >&2
    fi
    exit "${WATERUI_FAKE_CONTENT_DELETE_STATUS:-0}"
    ;;
esac
exit 0
