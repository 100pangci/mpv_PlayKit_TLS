from __future__ import annotations

import argparse
import hashlib
import os
import re
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

from .common import (
    DEFAULT_LOCK,
    BuildError,
    fail,
    human_size,
    log,
    read_json,
    sha256_file,
    write_json,
)

USER_AGENT = "mpv-PlayKit-build/1.0"


def safe_name(value: str) -> str:
    cleaned = re.sub(r"[^A-Za-z0-9_.-]+", "_", value)
    return cleaned or "unknown"


def selected(component: dict[str, Any], platform: str | None) -> bool:
    if platform is None:
        return True
    platforms = component.get("platforms", "all")
    return platforms == "all" or platform in platforms


def download(url: str, destination: Path, *, timeout: int = 180, retries: int = 3) -> tuple[str, int]:
    if not url:
        fail(f"no download URL for {destination}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    last_error: Exception | None = None
    for attempt in range(1, retries + 1):
        temporary = destination.with_suffix(destination.suffix + f".part{attempt}")
        try:
            request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
            digest = hashlib.sha256()
            size = 0
            with urllib.request.urlopen(request, timeout=timeout) as response, temporary.open("wb") as handle:
                while True:
                    block = response.read(1024 * 1024)
                    if not block:
                        break
                    digest.update(block)
                    handle.write(block)
                    size += len(block)
            if size < 32:
                raise BuildError(f"downloaded file is suspiciously small ({size} bytes): {url}")
            os.replace(temporary, destination)
            return digest.hexdigest(), size
        except Exception as exc:  # noqa: BLE001 - retried below
            last_error = exc
            try:
                temporary.unlink(missing_ok=True)
            except OSError:
                pass
            if attempt < retries:
                wait = attempt * 2
                log(f"download failed (attempt {attempt}/{retries}), retrying in {wait}s: {exc}")
                time.sleep(wait)
    raise BuildError(f"failed to download {url}: {last_error}")


def fetch_component(component: dict[str, Any], work: Path) -> dict[str, Any]:
    identifier = component["id"]
    asset_name = component["asset_name"]
    destination = work / "downloads" / identifier / asset_name
    expected = component.get("sha256")
    if destination.is_file() and expected and sha256_file(destination) == expected:
        log(f"cached {identifier}: {destination.name} ({human_size(destination.stat().st_size)})")
        actual, size = expected, destination.stat().st_size
    else:
        log(f"fetching {identifier}: {component.get('url')}")
        actual, size = download(component["url"], destination)
        if expected and actual != expected:
            destination.unlink(missing_ok=True)
            fail(f"{identifier}: sha256 mismatch: expected {expected}, got {actual}")
        if not expected:
            log(f"{identifier}: no digest in lock; recorded {actual}")

    license_entries = []
    version_dir = safe_name(str(component.get("version") or component.get("release_tag") or "unknown"))
    for item in (component.get("license") or {}).get("files", []):
        relative = Path(item["path"])
        if relative.is_absolute() or ".." in relative.parts:
            fail(f"{identifier}: unsafe license path {relative}")
        license_dest = work / "licenses" / identifier / version_dir / relative
        log(f"fetching license {identifier}/{relative}")
        license_hash, license_size = download(item["url"], license_dest)
        license_entries.append(
            {
                "path": str(relative).replace("\\", "/"),
                "local_path": str(license_dest.relative_to(work)).replace("\\", "/"),
                "license": item.get("license"),
                "url": item["url"],
                "sha256": license_hash,
                "size": license_size,
            }
        )

    return {
        "id": identifier,
        "asset_name": asset_name,
        "asset_path": str(destination.relative_to(work)).replace("\\", "/"),
        "sha256": actual,
        "size": size,
        "version": component.get("version"),
        "release_tag": component.get("release_tag"),
        "license_files": license_entries,
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Download and verify locked upstream assets")
    parser.add_argument("--lock", type=Path, default=DEFAULT_LOCK)
    parser.add_argument("--work", type=Path, default=Path(".work"))
    parser.add_argument("--platform", default=None)
    parser.add_argument("--timeout", type=int, default=180)
    args = parser.parse_args(argv)

    lock = read_json(args.lock)
    work = args.work.resolve()
    work.mkdir(parents=True, exist_ok=True)
    fetched: dict[str, Any] = {}
    for identifier, component in sorted(lock.get("components", {}).items()):
        if not selected(component, args.platform):
            continue
        fetched[identifier] = fetch_component(component, work)
    output = work / "fetched.json"
    write_json(output, {"schema": 1, "components": fetched})
    log(f"fetched {len(fetched)} components -> {output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
