from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import zipfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable, Sequence

import yaml

ROOT = Path(__file__).resolve().parent.parent
MANIFEST_DIR = ROOT / "manifests"
DEFAULT_LOCK = ROOT / "sources.lock.json"

PLATFORM_OS = {
    "windows-x64": "windows",
    "linux-x64": "linux",
}
PLATFORM_ARCHIVE = {
    "windows-x64": "zip",
    "linux-x64": "tar.gz",
}

SHADER_MANIFEST = "shaders.yml"
SHADER_MANIFEST_SCHEMA = 2
SHADER_STATUSES = frozenset({"SAFE", "SAFE_WITH_NOTICE"})
SHADER_AUDIT_STATUSES = frozenset(
    {
        "SAFE",
        "SAFE_WITH_NOTICE",
        "NONCOMMERCIAL",
        "NO_REDISTRIBUTION",
        "UNKNOWN",
        "CLEAN_ROOM_REIMPLEMENT_CANDIDATE",
    }
)
UNSAFE_LICENSE_MARKERS = (
    "noncommercial",
    "-nc",
    "noassertion",
    "no-redistribution",
    "no_redistribution",
    "proprietary",
    "unknown",
)


class BuildError(RuntimeError):
    """A build precondition or verification step failed."""


def log(message: str) -> None:
    print(f"[build] {message}", flush=True)


def fail(message: str) -> None:
    raise BuildError(message)


def is_platform_selected(component: dict[str, Any], platform: str) -> bool:
    platforms = component.get("platforms", "all")
    if platforms == "all":
        return True
    return platform in platforms


def load_yaml(path: Path) -> dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        data = yaml.safe_load(handle)
    if not isinstance(data, dict):
        fail(f"{path}: expected a YAML mapping")
    return data


def is_safe_license(expression: Any) -> bool:
    """Reject license expressions that cannot be redistributed freely."""
    folded = str(expression or "").strip().casefold()
    if not folded:
        return False
    return not any(marker in folded for marker in UNSAFE_LICENSE_MARKERS)


def validate_shader_component(identifier: str, component: dict[str, Any]) -> None:
    """Validate shader-specific provenance before any network work.

    Every shader component shipped by this repository is code plus, for AI
    shaders, embedded model weights.  A code licence does not automatically
    cover the weights, so the manifest must state both.  Only components whose
    complete chain is known to permit redistribution belong in this manifest;
    non-free and unknown entries live in manifests/shader-audit.json instead.
    """
    family = str(component.get("family") or "")
    if not re.fullmatch(r"[a-z0-9][a-z0-9_-]*", family):
        fail(f"{identifier}: shader component needs a lowercase family slug")
    status = str(component.get("status") or "")
    if status not in SHADER_STATUSES:
        fail(
            f"{identifier}: shader status {status!r} is not distributable; "
            f"only {', '.join(sorted(SHADER_STATUSES))} may be listed"
        )

    license_info = component.get("license") or {}
    spdx = str(license_info.get("spdx") or "")
    if not is_safe_license(spdx):
        fail(f"{identifier}: shader license {spdx!r} is not redistributable")
    for part in license_info.get("parts") or []:
        if not is_safe_license(part.get("license")):
            fail(f"{identifier}: shader license part {part.get('license')!r} is not redistributable")
    license_paths = set()
    for item in license_info.get("files") or []:
        if not is_safe_license(item.get("license")):
            fail(
                f"{identifier}: shader license file {item.get('path')} uses "
                f"non-redistributable license {item.get('license')!r}"
            )
        license_paths.add(str(item.get("path")))

    if component.get("kind") == "raw-files":
        ref = str(component.get("ref") or "")
        if not re.fullmatch(r"[0-9a-f]{40}", ref):
            fail(
                f"{identifier}: raw shader ref must be a full 40-character commit, "
                f"not {ref!r}"
            )

    update = component.get("update")
    if update is not None:
        if not isinstance(update, dict):
            fail(f"{identifier}: update must be a mapping")
        mode = str(update.get("mode") or "pinned")
        if mode not in {"pinned", "latest-commit", "latest-tag"}:
            fail(f"{identifier}: unsupported update mode {mode!r}")
        if mode != "pinned" and component.get("kind") != "raw-files":
            fail(f"{identifier}: only raw-files components can follow upstream commits")
        marker = update.get("license_marker")
        if marker is not None and (not isinstance(marker, str) or not marker.strip()):
            fail(f"{identifier}: update.license_marker must be a non-empty string")

    model = component.get("model")
    if model is not None:
        if not isinstance(model, dict) or model.get("required") is not True:
            fail(f"{identifier}: model provenance must be a mapping with required: true")
        for field in (
            "name",
            "author",
            "source_repository",
            "source_ref",
            "source_paths",
            "license_spdx",
            "license_path",
        ):
            if not model.get(field):
                fail(f"{identifier}: model provenance is missing {field}")
        source_repository = str(model["source_repository"])
        if source_repository.count("/") != 1 or any(
            part in ("", ".", "..") for part in source_repository.split("/")
        ):
            fail(f"{identifier}: model source_repository is invalid")
        if not re.fullmatch(r"[0-9a-f]{40}", str(model["source_ref"])):
            fail(f"{identifier}: model source_ref must be a full commit")
        paths = model["source_paths"]
        if not isinstance(paths, list) or not paths:
            fail(f"{identifier}: model source_paths must be a non-empty list")
        for value in paths:
            path = Path(str(value))
            if not value or path.is_absolute() or ".." in path.parts:
                fail(f"{identifier}: model source path is unsafe: {value!r}")
        if not is_safe_license(model["license_spdx"]):
            fail(f"{identifier}: model license is not redistributable")
        model_license_path = str(model["license_path"])
        if model_license_path not in license_paths:
            fail(
                f"{identifier}: model license_path {model_license_path!r} "
                "must be one of the component's license.files"
            )
        for item in license_info.get("files") or []:
            if item.get("path") == model_license_path and item.get("license") != model.get("license_spdx"):
                fail(
                    f"{identifier}: model license_path {model_license_path!r} "
                    "carries a different SPDX id than model.license_spdx"
                )

    derived = component.get("derived_from")
    if derived is not None:
        if not isinstance(derived, list) or not derived:
            fail(f"{identifier}: derived_from must be a non-empty list")
        for item in derived:
            if not isinstance(item, dict):
                fail(f"{identifier}: every derived_from entry must be a mapping")
            repository = str(item.get("repository") or "")
            if repository.count("/") != 1 or any(
                part in ("", ".", "..") for part in repository.split("/")
            ):
                fail(f"{identifier}: derived_from repository is invalid")
            if not re.fullmatch(r"[0-9a-f]{40}", str(item.get("commit") or "")):
                fail(f"{identifier}: derived_from commit must be a full commit")
            path = Path(str(item.get("path") or ""))
            if not str(item.get("path")) or path.is_absolute() or ".." in path.parts:
                fail(f"{identifier}: derived_from path is unsafe")
            if not is_safe_license(item.get("license")):
                fail(f"{identifier}: derived_from license is not redistributable")


