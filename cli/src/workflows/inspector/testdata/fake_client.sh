#!/bin/sh
# Fixture `waterui-inspector` for the client tests:
#  - WATERUI_FAKE_TOKEN_PROBE makes the binary exit 3 if the probe
#    environment carries WATERUI_INSPECTOR_TOKEN or
#    WATERUI_INSPECTOR_TARGET_ADDR — the session's credentials must not
#    leak into a `--identity` run;
#  - WATERUI_FAKE_HANG records the process's own pid in
#    WATERUI_FAKE_PID_FILE and sleeps — the deadline owner kills it;
#  - otherwise it prints the JSON in the file WATERUI_FAKE_IDENTITY names.
if [ -n "${WATERUI_FAKE_TOKEN_PROBE:-}" ]; then
    if [ -n "${WATERUI_INSPECTOR_TOKEN:-}" ] || [ -n "${WATERUI_INSPECTOR_TARGET_ADDR:-}" ]; then
        echo "probe environment carried session credentials" >&2
        exit 3
    fi
fi
if [ -n "${WATERUI_FAKE_HANG:-}" ]; then
    printf '%s\n' "$$" > "$WATERUI_FAKE_PID_FILE"
    exec /bin/sleep 60
fi
/bin/cat "$WATERUI_FAKE_IDENTITY"
