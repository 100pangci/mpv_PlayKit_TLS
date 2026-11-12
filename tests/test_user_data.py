from __future__ import annotations

import tempfile
import unittest
from pathlib import Path, PurePosixPath

from build import userstate, verify
from build.common import BuildError, ROOT

PACKAGE_USER_DATA = "portable_config/user/data"
PACKAGE_USER_DATA_README = f"{PACKAGE_USER_DATA}/README.md"
HISTORY_LOG = "user/data/mpvHistory.log"
BOOKMARK_LOG = "user/data/mpvBookmark.log"
ARCHIVE_PREFIX = "mpv-PlayKit-2026.11-windows-x64/"


class UserDataPathContractTests(unittest.TestCase):
    def _options(self, name: str) -> dict[str, str]:
        path = ROOT / "config" / "common" / "script-opts" / name
        result: dict[str, str] = {}
        for line in path.read_text(encoding="utf-8").splitlines():
            stripped = line.strip()
            if not stripped or stripped.startswith("#") or "=" not in stripped:
                continue
            key, value = stripped.split("=", 1)
            result[key.strip()] = value.strip()
        return result

    def test_history_and_bookmark_logs_live_in_user_data(self) -> None:
        expected = {
            "SimpleHistory.conf": HISTORY_LOG,
            "SimpleBookmark.conf": BOOKMARK_LOG,
        }
        for name, relative in expected.items():
            value = self._options(name).get("log_file")
            self.assertEqual(value, relative, name)
            path = PurePosixPath(value)
            self.assertFalse(path.is_absolute(), name)
            self.assertNotIn("..", path.parts, name)
            self.assertEqual(path.parts[:2], ("user", "data"), name)

    def test_user_data_directory_is_created_and_explained(self) -> None:
        self.assertEqual(userstate.USER_DATA_DIR, "user/data")
        self.assertEqual(userstate.USER_DATA_README, "user/data/README.md")
        self.assertIn("user/data/mpvHistory.log", userstate.USER_README)
        self.assertIn("user/data/mpvBookmark.log", userstate.USER_README)
        for text in (userstate.USER_README, userstate.DATA_README):
            self.assertIn("user/data/", text)
            self.assertIn("mpvHistory.log", text)
            self.assertIn("mpvBookmark.log", text)
            self.assertIn("更新", text)
            self.assertIn("回滚", text)
            self.assertIn("保留", text)


class UserDataPackageContractTests(unittest.TestCase):
    def test_package_keeps_only_the_readme_placeholder(self) -> None:
        self.assertIn(PACKAGE_USER_DATA_README, verify.REQUIRED)
        self.assertNotIn(PACKAGE_USER_DATA_README, verify.FORBIDDEN_PARTS)
        self.assertNotIn(f"{PACKAGE_USER_DATA}/mpvHistory.log", verify.REQUIRED)
        self.assertNotIn(f"{PACKAGE_USER_DATA}/mpvBookmark.log", verify.REQUIRED)
        for log_name in userstate.USER_DATA_LOGS:
            self.assertIn(log_name, verify.FORBIDDEN_PARTS)

    def test_package_paths_allow_readme_and_reject_runtime_data(self) -> None:
        allowed = [
            PACKAGE_USER_DATA_README,
            f"{ARCHIVE_PREFIX}{PACKAGE_USER_DATA_README}",
        ]
        for path in allowed:
            verify.check_forbidden(path)

        forbidden = [
            f"{PACKAGE_USER_DATA}/{userstate.USER_DATA_LOGS[0]}",
            f"{PACKAGE_USER_DATA}/{userstate.USER_DATA_LOGS[1]}",
            f"{PACKAGE_USER_DATA}/future.json",
            f"{ARCHIVE_PREFIX}{PACKAGE_USER_DATA}/future.log",
            f"{ARCHIVE_PREFIX}{PACKAGE_USER_DATA}/nested",
        ]
        for path in forbidden:
            with self.assertRaises(BuildError, msg=path):
                verify.check_forbidden(
                    path, is_dir=path.endswith("/nested")
                )

        with self.assertRaises(BuildError):
            verify.check_forbidden(f"{PACKAGE_USER_DATA}/nested", is_dir=True)

    def test_runtime_state_remains_forbidden(self) -> None:
        for path in (
            "portable_config/_cache/command_history.txt",
            "portable_config/saved-props.json",
            "portable_config/update-check-state.json",
            "portable_config/.playkit/update-status.json",
            "portable_config/.playkit/updater.log",
        ):
            with self.assertRaises(BuildError, msg=path):
                verify.check_forbidden(path)


class UserDataPackageStateTests(unittest.TestCase):
    def test_package_state_contains_placeholder_but_no_runtime_logs(self) -> None:
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

            userstate.write_package_state(
                stage, "2026.11", "windows-x64", "2026-01-01T00:00:00Z"
            )
            userstate.write_inventory(stage)
            userstate.verify_package_state(stage, "2026.11", "windows-x64")

            data = portable / "user" / "data"
            self.assertTrue((data / "README.md").is_file())
            self.assertFalse((data / "mpvHistory.log").exists())
            self.assertFalse((data / "mpvBookmark.log").exists())


class UserDataRustContractTests(unittest.TestCase):
    def test_update_preserves_user_data_on_install_and_rollback(self) -> None:
        text = (ROOT / "updater" / "helper" / "src" / "update.rs").read_text(
            encoding="utf-8"
        )
        self.assertIn('const USER_DATA_DIR: &str = "user/data";', text)
        self.assertIn("fn preserve_user_data", text)

        preserve = text[text.index("fn preserve_runtime") : text.index("fn install_user_assets")]
        self.assertIn(
            "preserve_user_data(current_portable, staged_portable)?;", preserve
        )

        rollback = text[text.index("fn rollback_inner") : text.index("pub fn verify_install")]
        self.assertIn(
            "preserve_user_data(&current_portable, &previous_portable)?;", rollback
        )
        self.assertLess(
            rollback.index("preserve_user_data("),
            rollback.index("perform_rollback(layout)?;"),
        )

        repair = text[text.index("fn repair_inner") : text.index("fn state_has_snapshots")]
        self.assertIn("ensure_user_data_dir(&portable)?;", repair)

    def test_archive_rejects_runtime_data_but_requires_readme(self) -> None:
        text = (ROOT / "updater" / "helper" / "src" / "archive.rs").read_text(
            encoding="utf-8"
        )
        self.assertIn(
            'const USER_DATA_README: &str = "portable_config/user/data/README.md";',
            text,
        )
        self.assertIn(
            '"portable_config/user/data/mpvHistory.log"',
            text,
        )
        self.assertIn(
            '"portable_config/user/data/mpvBookmark.log"',
            text,
        )
        self.assertIn("USER_DATA_README,", text)
        self.assertIn("verify_user_data_tree(top)?;", text)


if __name__ == "__main__":
    unittest.main()
