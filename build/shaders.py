from __future__ import annotations

import re
from pathlib import Path
from typing import Any

from .common import (
    MANIFEST_DIR,
    SHADER_AUDIT_STATUSES,
    SHADER_STATUSES,
    fail,
    is_safe_license,
    load_manifests,
    load_yaml,
    read_json,
)

SHADER_PREFIX = "portable_config/shaders/"
AUDIT_FILENAME = "shader-audit.json"
AUDIT_SCHEMA = 1
AUDIT_FIELDS = (
    "hooke_path",
    "family",
    "original_upstream",
    "upstream_path",
    "same_or_modified",
    "code_license",
    "model_license",
    "status",
    "reason",
    "replacement_source",
    "included_in_tls",
    "upstream_commit",
)


def shader_targets(manifest_dir: Path = MANIFEST_DIR) -> list[str]:
    """Return every GLSL target declared by manifests/shaders.yml.

    The build verifier allows .glsl files only when they are named here.  Both
    raw-file components and regular release assets can provide shaders, so the
    list is derived from the manifest rather than duplicated in build code.
    """
    path = manifest_dir / "shaders.yml"
    if not path.is_file():
        return []
    data = load_yaml(path)
    components = data.get("components")
    if not isinstance(components, list):
        return []
    targets: set[str] = set()
    for component in components:
        if not isinstance(component, dict):
            continue
        for item in component.get("files") or []:
            if not isinstance(item, dict):
                continue
            target = str(item.get("target") or "")
            if target.startswith(SHADER_PREFIX) and target.lower().endswith(".glsl"):
                targets.add(target)
        for item in component.get("install") or []:
            if not isinstance(item, dict) or item.get("tree"):
                continue
            source = str(item.get("source") or "")
            target = str(item.get("target") or "")
            if (
                source.lower().endswith(".glsl")
                and target.startswith(SHADER_PREFIX)
                and target.lower().endswith(".glsl")
            ):
                targets.add(target)
    return sorted(targets)


def _safe_relative(path: Any, suffix: str | None = None) -> bool:
    value = str(path or "")
    if not value:
        return False
    parsed = Path(value)
    if parsed.is_absolute() or ".." in parsed.parts:
        return False
    return suffix is None or parsed.suffix.lower() == suffix


