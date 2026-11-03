# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## 项目定位

面向 Windows x64 与 Linux x64 的便携式 mpv 发行版。仓库只保存配置、原创 Lua、
平台适配、安装器、构建工具、Rust 更新器/umpv 和上游来源清单；mpv、yt-dlp、
uosc、字体、GLSL shader 等第三方文件在构建时按 `sources.lock.json` 下载并校验，
不长期入库。

功能目标是与基线 `mpv-lazy-20260510.exe` 的用户体验一致，但排除 VapourSynth、
AI 滤镜/模型、CUDA/TensorRT/DirectML，以及许可证不允许再分发的 shader。历史
PlayKit Lua 不得直接复制：功能要么原创实现，要么使用官方上游实现（见
`docs/lua-disposition.md`、`LICENSE_PROVENANCE.md`）。

仓库文档和面向用户的文本使用中文；Python/Rust 代码注释以英文为主。

## 常用命令

Python 要求 3.11+，唯一第三方依赖是 PyYAML。

```sh
python -m pip install -r requirements.txt

# 全部 Python 单元测试（当前约 100 个，秒级完成）
python -m unittest discover -s tests -v

# 只跑一个测试类/用例（-k 支持点号路径）
python -m unittest discover -s tests -v -k ManifestTests.test_manifests_load

# CI 同款编译检查
python -m compileall -q build tests
```

Rust 更新器/umpv（`updater/helper/`，Rust 1.85+，crate 版本 1.1.0）：

```sh
cd updater/helper
cargo test --locked --all-targets
# 单个测试（模块路径）
cargo test --locked update::tests::repair_supports_a_development_install
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
```

集成测试位于 `updater/helper/tests/integration.rs`（更新器）和
`tests/umpv.rs`（单实例启动器）。

### 本地构建

依赖：Python 3.11+、7-Zip（Linux 为 `p7zip-full`）、Rust 1.85+；Linux 还需要
`squashfs-tools`。Linux 发行包使用 musl 静态构建，需要
`rustup target add x86_64-unknown-linux-musl` 和 `musl-tools`。Windows 上可用
`SEVEN_ZIP_PATH` 指定 `7z.exe`。

```sh
# 0. 构建更新器和 umpv（同一条命令产出两个二进制）
python -m build.helper --platform windows-x64
# -> dist/updater/windows-x64/playkit-updater.exe, umpv.exe
# Linux 换成 --platform linux-x64，产物无 .exe 后缀

# 1. 解析上游并写 lock（仓库已有 lock 时可跳过）
python -m build.resolve --output sources.lock.json
#    只更新某个组件：--only <component-id>（可重复），其余组件保留 lock 现值
#    只检查不写入：--check --json

# 2. 下载并校验
python -m build.fetch --lock sources.lock.json --work .work --platform windows-x64

# 3. 组装（默认从 dist/updater/<platform>/ 取 helper 与 umpv）
python -m build.assemble \
  --platform windows-x64 \
  --version 2026.11 \
  --lock sources.lock.json \
  --work .work \
  --out dist/stage/windows-x64 \
  --source-sha local

# 4. 冒烟、打包、校验
python -m build.smoke --stage dist/stage/windows-x64 --platform windows-x64
python -m build.package --stage dist/stage/windows-x64 --platform windows-x64 \
  --version 2026.11 --out-dir dist
python -m build.verify --stage dist/stage/windows-x64 --platform windows-x64 --version 2026.11
python -m build.verify --asset dist/mpv-PlayKit-2026.11-windows-x64.zip \
  --platform windows-x64 --version 2026.11
```

Linux 把 `windows-x64` 换成 `linux-x64`，产物是 `.tar.gz`。PowerShell 中续行用
反引号。

## 架构

### 构建与发布流水线

`manifests/*.yml` 声明上游仓库、Release 选择方式、资产正则、许可证、解包和安装
规则。流程为：

```text
manifests + GitHub API
  -> build.resolve      候选 sources.lock.json（版本、URL、SHA-256）
  -> build.decide       对比最新 Release、lock 内容哈希、inputs_hash
  -> build.fetch        下载并校验资产与许可证
  -> build.helper       cargo build --release --locked（更新器 + umpv）
  -> build.assemble     解包/安装，生成 portable_config、.playkit、inventory、manifest
  -> build.smoke        mpv/yt-dlp/updater/umpv 实际启动检查
  -> build.package      确定性 zip / tar.gz + .sha256
  -> build.verify       结构、禁用路径、元数据、压缩包门禁
  -> build.release      发布 manifest、release notes、SHA256SUMS
```

关键约束：

- `sources.lock.json` 是生成物。不要手改版本、URL 或哈希；用 `build.resolve`
  重新生成。`lock_hash` 只哈希 `components`，忽略 `generated_at`。
