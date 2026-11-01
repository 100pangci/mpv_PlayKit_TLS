#!/bin/sh
# Pure mode: start mpv without any configuration.
# Useful to verify the bundled mpv build itself before blaming the config.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$ROOT"
exec "$ROOT/mpv/AppRun" --config=no --log-file=mpv-pure.log --vo=gpu-next \
	--idle=once --force-window=yes --keep-open=yes --hidpi-window-scale=no \
	--autofit-smaller=500x500 "$@"