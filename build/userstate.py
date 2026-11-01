from __future__ import annotations

import hashlib
import json
from pathlib import Path
from typing import Any

from .common import fail, log, read_json, sha256_file, write_json

MANAGED_ROOT_FILES = ("mpv.conf", "input.conf", "profiles.conf", "platform.conf")
STATE_SCHEMA = 1
INVENTORY_SCHEMA = 1

USER_README = """mpv PlayKit 用户层
===================

这个目录里的内容在自动更新时会被保留，并重新应用到新版本的官方配置上。
不要直接修改 portable_config 根目录里的 mpv.conf、input.conf、profiles.conf
或 script-opts/*.conf：更新时它们会被新版本的官方文件整体替换。

配置修改
--------
把修改写进 user/patches/ 下以 .patch 结尾的文本文件。patch 只描述
“和官方配置的差异”，不是旧配置文件的副本。示例：

    schema=1
    target=mpv.conf

    [global]
    volume=70
    -osc
    +keep-open=yes

支持的操作：
    key=value      设置（未提到的键继续跟随新官方配置）
    -key           删除该键
    !key           同上，删除
    +key=value     追加一行（同名键最后一行生效）
    [section]      仅用于 profiles.conf
    [global]       文件顶部的全局作用域

target 必须是下列之一：
    mpv.conf
    input.conf
    profiles.conf
    platform.conf
    script-opts/<name>.conf

input.conf 的写法与官方文件一致：
    MBTN_LEFT=cycle mute #! 自定义标签

删除一个官方按键绑定：
    -MBTN_BACK

直接编辑官方配置也可以。下次检查更新或安装时，更新器会把你的改动捕获到
user/patches/auto/ 下，并在应用新官方配置后重新应用。无法无损表达的改动
（例如新增了无法解析的行）会中止更新，而不会静默覆盖。

用户脚本、字体和脚本选项
------------------------
    user/scripts/       复制到 scripts/（不覆盖官方脚本）
    user/fonts/         复制到 fonts/
    user/script-opts/   复制到 script-opts/（不覆盖官方选项）

同一个文件名如果和官方文件冲突，更新会中止。请换一个名字。

更新与回滚
----------
播放器菜单“更新”中提供检查、下载安装、保存当前修改和回滚。安装由独立的
updater 进程完成：它先下载并校验完整安装包，等待播放器退出，再把新版本
放到当前目录、把旧版本保留为 <安装目录>.previous。回滚会交换这两个目录。

日志和状态
----------
    .playkit/update-status.json   播放器菜单读取的更新进度
    .playkit/updater.log          更新器日志（最多 1 MiB，自动轮转）
    .playkit/state.json           安装版本、官方配置哈希和 patch 清单

关闭自动更新
------------
把 script-opts/update-check.conf 的 enabled 改为 no。此时脚本不联网、不启动
更新器进程，也不会下载或安装任何内容。
"""

PATCHES_README = """用户配置 patch
==============

手动 patch 放在本目录（user/patches/），文件名约定为：

    mpv.conf.patch
    input.conf.patch
    profiles.conf.patch
    platform.conf.patch
    script-opts/<name>.conf.patch

更新器按文件名把 patch 应用到对应的官方配置。user/patches/auto/ 是更新器
捕获直接编辑后自动生成的 patch，会在每次捕获时重建；不要手动编辑它，也不要
把它当作长期配置备份。格式说明见上一级 user/README.md。
"""

SCRIPTS_README = """用户脚本
========

放在这里的 .lua/.js 文件会在安装更新时复制到 portable_config/scripts/。
不会覆盖同名的官方脚本；重名会导致更新中止。更新器永远不会删除这里的文件。
"""

FONTS_README = """用户字体
========

放在这里的字体文件会在安装更新时复制到 portable_config/fonts/。
"""

SCRIPT_OPTS_README = """用户 script-opts
================

放在这里的 .conf 文件会在安装更新时复制到 portable_config/script-opts/。
不会覆盖同名的官方配置文件；重名会导致更新中止。需要修改官方选项时，请使用
user/patches/script-opts/<name>.conf.patch。
"""


def managed_configs(portable: Path) -> list[str]:
    result = [name for name in MANAGED_ROOT_FILES if (portable / name).is_file()]
    script_opts = portable / "script-opts"
    if script_opts.is_dir():
        for path in sorted(script_opts.glob("*.conf")):
            if path.is_file():
                result.append(f"script-opts/{path.name}")
    return result


def _write_user_docs(portable: Path) -> None:
    files = {
        "README.md": USER_README,
        "patches/README.md": PATCHES_README,
        "scripts/README.md": SCRIPTS_README,
        "fonts/README.md": FONTS_README,
        "script-opts/README.md": SCRIPT_OPTS_README,
    }
    for relative, text in files.items():
        path = portable / "user" / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8", newline="\n")