- 月度更新是事务式的：resolve 生成候选 lock 并上传为 workflow artifact；候选
  提交先推送到临时分支；Windows 与 Linux 用同一份 lock 构建；两者及全部测试
  成功后 `commit-lock` 才以 fast-forward 推进 main；Release 的 target 与包内
  `source_commit` 指向同一提交。任何一步失败，main 的 lock 保持不变。
- `build/inputs.py` 定义发布输入哈希：`config/`、`scripts/local/`、`updater/`、
  `manifests/`、`installer/`、`build/`、`.github/workflows/build.yml`、`VERSION`、
  `LICENSE`、`NOTICE`、`THIRD_PARTY.md`、`LICENSE_PROVENANCE.md`。`tests/` 和
  `docs/` 不在其中；改测试或文档不会触发 Release，改构建代码/配置会。
- 字节级可复现：归档时间戳、属主、权限被归一化；需要固定 `built_at` 时使用
  `SOURCE_DATE_EPOCH` 或 `--built-at`。
- `build.verify` 是发布门禁：必需文件、禁用路径、`mpv.conf` 的 `~~/` 引用、
  update-check 的联网门禁顺序、updater/umpv 元数据、inventory 哈希、压缩包
  结构、manifest schema。新增随包文件或元数据时必须同步更新
  `build/verify.py` 的 `REQUIRED` / `WINDOWS_REQUIRED` / `LINUX_REQUIRED` 等常量
  和对应测试。

### 配置与包布局

- `config/common/` 是两平台共用配置；`config/windows/` 与 `config/linux/` 覆盖
  `platform.conf` 及 `ytdl_hook.conf`。平台差异只放在平台目录，不要复制进
  `config/common/`。
- `scripts/local/*.lua` 和 `updater/update-check.lua` 在构建时复制到
  `portable_config/scripts/` 根目录；mpv 会自动加载，不要为它们添加显式
  `script=` 配置。
- `build/patch_uosc.py` 只修改上游 `scripts/uosc/main.lua` 以加入自动 UI 缩放。
  上游锚点变化时构建必须失败，不要放宽为静默跳过；除该文件外 uosc 保持上游。
- GLSL shader 由 `manifests/shaders.yml` 的 `raw-files` 固定到上游 commit，
  安装到 manifest 声明的精确目标；包内不允许其他 `.glsl`。
- shader pin 的月度刷新由 `build.shader_update` 完成；`update.mode` 仅允许
  `pinned`/`latest-commit`/`latest-tag`，同仓库许可证文本或 `license_marker`
  变化时必须人工复核。
- Windows 包根目录是播放器目录（`mpv.exe`、`yt-dlp.exe`、`umpv.exe`）；
  Linux 的 AppImage 解包到 `mpv/`，包根提供 `mpv-playkit` 启动器（设置
  `--config-dir`、`DISABLE_AUTO_UPDATES=1`、PATH），`yt-dlp` 与 `umpv` 在包根。
- `config/umpv/<os>.conf` 构建为包根 `umpv.conf`；umpv 通过 mpv JSON IPC
  实现单实例，首次调用启动 mpv，后续调用发送 `loadfile`。
- 运行时不污染包：冒烟测试和 `build.verify` 会拒绝 `_cache/`、
  `saved-props.json`、`update-check-state.json`、`.playkit/update-status.json`、
  `.playkit/updater.log` 等状态文件。`.playkit/state.json`、`official/`、
  `generated/`、`playkit-inventory.json` 是有意随包发布的更新基线。

### 自动更新与用户配置层

Lua 只负责检测 Release、显示 UI、以结构化参数启动 helper；所有落盘操作在 Rust
helper 中：

- `updater/update-check.lua`：`enabled=no` 时在读取配置后立即返回，不联网、不
  启动子进程。这个门禁必须位于任何 subprocess/网络代码之前（`build.verify`
  会静态检查）。`install_enabled=no` 可只关闭安装；启动约 6 秒后调用
  `playkit-updater repair-auto`。
- `updater/helper/`：`install` 下载完整新包、校验 SHA-256 链、解压到同级
  staging（拒绝路径穿越/链接逃逸）、等待 mpv 退出、迁移 `user/` 层并应用
  patch，最后用两次重命名交换 `current -> current.previous`、
  `staging -> current`。`capture` 把直接编辑写成语义 patch；`repair` 重建
  官方/生成基线与用户层；`rollback` 单向换回 `.previous`；`verify`/`status`
  只读。
- 用户层位于 `portable_config/user/`：`patches/`（schema 1）、`patches/v2/`
  （schema 2）、`patches/auto/v2/`（自动捕获）、`scripts/`、`fonts/`、
  `script-opts/`。patch 是键操作/行操作，不是 unified diff；未写出的键跟随
  新官方配置。
