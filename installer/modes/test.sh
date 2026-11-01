#!/bin/sh
# Test mode: ignore the portable configuration and load only test.conf.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$ROOT"
exec "$ROOT/mpv/AppRun" --config=no --include="$ROOT/installer/modes/test.conf" "$@"