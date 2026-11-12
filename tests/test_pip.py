from __future__ import annotations

import re
import shutil
import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "local" / "pip.lua"
CONFIG = ROOT / "config" / "common" / "script-opts" / "pip.conf"
LUA_TEST = ROOT / "tests" / "lua" / "test_pip.lua"
LUA = shutil.which("luajit") or shutil.which("lua")

EXPECTED_BINDINGS = {"toggle", "on", "off", "status"}
REQUIRED_SNAPSHOT_PROPS = {
    "geometry",
    "autofit",
    "autofit-larger",
    "autofit-smaller",
    "window-scale",
    "fullscreen",
    "border",
    "ontop",
    "keepaspect-window",
    "window-maximized",
}


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def parse_options(path: Path) -> dict[str, str]:
    options: dict[str, str] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#") or "=" not in stripped:
            continue
        key, value = stripped.split("=", 1)
        options[key.strip()] = value.strip()
    return options


class PiPLayoutTests(unittest.TestCase):
    def test_script_config_and_lua_test_exist(self) -> None:
        self.assertTrue(SCRIPT.is_file(), SCRIPT)
        self.assertTrue(CONFIG.is_file(), CONFIG)
        self.assertTrue(LUA_TEST.is_file(), LUA_TEST)

    def test_script_is_original_and_self_contained(self) -> None:
        text = read_text(SCRIPT)
        self.assertIn("newly authored for mpv_PlayKit", text)
        self.assertIn("never\n-- creates directories, writes files or starts subprocesses", text)
        for forbidden in ("os.execute", "io.popen", "io.open", "os.remove", "os.rename"):
            self.assertNotIn(forbidden, text)

    def test_config_defaults(self) -> None:
        options = parse_options(CONFIG)
        self.assertEqual(options.get("enabled"), "yes")
        self.assertEqual(options.get("position"), "bottom-right")
        self.assertEqual(options.get("width_percent"), "25")
        self.assertEqual(options.get("height_percent"), "0")
        self.assertEqual(options.get("margin"), "16")
        self.assertEqual(options.get("min_width"), "160")
        self.assertEqual(options.get("ontop"), "yes")
        self.assertEqual(options.get("border"), "no")
        self.assertEqual(options.get("exit_fullscreen"), "yes")
        self.assertEqual(options.get("follow_display"), "yes")
        self.assertEqual(options.get("keep_on_new_file"), "yes")
        self.assertEqual(options.get("toggle_keybind"), "alt+p")

    def test_menu_and_key_binding_are_wired(self) -> None:
        menu = read_text(ROOT / "config" / "common" / "menu.conf")
        input_conf = read_text(ROOT / "config" / "common" / "input.conf")
        self.assertIn("script-binding pip/toggle", menu)
        self.assertIn("script-binding pip/toggle", input_conf)
        self.assertIn("#! 视频 > 切换 画中画", input_conf)
        self.assertIn("Alt+p", input_conf)


class PiPBehaviourTests(unittest.TestCase):
    def test_snapshot_and_restore_cover_all_window_properties(self) -> None:
        text = read_text(SCRIPT)
        snapshot = re.search(r"local SNAPSHOT_PROPS = \{(.*?)\n\}", text, re.S)
        self.assertIsNotNone(snapshot)
        props = set(re.findall(r'"([\w-]+)"', snapshot.group(1)))
        self.assertEqual(props, REQUIRED_SNAPSHOT_PROPS)

        restore = re.search(r"local RESTORE_ORDER = \{(.*?)\n\}", text, re.S)
        self.assertIsNotNone(restore)
        order = re.findall(r'"([\w-]+)"', restore.group(1))
        # Fullscreen is restored last so the geometry written before it is not
        # discarded by the fullscreen transition.
        self.assertEqual(order[-1], "fullscreen")
        self.assertEqual(set(order), REQUIRED_SNAPSHOT_PROPS)

    def test_boolean_properties_use_native_setter(self) -> None:
        text = read_text(SCRIPT)
        self.assertIn("mp.set_property_native", text)
        self.assertIn('pcall(mp.set_property_native, name, value)', text)
        # mpv 0.41 rejects "yes"/"no" strings for native boolean properties.
        self.assertNotIn('mp.set_property("fullscreen"', text)
        self.assertNotIn('mp.set_property("border"', text)

    def test_display_fullscreen_and_file_changes_are_tracked(self) -> None:
        text = read_text(SCRIPT)
        for prop in ("display-width", "display-height", "fullscreen"):
            self.assertIn(f'mp.observe_property("{prop}"', text)
        self.assertIn('mp.register_event("file-loaded"', text)
        self.assertIn("follow_display", text)
        self.assertIn("keep_on_new_file", text)

    def test_bindings_and_messages_share_handlers(self) -> None:
        text = read_text(SCRIPT)
        block = re.search(r"local BINDING_NAMES = \{(.*?)\}", text, re.S)
        self.assertIsNotNone(block)
        names = set(re.findall(r'"([\w-]+)"', block.group(1)))
        self.assertEqual(names, EXPECTED_BINDINGS)
        self.assertIn("mp.add_key_binding(nil, name, HANDLERS[name])", text)
        self.assertIn("mp.register_script_message(name, HANDLERS[name])", text)


class PiPLuaTests(unittest.TestCase):
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
        self.assertIn("pip Lua tests passed", result.stdout)


if __name__ == "__main__":
    unittest.main()
