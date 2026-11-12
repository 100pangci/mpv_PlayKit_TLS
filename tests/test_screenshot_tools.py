from __future__ import annotations

import re
import shutil
import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "local" / "screenshot_tools.lua"
CONFIG = ROOT / "config" / "common" / "script-opts" / "screenshot_tools.conf"
LUA_TEST = ROOT / "tests" / "lua" / "test_screenshot_tools.lua"
LUA = shutil.which("luajit") or shutil.which("lua")

EXPECTED_BINDINGS = {
    "capture",
    "video",
    "subtitles",
    "window",
    "region",
    "burst",
    "stitch",
    "cancel",
}


def read_script() -> str:
    return SCRIPT.read_text(encoding="utf-8")


def parse_options(path: Path) -> dict[str, str]:
    options: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#") or "=" not in stripped:
            continue
        key, value = stripped.split("=", 1)
        options[key.strip()] = value.strip()
    return options


class ScreenshotToolsLayoutTests(unittest.TestCase):
    def test_script_config_and_lua_test_exist(self) -> None:
        self.assertTrue(SCRIPT.is_file(), SCRIPT)
        self.assertTrue(CONFIG.is_file(), CONFIG)
        self.assertTrue(LUA_TEST.is_file(), LUA_TEST)

    def test_script_is_original(self) -> None:
        text = read_script()
        self.assertIn("newly authored for mpv_PlayKit", text)
        self.assertIn("no shell command is assembled from strings", text)
        # The historical PlayKit scripts must not be copied.
        self.assertNotIn("mpv_PlayKit_TLS/scripts", text)
        self.assertNotIn("thumb_engine", text)

    def test_config_defaults_keep_user_data_in_the_user_layer(self) -> None:
        options = parse_options(CONFIG)
        self.assertEqual(options.get("enabled"), "yes")
        self.assertTrue(options.get("directory", "").startswith("~~/user/"))
        self.assertTrue(options.get("queue_file", "").startswith("~~/user/"))
        self.assertEqual(options.get("format"), "png")
        self.assertEqual(options.get("default_mode"), "subtitles")
        self.assertEqual(options.get("burst_count"), "5")
        self.assertEqual(options.get("burst_interval"), "1.0")
        self.assertEqual(options.get("burst_mode"), "subtitles")
        self.assertEqual(options.get("burst_max_count"), "200")
        self.assertEqual(options.get("stitch_direction"), "hstack")
        self.assertEqual(options.get("ffmpeg_path"), "ffmpeg")
        self.assertEqual(options.get("subprocess_timeout"), "120")
        self.assertIn("capture_delay", options)
        self.assertIn("file_timeout", options)
        self.assertEqual(options.get("region"), "")

    def test_script_does_not_patch_upstream_components(self) -> None:
        text = read_script()
        for marker in ("uosc/main.lua", "scripts/uosc"):
            self.assertNotIn(marker, text)


