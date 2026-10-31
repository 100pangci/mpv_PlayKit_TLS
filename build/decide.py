from __future__ import annotations

import argparse
import json
import os
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from .common import DEFAULT_LOCK, MANIFEST_DIR, fail, load_manifests, log, read_json
from .inputs import compute_inputs_hash
from .upstream import GitHubNotFound, download_asset_bytes, github_json, list_releases, lock_hash
from .version import next_release_version, tag_for_version


def compact_components(lock: dict[str, Any]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for identifier, component in sorted(lock.get("components", {}).items()):
        result[identifier] = {
            "version": component.get("version"),
            "upstream": component.get("upstream"),
            "release_tag": component.get("release_tag"),
            "asset_name": component.get("asset_name"),
            "sha256": component.get("sha256"),
        }
    return result


def release_components(manifest: dict[str, Any]) -> dict[str, Any]:
    """Normalize a published manifest to the same shape as compact_components.

    Release manifests carry extra fields (release_url, license); comparing the
    raw dictionaries would report an upstream change on every run.
    """
    components = manifest.get("components")
    if not isinstance(components, dict):
        return {}
    result: dict[str, Any] = {}
    for identifier, component in sorted(components.items()):
        if not isinstance(component, dict):
            continue
        result[identifier] = {
            "version": component.get("version"),
            "upstream": component.get("upstream"),
            "release_tag": component.get("release_tag"),
            "asset_name": component.get("asset_name"),
            "sha256": component.get("sha256"),
        }
    return result


def latest_release(repository: str) -> dict[str, Any] | None:
    try:
        release = github_json(f"https://api.github.com/repos/{repository}/releases/latest")
    except GitHubNotFound:
        return None
    if not isinstance(release, dict):
        fail("latest release response is invalid")
    return release


def latest_manifest(repository: str, release: dict[str, Any] | None) -> dict[str, Any] | None:
    if not release:
        return None
    for asset in release.get("assets", []):
        if asset.get("name") == "manifest.json":
            try:
                data = json.loads(download_asset_bytes(asset).decode("utf-8"))
            except Exception as exc:
                log(f"could not read latest manifest.json: {exc}")
                return None
            return data if isinstance(data, dict) else None
    log("latest release has no manifest.json asset")
    return None


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Decide whether this run needs a new PlayKit release")
    parser.add_argument("--manifests", type=Path, default=MANIFEST_DIR)
    parser.add_argument("--repository", default=None)
    parser.add_argument("--old-lock", type=Path, default=DEFAULT_LOCK)
    parser.add_argument("--new-lock", type=Path, required=True)
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--github-output", type=Path, default=None)
    parser.add_argument("--now", default=None, help="override UTC date, YYYY-MM-DD (for tests)")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)

    project, _ = load_manifests(args.manifests)
    repository = args.repository or project.get("repository")
    if not repository:
        fail("no repository configured")

    new_lock = read_json(args.new_lock)
    old_lock = read_json(args.old_lock) if args.old_lock.is_file() else {"components": {}}
    local_inputs_hash = compute_inputs_hash()

    release = latest_release(repository)
    manifest = latest_manifest(repository, release)

    lock_changed = (
        lock_hash(old_lock) != lock_hash(new_lock)
        or old_lock.get("inputs_hash") != new_lock.get("inputs_hash")
    )
    upstream_changed = manifest is None
    if manifest is not None:
        upstream_changed = release_components(manifest) != compact_components(new_lock)
    inputs_changed = manifest is None or manifest.get("inputs_hash") != local_inputs_hash

    release_needed = bool(args.force or release is None or upstream_changed or inputs_changed)
    reasons: list[str] = []
    if args.force:
        reasons.append("forced")
    if release is None:
        reasons.append("no existing release")
    if manifest is None:
        reasons.append("no readable latest manifest")
    elif upstream_changed:
        reasons.append("upstream components changed")
    if inputs_changed:
        reasons.append("config/scripts/manifests changed")

    releases = list_releases(repository)
    now = datetime.now(tz=timezone.utc)
    if args.now:
        now = datetime.fromisoformat(args.now).replace(tzinfo=timezone.utc)
    version = next_release_version([str(item.get("tag_name", "")) for item in releases], now)
    tag = tag_for_version(version, (project.get("release") or {}).get("tag_prefix", "v"))

    result = {
        "repository": repository,
        "release_needed": release_needed,
        "upstream_changed": upstream_changed,
        "inputs_changed": inputs_changed,
        "lock_changed": lock_changed,
        "version": version,
        "tag": tag,
        "reason": "; ".join(reasons) if reasons else "no changes",
        "latest_tag": release.get("tag_name") if release else None,
        "latest_manifest_version": (manifest or {}).get("playkit_version"),
        "lock_hash": lock_hash(new_lock),
        "inputs_hash": local_inputs_hash,
    }

    if args.github_output:
        with args.github_output.open("a", encoding="utf-8", newline="\n") as handle:
            for key, value in result.items():
                if isinstance(value, bool):
                    value = "true" if value else "false"
                if value is None:
                    value = ""
                handle.write(f"{key}={value}\n")

    if args.json:
        print(json.dumps(result, ensure_ascii=False, indent=2, sort_keys=True))
    else:
        log(
            f"release_needed={release_needed} version={version} tag={tag} "
            f"reason={result['reason']}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
