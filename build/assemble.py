from __future__ import annotations

import argparse
import os
import stat
from pathlib import Path
from typing import Any

from .common import (
    DEFAULT_LOCK,
    MANIFEST_DIR,
    PLATFORM_OS,
    ROOT,
    copy_file,
    copy_tree,
    fail,
    human_size,
    is_platform_selected,
    load_manifests,
    log,
    make_executable,
    now_iso,
    package_filename,
    read_json,
    remove_tree,
    sha256_file,
    write_json,
)
from .extract import DEFAULT_REMOVED_HOOKS, extract_component, hook_is_removed
from .inputs import compute_inputs_hash

LAUNCHER = """#!/bin/sh
# mpv PlayKit launcher for Linux.
set -eu
HERE=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
export LC_ALL=C.UTF-8
export DISABLE_AUTO_UPDATES=1
export PATH="$HERE:$HERE/mpv/bin:$PATH"
exec "$HERE/mpv/AppRun" --config-dir="$HERE/portable_config" "$@"
"""

README_TEMPLATE = """mpv PlayKit {version} ({platform})

启动
====
Windows: 双击 mpv.exe
Linux:   在终端运行 ./mpv-playkit

portable_config/ 内是完整配置。第三方组件及许可证见 licenses/。

更新检查
========
播放器默认会在启动后检查 PlayKit 的新版本，但不会自动下载或覆盖自身。
永久关闭：把 portable_config/script-opts/update-check.conf 里的 enabled 改为 no，
或删除 portable_config/update-check-state.json 后使用“关闭更新检查”菜单项。
关闭后不会发起任何联网请求。

本发行包不包含 GLSL shader、VapourSynth、AI 模型或 CUDA/TensorRT 组件。
"""


def component_source(
    component: dict[str, Any],
    locked: dict[str, Any],
    fetched: dict[str, Any],
    work: Path,
    platform: str,
) -> tuple[Path, str]:
    asset_path = work / fetched["asset_path"]
    if not asset_path.is_file():
        fail(f"missing fetched asset: {asset_path}")
    extract_config = component.get("extract")
    if not extract_config:
        return asset_path.parent, asset_path.name

    destination = work / "extract" / component["id"]
    marker = work / "extract" / f"{component['id']}.sha256"
    expected = fetched.get("sha256") or locked.get("sha256")
    if marker.is_file() and marker.read_text(encoding="utf-8").strip() == expected and destination.is_dir():
        log(f"reusing extracted {component['id']}")
        return destination, "."
    remove_tree(destination)
    extract_component(asset_path, destination, extract_config, platform=platform)
    marker.parent.mkdir(parents=True, exist_ok=True)
    marker.write_text(str(expected) + "\n", encoding="utf-8")
    return destination, "."


def install_component(
    stage: Path,
    component: dict[str, Any],
    source_root: Path,
    work: Path,
    platform: str,
) -> None:
    for entry in component.get("install", []):
        source = source_root / entry["source"]
        target = stage / entry["target"]
        if entry.get("tree"):
            copy_tree(source, target)
        else:
            copy_file(source, target, executable=bool(entry.get("executable")))
        log(f"installed {component['id']}: {entry['source']} -> {entry['target']}")

    platform_binaries = component.get("platform_binaries")
    if platform_binaries:
        allowed = set(platform_binaries.get(platform, []))
        if not allowed:
            fail(f"{component['id']}: no platform_binaries entry for {platform}")
        bin_dir = stage / "portable_config" / "scripts" / "uosc" / "bin"
        if not bin_dir.is_dir():
            fail(f"{component['id']}: expected {bin_dir} after install")
        for path in sorted(bin_dir.iterdir()):
            relative = f"scripts/uosc/bin/{path.name}"
            if relative not in allowed:
                log(f"pruning unused uosc helper: {relative}")
                if path.is_dir():
                    remove_tree(path)
                else:
                    path.unlink()
        for relative in allowed:
            if not (stage / "portable_config" / relative).is_file():
                fail(f"{component['id']}: expected platform helper missing: {relative}")


