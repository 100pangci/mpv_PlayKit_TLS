from __future__ import annotations

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
        # Components with a single aggregate license keep the existing shape.
        self.assertEqual(manifest_license_fields({"spdx": "MIT"}), {"license": "MIT"})

    def test_embedded_manifest_keeps_scoped_licenses(self) -> None:
        from build.assemble import write_metadata

        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            lock = {
                "components": {
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
                    }
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
                fetched={"uosc": {"sha256": "abc"}},
                lock=lock,
            )
            component = read_json(stage / "manifest.json")["components"]["uosc"]
            self.assertEqual(component["license"], "NOASSERTION")
            self.assertEqual(component["license_scope"], "component-aggregate")
            self.assertEqual(
                component["license_parts"], [{"license": "MIT", "scope": "lib/fzy.lua"}]
            )


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
                            "url": "https://example.invalid/MIT",
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


if __name__ == "__main__":
    unittest.main()