def validate_shader_audit(
    data: dict[str, Any],
    manifest_dir: Path = MANIFEST_DIR,
) -> None:
    """Validate the machine-readable shader audit against the manifest.

    The audit covers the whole reference collection, including entries that
    must never be packaged.  Any entry marked as included must carry a safe
    status and license, and the set of included targets must exactly match the
    GLSL targets declared by manifests/shaders.yml.
    """
    if data.get("schema") != AUDIT_SCHEMA:
        fail(f"{AUDIT_FILENAME}: schema must be {AUDIT_SCHEMA}")
    baseline = data.get("baseline")
    if not isinstance(baseline, dict):
        fail(f"{AUDIT_FILENAME}: baseline must be a mapping")
    for key in ("repository", "tag"):
        if not baseline.get(key):
            fail(f"{AUDIT_FILENAME}: baseline.{key} is missing")
    if not re.fullmatch(r"[0-9a-f]{40}", str(baseline.get("commit") or "")):
        fail(f"{AUDIT_FILENAME}: baseline.commit must be a full commit")

    entries = data.get("entries")
    if not isinstance(entries, list) or not entries:
        fail(f"{AUDIT_FILENAME}: entries must be a non-empty list")
    declared = set(shader_targets(manifest_dir))
    _, components = load_manifests(manifest_dir)
    target_sha: dict[str, str] = {}
    target_source: dict[str, tuple[str, str, str | None]] = {}
    for component in components.values():
        if component.get("_manifest") != "shaders.yml":
            continue
        upstream = str(component.get("upstream") or "")
        ref = str(component.get("ref") or "") or None
        for item in component.get("files") or []:
            target = str(item["target"])
            target_sha[target] = str(item["sha256"])
            target_source[target] = (upstream, str(item["source"]), ref)
        for item in component.get("install") or []:
            target = str(item.get("target") or "")
            if not target.lower().endswith(".glsl"):
                continue
            target_sha[target] = str(component.get("sha256") or "")
            target_source[target] = (upstream, str(item.get("source") or ""), ref)
    included_targets: set[str] = set()
    baseline_entries = 0
    for index, entry in enumerate(entries):
        label = f"{AUDIT_FILENAME}: entries[{index}]"
        if not isinstance(entry, dict):
            fail(f"{label} must be a mapping")
        for field in AUDIT_FIELDS:
            if field not in entry:
                fail(f"{label} is missing {field}")
        if entry["hooke_path"] is not None:
            baseline_entries += 1
            if not _safe_relative(entry["hooke_path"], ".glsl"):
                fail(f"{label}: hooke_path is not a safe .glsl path")
        family = str(entry.get("family") or "")
        if not re.fullmatch(r"[a-z0-9][a-z0-9_-]*", family):
            fail(f"{label}: family must be a lowercase slug")
        status = str(entry.get("status") or "")
        if status not in SHADER_AUDIT_STATUSES:
            fail(f"{label}: unknown audit status {status!r}")
        if not str(entry.get("reason") or "").strip():
            fail(f"{label}: reason must not be empty")
        included = entry.get("included_in_tls")
        if not isinstance(included, bool):
            fail(f"{label}: included_in_tls must be a boolean")
        same_or_modified = str(entry.get("same_or_modified") or "")
        if same_or_modified not in {"same", "modified", "not_applicable", "upstream_only"}:
            fail(f"{label}: invalid same_or_modified value {same_or_modified!r}")
        if included:
            if status not in SHADER_STATUSES:
                fail(f"{label}: only SAFE or SAFE_WITH_NOTICE may be included")
            if not is_safe_license(entry.get("code_license")):
                fail(f"{label}: included shader has a non-redistributable code license")
            if entry.get("model_license") is not None and not is_safe_license(entry["model_license"]):
                fail(f"{label}: included shader has a non-redistributable model license")
            if not entry.get("original_upstream") or not entry.get("upstream_path"):
                fail(f"{label}: included shader needs original_upstream and upstream_path")
            if not _safe_relative(entry["upstream_path"]):
                fail(f"{label}: upstream_path is unsafe")
            if not re.fullmatch(r"[0-9a-f]{40}", str(entry.get("upstream_commit") or "")):
                fail(f"{label}: included shader needs a full upstream_commit")
            if not re.fullmatch(r"[0-9a-f]{64}", str(entry.get("tls_sha256") or "")):
                fail(f"{label}: included shader needs a tls_sha256")
            target = str(entry.get("tls_target") or "")
            if target not in declared:
                fail(f"{label}: included target is not declared by shaders.yml: {target!r}")
            expected_sha = target_sha.get(target)
            if expected_sha and entry["tls_sha256"] != expected_sha:
                fail(f"{label}: tls_sha256 is stale for {target}")
            source = target_source.get(target)
            if source is not None:
                upstream, upstream_path, ref = source
                if entry.get("original_upstream") != upstream:
                    fail(f"{label}: original_upstream disagrees with shaders.yml")
                if entry.get("upstream_path") != upstream_path:
                    fail(f"{label}: upstream_path disagrees with shaders.yml")
                if ref and entry.get("upstream_commit") != ref:
                    fail(f"{label}: upstream_commit disagrees with shaders.yml")
            included_targets.add(target)
        elif entry.get("tls_target") not in (None, ""):
            fail(f"{label}: excluded entry must not name a tls_target")

    missing = declared - included_targets
    if missing:
        fail(
            f"{AUDIT_FILENAME}: declared shader target has no included audit entry: "
            f"{sorted(missing)[0]}"
        )
    extra = included_targets - declared
    if extra:
        fail(f"{AUDIT_FILENAME}: audit includes an undeclared target: {sorted(extra)[0]}")
    expected = baseline.get("file_count")
    if expected is not None and baseline_entries != expected:
        fail(
            f"{AUDIT_FILENAME}: expected {expected} baseline entries, "
            f"found {baseline_entries}"
        )


def load_shader_audit(manifest_dir: Path = MANIFEST_DIR) -> dict[str, Any]:
    path = manifest_dir / AUDIT_FILENAME
    if not path.is_file():
        fail(f"missing {path}")
    data = read_json(path)
    if not isinstance(data, dict):
        fail(f"{path}: expected a mapping")
    validate_shader_audit(data, manifest_dir)
    return data