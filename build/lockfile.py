from __future__ import annotations

import argparse
import re
from pathlib import Path

from .common import fail, log, read_json, sha256_file

SHA256 = re.compile(r"[0-9a-f]{64}")


def verify_lock_file(path: Path, expected_sha256: str) -> str:
    """Verify that a lock file is valid JSON with the expected exact bytes."""
    expected = str(expected_sha256 or "").strip().lower()
    if not SHA256.fullmatch(expected):
        fail(f"invalid expected sha256: {expected_sha256!r}")
    if not path.is_file():
        fail(f"missing lock file: {path}")
    read_json(path)
    actual = sha256_file(path)
    if actual != expected:
        fail(f"{path}: sha256 {actual} does not match the expected candidate lock {expected}")
    return actual


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Verify that sources.lock.json is exactly the candidate lock built in this run"
    )
    parser.add_argument("--lock", type=Path, required=True)
    parser.add_argument("--sha256", required=True)
    args = parser.parse_args(argv)

    actual = verify_lock_file(args.lock, args.sha256)
    log(f"verified {args.lock} sha256={actual}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
