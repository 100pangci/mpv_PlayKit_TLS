from __future__ import annotations

import argparse
import os
import re
import subprocess
import tempfile
from pathlib import Path

from .common import PLATFORM_OS, ROOT, fail, log

FATAL_PATTERNS = re.compile(
    r"(Error parsing option|Failed to parse|script error|Lua error|"
    r"Error loading (?:Lua )?script|"
    r"Could not open|Failed to open|Traceback|No such file or directory)",
    re.IGNORECASE,
)


def run_test(command: list[str], *, cwd: Path, timeout: int, env: dict[str, str] | None = None) -> str:
    log("smoke: " + " ".join(str(part) for part in command))
    process = subprocess.run(
        [str(part) for part in command],
        cwd=str(cwd),
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout,
        env=env,
    )
    output = (process.stdout or "") + (process.stderr or "")
    if process.returncode != 0:
        fail(f"smoke command failed ({process.returncode}): {command}\n{output[-8000:]}")
    match = FATAL_PATTERNS.search(output)
    if match:
        fail(f"smoke output contains a fatal-looking message ({match.group(0)}):\n{output[-8000:]}")
    return output


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Run platform smoke tests on an assembled tree")
    parser.add_argument("--stage", type=Path, required=True)
    parser.add_argument("--platform", required=True, choices=sorted(PLATFORM_OS))
    parser.add_argument("--timeout", type=int, default=90)
    args = parser.parse_args(argv)

    stage = args.stage.resolve()
    sample = ROOT / "tests" / "assets" / "1x1.png"
    if not sample.is_file():
        fail(f"missing test image: {sample}")

    with tempfile.TemporaryDirectory(prefix="playkit-smoke-") as temporary:
        home = Path(temporary)
        env = {
            **os.environ,
            "HOME": str(home),
            "XDG_CONFIG_HOME": str(home / ".config"),
            "XDG_CACHE_HOME": str(home / ".cache"),
            "XDG_DATA_HOME": str(home / ".local" / "share"),
            "LC_ALL": "C.UTF-8",
        }
        env.pop("DISPLAY", None)
        env.pop("WAYLAND_DISPLAY", None)

        runtime_options = [
            "--vo=null",
            "--ao=null",
            "--frames=1",
            "--keep-open=no",
            "--idle=no",
            "--force-window=no",
            "--save-position-on-quit=no",
            "--script-opts-append=update-check-enabled=no",
            f"--script-opts-append=save_global_props-file={home / 'saved-props.json'}",
            f"--script-opts-append=thumbfast-cache_dir={home / 'thumbfast'}",
            f"--watch-later-dir={home / 'watch_later'}",
            f"--icc-cache-dir={home / 'icc'}",
            f"--gpu-shader-cache-dir={home / 'shader'}",
            "--msg-level=all=warn",
        ]

        if args.platform == "windows-x64":
            mpv = stage / "mpv.exe"
            ytdlp = stage / "yt-dlp.exe"
            config_dir = stage / "portable_config"
            version = run_test([mpv, "--version"], cwd=stage, timeout=args.timeout, env=env)
            log(version.splitlines()[0] if version.splitlines() else "mpv --version")
            run_test(
                [mpv, f"--config-dir={config_dir}", *runtime_options, str(sample)],
                cwd=stage,
                timeout=args.timeout,
                env=env,
            )
        elif args.platform == "linux-x64":
            launcher = stage / "mpv-playkit"
            ytdlp = stage / "yt-dlp"
            version = run_test([launcher, "--version"], cwd=stage, timeout=args.timeout, env=env)
            log(version.splitlines()[0] if version.splitlines() else "mpv --version")
            run_test(
                [launcher, *runtime_options, str(sample)],
                cwd=stage,
                timeout=args.timeout,
                env=env,
            )
            # thumbfast decodes frames with the AppImage's inner mpv binary,
            # not through the launcher.  Verify that entry point works on its
            # own so timeline thumbnails cannot fail silently.
            inner_mpv = stage / "mpv" / "bin" / "mpv"
            if not inner_mpv.is_file():
                fail(f"thumbfast helper binary is missing: {inner_mpv}")
            inner_version = run_test(
                [inner_mpv, "--no-config", "--version"],
                cwd=stage,
                timeout=args.timeout,
                env=env,
            )
            log(
                inner_version.splitlines()[0]
                if inner_version.splitlines()
                else "inner mpv --version"
            )
        else:
            fail(f"unknown platform: {args.platform}")

        ytdlp_version = run_test([ytdlp, "--version"], cwd=stage, timeout=args.timeout, env=env)
        log(f"yt-dlp {ytdlp_version.strip().splitlines()[0] if ytdlp_version.strip() else '--version ok'}")
    log(f"smoke tests passed for {args.platform}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
