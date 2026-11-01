from __future__ import annotations

import hashlib
import io
import json
import re
import tarfile
import tempfile
import unittest
import zipfile
from datetime import datetime, timezone
from pathlib import Path

from build.common import ROOT, load_manifests, package_filename, read_json, sha256_file
from build.decide import compact_components, release_components
from build.extract import DEFAULT_REMOVED_HOOKS, find_squashfs_offset, hook_is_removed
from build.inputs import compute_inputs_hash, input_files
from build.release import checksum_lines
from build.upstream import StrictFormat, lock_hash, render
from build.version import compare_versions, next_release_version, parse_release_version


class ManifestTests(unittest.TestCase):
    def test_manifests_load(self) -> None:
        project, components = load_manifests()
        self.assertEqual(project["repository"], "100pangci/mpv_PlayKit_TLS")
        self.assertIn("mpv-windows", components)
        self.assertIn("mpv-linux", components)
        self.assertIn("uosc", components)
        self.assertIn("lxgw-wenkai-mono-lite", components)

    def test_license_files_have_spdx(self) -> None:
        _, components = load_manifests()
        labels = {}
        for identifier, component in components.items():
            for item in component["license"]["files"]:
                self.assertTrue(item.get("license"), f"{identifier}/{item.get('path')}")
                labels[(identifier, item["path"])] = item["license"]
        self.assertEqual(labels[("mpv-windows", "LICENSE.GPL")], "GPL-2.0-or-later")
        self.assertEqual(labels[("mpv-windows", "LICENSE.LGPL")], "LGPL-2.1-or-later")
        self.assertEqual(labels[("uosc", "LICENSE.LGPL")], "LGPL-2.1-or-later")
        self.assertEqual(labels[("uosc", "Material-Icons-Apache-2.0.txt")], "Apache-2.0")
        self.assertEqual(labels[("uosc", "MIT-fzy.txt")], "MIT")
        self.assertEqual(labels[("uosc", "MPL-2.0.txt")], "MPL-2.0")
        self.assertEqual(labels[("mpv-linux", "LICENSE.GPL")], "GPL-2.0-or-later")
        self.assertEqual(labels[("mpv-linux", "LICENSE.LGPL")], "LGPL-2.1-or-later")
        self.assertEqual(labels[("yt-dlp-windows", "THIRD_PARTY_LICENSES.txt")], "NOASSERTION")

    def test_multi_license_components_use_scoped_parts(self) -> None:
        _, components = load_manifests()
        expected = {
            "uosc": "NOASSERTION",
            "mpv-linux": "NOASSERTION",
            "mpv-windows": "GPL-2.0-or-later",
        }
        for identifier, spdx in expected.items():
            license_info = components[identifier]["license"]
            self.assertEqual(license_info["spdx"], spdx, identifier)
            self.assertEqual(license_info.get("scope"), "component-aggregate", identifier)
            parts = license_info.get("parts")
            self.assertTrue(parts, identifier)
            part_licenses = {part["license"] for part in parts}
            file_licenses = {item["license"] for item in license_info["files"]}
            self.assertEqual(part_licenses, file_licenses, identifier)
            for part in parts:
                self.assertTrue(part["scope"].strip(), identifier)
                self.assertNotIn(" AND ", part["license"], identifier)

        uosc_parts = {
            part["license"]: part["scope"]
            for part in components["uosc"]["license"]["parts"]
        }
        self.assertEqual(
            set(uosc_parts),
            {"LGPL-2.1-or-later", "Apache-2.0", "MIT", "MPL-2.0"},
        )
        self.assertIn("uosc_icons.otf", uosc_parts["Apache-2.0"])
        self.assertIn("fzy.lua", uosc_parts["MIT"])
        self.assertIn("utils.lua", uosc_parts["MPL-2.0"])

    def test_verify_license_coverage(self) -> None:
        from build.verify import REQUIRED_LICENSES

        _, components = load_manifests()
        for platform, paths in REQUIRED_LICENSES.items():
            for path in paths:
                parts = Path(path).parts
                self.assertEqual(parts[0], "licenses", platform)
                identifier = parts[1]
                relative = "/".join(parts[2:])
                self.assertIn(identifier, components, f"{platform}: {path}")
                declared = {
                    item["path"] for item in components[identifier]["license"]["files"]
                }
                self.assertIn(relative, declared, f"{platform}: {path}")

    def test_asset_patterns(self) -> None:
        _, components = load_manifests()
        samples = {
            "mpv-windows": "mpv-x86_64-20261021-git-e471496c56.7z",
            "mpv-linux": "mpv-v0.41.0-1349-gcf764c61d-anylinux-x86_64.AppImage",
            "yt-dlp-windows": "yt-dlp.exe",
            "yt-dlp-linux": "yt-dlp_linux",
            "uosc": "uosc.zip",
            "lxgw-wenkai-mono-lite": "LXGWWenKaiMonoLite-Regular.ttf",
        }
        for identifier, sample in samples.items():
            component = components[identifier]
            pattern = re.compile(component["asset"]["pattern"])
            match = pattern.match(sample)
            self.assertIsNotNone(match, identifier)
            context = {"release_tag": "2026.10.26", "asset_name": sample, **{
                key: value for key, value in match.groupdict().items() if value is not None
            }}
            rendered = render(component["asset"]["version"], context)
            self.assertTrue(rendered, identifier)

    def test_mpv_linux_stable_asset(self) -> None:
        _, components = load_manifests()
        pattern = re.compile(components["mpv-linux"]["asset"]["pattern"])
        match = pattern.match("mpv-v0.41.0-anylinux-x86_64.AppImage")
        self.assertIsNotNone(match)
        self.assertEqual(match.group("version"), "v0.41.0")
        self.assertIsNone(match.group("commit"))

    def test_strict_template(self) -> None:
        with self.assertRaises(Exception):
            render("{missing}", {})


class VersionTests(unittest.TestCase):
    def test_parse(self) -> None:
        self.assertEqual(parse_release_version("v2026.11"), (2026, 11))
        self.assertEqual(parse_release_version("2026.11.2"), (2026, 11, 2))
        self.assertIsNone(parse_release_version("ci-abc"))
        self.assertIsNone(parse_release_version("2026.11-beta"))

    def test_compare(self) -> None:
        self.assertGreater(compare_versions("2026.11", "2026.10"), 0)
        self.assertGreater(compare_versions("2026.11.1", "2026.11"), 0)
        self.assertEqual(compare_versions("2026.11", "2026.11.0"), 0)

    def test_next_version(self) -> None:
        now = datetime(2026, 11, 1, tzinfo=timezone.utc)
        self.assertEqual(next_release_version([], now), "2026.11")
        self.assertEqual(next_release_version(["v2026.10"], now), "2026.11")
        self.assertEqual(next_release_version(["v2026.11"], now), "2026.11.1")
        self.assertEqual(next_release_version(["v2026.11", "v2026.11.1"], now), "2026.11.2")


