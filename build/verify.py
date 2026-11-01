from __future__ import annotations

import argparse
import ntpath
import posixpath
import re
import stat
import tarfile
import zipfile
from pathlib import Path

from .common import PLATFORM_ARCHIVE, PLATFORM_OS, fail, log, read_json, sha256_file
from .extract import DEFAULT_REMOVED_HOOKS, hook_is_removed
from .shaders import shader_targets
from .userstate import verify_package_state

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
    "portable_config/.playkit/state.json",
    "portable_config/.playkit/official/mpv.conf",
    "portable_config/.playkit/official/input.conf",
    "portable_config/.playkit/official/profiles.conf",
    "portable_config/.playkit/official/platform.conf",
    "portable_config/.playkit/generated/mpv.conf",
    "portable_config/.playkit/generated/input.conf",
    "portable_config/.playkit/generated/profiles.conf",
    "portable_config/.playkit/generated/platform.conf",
    "portable_config/user/README.md",
    "playkit-inventory.json",
    "manifest.json",
    "playkit-version.json",
    "umpv.conf",
    "licenses/INDEX.json",
    "README.txt",
    "LICENSE",
    "NOTICE",
    "THIRD_PARTY.md",
    "installer/README.md",
]
WINDOWS_REQUIRED = [
    "mpv.exe",
    "umpv.exe",
    "yt-dlp.exe",
    "updater/playkit-updater.exe",
    "installer/windows/install.ps1",
    "installer/windows/uninstall.ps1",
    "installer/windows/install.cmd",
    "installer/windows/uninstall.cmd",
    "installer/windows/umpv-register.ps1",
    "installer/windows/umpv-unregister.ps1",
    "installer/windows/umpv-register.cmd",
    "installer/windows/umpv-unregister.cmd",
    "installer/windows/mpv-playkit.ico",
]
LINUX_REQUIRED = [
    "mpv/AppRun",
    "mpv-playkit",
    "umpv",
    "yt-dlp",
    "updater/playkit-updater",
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
        "licenses/anime4k-restore/LICENSE",
        "licenses/anime4k-upscale-gan/LICENSE",
        "licenses/fsrcnnx/LGPL-3.0-or-later.txt",
        "licenses/nnedi3/LGPL-3.0-or-later.txt",
        "licenses/k7-modernanime/MIT.txt",
        "licenses/adaptive-sharpen-lite/BSD-2-Clause.txt",
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
        "licenses/anime4k-restore/LICENSE",
        "licenses/anime4k-upscale-gan/LICENSE",
        "licenses/fsrcnnx/LGPL-3.0-or-later.txt",
        "licenses/nnedi3/LGPL-3.0-or-later.txt",
        "licenses/k7-modernanime/MIT.txt",
        "licenses/adaptive-sharpen-lite/BSD-2-Clause.txt",
    ],
}

FORBIDDEN_PARTS = {
    "baseline",
    "vs",
    "thumb_engine",
    "mpv-lazy",
    "_cache",
    "saved-props.json",
    "update-check-state.json",
    "update-status.json",
    "updater.log",
}
FORBIDDEN_SUFFIXES = {".vpy", ".onnx"}
FORBIDDEN_PARTS_FOLDED = frozenset(part.casefold() for part in FORBIDDEN_PARTS)


def _declared_shader(path: str, allowed: set[str]) -> bool:
    normalized = path.replace("\\", "/")
    while normalized.startswith("./"):
        normalized = normalized[2:]
    return any(
        normalized == target or normalized.endswith("/" + target)
        for target in allowed
    )


def check_forbidden(
    path: str, allowed_shaders: set[str] | None = None, *, is_dir: bool = False
) -> None:
    allowed = set(shader_targets()) if allowed_shaders is None else allowed_shaders
    parts = Path(path).parts
    for part in parts:
        if part.casefold() in FORBIDDEN_PARTS_FOLDED:
            fail(f"forbidden path in package: {path}")
    suffix = Path(path).suffix.lower()
    if suffix == ".glsl":
        # Shader assets are allowed only at the exact targets declared in
        # manifests/shaders.yml.  A random .glsl file, a VapourSynth script or a
        # model is still rejected.
        if not _declared_shader(path, allowed):
            fail(f"undeclared shader file in package: {path}")
    elif suffix in FORBIDDEN_SUFFIXES:
        fail(f"forbidden file type in package: {path}")
    if (
        not is_dir
        and any(part.casefold() == "shaders" for part in parts)
        and suffix != ".glsl"
    ):
        fail(f"non-shader file in shader directory: {path}")
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
        for name in ("umpv-register.ps1", "umpv-unregister.ps1"):
            text = (stage / "installer" / "windows" / name).read_text(encoding="utf-8")
            if "HKLM" in text:
                fail(f"{name} must only write HKCU, not HKLM")
            if "mpv-playkit.umpv" not in text:
                fail(f"{name} must use the dedicated mpv-playkit.umpv ProgID")
    elif platform == "linux-x64":
        script = (stage / "installer" / "linux" / "install.sh").read_text(encoding="utf-8")
        if "尚未实现" not in script:
            fail("Linux installer must stay an explicit placeholder")
        desktop = (stage / "installer" / "linux" / "mpv-playkit.desktop.in").read_text(encoding="utf-8")
        for token in ("@EXEC@", "@ICON@"):
            if token not in desktop:
                fail(f"desktop template is missing {token}")


