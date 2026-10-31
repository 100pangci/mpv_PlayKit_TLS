from __future__ import annotations

import argparse
import re
import tarfile
import zipfile
from pathlib import Path

from .common import PLATFORM_ARCHIVE, PLATFORM_OS, fail, log, read_json, sha256_file
from .extract import DEFAULT_REMOVED_HOOKS, hook_is_removed

REQUIRED = [
    "portable_config/mpv.conf",
    "portable_config/input.conf",
    "portable_config/profiles.conf",
    "portable_config/menu.conf",
    "portable_config/platform.conf",
    "portable_config/scripts/uosc/main.lua",
    "portable_config/scripts/update-check.lua",
    "portable_config/scripts/save_global_props.lua",
    "portable_config/scripts/idle-brand.lua",
    "portable_config/scripts/input_plus.lua",
    "portable_config/scripts/thumbfast.lua",
    "portable_config/fonts/LXGWWenKaiMonoLite-Regular.ttf",
    "manifest.json",
    "playkit-version.json",
    "licenses/INDEX.json",
    "README.txt",
    "LICENSE",
    "NOTICE",
    "THIRD_PARTY.md",
    "installer/README.md",
]
WINDOWS_REQUIRED = [
    "mpv.exe",
    "yt-dlp.exe",
    "installer/windows/install.ps1",
    "installer/windows/uninstall.ps1",
    "installer/windows/install.cmd",
    "installer/windows/uninstall.cmd",
    "installer/windows/mpv-playkit.ico",
]
LINUX_REQUIRED = [
    "mpv/AppRun",
    "mpv-playkit",
    "yt-dlp",
    "installer/linux/install.sh",
    "installer/linux/mpv-playkit.desktop.in",
    "installer/linux/mpv-playkit.svg",
]

# Every license text promised by THIRD_PARTY.md must be present, and
# licenses/INDEX.json must carry a per-file SPDX label and source URL for it.
REQUIRED_LICENSES = {
    "windows-x64": [
        "licenses/lxgw-wenkai-mono-lite/OFL.txt",
        "licenses/mpv-windows/LICENSE.GPL",
        "licenses/mpv-windows/LICENSE.LGPL",
        "licenses/uosc/LICENSE.LGPL",
        "licenses/uosc/Material-Icons-Apache-2.0.txt",
        "licenses/uosc/MIT-fzy.txt",
        "licenses/uosc/MPL-2.0.txt",
        "licenses/yt-dlp-windows/LICENSE",
        "licenses/yt-dlp-windows/THIRD_PARTY_LICENSES.txt",
    ],
    "linux-x64": [
        "licenses/lxgw-wenkai-mono-lite/OFL.txt",
        "licenses/mpv-linux/LICENSE",
        "licenses/mpv-linux/LICENSE.GPL",
        "licenses/mpv-linux/LICENSE.LGPL",
        "licenses/uosc/LICENSE.LGPL",
        "licenses/uosc/Material-Icons-Apache-2.0.txt",
        "licenses/uosc/MIT-fzy.txt",
        "licenses/uosc/MPL-2.0.txt",
        "licenses/yt-dlp-linux/LICENSE",
        "licenses/yt-dlp-linux/THIRD_PARTY_LICENSES.txt",
    ],
}

FORBIDDEN_PARTS = {
    "baseline",
    "shaders",
    "vs",
    "thumb_engine",
    "mpv-lazy",
    "_cache",
    "saved-props.json",
    "update-check-state.json",
}
FORBIDDEN_SUFFIXES = {".vpy", ".onnx", ".glsl"}


def check_forbidden(path: str) -> None:
    parts = Path(path).parts
    for part in parts:
        if part in FORBIDDEN_PARTS:
            fail(f"forbidden path in package: {path}")
    suffix = Path(path).suffix.lower()
    if suffix in FORBIDDEN_SUFFIXES:
        fail(f"forbidden file type in package: {path}")
    if suffix == ".hook" and hook_is_removed(Path(path).name, DEFAULT_REMOVED_HOOKS):
        fail(f"runtime self-update/download hook in package: {path}")


def check_installer(stage: Path, platform: str) -> None:
    if platform == "windows-x64":
        install = (stage / "installer" / "windows" / "install.ps1").read_text(encoding="utf-8")
        if "HKLM" in install:
            fail("install.ps1 must only write HKCU, not HKLM")
        if "Uninstall\\mpv-playkit" not in install:
            fail("install.ps1 does not register the uninstall entry")
        for marker in ("App Paths", "RegisteredApplications", "SystemFileAssociations", "Capabilities"):
            if marker not in install:
                fail(f"install.ps1 is missing the {marker} registration")
        uninstall = (stage / "installer" / "windows" / "uninstall.ps1").read_text(encoding="utf-8")
        if "HKLM" in uninstall:
            fail("uninstall.ps1 must only write HKCU, not HKLM")
        # Capabilities is a child of Software\Clients\Media\mpv-playkit, so
        # deleting that parent removes it.
        for marker in (
            "App Paths",
            "RegisteredApplications",
            "SystemFileAssociations",
            "Software\\Clients\\Media\\mpv-playkit",
        ):
            if marker not in uninstall:
                fail(f"uninstall.ps1 is missing the {marker} cleanup")
    elif platform == "linux-x64":
        script = (stage / "installer" / "linux" / "install.sh").read_text(encoding="utf-8")
        if "尚未实现" not in script:
            fail("Linux installer must stay an explicit placeholder")
        desktop = (stage / "installer" / "linux" / "mpv-playkit.desktop.in").read_text(encoding="utf-8")
        for token in ("@EXEC@", "@ICON@"):
            if token not in desktop:
                fail(f"desktop template is missing {token}")