- `portable_config/.playkit/`：`state.json`（版本、哈希、已应用 patch、用户层
  指纹）、`official/`（官方基线）、`generated/`（应用 patch 后的快照）、
  `update-status.json`、`updater.log`。`repair --if-needed` 只在指纹变化时
  重写配置，退出码 `0`=有变化、`3`=无需修复、`4`=已修复但内容未变。
- 开发版本（`ci`、`0.0.0-dev`）不会自动安装更新，但 patch 自动修复仍工作。
- 安全边界：只接受固定 GitHub 仓库的 HTTPS 发布地址和 GitHub 资产主机；所有
  下载、manifest、包内清单都走 SHA-256 校验链；不执行 Release 中的脚本；不拼
  接 shell。

### GitHub Actions

- `build.yml`：push/PR/手动/被调用。Python 单元测试、Ubuntu+Windows 的 Rust
  `test`/`fmt`/`clippy`，然后两个平台并行构建、冒烟、打包、校验。fork PR 不执行
  任何构建作业；所有 action 固定到完整 commit SHA。
- `monthly-update.yml`：每月 1 日 05:17 UTC 或手动（`force`）。按
  resolve → decide → 候选 lock artifact/临时分支 → build → commit-lock →
  release 运行；版本号为 `年.月`，同月重复发布追加 `.1`、`.2`。
- `release.yml`：被月度工作流调用；校验候选 lock 字节、从 Actions cache 取回
  两个平台产物，拒绝覆盖已存在的 tag/Release。
- `tests/test_build.py` 中的 `WorkflowTests` 会断言这些事务约束（候选 lock
  一致、只有 commit 作业写 main、release target、action 固定 SHA 等）。修改
  工作流后要同步更新测试。

## 修改约定

- 新增或替换上游：在 `manifests/{binaries,scripts,resources,shaders}.yml` 声明
  `upstream`、`release`、`asset.pattern`、`license`、`extract`、`install`；
  资产必须恰好匹配一个。许可证文件只能用 `repository` + `ref`（会被解析为完整
  commit），不接受字面 URL。raw shader 要填小写 SHA-256，目标必须是
  `portable_config/shaders/` 下的 `.glsl`。之后用
  `python -m build.resolve --only <id>` 更新 lock。
- 第三方二进制、字体、模型、shader 文件不入库；测试夹具只有
  `tests/assets/1x1.png`。
- `build/` 保持纯标准库 + PyYAML，不要引入新的运行时依赖。
- 更新器版本、`MIN_HELPER_VERSION`、`patch_schema`、manifest 中的 helper/umpv
  资产名与哈希是跨 Lua、Rust、Python、测试共享的契约。改协议或版本时必须同步
  `updater/update-check.lua`、`build/helper.py`、`build/assemble.py`、
  `build/verify.py`、`manifests/` 和 `tests/test_build.py`。
- `portable_config/` 根下的官方配置会被更新整体替换；用户偏好应写到
  `user/patches/`，不要把个人设置硬编码进 `config/common/`。
- 不要在 `update-check.lua` 的门禁之前加入任何网络或子进程调用。
- `build/verify.py` 的禁用路径不可绕过：VapourSynth、`.vpy`/`.onnx`、模型、
  `_cache/`、未在 `manifests/shaders.yml` 声明的 `.glsl`、运行时状态文件。
- `.gitattributes` 强制 `.cmd`/`.bat` 使用 CRLF、二进制文件不被转换；不要修改
  这些规则。
- Windows 安装器只写 `HKCU`，umpv 使用专属 ProgID `mpv-playkit.umpv`；Linux
  安装器只是预留接口。
- 新增/重命名随包脚本、配置或元数据时，同时更新 `build/verify.py` 的必需文件
  列表、`build/userstate.py` 的基线/清单逻辑和相关测试；否则发布门禁会失败。

## 文档索引

- `docs/architecture.md`：目录职责、流水线、确定性、平台差异、校验门禁
- `docs/upstreams.md`：上游组件表、资产匹配、lock 与许可证固定规则、shader pin 更新
- `docs/shader-license-audit.md`：shader 逐文件许可证审计（JSON 为权威记录）
- `docs/updater.md`：Rust helper 的命令、事务目录、失败恢复、安全边界
- `docs/user-patches.md`：schema 1/2 patch 语法与冲突语义
- `docs/update-checker.md`：Lua 检测器配置、联网门禁、状态文件
- `docs/umpv.md`：单实例启动器行为与 Windows 文件关联
- `docs/lua-disposition.md`、`docs/baseline-audit.md`：基线功能处置与许可证审计