def write_package_state(stage: Path, version: str, platform: str, installed_at: str) -> None:
    """Create the official baseline and the empty user layer shipped in a package.

    A fresh archive contains the unmodified official configuration, so the
    baseline and generated copies are byte-identical.  Every installed copy
    starts from this state; user patches are only applied by the updater.
    """
    portable = stage / "portable_config"
    targets = managed_configs(portable)
    if not targets:
        fail("no managed configuration files in portable_config")
    official: dict[str, str] = {}
    generated: dict[str, str] = {}
    for relative in targets:
        source = portable / relative
        baseline = portable / ".playkit" / "official" / relative
        snapshot = portable / ".playkit" / "generated" / relative
        for destination in (baseline, snapshot):
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(source.read_bytes())
        digest = sha256_file(source)
        official[relative] = digest
        generated[relative] = digest
        log(f"baseline {relative}")
    state = {
        "schema": STATE_SCHEMA,
        "installed_version": version,
        "platform": platform,
        "installed_at": installed_at,
        "previous_version": None,
        "updated_by": None,
        "official": dict(sorted(official.items())),
        "generated": dict(sorted(generated.items())),
        "patches": [],
        "patch_warnings": [],
        "user_scripts": [],
        "user_fonts": [],
        "user_script_opts": [],
    }
    write_json(portable / ".playkit" / "state.json", state)
    _write_user_docs(portable)


def _inventory_skip(relative: str) -> bool:
    parts = relative.split("/")
    if not parts:
        return True
    if parts[0] in {"user", ".playkit", "_cache"}:
        return True
    if relative in {"saved-props.json", "update-check-state.json"}:
        return True
    return False


def write_inventory(stage: Path) -> None:
    """Record every packaged file below portable_config.

    Paths are relative to portable_config because the updater compares the
    inventory against a walk of the installed portable_config directory.  User
    and state directories are deliberately excluded: they are preserved and
    regenerated independently of the official package.
    """
    portable = stage / "portable_config"
    files: dict[str, str] = {}
    for path in sorted(portable.rglob("*")):
        if not path.is_file():
            continue
        relative = str(path.relative_to(portable)).replace("\\", "/")
        if _inventory_skip(relative):
            continue
        files[relative] = sha256_file(path)
    if not files:
        fail("portable_config inventory is empty")
    write_json(
        stage / "playkit-inventory.json",
        {"schema": INVENTORY_SCHEMA, "files": dict(sorted(files.items()))},
    )
    log(f"inventory {len(files)} files")


def read_state(stage: Path) -> dict[str, Any]:
    path = stage / "portable_config" / ".playkit" / "state.json"
    try:
        return read_json(path)
    except (OSError, json.JSONDecodeError) as error:
        fail(f"cannot read {path}: {error}")
    raise AssertionError("unreachable")


def package_inventory_hashes(stage: Path) -> dict[str, str]:
    """Map package-root-relative path -> sha256 for helper/verification."""
    result: dict[str, str] = {}
    for path in sorted(stage.rglob("*")):
        if not path.is_file():
            continue
        relative = str(path.relative_to(stage)).replace("\\", "/")
        if relative == "playkit-inventory.json":
            continue
        result[relative] = sha256_file(path)
    return result


def verify_package_state(stage: Path, version: str, platform: str) -> None:
    state = read_state(stage)
    if state.get("schema") != STATE_SCHEMA:
        fail(f"state.json schema is not {STATE_SCHEMA}")
    if state.get("installed_version") != version:
        fail("state.json installed_version does not match the package version")
    if state.get("platform") != platform:
        fail("state.json platform does not match the package platform")
    official = state.get("official") or {}
    generated = state.get("generated") or {}
    if official != generated:
        fail("fresh package state must have identical official/generated maps")
    if not official:
        fail("state.json has no official configuration baseline")
    for relative, digest in official.items():
        if Path(relative).is_absolute() or ".." in Path(relative).parts:
            fail(f"unsafe path in state.json: {relative}")
        for directory in ("official", "generated"):
            path = stage / "portable_config" / ".playkit" / directory / relative
            if not path.is_file():
                fail(f"missing {directory} snapshot: {relative}")
            if sha256_file(path) != digest:
                fail(f"{directory} snapshot hash mismatch: {relative}")
    inventory_path = stage / "playkit-inventory.json"
    inventory = read_json(inventory_path)
    if inventory.get("schema") != INVENTORY_SCHEMA:
        fail("playkit-inventory.json schema is not 1")
    files = inventory.get("files") or {}
    portable = stage / "portable_config"
    for relative, digest in files.items():
        if Path(relative).is_absolute() or ".." in Path(relative).parts:
            fail(f"unsafe path in inventory: {relative}")
        path = portable / relative
        if not path.is_file():
            fail(f"inventory lists a missing file: {relative}")
        if sha256_file(path) != digest:
            fail(f"inventory hash mismatch: {relative}")
    for path in portable.rglob("*"):
        if not path.is_file():
            continue
        relative = str(path.relative_to(portable)).replace("\\", "/")
        if _inventory_skip(relative):
            continue
        if relative not in files:
            fail(f"packaged file is missing from playkit-inventory.json: {relative}")


def checksum_tree(root: Path) -> dict[str, str]:
    """Internal helper used by tests; stable relative-path hashes."""
    result: dict[str, str] = {}
    for path in sorted(root.rglob("*")):
        if path.is_file():
            result[str(path.relative_to(root)).replace("\\", "/")] = sha256_file(path)
    return result


def content_hash(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()
