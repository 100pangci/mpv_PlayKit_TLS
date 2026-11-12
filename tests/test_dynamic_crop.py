from __future__ import annotations

import json
import unittest
from pathlib import Path

import yaml

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "manifests" / "scripts.yml"
LOCK = ROOT / "sources.lock.json"
CONFIG = ROOT / "config" / "common" / "script-opts" / "dynamic_crop.conf"
LOCAL_SCRIPT = ROOT / "scripts" / "local" / "dynamic_crop.lua"
UPSTREAM = "Ashyni/mpv-scripts"
COMMIT = "d3f5685f5209ae548f8398b21d4dcbbea766d076"
SHA256 = "332f314b324617d7038c00c3ada08d257160e10766c801800dbdea325fe11022"
TARGET = "portable_config/scripts/dynamic_crop.lua"


def load_manifest() -> dict:
    data = yaml.safe_load(MANIFEST.read_text(encoding="utf-8"))
    return {item["id"]: item for item in data["components"]}


def parse_options(path: Path) -> dict[str, str]:
    options: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#") or "=" not in stripped:
            continue
        key, value = stripped.split("=", 1)
        options[key.strip()] = value.strip()
    return options


class DynamicCropManifestTests(unittest.TestCase):
    def test_manifest_pins_the_original_upstream(self) -> None:
        component = load_manifest()["ashyni-dynamic-crop"]
        self.assertEqual(component["kind"], "raw-files")
        self.assertEqual(component["upstream"], UPSTREAM)
        self.assertEqual(component["ref"], COMMIT)
        self.assertEqual(component["license"]["spdx"], "MIT")
        self.assertEqual(component["update"]["mode"], "latest-commit")
        self.assertIn("lavfi cropdetect", component["update"]["license_marker"])

        [item] = component["files"]
        self.assertEqual(item["source"], "dynamic-crop.lua")
        self.assertEqual(item["target"], TARGET)
        self.assertEqual(item["sha256"], SHA256)
        self.assertEqual(item["license"], "MIT")

    def test_lock_matches_the_manifest(self) -> None:
        lock = json.loads(LOCK.read_text(encoding="utf-8"))["components"]
        component = lock["ashyni-dynamic-crop"]
        self.assertEqual(component["ref"], COMMIT)
        self.assertEqual(component["upstream"], UPSTREAM)
        [item] = component["files"]
        self.assertEqual(item["target"], TARGET)
        self.assertEqual(item["sha256"], SHA256)
        [license_file] = component["license"]["files"]
        self.assertEqual(license_file["repository"], UPSTREAM)
        self.assertEqual(license_file["path"], "LICENSE")

    def test_script_is_downloaded_not_vendored(self) -> None:
        self.assertFalse(LOCAL_SCRIPT.exists(), LOCAL_SCRIPT)


class DynamicCropConfigTests(unittest.TestCase):
    def test_defaults_are_on_demand_and_use_video_crop(self) -> None:
        options = parse_options(CONFIG)
        self.assertEqual(options.get("mode"), "1")
        self.assertEqual(options.get("crop_method"), "1")
        self.assertEqual(options.get("detect_limit"), "26")
        self.assertEqual(options.get("detect_round"), "2")
        self.assertEqual(options.get("fix_windowed_behavior"), "1")
        self.assertEqual(options.get("prevent_change_mode"), "0")
        for key in (
            "fast_change_timer",
            "ratio_timer",
            "offset_timer",
            "fallback_timer",
            "linked_tolerance",
            "ratio_tolerance",
            "segmentation",
        ):
            self.assertIn(key, options)
        # The old locally authored crop script used its own enabled/mode names.
        self.assertNotIn("enabled", options)

    def test_menu_and_key_binding_are_wired(self) -> None:
        menu = (ROOT / "config" / "common" / "menu.conf").read_text(encoding="utf-8")
        input_conf = (ROOT / "config" / "common" / "input.conf").read_text(encoding="utf-8")
        self.assertIn("script-binding dynamic_crop/toggle_crop", menu)
        self.assertIn("script-binding dynamic_crop/toggle_crop", input_conf)
        self.assertIn("#! 视频 > 切换 动态裁切", input_conf)
        self.assertRegex(input_conf, r"(?m)^Alt\+c\s+script-binding dynamic_crop/toggle_crop")


class DynamicCropReleaseGateTests(unittest.TestCase):
    def test_verify_requires_the_script_options_and_license(self) -> None:
        from build.verify import REQUIRED, REQUIRED_LICENSES

        self.assertIn(TARGET, REQUIRED)
        self.assertIn("portable_config/script-opts/dynamic_crop.conf", REQUIRED)
        for platform in ("windows-x64", "linux-x64"):
            self.assertIn(
                "licenses/ashyni-dynamic-crop/LICENSE", REQUIRED_LICENSES[platform]
            )


if __name__ == "__main__":
    unittest.main()
