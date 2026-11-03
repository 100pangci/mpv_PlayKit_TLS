from __future__ import annotations

import argparse
import json
import re
import shutil
import tempfile
from pathlib import Path
from typing import Any

import yaml

from .common import (
    MANIFEST_DIR,
    SHADER_MANIFEST_SCHEMA,
    fail,
    load_manifests,
    load_yaml,
    log,
    read_json,
    sha256_bytes,
    validate_shader_component,
    write_json,
)
from .shaders import AUDIT_FILENAME, load_shader_audit
from .upstream import (
    FULL_COMMIT,
    download_raw,
    latest_commit,
    latest_tag_commit,
    raw_url,
)

SHADER_MANIFEST_FILENAME = "shaders.yml"
RAW_LIMIT = 8 * 1024 * 1024


def _component_map(data: dict[str, Any]) -> dict[str, dict[str, Any]]:
    components = data.get("components")
    if not isinstance(components, list):
        fail(f"{SHADER_MANIFEST_FILENAME}: components must be a list")
    result: dict[str, dict[str, Any]] = {}
    for component in components:
        if not isinstance(component, dict):
            fail(f"{SHADER_MANIFEST_FILENAME}: every component must be a mapping")
        identifier = str(component.get("id") or "")
        if not identifier or identifier in result:
            fail(f"{SHADER_MANIFEST_FILENAME}: duplicate or empty component id {identifier!r}")
        result[identifier] = component
    return result


def _split_manifest(text: str) -> tuple[str, str]:
    """Split the manifest into its comment header and components section."""
    match = re.search(r"^components:[ \t]*$", text, flags=re.MULTILINE)
    if match is None:
        fail(f"{SHADER_MANIFEST_FILENAME}: missing top-level components key")
    prefix = text[: match.start()]
    if not prefix.endswith("\n"):
        prefix += "\n"
    return prefix, text[match.start() :]


def _download_text(url: str) -> bytes:
    return download_raw(url, limit=RAW_LIMIT)


def _check_license_text_unchanged(
    component: dict[str, Any],
    repository: str,
    old_commit: str,
    new_commit: str,
) -> None:
    """Refuse to follow a commit that also changes a same-repository license.

    A shader update is only mechanical while its licence stays the same.  If
    the upstream repository edits or replaces the licence text, a human must
    re-audit the component instead of accepting the new terms automatically.
    """
    identifier = str(component["id"])
    for item in (component.get("license") or {}).get("files") or []:
        if str(item.get("repository")) != repository:
            continue
        source_path = str(item.get("source_path") or item.get("path") or "")
        if not source_path:
            fail(f"{identifier}: license file has no source_path")
        old_url = raw_url(repository, old_commit, source_path)
        new_url = raw_url(repository, new_commit, source_path)
        old_text = _download_text(old_url)
        new_text = _download_text(new_url)
        if old_text != new_text:
            fail(
                f"{identifier}: upstream license text changed for {source_path} "
                f"between {old_commit} and {new_commit}; manual license audit required"
            )
        item["ref"] = new_commit
        log(f"{identifier}: license {source_path} unchanged at {new_commit}")


def refresh_component(component: dict[str, Any]) -> dict[str, str] | None:
    """Refresh one component to its upstream head.

    Returns ``{target: sha256}`` when the component moved and ``None`` when it
    was already current.  Any missing declared file, license change or manifest
    invariant violation raises BuildError; nothing is written by this helper.
    """
    identifier = str(component["id"])
    update = component.get("update") or {}
    mode = str(update.get("mode") or "pinned")
    if mode == "pinned":
        return None
    if component.get("kind") != "raw-files":
        fail(f"{identifier}: only raw-files components can follow upstream")
    repository = str(component.get("upstream") or "")
    old_commit = str(component.get("ref") or "")
    if not FULL_COMMIT.fullmatch(old_commit):
        fail(f"{identifier}: current ref must be a full commit")

    if mode == "latest-tag":
        new_commit, tag = latest_tag_commit(repository)
        version = tag
    elif mode == "latest-commit":
        new_commit, date = latest_commit(repository)
        if not date:
            fail(f"{identifier}: latest commit has no committer date")
        version = date
    else:
        fail(f"{identifier}: unsupported update mode {mode!r}")

    if new_commit == old_commit:
        log(f"{identifier}: already at {new_commit}")
        return None

    log(f"{identifier}: {old_commit} -> {new_commit} ({mode})")
    # Refuse a commit that changes the licence before accepting any file.
    _check_license_text_unchanged(component, repository, old_commit, new_commit)

    marker = str(update.get("license_marker") or "")
    changed: dict[str, str] = {}
    for item in component.get("files") or []:
        source = str(item["source"])
        target = str(item["target"])
        payload = _download_text(raw_url(repository, new_commit, source))
        if marker and marker.casefold() not in payload.decode("utf-8", errors="replace").casefold():
            fail(
                f"{identifier}: {source} no longer contains the license marker "
                f"{marker!r}; manual license audit required"
            )
        digest = sha256_bytes(payload)
        item["sha256"] = digest
        changed[target] = digest

    component["ref"] = new_commit
    component["version"] = version
    model = component.get("model")
    if isinstance(model, dict) and str(model.get("source_repository")) == repository:
        # Embedded weights are versioned with the shader files.  A separate
        # weight repository keeps its own pinned commit.
        model["source_ref"] = new_commit
    validate_shader_component(identifier, component)
    return changed