def load_manifests(manifest_dir: Path = MANIFEST_DIR) -> tuple[dict[str, Any], dict[str, dict[str, Any]]]:
    project_path = manifest_dir / "project.yml"
    if not project_path.is_file():
        fail(f"missing {project_path}")
    project = load_yaml(project_path)

    components: dict[str, dict[str, Any]] = {}
    for name in ("binaries.yml", "scripts.yml", "resources.yml", "shaders.yml"):
        path = manifest_dir / name
        if not path.is_file():
            fail(f"missing {path}")
        data = load_yaml(path)
        if name == SHADER_MANIFEST and data.get("schema") != SHADER_MANIFEST_SCHEMA:
            fail(
                f"{path}: schema must be {SHADER_MANIFEST_SCHEMA} "
                "(shader provenance and model licenses are required)"
            )
        items = data.get("components")
        if not isinstance(items, list):
            fail(f"{path}: components must be a list")
        for item in items:
            if not isinstance(item, dict) or not item.get("id"):
                fail(f"{path}: every component needs an id")
            identifier = str(item["id"])
            if identifier in components:
                fail(f"duplicate component id: {identifier}")
            item["_manifest"] = name
            components[identifier] = item

    for identifier, component in components.items():
        if not component.get("upstream"):
            fail(f"{identifier}: missing upstream")
        if component.get("_manifest") == SHADER_MANIFEST:
            validate_shader_component(identifier, component)
        kind = str(component.get("kind") or "binary")
        if kind == "raw-files":
            # Raw files are immutable blobs pinned by a repository commit.  The
            # manifest ref may be a tag or a commit, but resolve turns it into a
            # full commit; the lock records that commit and every file has its
            # own SHA-256.
            if not component.get("ref"):
                fail(f"{identifier}: raw-files component needs ref")
            files = component.get("files")
            if not isinstance(files, list) or not files:
                fail(f"{identifier}: raw-files component needs a non-empty files list")
            for item in files:
                if not isinstance(item, dict):
                    fail(f"{identifier}: every raw file entry must be a mapping")
                source = str(item.get("source") or "")
                target = str(item.get("target") or "")
                for label, value in (("source", source), ("target", target)):
                    path = Path(value)
                    if not value or path.is_absolute() or ".." in path.parts:
                        fail(f"{identifier}: raw file {label} is unsafe: {value!r}")
                if Path(source).suffix.lower() not in {".glsl", ".hook"}:
                    fail(f"{identifier}: raw file source must be .glsl or .hook: {source}")
                if Path(target).suffix.lower() != ".glsl" or not target.startswith(
                    "portable_config/shaders/"
                ):
                    fail(
                        f"{identifier}: raw file target must be a .glsl file under "
                        f"portable_config/shaders/: {target}"
                    )
                digest = str(item.get("sha256") or "")
                if not re.fullmatch(r"[0-9a-f]{64}", digest):
                    fail(f"{identifier}: raw file {source} needs a lowercase sha256")
                if not item.get("license"):
                    fail(f"{identifier}: raw file {source} needs a license")
        else:
            if not component.get("asset", {}).get("pattern"):
                fail(f"{identifier}: missing asset.pattern")
        license_info = component.get("license") or {}
        if not license_info:
            fail(f"{identifier}: missing license metadata")
        spdx = license_info.get("spdx")
        if not isinstance(spdx, str) or not spdx.strip():
            fail(f"{identifier}: license.spdx must be a non-empty SPDX expression or NOASSERTION")
        scope = license_info.get("scope")
        if scope is not None and (not isinstance(scope, str) or not scope.strip()):
            fail(f"{identifier}: license.scope must be a non-empty string")
        parts = license_info.get("parts")
        if parts is not None:
            if not isinstance(parts, list) or not parts:
                fail(f"{identifier}: license.parts must be a non-empty list")
            for part in parts:
                if not isinstance(part, dict) or not part.get("license") or not part.get("scope"):
                    fail(f"{identifier}: every license.parts entry needs license and scope")
        license_files = license_info.get("files") or []
        if not license_files:
            fail(f"{identifier}: license metadata has no files")
        for item in license_files:
            if not item.get("path"):
                fail(f"{identifier}: license file needs a path")
            if not item.get("license"):
                fail(f"{identifier}: license file {item.get('path')} needs an SPDX id")
            # License texts are pinned by repository + ref.  The ref may be a
            # tag, branch or short commit in the manifest, but resolve turns it
            # into a full commit and the lock/build only use that commit.
            # A literal URL is rejected so a floating branch cannot slip in.
            if item.get("url"):
                fail(
                    f"{identifier}: license file {item['path']} must use repository/ref, "
                    "not a literal url"
                )
            if not item.get("repository") or not item.get("ref"):
                fail(f"{identifier}: license file {item['path']} needs repository and ref")
            repository = str(item["repository"])
            if repository.count("/") != 1 or any(part in ("", ".", "..") for part in repository.split("/")):
                fail(f"{identifier}: license file {item['path']} has an invalid repository")
            source_path = str(item.get("source_path") or item["path"])
            if source_path.startswith("/") or ".." in Path(source_path).parts:
                fail(f"{identifier}: license file {item['path']} has an unsafe source_path")
    return project, components