class ScreenshotToolsBehaviourTests(unittest.TestCase):
    def test_capture_modes_and_crop_restore(self) -> None:
        text = read_script()
        self.assertIn('video = true, subtitles = true, window = true', text)
        self.assertIn('"screenshot-to-file", path, mode', text)
        # The user's crop must be read before the temporary crop is applied and
        # written back after the screenshot file appears.
        self.assertIn('mp.get_property("video-crop")', text)
        self.assertIn('format_crop(job.region)', text)
        self.assertIn('pcall(mp.set_property, "video-crop", context.original_crop)', text)
        self.assertIn("function validate_region", text)
        self.assertIn("截图区域超出视频范围", text)
        # The no-argument menu binding prompts for a rectangle, and a file
        # switch cancels a pending crop capture so the temporary property can
        # never leak onto the next file.
        self.assertIn('pcall(require, "mp.input")', text)
        self.assertIn('mp.register_event("end-file"', text)

    def test_screenshot_quality_follows_mpv_properties(self) -> None:
        text = read_script()
        for prop in (
            "screenshot-jpeg-quality",
            "screenshot-png-compression",
            "screenshot-webp-lossless",
            "screenshot-webp-quality",
            "screenshot-webp-compression",
        ):
            self.assertIn(prop, text)
        # The script must not rewrite the player's quality settings.
        self.assertNotIn('set_property("screenshot-', text)
        self.assertNotIn("set screenshot-", text)

    def test_burst_avoids_seeking_during_playback(self) -> None:
        text = read_script()
        playing = text.split("local function capture_playing", 1)[1]
        playing = playing.split("next_frame = function", 1)[0]
        self.assertNotIn('"seek"', playing)
        paused = text.split("local function capture_paused", 1)[1]
        paused = paused.split("local function capture_playing", 1)[0]
        self.assertIn('"seek"', paused)
        self.assertIn('"absolute", "exact"', paused)
        self.assertIn('"window"', text)
        # Window screenshots include OSD, so the burst progress OSD is skipped.
        self.assertIn('if burst.mode ~= "window" then', text)

    def test_queue_file_is_append_only_jsonl_in_the_user_layer(self) -> None:
        text = read_script()
        self.assertIn('io.open(path, "ab")', text)
        self.assertIn('file:write(encoded, "\\n")', text)
        self.assertIn("utils.format_json(entry)", text)
        self.assertIn("latest_burst(read_queue_entries())", text)
        self.assertIn("queue_file must stay inside the user layer", text)

    def test_subprocess_calls_are_structured(self) -> None:
        text = read_script()
        self.assertNotIn("os.execute", text)
        self.assertNotIn("io.popen", text)
        self.assertNotIn("popen", text)
        self.assertIn('name = "subprocess"', text)
        self.assertIn("args = args", text)
        self.assertIn("args = { configured, \"-version\" }", text)
        self.assertIn('args = { "mkdir", "-p", "--", path }', text)
        self.assertIn('args = { "cmd.exe", "/d", "/c", "mkdir", path }', text)

    def test_stitch_prefers_bundled_mpv_and_falls_back_to_ffmpeg(self) -> None:
        text = read_script()
        for marker in (
            "--no-config",
            "--frames=1",
            "--of=image2",
            "--external-file=",
            "--lavfi-complex=",
            "--ofopts=update=1",
            "build_mpv_stitch_args",
            "build_ffmpeg_stitch_args",
            "bundled_mpv_path",
            "ffmpeg_executable",
            "-filter_complex",
            '"[vid" .. index',
            'inputs[#inputs + 1] = "[" .. label .. "]"',
            'table.concat(chains, ";")',
            "未找到随包 mpv，且 PATH 中没有可用的 ffmpeg",
        ):
            self.assertIn(marker, text)
        self.assertIn("~~/../mpv.exe", text)
        self.assertIn("~~/../mpv/bin/mpv", text)

    def test_binding_names_are_complete_and_registered(self) -> None:
        text = read_script()
        block = re.search(r"local BINDING_NAMES = \{(.*?)\n\}", text, re.S)
        self.assertIsNotNone(block)
        names = set(re.findall(r'"([\w-]+)"', block.group(1)))
        self.assertEqual(names, EXPECTED_BINDINGS)

        handlers = re.search(r"local HANDLERS = \{(.*?)\n\}", text, re.S)
        self.assertIsNotNone(handlers)
        handler_names = set(re.findall(r"^\s*(\w+) = handler_", handlers.group(1), re.M))
        self.assertEqual(handler_names, EXPECTED_BINDINGS)

        self.assertIn("mp.add_key_binding(nil, name", text)
        self.assertIn("mp.register_script_message(name", text)


class ScreenshotToolsLuaTests(unittest.TestCase):
    @unittest.skipUnless(LUA, "luajit or lua is not installed")
    def test_lua_unit_tests_pass(self) -> None:
        result = subprocess.run(
            [LUA, str(LUA_TEST), str(SCRIPT)],
            cwd=str(ROOT),
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=60,
        )
        self.assertEqual(
            result.returncode,
            0,
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}",
        )
        self.assertIn("screenshot_tools Lua tests passed", result.stdout)


if __name__ == "__main__":
    unittest.main()