def check_license_entry(relative: str, entry: dict) -> None:
    """A shipped license text must be pinned to a full commit and a hash."""
    if not entry.get("license"):
        fail(f"license text has no SPDX label in licenses/INDEX.json: {relative}")
    commit = str(entry.get("commit") or "")
    if not re.fullmatch(r"[0-9a-f]{40}", commit):
        fail(f"license text is not pinned to a full commit in licenses/INDEX.json: {relative}")
    source_url = str(entry.get("source_url") or "")
    if not source_url.startswith("https://raw.githubusercontent.com/"):
        fail(f"license text has no pinned raw source URL in licenses/INDEX.json: {relative}")
    if f"/{commit}/" not in source_url:
        fail(
            f"license source URL does not use the pinned commit "
            f"in licenses/INDEX.json: {relative}"
        )
    if not re.fullmatch(r"[0-9a-f]{64}", str(entry.get("sha256") or "")):
        fail(f"license text has no sha256 in licenses/INDEX.json: {relative}")


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
        check_license_entry(relative, entry)


def check_manifest_license_files(stage: Path, manifest: dict) -> None:
    """Manifest license hashes must match the license texts shipped in the package."""
    index_path = stage / "licenses" / "INDEX.json"
    if not index_path.is_file():
        fail("licenses/INDEX.json missing")
    index = read_json(index_path)
    entries = {
        entry.get("file"): entry
        for entry in index.get("files", [])
        if isinstance(entry, dict)
    }
    for identifier, component in (manifest.get("components") or {}).items():
        for item in component.get("license_files") or []:
            relative = f"licenses/{identifier}/{item.get('path')}"
            entry = entries.get(relative)
            if not entry:
                fail(f"manifest license file is missing from licenses/INDEX.json: {relative}")
            if entry.get("sha256") != item.get("sha256"):
                fail(f"manifest license hash differs from licenses/INDEX.json: {relative}")


SUBPROCESS_CALL = re.compile(r"""name\s*=\s*["']subprocess["']""")


def update_check_gate_ok(text: str) -> bool:
    """The enabled=no gate must precede every subprocess call.

    Match the assignment flexibly so spacing or quote changes cannot hide a
    network call from the static check.
    """
    gate = text.find("truthy_disabled(opts.enabled)")
    if gate == -1:
        return False
    calls = list(SUBPROCESS_CALL.finditer(text))
    if not calls:
        return False
    return gate < calls[0].start()


def check_umpv_metadata(stage: Path, manifest: dict, platform: str) -> None:
    umpv = manifest.get("umpv") or {}
    name = "umpv.exe" if platform == "windows-x64" else "umpv"
    path = stage / name
    if not path.is_file():
        fail(f"missing umpv binary: {name}")
    if umpv.get("asset") != name:
        fail(f"manifest umpv.asset must be {name}")
    if not isinstance(umpv.get("version"), str) or not umpv["version"]:
        fail("manifest umpv.version is missing")
    if umpv.get("sha256") != sha256_file(path):
        fail("manifest umpv.sha256 does not match the packaged umpv")
    if umpv.get("size") != path.stat().st_size:
        fail("manifest umpv.size does not match the packaged umpv")
    if platform == "linux-x64" and not (path.stat().st_mode & stat.S_IXUSR):
        fail("Linux umpv binary is not executable")


def check_updater_metadata(stage: Path, manifest: dict, platform: str) -> None:
    updater = manifest.get("updater") or {}
    if updater.get("protocol") != 1:
        fail("manifest updater.protocol must be 1")
    if updater.get("patch_schema") != 1:
        fail("manifest updater.patch_schema must be 1")
    if updater.get("user_layer") != "portable_config/user":
        fail("manifest updater.user_layer must be portable_config/user")
    if updater.get("state_dir") != "portable_config/.playkit":
        fail("manifest updater.state_dir must be portable_config/.playkit")
    helper = manifest.get("helper") or {}
    helper_name = "playkit-updater.exe" if platform == "windows-x64" else "playkit-updater"
    path = stage / "updater" / helper_name
    if helper.get("asset") != helper_name:
        fail(f"manifest helper.asset must be {helper_name}")
    if not isinstance(helper.get("version"), str) or not helper["version"]:
        fail("manifest helper.version is missing")
    if helper.get("sha256") != sha256_file(path):
        fail("manifest helper.sha256 does not match the packaged updater")
    if helper.get("size") != path.stat().st_size:
        fail("manifest helper.size does not match the packaged updater")
    if platform == "linux-x64" and not (path.stat().st_mode & stat.S_IXUSR):
        fail("Linux updater helper is not executable")


