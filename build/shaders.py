from __future__ import annotations

from pathlib import Path

from .common import MANIFEST_DIR, load_yaml

SHADER_PREFIX = "portable_config/shaders/"


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