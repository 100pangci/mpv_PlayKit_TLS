#!/bin/sh
# Benchmark mode: ignore the portable configuration and load benchmark.conf.
# Linux defaults to the OpenGL backend; pass --profile=bench_api_2 to use
# Vulkan when the graphics stack supports it.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$ROOT"
exec "$ROOT/mpv/AppRun" --config=no \
	--include="$ROOT/installer/modes/benchmark.conf" \
	--profile=bench_api_3 "$@"