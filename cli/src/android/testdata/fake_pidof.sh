#!/bin/sh
# Fake `pidof` inside the fake device: a staged failure never answers a pid.
if [ -n "${WATERUI_FAKE_PIDOF_FAIL-}" ]; then
    exit 1
fi
printf '4242\n'
