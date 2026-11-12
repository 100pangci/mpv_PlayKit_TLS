from __future__ import annotations

import argparse
import json
import re
import shutil
import sys
import tempfile
from pathlib import Path
from typing import Any

from .common import (
    MANIFEST_DIR,
    BuildError,
    fail,
    load_manifests,
    load_yaml,
    log,
    validate_raw_target,
)
from .shader_update import _emit_manifest, _split_manifest_text, refresh_component

SCRIPTS_MANIFEST_FILENAME = "scripts.yml"
ALLOWED_UPDATE_MODES = frozenset({"pinned", "latest-commit", "latest-tag"})
RAW_LUA_POLICIES = frozenset({"script", "script-opt"})


def _component_map(data: dict[str, Any]) -> dict[str, dict[str, Any]]:
    components = data.get("components")
    if not isinstance(components, list):
        fail(f"{SCRIPTS_MANIFEST_FILENAME}: components must be a list")
    result: dict[str, dict[str, Any]] = {}
    for component in components:
        if not isinstance(component, dict):
            fail(f"{SCRIPTS_MANIFEST_FILENAME}: every component must be a mapping")
        identifier = str(component.get("id") or "")
        if not identifier or identifier in result:
            fail(
                f"{SCRIPTS_MANIFEST_FILENAME}: duplicate or empty component id "
                f"{identifier!r}"
            )
        result[identifier] = component
    return result


def _split_manifest(text: str) -> tuple[str, str]:
    """Split the manifest into its comment header and components section."""
    return _split_manifest_text(text, SCRIPTS_MANIFEST_FILENAME)


def _validate_candidate(manifest_dir: Path, manifest_text: str) -> None:
    """Run the full manifest validator against a scratch copy of the tree."""
    with tempfile.TemporaryDirectory() as raw:
        candidate = Path(raw)
        for path in manifest_dir.glob("*.yml"):
            shutil.copy2(path, candidate / path.name)
        (candidate / SCRIPTS_MANIFEST_FILENAME).write_text(
            manifest_text, encoding="utf-8", newline="\n"
        )
        load_manifests(candidate)


def _validate_script_component(identifier: str, component: dict[str, Any]) -> None:
    """A script component may only declare direct Lua/conf raw targets.

    The generic manifest loader already checks hashes and license metadata;
    this validator is the last gate before a refreshed commit can be written.
    """
    for item in component.get("files") or []:
        if not isinstance(item, dict):
            fail(f"{identifier}: every raw file entry must be a mapping")
        policy = validate_raw_target(
            str(item.get("source") or ""),
            str(item.get("target") or ""),
            identifier,
        )
        if policy not in RAW_LUA_POLICIES:
            fail(
                f"{identifier}: script raw file must target a direct .lua or "
                f".conf child: {item.get('target')!r}"
            )
        if not re.fullmatch(r"[0-9a-f]{64}", str(item.get("sha256") or "")):
            fail(f"{identifier}: raw file needs a lowercase sha256: {item.get('target')!r}")
    if not re.fullmatch(r"[0-9a-f]{40}", str(component.get("ref") or "")):
        fail(f"{identifier}: refreshed ref must be a full commit")
    for item in (component.get("license") or {}).get("files") or []:
        if str(item.get("repository")) != str(component.get("upstream")):
            continue
        if not re.fullmatch(r"[0-9a-f]{40}", str(item.get("ref") or "")):
            fail(f"{identifier}: same-repository license ref must be a full commit")


def _follows_upstream(component: dict[str, Any]) -> bool:
    """Only raw-lua components explicitly following HEAD are eligible."""
    if component.get("kind") != "raw-files":
        return False
    update = component.get("update") or {}
    if not isinstance(update, dict):
        fail(f"{component.get('id')}: update must be a mapping")
    mode = str(update.get("mode") or "pinned")
    if mode not in ALLOWED_UPDATE_MODES:
        fail(f"{component.get('id')}: unsupported update mode {mode!r}")
    if mode != "latest-commit":
        return False
    files = component.get("files")
    if not isinstance(files, list) or not files:
        fail(f"{component.get('id')}: raw-lua component needs a non-empty files list")
    if not any(
        isinstance(item, dict) and str(item.get("source") or "").lower().endswith(".lua")
        for item in files
    ):
        return False
    _validate_script_component(str(component.get("id")), component)
    return True


def refresh(
    manifest_dir: Path = MANIFEST_DIR,
    *,
    check: bool = False,
) -> dict[str, Any]:
    """Refresh ``latest-commit`` raw-lua components in scripts.yml.

    A candidate commit is only accepted when every declared file is still
    present, its license marker is intact, same-repository license texts are
    byte-identical, and the whole manifest set revalidates.  No floating URL
    is written: refs always remain full commits and resolve derives raw URLs
    from them.  ``--check`` reports changes without writing anything.
    """
    manifest_path = manifest_dir / SCRIPTS_MANIFEST_FILENAME
    if not manifest_path.is_file():
        fail(f"missing {manifest_path}")

    text = manifest_path.read_text(encoding="utf-8")
    data = load_yaml(manifest_path)
    prefix, _ = _split_manifest(text)
    components = _component_map(data)

    updates: dict[str, tuple[str, str, dict[str, str]]] = {}
    for identifier, component in components.items():
        if not _follows_upstream(component):
            continue
        old_commit = str(component.get("ref") or "")
        hashes = refresh_component(component, validate=_validate_script_component)
        if hashes is not None:
            updates[identifier] = (old_commit, str(component["ref"]), hashes)

    summary = {
        "changed_components": sorted(updates),
        "changed_files": sum(len(item[2]) for item in updates.values()),
    }
    if not updates:
        return summary

    new_text = _emit_manifest(prefix, list(components.values()))
    _validate_candidate(manifest_dir, new_text)
    if check:
        return summary

    manifest_path.write_text(new_text, encoding="utf-8", newline="\n")
    return summary


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Refresh unpinned Lua script manifests to upstream commits"
    )
    parser.add_argument("--manifests", type=Path, default=MANIFEST_DIR)
    parser.add_argument(
        "--check",
        action="store_true",
        help="report whether an update exists without writing files",
    )
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)

    try:
        summary = refresh(args.manifests, check=args.check)
    except (BuildError, OSError) as error:
        # Exit 1 is reserved for "pins changed"; a license/manifest/network
        # failure must be distinguishable so the monthly job fails closed.
        print(f"{error}", file=sys.stderr)
        return 2
    if args.json:
        print(json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True))
    else:
        changed = summary["changed_components"]
        log(
            "script pins updated: "
            + (", ".join(changed) if changed else "no")
            + f" ({summary['changed_files']} files)"
        )
    return 1 if summary["changed_components"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
