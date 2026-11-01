from __future__ import annotations

import argparse
import os
import shutil
from pathlib import Path

from .common import PLATFORM_OS, ROOT, fail, human_size, log, make_executable, run, sha256_file

HELPER_CRATE = ROOT / "updater" / "helper"
HELPER_NAME = "playkit-updater"
DEFAULT_TARGETS = {
    "windows-x64": "x86_64-pc-windows-msvc",
    "linux-x64": "x86_64-unknown-linux-musl",
}


def helper_filename(platform: str) -> str:
    if platform == "windows-x64":
        return f"{HELPER_NAME}.exe"
    if platform == "linux-x64":
        return HELPER_NAME
    fail(f"unknown platform: {platform}")


def default_output(platform: str) -> Path:
    return ROOT / "dist" / "updater" / platform / helper_filename(platform)


def helper_version() -> str:
    """Read the crate version without importing a TOML dependency."""
    try:
        import tomllib
    except ImportError:  # pragma: no cover - Python >= 3.11 is required
        fail("Python 3.11 or newer is required to read Cargo.toml")
    with (HELPER_CRATE / "Cargo.toml").open("rb") as handle:
        data = tomllib.load(handle)
    version = (data.get("package") or {}).get("version")
    if not isinstance(version, str) or not version:
        fail("updater/helper/Cargo.toml has no package.version")
    return version


def build(platform: str, target: str | None, target_dir: Path) -> Path:
    if platform not in PLATFORM_OS:
        fail(f"unknown platform: {platform}")
    target = target or DEFAULT_TARGETS[platform]
    env = {
        **os.environ,
        "CARGO_TARGET_DIR": str(target_dir.resolve()),
        "CARGO_INCREMENTAL": "0",
    }
    if platform == "windows-x64":
        # Avoid a dependency on the Visual C++ redistributable.  The helper is
        # launched by an installed player that may live on a clean machine.
        env["RUSTFLAGS"] = (env.get("RUSTFLAGS", "") + " -C target-feature=+crt-static").strip()
    run(
        [
            "cargo",
            "build",
            "--release",
            "--locked",
            "--manifest-path",
            HELPER_CRATE / "Cargo.toml",
            "--target",
            target,
        ],
        cwd=ROOT,
        env=env,
        timeout=1800,
    )
    binary = target_dir.resolve() / target / "release" / helper_filename(platform)
    if not binary.is_file():
        fail(f"cargo did not produce {binary}")
    return binary


def verify_binary(binary: Path, version: str) -> None:
    if binary.stat().st_size < 64 * 1024:
        fail(f"updater binary looks too small: {binary}")
    process = run([binary, "--version"], cwd=binary.parent, timeout=60)
    output = (process.stdout or "") + (process.stderr or "")
    if HELPER_NAME not in output or version not in output:
        fail(f"unexpected updater --version output: {output!r}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Build the Rust update helper")
    parser.add_argument("--platform", required=True, choices=sorted(PLATFORM_OS))
    parser.add_argument("--target", default=None, help="Rust target triple override")
    parser.add_argument("--target-dir", type=Path, default=ROOT / ".work" / "cargo-target")
    parser.add_argument("--out", type=Path, default=None)
    parser.add_argument(
        "--binary",
        type=Path,
        default=None,
        help="Copy a prebuilt binary instead of invoking cargo (packaging tests)",
    )
    args = parser.parse_args(argv)

    version = helper_version()
    output = (args.out or default_output(args.platform)).resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    if args.binary:
        source = args.binary.resolve()
        if not source.is_file():
            fail(f"prebuilt updater binary is missing: {source}")
    else:
        source = build(args.platform, args.target, args.target_dir)
    shutil.copy2(source, output)
    if args.platform == "linux-x64":
        make_executable(output)
    verify_binary(output, version)
    log(
        f"updater {version} for {args.platform}: {output} "
        f"({human_size(output.stat().st_size)}, sha256 {sha256_file(output)})"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