def copy_configuration(stage: Path, platform: str) -> None:
    os_name = PLATFORM_OS.get(platform)
    if not os_name:
        fail(f"unknown platform: {platform}")
    portable = stage / "portable_config"
    copy_tree(ROOT / "config" / "common", portable)
    copy_tree(ROOT / "config" / os_name, portable)
    if not (portable / "mpv.conf").is_file():
        fail("portable_config/mpv.conf missing")
    if not (portable / "platform.conf").is_file():
        fail(f"portable_config/platform.conf missing for {platform}")

    # mpv auto-loads every script below portable_config/scripts/.  Keep local
    # scripts as direct children so they are loaded exactly once, and let
    # uosc/ load as a directory script (it relies on its own require() paths).
    package_scripts = portable / "scripts"
    package_scripts.mkdir(parents=True, exist_ok=True)
    for script in sorted((ROOT / "scripts" / "local").glob("*.lua")):
        copy_file(script, package_scripts / script.name)
    copy_file(ROOT / "updater" / "update-check.lua", package_scripts / "update-check.lua")


def copy_licenses(stage: Path, work: Path, fetched: dict[str, Any], lock: dict[str, Any]) -> None:
    index: list[dict[str, Any]] = []
    for identifier, entry in sorted(fetched.items()):
        component = lock["components"].get(identifier, {})
        for item in entry.get("license_files", []):
            source = work / item["local_path"]
            target = stage / "licenses" / identifier / item["path"]
            copy_file(source, target)
            index.append(
                {
                    "component": identifier,
                    "version": component.get("version"),
                    "upstream": component.get("upstream"),
                    "license": item.get("license")
                    or (component.get("license") or {}).get("spdx"),
                    "file": f"licenses/{identifier}/{item['path']}",
                    "source_url": item.get("url"),
                    "sha256": item.get("sha256"),
                }
            )
    write_json(stage / "licenses" / "INDEX.json", {"schema": 1, "files": index})
    for name in ("THIRD_PARTY.md", "LICENSE_PROVENANCE.md", "LICENSE"):
        path = ROOT / name
        if path.is_file():
            copy_file(path, stage / name)


def write_metadata(
    stage: Path,
    *,
    version: str,
    platform: str,
    repository: str,
    built_at: str,
    source_sha: str,
    inputs_hash: str,
    fetched: dict[str, Any],
    lock: dict[str, Any],
) -> None:
    components: dict[str, Any] = {}
    for identifier, entry in sorted(fetched.items()):
        locked = lock["components"][identifier]
        if locked.get("sha256") and entry.get("sha256") != locked.get("sha256"):
            fail(f"{identifier}: fetched hash differs from lock")
        components[identifier] = {
            "version": locked.get("version"),
            "upstream": locked.get("upstream"),
            "release_tag": locked.get("release_tag"),
            "release_url": locked.get("release_url"),
            "asset_name": locked.get("asset_name"),
            "sha256": entry.get("sha256"),
            "license": (locked.get("license") or {}).get("spdx"),
        }

    version_info = {
        "schema": 1,
        "playkit_version": version,
        "platform": platform,
        "repository": repository,
        "built_at": built_at,
        "source_commit": source_sha,
        "inputs_hash": inputs_hash,
        "components": components,
    }
    write_json(stage / "playkit-version.json", version_info)

    package_manifest = {
        **version_info,
        "package": {
            "asset": package_filename(version, platform),
            "platform": platform,
            "format": "zip" if platform.startswith("windows") else "tar.gz",
            "root": f"mpv-PlayKit-{version}-{platform}",
        },
    }
    write_json(stage / "manifest.json", package_manifest)
    (stage / "README.txt").write_text(
        README_TEMPLATE.format(version=version, platform=platform), encoding="utf-8", newline="\n"
    )