def check_stage(stage: Path, platform: str, version: str | None) -> None:
    required = list(REQUIRED)
    if platform == "windows-x64":
        required.extend(WINDOWS_REQUIRED)
    elif platform == "linux-x64":
        required.extend(LINUX_REQUIRED)
    else:
        fail(f"unknown platform: {platform}")

    allowed_shaders = set(shader_targets())
    required.extend(sorted(allowed_shaders))
    for relative in required:
        if not (stage / relative).exists():
            fail(f"missing required file: {relative}")
    for path in stage.rglob("*"):
        check_forbidden(
            str(path.relative_to(stage)).replace("\\", "/"),
            allowed_shaders,
            is_dir=path.is_dir(),
        )

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
    if not update_check_gate_ok(update_check):
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

    mpv = manifest.get("mpv") or {}
    mpv_component = (manifest.get("components") or {}).get(mpv.get("id")) or {}
    if not mpv.get("version") or not mpv.get("commit"):
        fail("manifest has no per-platform mpv version/commit")
    if (
        mpv_component.get("version") != mpv["version"]
        or mpv_component.get("commit") != mpv["commit"]
    ):
        fail("manifest mpv block disagrees with its component entry")

    check_installer(stage, platform)
    check_licenses(stage, platform)
    check_manifest_license_files(stage, manifest)
    check_updater_metadata(stage, manifest, platform)
    check_umpv_metadata(stage, manifest, platform)
    verify_package_state(stage, manifest.get("playkit_version", ""), platform)
    log(f"verified stage {stage} ({platform})")


def check_archive_symlink(member_name: str, linkname: str, expected_top: str) -> None:
    if (
        not linkname
        or posixpath.isabs(linkname)
        or ntpath.isabs(linkname)
        or ntpath.splitdrive(linkname)[0]
    ):
        fail(f"unsafe symlink target in archive: {member_name} -> {linkname}")
    target = posixpath.normpath(posixpath.join(posixpath.dirname(member_name), linkname))
    if target != expected_top and not target.startswith(expected_top + "/"):
        fail(f"symlink target escapes the package root: {member_name} -> {linkname}")


def check_archive_hardlink(member_name: str, linkname: str, expected_top: str) -> None:
    pure = linkname.rstrip("/")
    if (
        not pure
        or posixpath.isabs(pure)
        or ntpath.isabs(pure)
        or ntpath.splitdrive(pure)[0]
    ):
        fail(f"unsafe hardlink target in archive: {member_name} -> {linkname}")
    if pure != expected_top and not pure.startswith(expected_top + "/"):
        fail(f"hardlink target escapes the package root: {member_name} -> {linkname}")


def list_archive(asset: Path, expected_top: str) -> list[str]:
    if asset.suffix == ".zip":
        with zipfile.ZipFile(asset) as archive:
            for info in archive.infolist():
                mode = (info.external_attr >> 16) & 0o170000
                if mode == stat.S_IFLNK:
                    fail(f"zip archive contains a symlink: {info.filename}")
            return archive.namelist()
    if asset.name.endswith(".tar.gz"):
        with tarfile.open(asset, "r:gz") as archive:
            members = archive.getmembers()
            for member in members:
                if member.issym():
                    check_archive_symlink(member.name, member.linkname, expected_top)
                elif member.islnk():
                    check_archive_hardlink(member.name, member.linkname, expected_top)
            # tarfile stores directory names without the trailing slash that
            # zip uses.  Normalize them so callers can distinguish directories
            # from files by the same rule for both formats.
            return [
                member.name + "/"
                if member.isdir() and not member.name.endswith("/")
                else member.name
                for member in members
            ]
    fail(f"unsupported archive: {asset}")


def check_archive(asset: Path, version: str, platform: str) -> None:
    expected_top = f"mpv-PlayKit-{version}-{platform}"
    allowed_shaders = set(shader_targets())
    names = list_archive(asset, expected_top)
    if not names:
        fail(f"archive is empty: {asset}")
    for name in names:
        pure = name.rstrip("/")
        if not pure.startswith(expected_top + "/") and pure != expected_top:
            fail(f"archive member outside expected top folder {expected_top}: {name}")
        if pure.startswith("/") or ".." in Path(pure).parts:
            fail(f"unsafe archive member: {name}")
        check_forbidden(pure, allowed_shaders, is_dir=name.endswith("/"))
    relative = [name[len(expected_top) + 1 :] for name in names if name.startswith(expected_top + "/")]
    required = list(REQUIRED)
    required.extend(WINDOWS_REQUIRED if platform == "windows-x64" else LINUX_REQUIRED)
    required.extend(sorted(allowed_shaders))
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