def scoped_license_fields(license_info: dict[str, Any] | None) -> dict[str, Any]:
    """Return optional component-level license scope metadata.

    A component can bundle works under different licenses.  Its `spdx` value is
    only an aggregate; `scope` and `parts` record which license applies to
    which object, so consumers do not read them as cumulative terms on a single
    work.
    """
    info = license_info or {}
    result: dict[str, Any] = {}
    scope = info.get("scope")
    if scope:
        result["scope"] = str(scope)
    parts = info.get("parts")
    if parts:
        result["parts"] = [
            {"license": str(part["license"]), "scope": str(part["scope"])}
            for part in parts
        ]
    return result


def license_scope_for(license_info: dict[str, Any] | None, license_id: str | None) -> str | None:
    """Return the scope declared for one SPDX id inside a component."""
    for part in (license_info or {}).get("parts") or []:
        if part.get("license") == license_id:
            return str(part.get("scope")) or None
    return None


def license_files_summary(license_files: Any) -> list[dict[str, Any]]:
    """Content-only license file provenance used in manifests.

    The commit and URL are recorded in sources.lock.json and licenses/INDEX.json
    but deliberately not compared here: re-pinning a license source to a newer
    commit with unchanged text must not force a new release.
    """
    if not isinstance(license_files, list):
        return []
    summary = []
    for item in license_files:
        if not isinstance(item, dict):
            continue
        summary.append(
            {
                "path": item.get("path"),
                "license": item.get("license"),
                "sha256": item.get("sha256"),
            }
        )
    return sorted(summary, key=lambda entry: str(entry.get("path")))


def manifest_license_fields(license_info: dict[str, Any] | None) -> dict[str, Any]:
    """Component license fields for manifest.json and playkit-version.json."""
    info = license_info or {}
    fields: dict[str, Any] = {"license": info.get("spdx")}
    scoped = scoped_license_fields(info)
    if "scope" in scoped:
        fields["license_scope"] = scoped["scope"]
    if "parts" in scoped:
        fields["license_parts"] = scoped["parts"]
    fields["license_files"] = license_files_summary(info.get("files"))
    return fields