def verify_stage_minimum(stage: Path, platform: str) -> None:
    portable = stage / "portable_config"
    required = [
        portable / "mpv.conf",
        portable / "input.conf",
        portable / "profiles.conf",
        portable / "menu.conf",
        portable / "platform.conf",
        portable / "scripts" / "uosc" / "main.lua",
        portable / "scripts" / "update-check.lua",
        portable / "scripts" / "save_global_props.lua",
        portable / "scripts" / "idle-brand.lua",
        portable / "scripts" / "input_plus.lua",
        portable / "scripts" / "thumbfast.lua",
        stage / "manifest.json",
        stage / "playkit-version.json",
    ]
    if platform == "windows-x64":
        required.extend([stage / "mpv.exe", stage / "yt-dlp.exe"])
    elif platform == "linux-x64":
        required.extend([stage / "mpv" / "AppRun", stage / "mpv-playkit", stage / "yt-dlp"])
    else:
        fail(f"unknown platform: {platform}")
    for path in required:
        if not path.exists():
            fail(f"missing required path in stage: {path}")
    if platform == "linux-x64":
        for hook in (stage / "mpv" / "bin").glob("*.hook"):
            if hook_is_removed(hook.name, DEFAULT_REMOVED_HOOKS):
                fail(f"unexpected AppImage self-update/download hook in package: {hook}")
        make_executable(stage / "mpv-playkit")
        make_executable(stage / "mpv" / "AppRun")
        make_executable(stage / "mpv" / "AppRun.sh")
        make_executable(stage / "yt-dlp")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Assemble one platform package from locked upstream assets")
    parser.add_argument("--platform", required=True, choices=sorted(PLATFORM_OS))
    parser.add_argument("--version", required=True)
    parser.add_argument("--lock", type=Path, default=DEFAULT_LOCK)
    parser.add_argument("--work", type=Path, default=Path(".work"))
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--manifests", type=Path, default=MANIFEST_DIR)
    parser.add_argument("--repository", default=None)
    parser.add_argument("--built-at", default=None)
    parser.add_argument("--source-sha", default=os.environ.get("GITHUB_SHA", "local"))
    args = parser.parse_args(argv)

    project, components = load_manifests(args.manifests)
    repository = args.repository or project.get("repository")
    if not repository:
        fail("no repository configured")
    lock = read_json(args.lock)
    work = args.work.resolve()
    fetched_path = work / "fetched.json"
    if not fetched_path.is_file():
        fail(f"missing {fetched_path}; run build.fetch first")
    fetched_all = read_json(fetched_path).get("components", {})
    fetched = {
        identifier: entry
        for identifier, entry in fetched_all.items()
        if identifier in lock.get("components", {})
        and is_platform_selected(lock["components"][identifier], args.platform)
    }
    if not fetched:
        fail(f"no fetched components for {args.platform}")

    stage = args.out.resolve()
    remove_tree(stage)
    stage.mkdir(parents=True, exist_ok=True)

    copy_configuration(stage, args.platform)

    for identifier, component in sorted(components.items()):
        if identifier not in lock.get("components", {}):
            fail(f"{identifier}: manifest component has no lock entry")
        if not is_platform_selected(component, args.platform):
            continue
        if identifier not in fetched:
            fail(f"{identifier}: not fetched for {args.platform}")
        source_root, _ = component_source(
            component, lock["components"][identifier], fetched[identifier], work, args.platform
        )
        install_component(stage, component, source_root, work, args.platform)

    if args.platform == "linux-x64":
        launcher = stage / "mpv-playkit"
        launcher.write_text(LAUNCHER, encoding="utf-8", newline="\n")
        make_executable(launcher)

    copy_licenses(stage, work, fetched, lock)
    write_metadata(
        stage,
        version=args.version,
        platform=args.platform,
        repository=repository,
        built_at=args.built_at or now_iso(),
        source_sha=args.source_sha,
        inputs_hash=compute_inputs_hash(),
        fetched=fetched,
        lock=lock,
    )
    verify_stage_minimum(stage, args.platform)
    log(f"assembled {args.platform} at {stage} ({human_size(sum(p.stat().st_size for p in stage.rglob('*') if p.is_file()))})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
