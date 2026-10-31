from __future__ import annotations

import hashlib
from pathlib import Path

from .common import ROOT, sha256_file

INPUT_PATHS = (
    "config",
    "scripts/local",
    "updater",
    "manifests",
    "VERSION",
    # These are copied into every package; changing them must trigger a build.
    "LICENSE",
    "NOTICE",
    "THIRD_PARTY.md",
    "LICENSE_PROVENANCE.md",
)


def input_files(root: Path = ROOT) -> list[Path]:
    files: list[Path] = []
    for relative in INPUT_PATHS:
        path = root / relative
        if path.is_file():
            files.append(path)
        elif path.is_dir():
            files.extend(
                child
                for child in path.rglob("*")
                if child.is_file() and "__pycache__" not in child.parts and child.suffix != ".pyc"
            )
    return sorted(set(files), key=lambda item: str(item.relative_to(root)).replace("\\", "/"))


def compute_inputs_hash(root: Path = ROOT) -> str:
    digest = hashlib.sha256()
    for path in input_files(root):
        relative = str(path.relative_to(root)).replace("\\", "/")
        digest.update(relative.encode("utf-8"))
        digest.update(b"\0")
        digest.update(sha256_file(path).encode("ascii"))
        digest.update(b"\n")
    return digest.hexdigest()
