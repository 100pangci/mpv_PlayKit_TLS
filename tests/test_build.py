from __future__ import annotations

import json
import re
import tempfile
import unittest
import zipfile
from datetime import datetime, timezone
from pathlib import Path

from build.common import ROOT, load_manifests, package_filename, sha256_file
from build.decide import compact_components, release_components
from build.extract import DEFAULT_REMOVED_HOOKS, find_squashfs_offset, hook_is_removed
from build.inputs import compute_inputs_hash
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
        self.assertEqual(labels[("yt-dlp-windows", "THIRD_PARTY_LICENSES.txt")], "NOASSERTION")

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
                    "asset_name": "a.7z",
                    "sha256": "abc",
                }
            }
        }
        published = {
            "components": {
                "mpv": {
                    "version": "1",
                    "upstream": "owner/repo",
                    "release_tag": "v1",
                    "asset_name": "a.7z",
                    "sha256": "abc",
                    "release_url": "https://example.invalid",
                    "license": "MIT",
                }
            }
        }
        self.assertEqual(release_components(published), compact_components(lock))

    def test_package_name(self) -> None:
        self.assertEqual(package_filename("2026.11", "windows-x64"), "mpv-PlayKit-2026.11-windows-x64.zip")
        self.assertEqual(package_filename("2026.11", "linux-x64"), "mpv-PlayKit-2026.11-linux-x64.tar.gz")


class InputsTests(unittest.TestCase):
    def test_inputs_hash_changes_with_config(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "config").mkdir()
            (root / "config" / "mpv.conf").write_text("a=1\n", encoding="utf-8")
            first = compute_inputs_hash(root)
            (root / "config" / "mpv.conf").write_text("a=2\n", encoding="utf-8")
            second = compute_inputs_hash(root)
            self.assertNotEqual(first, second)


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


class SquashfsTests(unittest.TestCase):
    def test_find_offset(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "fake.AppImage"
            path.write_bytes(b"x" * 1234 + b"hsqs" + b"y" * 10)
            self.assertEqual(find_squashfs_offset(path), 1234)

    def test_hook_selection(self) -> None:
        self.assertTrue(hook_is_removed("05-self-updater.hook", DEFAULT_REMOVED_HOOKS))
        self.assertTrue(hook_is_removed("05-get-yt-dlp.hook", DEFAULT_REMOVED_HOOKS))
        self.assertFalse(hook_is_removed("01-check-ca-certs.hook", DEFAULT_REMOVED_HOOKS))
        self.assertFalse(hook_is_removed("01-path-mapping-hardcoded.hook", DEFAULT_REMOVED_HOOKS))
        self.assertFalse(hook_is_removed("05-fix-namespaces.hook", DEFAULT_REMOVED_HOOKS))


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


if __name__ == "__main__":
    unittest.main()
