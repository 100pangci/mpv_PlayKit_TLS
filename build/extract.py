from __future__ import annotations

import fnmatch
import shutil
import subprocess
from pathlib import Path

from .common import fail, find_7z, find_unsquashfs, log, make_executable, remove_tree, safe_extract_zip

# Hooks that would update the AppImage or download another yt-dlp at runtime.
# Other upstream hooks (CA certificates, path mapping, namespaces, ...) are
# part of the AppDir runtime and must be kept.
DEFAULT_REMOVED_HOOKS = ("*self-updater*.hook", "*get-yt-dlp*.hook")


def hook_is_removed(name: str, patterns: tuple[str, ...] | list[str]) -> bool:
    return any(fnmatch.fnmatchcase(name, pattern) for pattern in patterns)


def extract_7z(archive: Path, destination: Path, members: list[str] | None = None) -> None:
    seven = find_7z()
    remove_tree(destination)
    destination.mkdir(parents=True, exist_ok=True)
    command = [seven, "x", "-y", f"-o{destination}", str(archive)]
    if members:
        command.extend(members)
    process = subprocess.run(command, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    if process.returncode != 0:
        fail(f"7z extraction failed for {archive}\n{process.stdout[-4000:]}\n{process.stderr[-4000:]}")
    for member in members or []:
        if not (destination / member).exists():
            fail(f"{archive}: expected member missing after extraction: {member}")


def extract_zip(archive: Path, destination: Path) -> None:
    remove_tree(destination)
    safe_extract_zip(archive, destination)


def find_squashfs_offset(archive: Path) -> int:
    marker = b"hsqs"
    overlap = len(marker) - 1
    offset = 0
    carry = b""
    with archive.open("rb") as handle:
        while True:
            block = handle.read(1024 * 1024)
            if not block:
                break
            data = carry + block
            index = data.find(marker)
            if index != -1:
                return offset - len(carry) + index
            carry = data[-overlap:] if overlap else b""
            offset += len(block)
    fail(f"could not find squashfs magic in {archive}")


def extract_appimage(
    archive: Path,
    destination: Path,
    *,
    remove_hooks: bool | list[str] = True,
) -> None:
    remove_tree(destination)
    destination.parent.mkdir(parents=True, exist_ok=True)
    unsquashfs = find_unsquashfs()
    extracted = False
    if unsquashfs:
        offset = find_squashfs_offset(archive)
        log(f"extracting AppImage squashfs at offset {offset} with unsquashfs")
        process = subprocess.run(
            [unsquashfs, "-o", str(offset), "-d", str(destination), str(archive)],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=600,
        )
        if process.returncode == 0 and (destination / "AppRun").exists():
            extracted = True
        else:
            log(f"unsquashfs failed, falling back to --appimage-extract: {process.stderr[-1000:]}")
            remove_tree(destination)

    if not extracted:
        log("extracting AppImage with its own runtime (--appimage-extract)")
        make_executable(archive)
        temporary = destination.parent / (destination.name + "-appimage-extract")
        remove_tree(temporary)
        temporary.mkdir(parents=True, exist_ok=True)
        process = subprocess.run(
            [str(archive), "--appimage-extract"],
            cwd=str(temporary),
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=600,
        )
        root = temporary / "AppDir"
        if not root.is_dir():
            # Older AppImage runtimes extract to squashfs-root.  uruntime
            # creates AppDir and a squashfs-root symlink to it, so prefer the
            # real directory and resolve symlinks before moving it.
            root = temporary / "squashfs-root"
        if root.is_symlink():
            root = root.resolve()
        if process.returncode != 0 or not root.is_dir():
            fail(
                f"AppImage extraction failed for {archive}\n"
                f"{process.stdout[-4000:]}\n{process.stderr[-4000:]}"
            )
        log(f"extracted AppDir at {root}")
        shutil.move(str(root), str(destination))
        remove_tree(temporary)

    if not (destination / "AppRun").exists():
        fail(f"{archive}: extracted AppDir has no AppRun")
    if not (destination / "bin").is_dir():
        fail(f"{archive}: extracted AppDir has no bin directory")
    if not (destination / "bin" / "mpv").exists() and not (destination / "AppRun.sh").exists():
        fail(f"{archive}: extracted AppDir does not look like mpv")

    if remove_hooks:
        patterns = list(DEFAULT_REMOVED_HOOKS) if remove_hooks is True else list(remove_hooks)
        hooks = sorted(
            path
            for path in (destination / "bin").glob("*.hook")
            if hook_is_removed(path.name, patterns)
        )
        for hook in hooks:
            log(f"removing runtime hook: {hook.name}")
            hook.unlink()
        if hooks:
            log(f"removed {len(hooks)} AppImage runtime hook(s)")
        else:
            log("no self-update or yt-dlp download hook found in the AppImage")


def extract_component(archive: Path, destination: Path, extract_config: dict, *, platform: str) -> None:
    kind = extract_config.get("type")
    if kind == "7z":
        extract_7z(archive, destination, extract_config.get("members"))
    elif kind == "zip":
        extract_zip(archive, destination)
    elif kind == "appimage":
        extract_appimage(archive, destination, remove_hooks=extract_config.get("remove_hooks", True))
    else:
        fail(f"unsupported extraction type {kind!r} for platform {platform}")