class LockTests(unittest.TestCase):
    def test_lock_hash_ignores_timestamp(self) -> None:
        first = {"generated_at": "a", "components": {"x": {"version": "1"}}}
        second = {"generated_at": "b", "components": {"x": {"version": "1"}}}
        self.assertEqual(lock_hash(first), lock_hash(second))
        third = {"generated_at": "b", "components": {"x": {"version": "2"}}}
        self.assertNotEqual(lock_hash(first), lock_hash(third))

    def test_release_components_ignore_extra_fields(self) -> None:
        lock = {
            "components": {
                "mpv": {
                    "version": "1",
                    "upstream": "owner/repo",
                    "release_tag": "v1",
                    "release_url": "https://example.invalid/tag/v1",
                    "asset_name": "a.7z",
                    "sha256": "abc",
                    "license": {
                        "spdx": "GPL-2.0-or-later",
                        "scope": "component-aggregate",
                        "parts": [
                            {"license": "GPL-2.0-or-later", "scope": "mpv"},
                        ],
                        "files": [],
                    },
                    "published_at": "2026-01-01T00:00:00Z",
                    "size": 1,
                    "asset_id": 2,
                }
            }
        }
        published = {
            "components": {
                "mpv": {
                    "version": "1",
                    "upstream": "owner/repo",
                    "release_tag": "v1",
                    "release_url": "https://example.invalid/tag/v1",
                    "asset_name": "a.7z",
                    "sha256": "abc",
                    "license": "GPL-2.0-or-later",
                    "license_scope": "component-aggregate",
                    "license_parts": [
                        {"license": "GPL-2.0-or-later", "scope": "mpv"},
                    ],
                    "published_at": "2026-01-02T00:00:00Z",
                    "size": 999,
                }
            }
        }
        self.assertEqual(release_components(published), compact_components(lock))

    def test_manifest_metadata_changes_are_detected(self) -> None:
        lock = {
            "components": {
                "mpv": {
                    "version": "1",
                    "upstream": "owner/repo",
                    "release_tag": "v1",
                    "release_url": "https://example.invalid/tag/v1",
                    "asset_name": "a.7z",
                    "sha256": "abc",
                    "license": {
                        "spdx": "NOASSERTION",
                        "scope": "component-aggregate",
                        "parts": [{"license": "MIT", "scope": "a.lua"}],
                        "files": [],
                    },
                }
            }
        }
        published = {
            "components": {
                "mpv": {
                    "version": "1",
                    "upstream": "owner/repo",
                    "release_tag": "v1",
                    "release_url": "https://example.invalid/other",
                    "asset_name": "a.7z",
                    "sha256": "abc",
                    "license": "NOASSERTION",
                    "license_scope": "component-aggregate",
                    "license_parts": [{"license": "MIT", "scope": "a.lua"}],
                }
            }
        }
        self.assertNotEqual(release_components(published), compact_components(lock))

        changed_parts = json.loads(json.dumps(published))
        changed_parts["components"]["mpv"]["license_parts"] = [
            {"license": "Apache-2.0", "scope": "a.lua"}
        ]
        self.assertNotEqual(release_components(changed_parts), compact_components(lock))

        changed_scope = json.loads(json.dumps(published))
        changed_scope["components"]["mpv"]["license_scope"] = "file"
        self.assertNotEqual(release_components(changed_scope), compact_components(lock))

    def test_package_name(self) -> None:
        self.assertEqual(package_filename("2026.11", "windows-x64"), "mpv-PlayKit-2026.11-windows-x64.zip")
        self.assertEqual(package_filename("2026.11", "linux-x64"), "mpv-PlayKit-2026.11-linux-x64.tar.gz")