def shader_manifest_fields(component: dict[str, Any] | None) -> dict[str, Any]:
    """Shader provenance fields shared by package and release manifests."""
    component = component or {}
    fields: dict[str, Any] = {}
    for key in ("family", "status", "model", "derived_from"):
        value = component.get(key)
        if value:
            if key == "status":
                fields["license_status"] = value
            else:
                fields[key] = value
    return fields


def platform_of(component: dict[str, Any], platform: str) -> bool:
    return is_platform_selected(component, platform)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def canonical_json(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")


def canonical_hash(value: Any) -> str:
    return sha256_bytes(canonical_json(value))


def read_json(path: Path) -> Any:
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def write_json(path: Path, value: Any, indent: int = 2) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    text = json.dumps(value, ensure_ascii=False, indent=indent, sort_keys=True) + "\n"
    path.write_text(text, encoding="utf-8", newline="\n")


def now_iso() -> str:
    epoch = os.environ.get("SOURCE_DATE_EPOCH")
    if epoch:
        return datetime.fromtimestamp(int(epoch), tz=timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    return datetime.now(tz=timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def run(
    command: Sequence[str] | list[str],
    *,
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
    timeout: int | None = None,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    rendered = " ".join(str(part) for part in command)
    log(f"run: {rendered}")
    process = subprocess.run(
        [str(part) for part in command],
        cwd=str(cwd) if cwd else None,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout,
    )
    if check and process.returncode != 0:
        fail(
            f"command failed ({process.returncode}): {rendered}\n"
            f"stdout:\n{process.stdout[-4000:]}\n"
            f"stderr:\n{process.stderr[-4000:]}"
        )
    return process


def copy_tree(source: Path, destination: Path, *, ignore: Iterable[str] = ()) -> None:
    if not source.is_dir():
        fail(f"missing directory: {source}")
    shutil.copytree(
        source,
        destination,
        dirs_exist_ok=True,
        symlinks=True,
        ignore=shutil.ignore_patterns(*ignore) if ignore else None,
    )


def copy_file(source: Path, destination: Path, *, executable: bool = False) -> None:
    if not source.is_file():
        fail(f"missing file: {source}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)
    if executable:
        make_executable(destination)


def make_executable(path: Path) -> None:
    mode = path.stat().st_mode
    path.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)


def remove_tree(path: Path) -> None:
    if path.exists():
        if path.is_dir():
            shutil.rmtree(path)
        else:
            path.unlink()


def find_executable(names: Iterable[str], extra_paths: Iterable[str] = ()) -> str | None:
    for name in names:
        found = shutil.which(name)
        if found:
            return found
    for candidate in extra_paths:
        path = Path(candidate)
        if path.is_file():
            return str(path)
    return None


def find_7z() -> str:
    explicit = os.environ.get("SEVEN_ZIP_PATH")
    found = find_executable(
        ("7z", "7za", "7zr", "7z.exe"),
        tuple(
            item
            for item in (
                explicit,
                r"C:\Program Files\7-Zip\7z.exe",
                r"C:\Program Files (x86)\7-Zip\7z.exe",
                os.path.expandvars(r"%LOCALAPPDATA%\Programs\7-Zip\7z.exe"),
            )
            if item
        ),
    )
    if not found:
        fail("7-Zip is required (install p7zip-full on Linux or 7-Zip on Windows)")
    return found


def find_unsquashfs() -> str | None:
    return find_executable(("unsquashfs", "unsquashfs.exe"))


def safe_extract_zip(archive: Path, destination: Path) -> None:
    destination.mkdir(parents=True, exist_ok=True)
    root = destination.resolve()
    with zipfile.ZipFile(archive) as handle:
        for member in handle.infolist():
            target = (destination / member.filename).resolve()
            if not str(target).startswith(str(root) + os.sep) and target != root:
                fail(f"unsafe zip member: {member.filename}")
        handle.extractall(destination)


def tree_listing(root: Path) -> list[str]:
    if not root.is_dir():
        return []
    return sorted(str(path.relative_to(root)).replace("\\", "/") for path in root.rglob("*") if path.is_file())


def package_basename(version: str, platform: str) -> str:
    return f"mpv-PlayKit-{version}-{platform}"


def package_filename(version: str, platform: str) -> str:
    archive = PLATFORM_ARCHIVE.get(platform)
    if not archive:
        fail(f"unknown platform: {platform}")
    return f"{package_basename(version, platform)}.{archive}"


def human_size(value: int) -> str:
    for unit in ("B", "KiB", "MiB", "GiB"):
        if value < 1024 or unit == "GiB":
            return f"{value:.0f} {unit}" if unit == "B" else f"{value:.1f} {unit}"
        value /= 1024.0
    return f"{value:.1f} GiB"