def check_licenses(stage: Path, platform: str) -> None:
    index_path = stage / "licenses" / "INDEX.json"
    if not index_path.is_file():
        fail("licenses/INDEX.json missing")
    index = read_json(index_path)
    entries = {
        entry.get("file"): entry
        for entry in index.get("files", [])
        if isinstance(entry, dict)
    }
    for relative in REQUIRED_LICENSES.get(platform, []):
        if not (stage / relative).is_file():
            fail(f"missing required license text: {relative}")
        entry = entries.get(relative)
        if not entry:
            fail(f"license text not indexed in licenses/INDEX.json: {relative}")
        if not entry.get("license"):
            fail(f"license text has no SPDX label in licenses/INDEX.json: {relative}")
        if not entry.get("source_url"):
            fail(f"license text has no source URL in licenses/INDEX.json: {relative}")


def check_stage(stage: Path, platform: str, version: str | None) -> None:
    required = list(REQUIRED)
    if platform == "windows-x64":
        required.extend(WINDOWS_REQUIRED)
    elif platform == "linux-x64":
        required.extend(LINUX_REQUIRED)
    else:
        fail(f"unknown platform: {platform}")

    for relative in required:
        if not (stage / relative).exists():
            fail(f"missing required file: {relative}")
    for path in stage.rglob("*"):
        check_forbidden(str(path.relative_to(stage)).replace("\\", "/"))

    portable = stage / "portable_config"
    for config_name in ("mpv.conf",):
        for line in (portable / config_name).read_text(encoding="utf-8").splitlines():
            stripped = line.strip()
            if not stripped or stripped.startswith("#"):
                continue
            match = re.match(r"^(script|input-conf|include)=~~/(.+)$", stripped)
            if match:
                target = portable / match.group(2)
                if not target.exists():
                    fail(f"{config_name}: referenced file is missing: {target}")

    update_check = (portable / "scripts" / "update-check.lua").read_text(encoding="utf-8")
    gate = update_check.find("truthy_disabled(opts.enabled)")
    subprocess_call = update_check.find('name = "subprocess"')
    if gate == -1 or subprocess_call == -1 or gate > subprocess_call:
        fail("update-check.lua must gate on enabled=no before any subprocess call")

    uosc_main = (portable / "scripts" / "uosc" / "main.lua").read_text(encoding="utf-8")
    if "-- Modified by mpv PlayKit" not in uosc_main:
        fail("uosc main.lua is missing the mpv PlayKit automatic UI-scale patch")
    uosc_conf = (portable / "script-opts" / "uosc.conf").read_text(encoding="utf-8")
    if not re.search(r"^scale=0\s*$", uosc_conf, re.MULTILINE):
        fail("uosc.conf must enable the automatic UI scale (scale=0)")

    manifest = read_json(stage / "manifest.json")
    if version and manifest.get("playkit_version") != version:
        fail(f"manifest version mismatch: {manifest.get('playkit_version')} != {version}")
    if manifest.get("platform") != platform:
        fail(f"manifest platform mismatch: {manifest.get('platform')} != {platform}")
    if not manifest.get("components"):
        fail("manifest has no components")
    check_installer(stage, platform)
    check_licenses(stage, platform)
    log(f"verified stage {stage} ({platform})")


def list_archive(asset: Path) -> list[str]:
    if asset.suffix == ".zip":
        with zipfile.ZipFile(asset) as archive:
            return archive.namelist()
    if asset.name.endswith(".tar.gz"):
        with tarfile.open(asset, "r:gz") as archive:
            return archive.getnames()
    fail(f"unsupported archive: {asset}")


def check_archive(asset: Path, version: str, platform: str) -> None:
    expected_top = f"mpv-PlayKit-{version}-{platform}"
    names = list_archive(asset)
    if not names:
        fail(f"archive is empty: {asset}")
    for name in names:
        pure = name.rstrip("/")
        if not pure.startswith(expected_top + "/") and pure != expected_top:
            fail(f"archive member outside expected top folder {expected_top}: {name}")
        if pure.startswith("/") or ".." in Path(pure).parts:
            fail(f"unsafe archive member: {name}")
        check_forbidden(pure)
    relative = [name[len(expected_top) + 1 :] for name in names if name.startswith(expected_top + "/")]
    required = list(REQUIRED)
    required.extend(WINDOWS_REQUIRED if platform == "windows-x64" else LINUX_REQUIRED)
    for item in required:
        if item not in relative and item + "/" not in relative:
            fail(f"archive missing required member: {item}")
    checksum = asset.with_suffix(asset.suffix + ".sha256")
    if checksum.is_file():
        expected = checksum.read_text(encoding="utf-8").split()[0]
        actual = sha256_file(asset)
        if expected != actual:
            fail(f"checksum mismatch for {asset}: {expected} != {actual}")
    log(f"verified archive {asset} ({len(names)} entries)")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Verify an assembled tree and/or packaged archive")
    parser.add_argument("--stage", type=Path, default=None)
    parser.add_argument("--asset", type=Path, default=None)
    parser.add_argument("--platform", required=True, choices=sorted(PLATFORM_OS))
    parser.add_argument("--version", default=None)
    args = parser.parse_args(argv)

    if args.stage:
        check_stage(args.stage.resolve(), args.platform, args.version)
    if args.asset:
        if not args.version:
            fail("--asset requires --version")
        check_archive(args.asset.resolve(), args.version, args.platform)
    if not args.stage and not args.asset:
        fail("nothing to verify")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