class InputsTests(unittest.TestCase):
    def _make_root(self, temporary: str) -> Path:
        root = Path(temporary)
        (root / "config").mkdir()
        (root / "config" / "mpv.conf").write_text("a=1\n", encoding="utf-8")
        build = root / "build"
        build.mkdir()
        (build / "__init__.py").write_text("", encoding="utf-8")
        (build / "assemble.py").write_text("VALUE = 1\n", encoding="utf-8")
        (build / "patch_uosc.py").write_text("MARKER = 'a'\n", encoding="utf-8")
        return root

    def test_inputs_hash_changes_with_config(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = self._make_root(temporary)
            first = compute_inputs_hash(root)
            (root / "config" / "mpv.conf").write_text("a=2\n", encoding="utf-8")
            second = compute_inputs_hash(root)
            self.assertNotEqual(first, second)

    def test_inputs_hash_changes_with_build_sources(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = self._make_root(temporary)
            first = compute_inputs_hash(root)
            (root / "build" / "patch_uosc.py").write_text("MARKER = 'b'\n", encoding="utf-8")
            second = compute_inputs_hash(root)
            self.assertNotEqual(first, second)
            (root / "build" / "assemble.py").write_text("VALUE = 2\n", encoding="utf-8")
            third = compute_inputs_hash(root)
            self.assertNotEqual(second, third)

    def test_generated_python_files_do_not_change_hash(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = self._make_root(temporary)
            first = compute_inputs_hash(root)
            cache = root / "build" / "__pycache__"
            cache.mkdir()
            (cache / "assemble.cpython-312.pyc").write_bytes(b"bytecode")
            (root / "build" / "assemble.pyc").write_bytes(b"bytecode")
            (root / "build" / "notes.tmp").write_text("scratch\n", encoding="utf-8")
            (root / "build" / "notes.swp").write_bytes(b"scratch")
            self.assertEqual(first, compute_inputs_hash(root))

    def test_tests_are_not_package_inputs(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = self._make_root(temporary)
            first = compute_inputs_hash(root)
            tests = root / "tests"
            tests.mkdir()
            (tests / "test_new.py").write_text("def test(): pass\n", encoding="utf-8")
            self.assertEqual(first, compute_inputs_hash(root))
            names = {
                str(path.relative_to(root)).replace("\\", "/")
                for path in input_files(root)
            }
            self.assertNotIn("tests/test_new.py", names)

    def test_repository_build_sources_are_inputs(self) -> None:
        names = {
            str(path.relative_to(ROOT)).replace("\\", "/")
            for path in input_files()
        }
        for relative in (
            "build/inputs.py",
            "build/assemble.py",
            "build/patch_uosc.py",
            "build/package.py",
            "build/verify.py",
            ".github/workflows/build.yml",
        ):
            self.assertIn(relative, names)
        self.assertFalse(any(name.startswith("tests/") for name in names))
        self.assertFalse(
            any("__pycache__" in name or name.endswith((".pyc", ".pyo")) for name in names)
        )


class LicenseScopeTests(unittest.TestCase):
    def test_manifest_fields_keep_scope(self) -> None:
        from build.common import manifest_license_fields

        fields = manifest_license_fields(
            {
                "spdx": "NOASSERTION",
                "scope": "component-aggregate",
                "parts": [{"license": "MIT", "scope": "lib/fzy.lua"}],
            }
        )
        self.assertEqual(fields["license"], "NOASSERTION")
        self.assertEqual(fields["license_scope"], "component-aggregate")
        self.assertEqual(
            fields["license_parts"], [{"license": "MIT", "scope": "lib/fzy.lua"}]
        )
        # Components with a single aggregate license keep the existing shape;
        # license file content hashes are always recorded.
        self.assertEqual(
            manifest_license_fields({"spdx": "MIT"}),
            {"license": "MIT", "license_files": []},
        )

    def test_embedded_manifest_keeps_scoped_licenses(self) -> None:
        from build.assemble import write_metadata

        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            helper = stage / "playkit-updater.exe"
            helper.write_bytes(b"fake updater binary" * 4096)
            lock = {
                "components": {
                    "mpv-windows": {
                        "version": "20261021-git-abc",
                        "commit": "a" * 40,
                        "upstream": "shinchiro/mpv-winbuild-cmake",
                        "platforms": ["windows-x64"],
                        "provides": "mpv",
                        "sha256": "abc",
                        "license": {"spdx": "GPL-2.0-or-later", "files": []},
                    },
                    "uosc": {
                        "version": "1",
                        "upstream": "tomasklaen/uosc",
                        "release_tag": "1",
                        "release_url": "https://example.invalid",
                        "asset_name": "uosc.zip",
                        "sha256": "abc",
                        "license": {
                            "spdx": "NOASSERTION",
                            "scope": "component-aggregate",
                            "parts": [{"license": "MIT", "scope": "lib/fzy.lua"}],
                            "files": [],
                        },
                    },
                }
            }
            write_metadata(
                stage,
                version="2026.11",
                platform="windows-x64",
                repository="owner/repo",
                built_at="2026-10-31T00:00:00Z",
                source_sha="abc",
                inputs_hash="def",
                fetched={
                    "uosc": {"sha256": "abc"},
                    "mpv-windows": {"sha256": "abc"},
                },
                lock=lock,
                helper=helper,
            )
            manifest = read_json(stage / "manifest.json")
            component = manifest["components"]["uosc"]
            self.assertEqual(component["license"], "NOASSERTION")
            self.assertEqual(component["license_scope"], "component-aggregate")
            self.assertEqual(
                component["license_parts"], [{"license": "MIT", "scope": "lib/fzy.lua"}]
            )
            self.assertEqual(
                manifest["mpv"],
                {
                    "id": "mpv-windows",
                    "version": "20261021-git-abc",
                    "commit": "a" * 40,
                },
            )
            self.assertEqual(manifest["components"]["mpv-windows"]["commit"], "a" * 40)


class LicenseIndexTests(unittest.TestCase):
    def test_index_carries_per_file_scope(self) -> None:
        from build.assemble import copy_licenses

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            stage = root / "stage"
            work = root / "work"
            license_file = work / "licenses" / "uosc" / "MIT-fzy.txt"
            license_file.parent.mkdir(parents=True)
            license_file.write_text("MIT\n", encoding="utf-8")
            fetched = {
                "uosc": {
                    "license_files": [
                        {
                            "path": "MIT-fzy.txt",
                            "local_path": "licenses/uosc/MIT-fzy.txt",
                            "license": "MIT",
                            "repository": "swarn/fzy-lua",
                            "commit": "a" * 40,
                            "source_path": "LICENSE",
                            "scope": "lib/fzy.lua",
                            "url": "https://raw.githubusercontent.com/swarn/fzy-lua/"
                            + "a" * 40
                            + "/LICENSE",
                            "sha256": "abc",
                        }
                    ]
                }
            }
            lock = {
                "components": {
                    "uosc": {
                        "version": "1",
                        "upstream": "tomasklaen/uosc",
                        "license": {
                            "spdx": "NOASSERTION",
                            "scope": "component-aggregate",
                            "parts": [{"license": "MIT", "scope": "lib/fzy.lua"}],
                            "files": [],
                        },
                    }
                }
            }
            copy_licenses(stage, work, fetched, lock)
            entry = read_json(stage / "licenses" / "INDEX.json")["files"][0]
            self.assertEqual(entry["license"], "MIT")
            self.assertEqual(entry["scope"], "lib/fzy.lua")
            self.assertEqual(entry["repository"], "swarn/fzy-lua")
            self.assertEqual(entry["commit"], "a" * 40)
            self.assertEqual(entry["source_path"], "LICENSE")
            self.assertIn(f"/{'a' * 40}/", entry["source_url"])
            self.assertTrue((stage / "licenses" / "uosc" / "MIT-fzy.txt").is_file())


class UoscPatchTests(unittest.TestCase):
    def test_patch_is_applied_and_idempotent(self) -> None:
        from build.patch_uosc import MARKER, OBSERVER_ANCHOR, SCALE_ANCHOR, patch_uosc

        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            main = stage / "portable_config" / "scripts" / "uosc" / "main.lua"
            main.parent.mkdir(parents=True)
            main.write_text(
                "function update_display_dimensions()\n"
                + SCALE_ANCHOR
                + "\nend\n"
                + OBSERVER_ANCHOR
                + "\n",
                encoding="utf-8",
            )
            patch_uosc(stage)
            patched = main.read_text(encoding="utf-8")
            self.assertIn(MARKER, patched)
            self.assertIn("local user_scale = options.scale", patched)
            self.assertIn("mp.observe_property('display-width'", patched)
            patch_uosc(stage)
            self.assertEqual(main.read_text(encoding="utf-8"), patched)

    def test_patch_fails_when_upstream_moves_anchor(self) -> None:
        from build.common import BuildError
        from build.patch_uosc import patch_uosc

        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            main = stage / "portable_config" / "scripts" / "uosc" / "main.lua"
            main.parent.mkdir(parents=True)
            main.write_text("function update_display_dimensions()\nend\n", encoding="utf-8")
            with self.assertRaises(BuildError):
                patch_uosc(stage)


class InstallerTests(unittest.TestCase):
    def test_windows_installer_files(self) -> None:
        windows = ROOT / "installer" / "windows"
        for name in (
            "install.ps1",
            "uninstall.ps1",
            "install.cmd",
            "uninstall.cmd",
            "mpv-playkit.ico",
        ):
            self.assertTrue((windows / name).is_file(), name)
        install = (windows / "install.ps1").read_text(encoding="utf-8-sig")
        self.assertIn("HKCU", install)
        self.assertNotIn("HKLM", install)
        self.assertIn("Uninstall\\mpv-playkit", install)
        for marker in ("App Paths", "RegisteredApplications", "SystemFileAssociations", "Capabilities"):
            self.assertIn(marker, install)
        # SupportedTypes is a list of value names, not subkeys.
        self.assertIn("Applications\\mpv.exe\\SupportedTypes", install)
        self.assertNotIn("SupportedTypes\\.$ext", install)
        uninstall = (windows / "uninstall.ps1").read_text(encoding="utf-8-sig")
        self.assertIn("KeepUserData", uninstall)
        # The Capabilities key lives under Software\Clients\Media\mpv-playkit,
        # so removing that parent key covers it.
        for marker in (
            "App Paths",
            "RegisteredApplications",
            "SystemFileAssociations",
            "Software\\Clients\\Media\\mpv-playkit",
        ):
            self.assertIn(marker, uninstall)

    def test_linux_installer_is_a_placeholder(self) -> None:
        linux = ROOT / "installer" / "linux"
        script = (linux / "install.sh").read_text(encoding="utf-8")
        self.assertIn("尚未实现", script)
        self.assertIn("--prefix", script)
        desktop = (linux / "mpv-playkit.desktop.in").read_text(encoding="utf-8")
        self.assertIn("@EXEC@", desktop)
        self.assertIn("@ICON@", desktop)

    def test_installer_is_part_of_inputs(self) -> None:
        from build.inputs import input_files

        names = {str(path.relative_to(ROOT)).replace("\\", "/") for path in input_files()}
        self.assertIn("installer/README.md", names)
        self.assertIn("installer/windows/install.ps1", names)
        self.assertIn("installer/linux/install.sh", names)


class ReleaseChecksumTests(unittest.TestCase):
    def test_checksums_only_list_published_assets(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "manifest.json").write_text("{}\n", encoding="utf-8")
            (root / "mpv-PlayKit-2026.11-windows-x64.zip").write_bytes(b"zip")
            (root / "release-notes.md").write_text("notes\n", encoding="utf-8")
            (root / "SHA256SUMS").write_text("stale\n", encoding="utf-8")
            names = [line.split("  ", 1)[1] for line in checksum_lines(root)]
            self.assertEqual(
                names,
                ["manifest.json", "mpv-PlayKit-2026.11-windows-x64.zip"],
            )


class SmokePatternTests(unittest.TestCase):
    def test_script_load_failures_are_fatal(self) -> None:
        from build.smoke import FATAL_PATTERNS

        for text in (
            "Error loading script update-check.lua",
            "Error loading Lua script: thumbfast.lua",
            "Lua error: bad argument",
        ):
            self.assertIsNotNone(FATAL_PATTERNS.search(text), text)
        self.assertIsNone(FATAL_PATTERNS.search("playing 1x1.png normally"))


class ForbiddenContentTests(unittest.TestCase):
    def test_forbidden_paths_are_case_insensitive(self) -> None:
        from build.common import BuildError
        from build.verify import check_forbidden

        for path in (
            "portable_config/SHADERS/x.txt",
            "portable_config/VS/x.txt",
            "BASELINE/file.conf",
            "portable_config/SAVED-PROPS.JSON",
            "portable_config/UPDATE-CHECK-STATE.JSON",
            "a/b.GLSL",
            "a/b.VPY",
            "a/b.ONNX",
        ):
            with self.assertRaises(BuildError, msg=path):
                check_forbidden(path)
        check_forbidden("portable_config/scripts/uosc/main.lua")


class ZipSafetyTests(unittest.TestCase):
    def test_rejects_traversal(self) -> None:
        from build.common import safe_extract_zip

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            archive = root / "bad.zip"
            with zipfile.ZipFile(archive, "w") as handle:
                handle.writestr("../escape.txt", "bad")
            with self.assertRaises(Exception):
                safe_extract_zip(archive, root / "out")


class ArchiveLinkTests(unittest.TestCase):
    def _tar_with_link(self, root: Path, linkname: str) -> Path:
        asset = root / "links.tar.gz"
        with tarfile.open(asset, "w:gz") as archive:
            info = tarfile.TarInfo("top/bin/link")
            info.type = tarfile.SYMTYPE
            info.linkname = linkname
            archive.addfile(info)
        return asset

    def test_symlink_escape_rejected(self) -> None:
        from build.verify import list_archive

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for linkname in ("../../etc/passwd", "/etc/passwd", "C:\\Windows\\win.ini"):
                asset = self._tar_with_link(root, linkname)
                with self.assertRaises(Exception, msg=linkname):
                    list_archive(asset, "top")
                asset.unlink()

    def test_symlink_inside_package_allowed(self) -> None:
        from build.verify import list_archive

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            asset = self._tar_with_link(root, "../lib/x")
            self.assertEqual(list_archive(asset, "top"), ["top/bin/link"])

    def test_package_link_target(self) -> None:
        from build.package import safe_link_target

        self.assertEqual(safe_link_target("mpv/bin/link", "../lib/x"), "../lib/x")
        # Two levels up stays inside the package root (mpv/bin -> mpv -> root).
        self.assertEqual(
            safe_link_target("mpv/bin/link", "../../etc/passwd"), "../../etc/passwd"
        )
        for linkname in ("../../../etc/passwd", "/etc/passwd", "C:\\Windows\\win.ini"):
            with self.assertRaises(Exception, msg=linkname):
                safe_link_target("mpv/bin/link", linkname)


class SquashfsTests(unittest.TestCase):
    def test_find_offset(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "fake.AppImage"
            path.write_bytes(b"x" * 1234 + b"hsqs" + b"y" * 10)
            self.assertEqual(find_squashfs_offset(path), 1234)

    def test_hook_selection(self) -> None:
        for name in (
            "05-self-updater.hook",
            "05-self-update.hook",
            "05-get-yt-dlp.hook",
            "05-download-yt-dlp.hook",
            "05-Self-Updater.hook",
            "05-GET-YT-DLP.hook",
        ):
            self.assertTrue(hook_is_removed(name, DEFAULT_REMOVED_HOOKS), name)
        for name in (
            "01-check-ca-certs.hook",
            "01-path-mapping-hardcoded.hook",
            "05-fix-namespaces.hook",
        ):
            self.assertFalse(hook_is_removed(name, DEFAULT_REMOVED_HOOKS), name)


class UpdateCheckerTests(unittest.TestCase):
    def test_gate_precedes_network(self) -> None:
        text = (ROOT / "updater" / "update-check.lua").read_text(encoding="utf-8")
        gate = text.find("truthy_disabled(opts.enabled)")
        subprocess_call = text.find('name = "subprocess"')
        self.assertNotEqual(gate, -1)
        self.assertNotEqual(subprocess_call, -1)
        self.assertLess(gate, subprocess_call)
        self.assertIn("100pangci/mpv_PlayKit_TLS", (ROOT / "config" / "common" / "script-opts" / "update-check.conf").read_text(encoding="utf-8"))
        self.assertNotIn("os.execute", text)
        self.assertNotIn("io.popen", text)

    def test_gate_check_rejects_spacing_variants(self) -> None:
        from build.verify import update_check_gate_ok

        gate = "if truthy_disabled(opts.enabled) then return end\n"
        self.assertTrue(update_check_gate_ok(gate + 'name = "subprocess"'))
        self.assertTrue(update_check_gate_ok(gate + 'name="subprocess"'))
        self.assertFalse(update_check_gate_ok('name="subprocess"\n' + gate))
        self.assertFalse(update_check_gate_ok(gate))

    def test_saved_disable_keeps_menu_reset(self) -> None:
        text = (ROOT / "updater" / "update-check.lua").read_text(encoding="utf-8")
        # A saved disabled state must not return before the bindings are
        # registered, otherwise the in-player "reset" action disappears.
        self.assertIn(
            "if state.disabled == true then\n"
            "\t\tauto_check_allowed = false\n"
            "\t\tmp.msg.info",
            text,
        )
        self.assertIn("if auto_check_allowed then", text)
        self.assertIn("if state.disabled or checking then return end", text)
        self.assertIn(
            "if not local_version or not parse_version(local_version) then return end",
            text,
        )

    def test_disabled_config_returns_before_state_or_network(self) -> None:
        text = (ROOT / "updater" / "update-check.lua").read_text(encoding="utf-8")
        gate = text.find("truthy_disabled(opts.enabled)")
        state_path = text.find("local state_path")
        subprocess_call = text.find('name = "subprocess"')
        self.assertNotEqual(gate, -1)
        self.assertNotEqual(state_path, -1)
        self.assertNotEqual(subprocess_call, -1)
        self.assertLess(gate, state_path)
        self.assertLess(state_path, subprocess_call)

    def test_in_flight_disable_stops_the_next_http_client(self) -> None:
        text = (ROOT / "updater" / "update-check.lua").read_text(encoding="utf-8")
        marker = text.find(
            "The user may have disabled the checker while curl/wget was in"
        )
        self.assertNotEqual(marker, -1)
        self.assertLess(marker, text.find("try(index + 1)"))
        self.assertIn(
            "if state.disabled then\n"
            "\t\t\t\tchecking = false\n"
            "\t\t\t\treturn\n"
            "\t\t\tend",
            text,
        )

    def test_reset_clears_persistent_disable(self) -> None:
        text = (ROOT / "updater" / "update-check.lua").read_text(encoding="utf-8")
        reset = text.find('mp.add_key_binding(nil, "reset"')
        self.assertNotEqual(reset, -1)
        tail = text[reset:]
        self.assertIn("state.disabled = false", tail)
        self.assertIn("state.snoozed_until = 0", tail)
        self.assertIn("run_check(true)", tail)
        # The disable/reset bindings must exist regardless of saved state.
        self.assertIn('mp.add_key_binding(nil, "disable"', text)
        self.assertIn('mp.add_key_binding(nil, "check-now"', text)
        self.assertIn('mp.add_key_binding(nil, "open-release"', text)
        self.assertIn('mp.add_key_binding(nil, "snooze"', text)

    def test_state_write_failures_warn_instead_of_aborting(self) -> None:
        text = (ROOT / "updater" / "update-check.lua").read_text(encoding="utf-8")
        self.assertIn("could not write state file", text)
        self.assertIn("could not replace state file", text)
        # Temporary file + rename keeps a half-written state from replacing a
        # good one, and a failed write only warns.
        self.assertIn("local temp_path = state_path .. \".tmp\"", text)
        self.assertIn("os.remove(temp_path)", text)
        start = text.find("local function write_state()")
        end = text.find("local auto_check_allowed", start)
        self.assertNotEqual(start, -1)
        self.assertNotEqual(end, -1)
        # Only log via mp.msg.error; a failed state write must not call Lua's
        # error() and abort the script.
        body = text[start:end].replace("mp.msg.error(", "")
        self.assertNotIn("error(", body)


class LicensePinningTests(unittest.TestCase):
    def test_license_sources_use_repository_and_ref(self) -> None:
        _, components = load_manifests()
        for identifier, component in components.items():
            for item in component["license"]["files"]:
                self.assertNotIn("url", item, f"{identifier}/{item['path']}")
                self.assertTrue(item.get("repository"), f"{identifier}/{item['path']}")
                self.assertTrue(item.get("ref"), f"{identifier}/{item['path']}")

    def test_raw_url_uses_the_pinned_commit(self) -> None:
        from build.upstream import raw_url

        commit = "a" * 40
        url = raw_url("google/material-design-icons", commit, "LICENSE")
        self.assertEqual(
            url,
            f"https://raw.githubusercontent.com/google/material-design-icons/{commit}/LICENSE",
        )
        self.assertNotIn("/main/", url)
        self.assertNotIn("/master/", url)

    def test_committed_lock_licenses_are_pinned(self) -> None:
        lock = read_json(ROOT / "sources.lock.json")
        for identifier, component in lock["components"].items():
            for item in component["license"]["files"]:
                self.assertRegex(item["commit"], r"^[0-9a-f]{40}$", identifier)
                self.assertRegex(item["sha256"], r"^[0-9a-f]{64}$", identifier)
                self.assertIn(f"/{item['commit']}/", item["url"], identifier)
                self.assertNotRegex(item["url"], r"/(main|master|latest)/", identifier)
                self.assertTrue(item.get("repository"), identifier)

    def _component(self) -> dict:
        return {
            "id": "demo",
            "kind": "binary",
            "platforms": ["windows-x64"],
            "upstream": "owner/repo",
            "release": {"mode": "latest"},
            "asset": {
                "pattern": r"^demo-(?P<commit>[0-9a-f]+)\.bin$",
                "version": "{commit}",
                "commit": "{commit}",
                "commit_repository": "player/mpv",
            },
            "license": {
                "spdx": "MIT",
                "files": [
                    {
                        "path": "LICENSE",
                        "license": "MIT",
                        "repository": "player/mpv",
                        "ref": "{commit}",
                    }
                ],
            },
        }

    def _release(self) -> dict:
        return {
            "tag_name": "v1",
            "html_url": "https://example.invalid/v1",
            "published_at": "2026-01-01T00:00:00Z",
            "assets": [
                {
                    "name": "demo-abc123.bin",
                    "id": 1,
                    "url": "https://api.example.invalid/assets/1",
                    "browser_download_url": "https://example.invalid/demo.bin",
                    "size": 10,
                    "digest": "sha256:" + "b" * 64,
                }
            ],
        }

    def test_resolver_records_pinned_license_provenance(self) -> None:
        from unittest import mock

        from build.upstream import resolve_component

        with mock.patch("build.upstream.get_release", return_value=self._release()), \
                mock.patch("build.upstream.resolve_commit", return_value="c" * 40), \
                mock.patch("build.upstream.download_raw", return_value=b"MIT text\n"):
            locked = resolve_component(self._component())

        self.assertEqual(locked["commit"], "c" * 40)
        entry = locked["license"]["files"][0]
        self.assertEqual(entry["repository"], "player/mpv")
        self.assertEqual(entry["commit"], "c" * 40)
        self.assertEqual(entry["source_path"], "LICENSE")
        self.assertEqual(
            entry["url"],
            f"https://raw.githubusercontent.com/player/mpv/{'c' * 40}/LICENSE",
        )
        self.assertEqual(entry["sha256"], hashlib.sha256(b"MIT text\n").hexdigest())

    def test_unavailable_license_fails_closed(self) -> None:
        from unittest import mock

        from build.common import BuildError
        from build.upstream import resolve_component

        with mock.patch("build.upstream.get_release", return_value=self._release()), \
                mock.patch("build.upstream.resolve_commit", return_value="c" * 40), \
                mock.patch(
                    "build.upstream.download_raw",
                    side_effect=BuildError("license unavailable"),
                ):
            with self.assertRaises(BuildError):
                resolve_component(self._component())


class LockfileTests(unittest.TestCase):
    def test_verified_bytes_match(self) -> None:
        from build.lockfile import verify_lock_file

        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "sources.lock.json"
            path.write_text('{"components": {}}\n', encoding="utf-8")
            digest = sha256_file(path)
            self.assertEqual(verify_lock_file(path, digest), digest)

    def test_changed_bytes_are_rejected(self) -> None:
        from build.common import BuildError
        from build.lockfile import verify_lock_file

        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "sources.lock.json"
            path.write_text('{"components": {}}\n', encoding="utf-8")
            with self.assertRaises(BuildError):
                verify_lock_file(path, "0" * 64)
            with self.assertRaises(BuildError):
                verify_lock_file(path, "not-a-sha256")


class DecideTests(unittest.TestCase):
    def _lock(self, version: str = "1", sha: str = "a" * 64, inputs: str = "inputs") -> dict:
        return {
            "inputs_hash": inputs,
            "components": {
                "mpv-windows": {
                    "version": version,
                    "commit": "c" * 40,
                    "upstream": "owner/repo",
                    "release_tag": "1",
                    "release_url": "https://example.invalid/1",
                    "asset_name": "a.7z",
                    "sha256": sha,
                    "license": {"spdx": "MIT", "files": []},
                }
            },
        }

    def _manifest(self, lock: dict) -> dict:
        return {
            "components": compact_components(lock),
            "inputs_hash": lock["inputs_hash"],
        }

    def test_no_changes_does_not_release(self) -> None:
        from build.decide import decide_release

        lock = self._lock()
        decision = decide_release(
            force=False,
            release={"tag_name": "v2026.10"},
            manifest=self._manifest(lock),
            old_lock=lock,
            new_lock=lock,
            local_inputs_hash=lock["inputs_hash"],
        )
        self.assertFalse(decision["release_needed"])
        self.assertFalse(decision["upstream_changed"])
        self.assertFalse(decision["inputs_changed"])
        self.assertFalse(decision["lock_changed"])
        self.assertEqual(decision["reason"], "no changes")

    def test_force_always_releases(self) -> None:
        from build.decide import decide_release

        lock = self._lock()
        decision = decide_release(
            force=True,
            release={"tag_name": "v2026.10"},
            manifest=self._manifest(lock),
            old_lock=lock,
            new_lock=lock,
            local_inputs_hash=lock["inputs_hash"],
        )
        self.assertTrue(decision["release_needed"])
        self.assertIn("forced", decision["reason"])

    def test_upstream_change_releases(self) -> None:
        from build.decide import decide_release

        old = self._lock()
        new = self._lock(version="2", sha="d" * 64)
        decision = decide_release(
            force=False,
            release={"tag_name": "v2026.10"},
            manifest=self._manifest(old),
            old_lock=old,
            new_lock=new,
            local_inputs_hash=new["inputs_hash"],
        )
        self.assertTrue(decision["release_needed"])
        self.assertTrue(decision["upstream_changed"])
        self.assertTrue(decision["lock_changed"])

    def test_inputs_change_releases(self) -> None:
        from build.decide import decide_release

        lock = self._lock()
        manifest = self._manifest(lock)
        manifest["inputs_hash"] = "older-inputs"
        decision = decide_release(
            force=False,
            release={"tag_name": "v2026.10"},
            manifest=manifest,
            old_lock=lock,
            new_lock=lock,
            local_inputs_hash=lock["inputs_hash"],
        )
        self.assertTrue(decision["release_needed"])
        self.assertTrue(decision["inputs_changed"])
        self.assertFalse(decision["upstream_changed"])

    def test_missing_manifest_releases(self) -> None:
        from build.decide import decide_release

        lock = self._lock()
        decision = decide_release(
            force=False,
            release={"tag_name": "v2026.10"},
            manifest=None,
            old_lock=lock,
            new_lock=lock,
            local_inputs_hash=lock["inputs_hash"],
        )
        self.assertTrue(decision["release_needed"])
        self.assertIn("no readable latest manifest", decision["reason"])

    def test_lock_only_repair_does_not_release(self) -> None:
        from build.decide import decide_release

        old = self._lock(version="0", sha="0" * 64)
        new = self._lock()
        decision = decide_release(
            force=False,
            release={"tag_name": "v2026.10"},
            manifest=self._manifest(new),
            old_lock=old,
            new_lock=new,
            local_inputs_hash=new["inputs_hash"],
        )
        self.assertFalse(decision["release_needed"])
        self.assertTrue(decision["lock_changed"])


class PlatformMpvTests(unittest.TestCase):
    def _lock(self) -> dict:
        return {
            "components": {
                "mpv-windows": {
                    "version": "20261021-git-e471496c56",
                    "commit": "a" * 40,
                    "provides": "mpv",
                    "platforms": ["windows-x64"],
                },
                "mpv-linux": {
                    "version": "v0.41.0-1373-g9e2ed65eb",
                    "commit": "b" * 40,
                    "provides": "mpv",
                    "platforms": ["linux-x64"],
                },
            }
        }

    def test_each_platform_keeps_its_own_mpv(self) -> None:
        from build.release import mpv_for_platform

        windows = mpv_for_platform(self._lock(), "windows-x64")
        linux = mpv_for_platform(self._lock(), "linux-x64")
        self.assertEqual(windows["id"], "mpv-windows")
        self.assertEqual(windows["commit"], "a" * 40)
        self.assertEqual(linux["id"], "mpv-linux")
        self.assertEqual(linux["commit"], "b" * 40)
        self.assertNotEqual(windows["commit"], linux["commit"])

    def test_missing_mpv_is_fatal(self) -> None:
        from build.common import BuildError
        from build.release import mpv_for_platform

        with self.assertRaises(BuildError):
            mpv_for_platform({"components": {}}, "windows-x64")


class LicenseVerifyTests(unittest.TestCase):
    def _entry(self, **overrides) -> dict:
        entry = {
            "license": "MIT",
            "commit": "a" * 40,
            "source_url": "https://raw.githubusercontent.com/owner/repo/"
            + "a" * 40
            + "/LICENSE",
            "sha256": "b" * 64,
        }
        entry.update(overrides)
        return entry

    def test_pinned_entry_passes(self) -> None:
        from build.verify import check_license_entry

        check_license_entry("licenses/demo/LICENSE", self._entry())

    def test_branch_urls_are_rejected(self) -> None:
        from build.common import BuildError
        from build.verify import check_license_entry

        for branch in ("main", "master", "latest"):
            url = f"https://raw.githubusercontent.com/owner/repo/{branch}/LICENSE"
            with self.assertRaises(BuildError, msg=branch):
                check_license_entry("licenses/demo/LICENSE", self._entry(source_url=url))

    def test_missing_commit_or_hash_is_rejected(self) -> None:
        from build.common import BuildError
        from build.verify import check_license_entry

        with self.assertRaises(BuildError):
            check_license_entry("licenses/demo/LICENSE", self._entry(commit=None))
        with self.assertRaises(BuildError):
            check_license_entry("licenses/demo/LICENSE", self._entry(sha256=None))

    def test_manifest_license_hashes_must_match_index(self) -> None:
        from build.common import BuildError
        from build.verify import check_manifest_license_files

        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            (stage / "licenses" / "demo").mkdir(parents=True)
            (stage / "licenses" / "demo" / "LICENSE").write_text("MIT\n", encoding="utf-8")
            (stage / "licenses" / "INDEX.json").write_text(
                json.dumps(
                    {
                        "schema": 1,
                        "files": [
                            {
                                "component": "demo",
                                "file": "licenses/demo/LICENSE",
                                "license": "MIT",
                                "sha256": "b" * 64,
                                "commit": "a" * 40,
                                "source_url": "https://raw.githubusercontent.com/o/r/"
                                + "a" * 40
                                + "/LICENSE",
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            good = {
                "components": {
                    "demo": {
                        "license_files": [
                            {"path": "LICENSE", "license": "MIT", "sha256": "b" * 64}
                        ]
                    }
                }
            }
            check_manifest_license_files(stage, good)
            bad_hash = {
                "components": {
                    "demo": {
                        "license_files": [
                            {"path": "LICENSE", "license": "MIT", "sha256": "c" * 64}
                        ]
                    }
                }
            }
            with self.assertRaises(BuildError):
                check_manifest_license_files(stage, bad_hash)
            missing = {
                "components": {
                    "demo": {
                        "license_files": [
                            {"path": "OTHER", "license": "MIT", "sha256": "b" * 64}
                        ]
                    }
                }
            }
            with self.assertRaises(BuildError):
                check_manifest_license_files(stage, missing)


class ReleaseManifestTests(unittest.TestCase):
    VERSION = "2026.11"
    SOURCE_SHA = "c" * 40

    def _lock(self) -> dict:
        return {
            "schema": 1,
            "components": {
                "mpv-windows": {
                    "version": "20261021-git-e471496c56",
                    "commit": "a" * 40,
                    "upstream": "shinchiro/mpv-winbuild-cmake",
                    "release_tag": "20261021",
                    "release_url": "https://example.invalid/windows",
                    "asset_name": "mpv.7z",
                    "sha256": "1" * 64,
                    "provides": "mpv",
                    "platforms": ["windows-x64"],
                    "license": {"spdx": "GPL-2.0-or-later", "files": []},
                },
                "mpv-linux": {
                    "version": "v0.41.0-1373-g9e2ed65eb",
                    "commit": "b" * 40,
                    "upstream": "pkgforge-dev/mpv-AppImage",
                    "release_tag": "nightly",
                    "release_url": "https://example.invalid/linux",
                    "asset_name": "mpv.AppImage",
                    "sha256": "2" * 64,
                    "provides": "mpv",
                    "platforms": ["linux-x64"],
                    "license": {"spdx": "NOASSERTION", "files": []},
                },
            },
        }

    def _embedded(self, platform: str, identifier: str, version: str, commit: str, sha: str) -> dict:
        return {
            "schema": 1,
            "playkit_version": self.VERSION,
            "platform": platform,
            "source_commit": self.SOURCE_SHA,
            "inputs_hash": "i" * 64,
            "mpv": {"id": identifier, "version": version, "commit": commit},
            "components": {
                identifier: {"version": version, "commit": commit, "sha256": sha}
            },
        }

    def _dist(self, root: Path) -> Path:
        dist = root / "dist"
        dist.mkdir()
        windows_top = f"mpv-PlayKit-{self.VERSION}-windows-x64"
        linux_top = f"mpv-PlayKit-{self.VERSION}-linux-x64"
        windows_manifest = self._embedded(
            "windows-x64", "mpv-windows", "20261021-git-e471496c56", "a" * 40, "1" * 64
        )
        linux_manifest = self._embedded(
            "linux-x64", "mpv-linux", "v0.41.0-1373-g9e2ed65eb", "b" * 40, "2" * 64
        )
        with zipfile.ZipFile(dist / f"{windows_top}.zip", "w") as archive:
            archive.writestr(f"{windows_top}/manifest.json", json.dumps(windows_manifest))
        payload = json.dumps(linux_manifest).encode("utf-8")
        with tarfile.open(dist / f"{linux_top}.tar.gz", "w:gz") as archive:
            info = tarfile.TarInfo(f"{linux_top}/manifest.json")
            info.size = len(payload)
            archive.addfile(info, io.BytesIO(payload))
        return dist

    def _args(self, root: Path, dist: Path, **extra) -> list[str]:
        lock = root / "sources.lock.json"
        lock.write_text(json.dumps(self._lock()), encoding="utf-8")
        args = [
            "--version", self.VERSION,
            "--tag", "v" + self.VERSION,
            "--dist", str(dist),
            "--lock", str(lock),
            "--repository", "owner/repo",
            "--out-dir", str(root / "release"),
            "--source-sha", self.SOURCE_SHA,
        ]
        for key, value in extra.items():
            args.extend([key, value])
        return args

    def test_release_manifest_names_each_platform_mpv(self) -> None:
        from build.release import main as release_main

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dist = self._dist(root)
            self.assertEqual(release_main(self._args(root, dist)), 0)
            manifest = read_json(root / "release" / "manifest.json")
            windows = manifest["platforms"]["windows-x64"]
            linux = manifest["platforms"]["linux-x64"]
            self.assertEqual(windows["mpv_version"], "20261021-git-e471496c56")
            self.assertEqual(windows["mpv_commit"], "a" * 40)
            self.assertEqual(linux["mpv_version"], "v0.41.0-1373-g9e2ed65eb")
            self.assertEqual(linux["mpv_commit"], "b" * 40)
            self.assertNotEqual(windows["mpv_commit"], linux["mpv_commit"])
            notes = (root / "release" / "release-notes.md").read_text(encoding="utf-8")
            self.assertIn("20261021-git-e471496c56", notes)
            self.assertIn("v0.41.0-1373-g9e2ed65eb", notes)

    def test_release_rejects_a_package_from_another_commit(self) -> None:
        from build.common import BuildError
        from build.release import main as release_main

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dist = self._dist(root)
            args = self._args(root, dist)
            args[args.index("--source-sha") + 1] = "d" * 40
            with self.assertRaises(BuildError):
                release_main(args)

    def test_release_rejects_a_different_lock(self) -> None:
        from build.common import BuildError
        from build.release import main as release_main

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dist = self._dist(root)
            args = self._args(root, dist, **{"--expected-lock-sha256": "0" * 64})
            with self.assertRaises(BuildError):
                release_main(args)


class WorkflowTests(unittest.TestCase):
    WORKFLOW_DIR = ROOT / ".github" / "workflows"

    def _workflow(self, name: str) -> dict:
        from build.common import load_yaml

        return load_yaml(self.WORKFLOW_DIR / name)

    def _triggers(self, name: str) -> dict:
        data = self._workflow(name)
        triggers = data.get("on")
        if triggers is None:
            # PyYAML follows YAML 1.1 and resolves the bare key `on` to True.
            triggers = data.get(True)
        return triggers or {}

    def _uses(self, data: dict):
        for job in (data.get("jobs") or {}).values():
            if job.get("uses"):
                yield job["uses"]
            for step in job.get("steps") or []:
                if step.get("uses"):
                    yield step["uses"]

    def _runs(self, job: dict) -> str:
        return "\n".join(str(step.get("run", "")) for step in job.get("steps") or [])

    def test_every_remote_action_is_pinned_to_a_full_commit(self) -> None:
        pattern = re.compile(
            r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)*@[0-9a-f]{40}$"
        )
        for path in sorted(self.WORKFLOW_DIR.glob("*.yml")):
            data = self._workflow(path.name)
            for uses in self._uses(data):
                if uses.startswith("./"):
                    continue
                self.assertRegex(uses, pattern, f"{path.name}: {uses}")

    def test_candidate_lock_inputs_are_declared(self) -> None:
        for name in ("build.yml", "release.yml"):
            inputs = (self._triggers(name).get("workflow_call") or {}).get("inputs") or {}
            self.assertIn("lock_artifact", inputs, name)
            self.assertIn("lock_sha256", inputs, name)

    def test_monthly_builds_the_candidate_before_promoting_it(self) -> None:
        jobs = self._workflow("monthly-update.yml")["jobs"]
        build = jobs["build"]
        self.assertEqual(build["needs"], "resolve")
        self.assertEqual(build["uses"], "./.github/workflows/build.yml")
        self.assertEqual(
            build["with"]["lock_artifact"], "${{ needs.resolve.outputs.lock_artifact }}"
        )
        self.assertEqual(
            build["with"]["lock_sha256"], "${{ needs.resolve.outputs.lock_sha256 }}"
        )
        self.assertEqual(
            build["with"]["source_sha"], "${{ needs.resolve.outputs.candidate_sha }}"
        )

        commit = jobs["commit-lock"]
        self.assertEqual(commit["needs"], ["resolve", "build"])
        self.assertIn("always()", commit["if"])
        self.assertIn("needs.build.result == 'success'", commit["if"])
        self.assertIn("needs.resolve.outputs.release_needed != 'true'", commit["if"])

        release = jobs["release"]
        self.assertIn("build", release["needs"])
        self.assertIn("commit-lock", release["needs"])
        self.assertEqual(
            release["with"]["source_sha"], "${{ needs.commit-lock.outputs.final_sha }}"
        )
        self.assertEqual(
            release["with"]["lock_sha256"], "${{ needs.resolve.outputs.lock_sha256 }}"
        )

    def test_candidate_artifact_always_matches_the_repository_lock(self) -> None:
        jobs = self._workflow("monthly-update.yml")["jobs"]
        runs = self._runs(jobs["resolve"])
        self.assertIn("cp .work/candidate/sources.lock.json sources.lock.json", runs)
        # When only generated_at would differ, the repository lock is used
        # verbatim so every consumer sees identical bytes.
        self.assertIn("cp sources.lock.json .work/candidate/sources.lock.json", runs)
        self.assertLess(
            runs.index("cp sources.lock.json .work/candidate/sources.lock.json"),
            runs.index('lock_sha="$(python'),
        )

    def test_only_the_commit_job_updates_main(self) -> None:
        jobs = self._workflow("monthly-update.yml")["jobs"]
        for name, job in jobs.items():
            runs = self._runs(job)
            if name == "commit-lock":
                self.assertIn("refs/heads/${TARGET_BRANCH}", runs)
            else:
                self.assertNotIn("refs/heads/${TARGET_BRANCH}", runs)
                self.assertNotIn(":refs/heads/main", runs)
        resolve_runs = self._runs(jobs["resolve"])
        self.assertIn("refs/heads/${branch}", resolve_runs)
        self.assertNotIn("TARGET_BRANCH", resolve_runs)

    def test_both_platforms_install_the_same_candidate_lock(self) -> None:
        jobs = self._workflow("build.yml")["jobs"]
        for name in ("windows-x64", "linux-x64"):
            steps = jobs[name]["steps"]
            by_name = {step.get("name"): step for step in steps}
            download = by_name["Download candidate lock"]
            install = by_name["Install verified candidate lock"]
            self.assertEqual(download["with"]["name"], "${{ inputs.lock_artifact }}")
            self.assertIn("inputs.lock_sha256", install["run"])
            self.assertIn(
                "cp .work/candidate-lock/sources.lock.json sources.lock.json",
                install["run"],
            )
            names = [step.get("name") for step in steps]
            self.assertLess(
                names.index("Install verified candidate lock"),
                names.index("Fetch upstream assets"),
            )

    def test_build_workflow_never_commits(self) -> None:
        text = (self.WORKFLOW_DIR / "build.yml").read_text(encoding="utf-8")
        self.assertNotIn("git commit", text)
        self.assertNotIn("git push", text)

    def test_release_verifies_the_candidate_lock_and_target(self) -> None:
        job = self._workflow("release.yml")["jobs"]["release"]
        runs = self._runs(job)
        self.assertIn("build.lockfile", runs)
        self.assertIn(
            "cmp .work/candidate-lock/sources.lock.json sources.lock.json", runs
        )
        self.assertIn("--expected-lock-sha256", runs)
        self.assertIn('--target "$SOURCE_SHA"', runs)
        self.assertIn("refs/tags/$TAG", runs)
        self.assertIn("release $TAG already exists", runs)

    def test_force_input_is_forwarded_to_decide(self) -> None:
        job = self._workflow("monthly-update.yml")["jobs"]["resolve"]
        decide = next(step for step in job["steps"] if step.get("id") == "decide")
        self.assertEqual(decide["env"]["FORCE"], "${{ inputs.force }}")
        self.assertIn("--force", decide["run"])

    def test_fork_pull_requests_do_not_run_build_jobs(self) -> None:
        jobs = self._workflow("build.yml")["jobs"]
        for name in ("tests", "windows-x64", "linux-x64"):
            self.assertIn(
                "pull_request.head.repo.full_name == github.repository",
                jobs[name]["if"],
            )


    def test_installs_include_verified_updater_and_user_state(self) -> None:
        from build.userstate import (
            read_state,
            verify_package_state,
            write_inventory,
            write_package_state,
        )

        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            portable = stage / "portable_config"
            portable.mkdir(parents=True)
            (portable / "mpv.conf").write_text("volume=100\n", encoding="utf-8")
            (portable / "input.conf").write_text(
                "MBTN_LEFT cycle pause\n", encoding="utf-8"
            )
            (portable / "profiles.conf").write_text("[gpu-hq]\n", encoding="utf-8")
            (portable / "platform.conf").write_text("hwdec=auto\n", encoding="utf-8")
            (portable / "script-opts").mkdir()
            (portable / "script-opts" / "update-check.conf").write_text(
                "enabled=yes\n", encoding="utf-8"
            )
            write_package_state(stage, "2026.11", "windows-x64", "2026-01-01T00:00:00Z")
            write_inventory(stage)

            state = read_state(stage)
            self.assertEqual(state["installed_version"], "2026.11")
            self.assertEqual(state["official"], state["generated"])
            self.assertIn("mpv.conf", state["official"])
            self.assertIn("script-opts/update-check.conf", state["official"])
            self.assertTrue((portable / "user" / "README.md").is_file())
            self.assertTrue(
                (portable / ".playkit" / "official" / "mpv.conf").is_file()
            )
            verify_package_state(stage, "2026.11", "windows-x64")

            (portable / "mpv.conf").write_text("volume=100\nchanged=yes\n", encoding="utf-8")
            with self.assertRaises(Exception):
                verify_package_state(stage, "2026.11", "windows-x64")

    def test_updater_version_matches_crate(self) -> None:
        from build.helper import MIN_HELPER_VERSION, helper_version

        self.assertEqual(helper_version(), "1.1.0")
        self.assertEqual(MIN_HELPER_VERSION, "1.0.0")

    def test_install_binding_requires_the_enabled_gate(self) -> None:
        text = (ROOT / "updater" / "update-check.lua").read_text(encoding="utf-8")
        start = text.find("local function updater_available")
        install = text.find("local function install_now")
        capture = text.find("local function capture_now")
        rollback = text.find("local function rollback_now")
        repair = text.find("local function repair_now")
        end = text.find("mp.add_key_binding(nil, \"check-now\"")
        self.assertTrue(0 < start < install < capture < rollback < repair < end)
        gate = text[start:install]
        self.assertIn("truthy_disabled(opts.install_enabled)", gate)
        self.assertIn("state.disabled", gate)
        self.assertIn("file_exists(helper_path)", gate)
        for name, begin, finish in (
            ("install", install, capture),
            ("capture", capture, rollback),
            ("rollback", rollback, repair),
            ("repair", repair, end),
        ):
            body = text[begin:finish]
            self.assertIn("updater_available()", body, name)
        self.assertIn('"--wait-pid"', text[install:capture])
        self.assertIn('"--detach"', text[install:capture])
        self.assertIn('"--wait-pid"', text[rollback:repair])
        self.assertIn('"--detach"', text[rollback:repair])
        for name in ("install-now", "capture-now", "rollback", "repair-now"):
            self.assertIn(f"mp.add_key_binding(nil, \"{name}\"", text)
            self.assertIn(
                f"script-binding update-check/{name}",
                (ROOT / "config" / "common" / "input.conf").read_text(encoding="utf-8"),
            )

    def test_auto_repair_is_gated_by_the_enabled_switch(self) -> None:
        text = (ROOT / "updater" / "update-check.lua").read_text(encoding="utf-8")
        gate = text.find("truthy_disabled(opts.enabled)")
        auto_repair = text.find("local function auto_repair_allowed")
        self.assertNotEqual(gate, -1)
        self.assertNotEqual(auto_repair, -1)
        self.assertLess(gate, auto_repair)
        self.assertIn("truthy_disabled(opts.auto_repair)", text)
        self.assertIn('"repair-auto"', text)
        self.assertIn("auto_repair_relaunch", text)
        self.assertIn("last_repaired_version = \"\"", text)
        config = (
            ROOT / "config" / "common" / "script-opts" / "update-check.conf"
        ).read_text(encoding="utf-8")
        self.assertIn("auto_repair=yes", config)
        self.assertIn("auto_repair_relaunch=auto", config)

    def test_commit_lock_installs_python_dependencies(self) -> None:
        steps = self._workflow("monthly-update.yml")["jobs"]["commit-lock"]["steps"]
        commands = " ".join(str(step.get("run", "")) for step in steps)
        self.assertIn("pip install -r requirements.txt", commands)

    def test_rust_updater_tests_run_on_both_platforms(self) -> None:
        jobs = self._workflow("build.yml")["jobs"]
        self.assertIn("rust-tests", jobs)
        matrix = jobs["rust-tests"]["strategy"]["matrix"]["os"]
        self.assertIn("ubuntu-24.04", matrix)
        self.assertIn("windows-latest", matrix)
        self.assertIn("cargo test", self._runs(jobs["rust-tests"]))
        self.assertIn("cargo clippy", self._runs(jobs["rust-tests"]))
        for platform in ("windows-x64", "linux-x64"):
            self.assertIn("build.helper", self._runs(jobs[platform]))


if __name__ == "__main__":
    unittest.main()