def _updated_reason(value: Any, old_commit: str, new_commit: str) -> Any:
    if not isinstance(value, str):
        return value
    return value.replace(old_commit, new_commit)


def _apply_upstream_commits(
    audit: dict[str, Any],
    updates: dict[str, tuple[str, str, dict[str, str]]],
) -> int:
    by_target: dict[str, tuple[str, str, str]] = {}
    for old_commit, new_commit, hashes in updates.values():
        for target, digest in hashes.items():
            by_target[target] = (old_commit, new_commit, digest)
    changed = 0
    for entry in audit.get("entries") or []:
        if not isinstance(entry, dict) or not entry.get("included_in_tls"):
            continue
        target = str(entry.get("tls_target") or "")
        update = by_target.get(target)
        if update is None:
            continue
        old_commit, new_commit, digest = update
        entry["tls_sha256"] = digest
        entry["upstream_commit"] = new_commit
        entry["reason"] = _updated_reason(entry.get("reason"), old_commit, new_commit)
        entry["replacement_source"] = _updated_reason(
            entry.get("replacement_source"), old_commit, new_commit
        )
        changed += 1
    return changed


def _emit_manifest(prefix: str, components: list[dict[str, Any]]) -> str:
    body = yaml.safe_dump(
        {"components": components},
        sort_keys=False,
        allow_unicode=True,
        width=200,
        default_flow_style=False,
    )
    return prefix + body


def _validate_candidate(
    manifest_dir: Path,
    manifest_text: str,
    audit: dict[str, Any],
) -> None:
    """Run the full manifest/audit validators against a candidate write.

    A scratch copy keeps check mode side-effect free while still exercising
    exactly the same code path that protects the real manifest.
    """
    with tempfile.TemporaryDirectory() as raw:
        candidate = Path(raw)
        for pattern in ("*.yml", "*.json"):
            for path in manifest_dir.glob(pattern):
                shutil.copy2(path, candidate / path.name)
        (candidate / SHADER_MANIFEST_FILENAME).write_text(
            manifest_text, encoding="utf-8", newline="\n"
        )
        write_json(candidate / AUDIT_FILENAME, audit)
        load_manifests(candidate)
        load_shader_audit(candidate)


def refresh(
    manifest_dir: Path = MANIFEST_DIR,
    *,
    check: bool = False,
) -> dict[str, Any]:
    """Refresh all unpinned shader components and return a change summary."""
    manifest_path = manifest_dir / SHADER_MANIFEST_FILENAME
    audit_path = manifest_dir / AUDIT_FILENAME
    if not manifest_path.is_file():
        fail(f"missing {manifest_path}")
    if not audit_path.is_file():
        fail(f"missing {audit_path}")

    text = manifest_path.read_text(encoding="utf-8")
    data = load_yaml(manifest_path)
    if data.get("schema") != SHADER_MANIFEST_SCHEMA:
        fail(f"{manifest_path}: schema must be {SHADER_MANIFEST_SCHEMA}")
    prefix, _ = _split_manifest(text)
    components = _component_map(data)
    audit = read_json(audit_path)
    if not isinstance(audit, dict):
        fail(f"{audit_path}: expected a mapping")

    updates: dict[str, tuple[str, str, dict[str, str]]] = {}
    for identifier, component in components.items():
        old_commit = str(component.get("ref") or "")
        hashes = refresh_component(component)
        if hashes is not None:
            updates[identifier] = (old_commit, str(component["ref"]), hashes)

    summary = {
        "changed_components": sorted(updates),
        "changed_files": sum(len(item[2]) for item in updates.values()),
        "updated_audit_entries": _apply_upstream_commits(audit, updates),
    }
    if not updates:
        return summary

    new_text = _emit_manifest(prefix, list(components.values()))
    _validate_candidate(manifest_dir, new_text, audit)
    if check:
        return summary

    manifest_path.write_text(new_text, encoding="utf-8", newline="\n")
    write_json(audit_path, audit)
    return summary


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Refresh unpinned shader manifests to upstream commits"
    )
    parser.add_argument("--manifests", type=Path, default=MANIFEST_DIR)
    parser.add_argument(
        "--check",
        action="store_true",
        help="report whether an update exists without writing files",
    )
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args(argv)

    summary = refresh(args.manifests, check=args.check)
    if args.json:
        print(json.dumps(summary, ensure_ascii=False, indent=2, sort_keys=True))
    else:
        changed = summary["changed_components"]
        log(
            "shader pins updated: "
            + (", ".join(changed) if changed else "no")
            + f" ({summary['changed_files']} files)"
        )
    return 1 if summary["changed_components"] else 0


if __name__ == "__main__":
    raise SystemExit(main())
