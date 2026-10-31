from __future__ import annotations

import argparse
import json
import os
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from .common import (
    DEFAULT_LOCK,
    MANIFEST_DIR,
    fail,
    license_files_summary,
    load_manifests,
    log,
    read_json,
)
from .inputs import compute_inputs_hash
from .upstream import GitHubNotFound, download_asset_bytes, github_json, list_releases, lock_hash
from .version import next_release_version, tag_for_version


def _manifest_component_fields(component: dict[str, Any]) -> dict[str, Any]:
    """Fields written into the package and release manifests.

    Lock entries store license metadata as a nested mapping, while published
    manifests store the aggregate SPDX string plus optional sibling scope
    fields.  Normalize both shapes so a metadata-only lock change that affects
    the shipped manifest (for example release_url or license scope) still
    triggers a release.
    """
    license_info = component.get("license")
    if isinstance(license_info, dict):
        license_value = license_info.get("spdx")
        license_scope = license_info.get("scope")
        license_parts = license_info.get("parts")
        license_files = license_info.get("files")
    else:
        license_value = license_info
        license_scope = component.get("license_scope")
        license_parts = component.get("license_parts")
        license_files = component.get("license_files")
    return {
        "version": component.get("version"),
        # Windows and Linux mpv builds can use different commits; the commit is
        # part of the published manifest and therefore part of this comparison.
        "commit": component.get("commit"),
        "upstream": component.get("upstream"),
        "release_tag": component.get("release_tag"),
        "release_url": component.get("release_url"),
        "asset_name": component.get("asset_name"),
        "sha256": component.get("sha256"),
        "license": license_value,
        "license_scope": license_scope,
        "license_parts": license_parts,
        # Content hashes only: re-pinning a license source to a newer commit
        # with identical text must not force a new release.
        "license_files": license_files_summary(license_files),
    }


def compact_components(lock: dict[str, Any]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for identifier, component in sorted(lock.get("components", {}).items()):
        result[identifier] = _manifest_component_fields(component)
    return result


def release_components(manifest: dict[str, Any]) -> dict[str, Any]:
    """Normalize a published manifest to the same shape as compact_components.

    Only fields that are actually written into the package manifest are
    compared.  Extra release metadata (published_at, size, asset_id, ...) is
    ignored so a routine resolver run does not report an upstream change.
    """
    components = manifest.get("components")
    if not isinstance(components, dict):
        return {}
    result: dict[str, Any] = {}
    for identifier, component in sorted(components.items()):
        if not isinstance(component, dict):
            continue
        result[identifier] = _manifest_component_fields(component)
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


def decide_release(
    *,
    force: bool,
    release: dict[str, Any] | None,
    manifest: dict[str, Any] | None,
    old_lock: dict[str, Any],
    new_lock: dict[str, Any],
    local_inputs_hash: str,
) -> dict[str, Any]:
    """Pure decision function, kept separate so it is testable without network."""
    lock_changed = (
        lock_hash(old_lock) != lock_hash(new_lock)
        or old_lock.get("inputs_hash") != new_lock.get("inputs_hash")
    )
    upstream_changed = manifest is None
    if manifest is not None:
        upstream_changed = release_components(manifest) != compact_components(new_lock)
    inputs_changed = manifest is None or manifest.get("inputs_hash") != local_inputs_hash

    release_needed = bool(force or release is None or upstream_changed or inputs_changed)
    reasons: list[str] = []
    if force:
        reasons.append("forced")
    if release is None:
        reasons.append("no existing release")
    if manifest is None:
        reasons.append("no readable latest manifest")
    elif upstream_changed:
        reasons.append("upstream components changed")
    if inputs_changed:
        reasons.append("repository inputs changed")

    return {
        "release_needed": release_needed,
        "upstream_changed": upstream_changed,
        "inputs_changed": inputs_changed,
        "lock_changed": lock_changed,
        "reason": "; ".join(reasons) if reasons else "no changes",
    }


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

    decision = decide_release(
        force=args.force,
        release=release,
        manifest=manifest,
        old_lock=old_lock,
        new_lock=new_lock,
        local_inputs_hash=local_inputs_hash,
    )

    releases = list_releases(repository)
    now = datetime.now(tz=timezone.utc)
    if args.now:
        now = datetime.fromisoformat(args.now).replace(tzinfo=timezone.utc)
    version = next_release_version([str(item.get("tag_name", "")) for item in releases], now)
    tag = tag_for_version(version, (project.get("release") or {}).get("tag_prefix", "v"))

    result = {
        "repository": repository,
        **decision,
        "version": version,
        "tag": tag,
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
            f"release_needed={decision['release_needed']} version={version} tag={tag} "
            f"reason={decision['reason']}"
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
