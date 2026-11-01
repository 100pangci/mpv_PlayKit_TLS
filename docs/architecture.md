# 架构与构建流程

## 目标

仓库只维护“如何构建”和“本项目自己的代码”，不维护第三方二进制或未修改的上游
脚本。任意一次发布都必须能追溯到固定的上游资产、固定的仓库提交和校验和。

## 目录职责

| 路径 | 职责 |
| --- | --- |
| `config/common/` | 两平台共用的 `mpv.conf`、`input.conf`、`menu.conf`、`profiles.conf`、`script-opts/` |
| `config/windows/` | `platform.conf`（WASAPI、IME）与 `ytdl_hook.conf` |
| `config/linux/` | `platform.conf`（PipeWire/PulseAudio/ALSA）与 `ytdl_hook.conf` |
| `scripts/local/` | 本项目原创 Lua（`idle-brand`、`save_global_props`、`input_plus`、`thumbfast`）；构建时复制到包内 `portable_config/scripts/` 根目录 |
| `updater/update-check.lua` | 播放器内更新检测与安装界面；构建时同样复制到 scripts 根目录 |
| `updater/helper/` | Rust 更新器（下载、校验、patch、事务交换）；构建时放入安装目录根部 `updater/` |
| `installer/` | Windows 安装/卸载脚本与图标；Linux 安装接口占位（未实现）；构建时原样复制到包内 |
| `manifests/*.yml` | 上游仓库、Release 选择方式、资产正则、许可证、解包与安装规则 |
| `sources.lock.json` | 解析后的固定版本、URL、SHA-256；每月由工作流刷新 |
| `build/` | 构建工具（纯 Python，无第三方运行时依赖，除 PyYAML） |
| `.github/workflows/` | 月度检查、双平台构建、发布 |
| `docs/` | 设计、上游、更新检测器和许可证处置记录 |

## 构建流水线

```text
manifests/*.yml
      │
      ▼
build.resolve ── GitHub Releases API ──► 候选 sources.lock.json
      │                                   (版本、URL、sha256、许可证 commit/sha256)
      ▼
build.decide ── 比较最新 Release 的 manifest.json
      │          + 当前 lock + inputs_hash
      ├── 无变化 ──► 结束：不上传、不构建、不提交、不发布
      ▼ 有变化
候选 lock 上传为 workflow artifact（resolve 记录文件 SHA-256）
候选提交（base + 候选 lock）推送到临时分支 playkit/candidate-<run>-<attempt>
      │
      ▼
build.fetch ── 两个平台分别下载资产与许可证，校验 lock 中的 SHA-256
      │
      ▼
build.helper ── cargo build --release --locked；Windows 静态 CRT，Linux musl
      │
      ▼
build.assemble ── 按平台解包、安装、生成 playkit-version.json / manifest.json
      │            + .playkit 状态、playkit-inventory.json、updater helper
      ▼
build.smoke ── mpv --version + 配置加载播放 + yt-dlp --version
      │
      ▼
build.package ── 确定性 zip / tar.gz + .sha256
      │
      ▼
build.verify ── 结构、禁用路径、引用、门禁顺序、许可证固定、压缩包校验
      │
      ▼
commit-lock ── 校验 artifact 与候选提交中的 lock 字节一致
      │         fast-forward 到 main，删除临时分支
      ▼
build.release ── 校验包内 source_commit、mpv 信息、lock 与产物一致
      │
      ▼
gh release create --target <final_sha> ── 仅在此前全部成功时执行
```

## 事务式月度更新

月度工作流的核心约束是：**main 中的 `sources.lock.json` 只能代表已经通过完整
双平台构建验证的上游组合**。

- `resolve` 只把候选 lock 写成文件并上传 artifact，不修改 main。
- 候选提交（仓库源码 + 候选 lock）先推送到临时分支。构建作业 checkout 的正是
  这个提交，因此包内 `playkit-version.json` / `manifest.json` 的 `source_commit`
  就是最终提交。
- Windows 与 Linux 都从同一个 artifact 安装候选 lock，并在安装前校验
  `lock_sha256`；两个平台不会各自重新解析上游。
- `commit-lock` 再次校验 artifact 与候选提交中的 lock 完全一致，然后用普通
  （非 force）push 把候选提交 fast-forward 到 main。main 在此期间被别人推进时
  push 会失败，流程停止。
- `release.yml` 只接受这个最终提交：它再次比较 artifact 与 checkout 的 lock，
  `build.release` 校验两个包内的 `source_commit` 等于 `--source-sha`，并把
  Release 的 `--target` 设为同一个提交。

因此不存在“检查时是 A、构建时是 B、Release 记录成 C”的 TOCTOU 窗口：解析结果
只解析一次，构建和发布都使用同一份 lock 与同一个提交。

## 播放器内自动更新

安装包内的 Rust helper 与 Lua 检测器共同完成更新。Lua 只负责检查 Release、
显示 UI 和以结构化参数启动 helper；任何文件修改都在 helper 中完成。

更新遵循“安装完整新版本，再应用用户层”的模型：

1. 从固定仓库的 Release 下载 `manifest.json`、`SHA256SUMS` 与平台归档。
2. 用 `SHA256SUMS` 校验归档；manifest 中的版本、tag、仓库、平台资产名、大小
   和 `updater` 协议必须匹配；helper 版本不能低于 `min_helper_version`。
3. 在安装目录之外的临时目录解压，拒绝路径穿越、绝对路径和链接逃逸。
4. 校验包内 `playkit-version.json`、`manifest.json`、`playkit-inventory.json`
   的完整文件哈希，以及 `.playkit/state.json` 记录的官方基线。
5. 等待 mpv 退出后，在同级 staging 目录复制新包、保留运行时状态、迁移
   `portable_config/user/`、应用手动与自动 patch。
6. 用两次重命名完成交换：`current → current.previous`，
   `staging → current`。失败时尝试恢复；成功后最多保留 current 与 previous。

`.playkit/official/` 保存本版本官方配置基线，`.playkit/generated/` 保存应用
patch 后最后写入的快照，二者用于捕获用户直接编辑和校验状态。用户 patch 是
语义差异（set/remove/append），不是 unified diff；未出现在 patch 中的键跟随
新官方配置。详见 [自动更新](updater.md) 与 [用户配置 Patch](user-patches.md)。

## 确定性

- 资产由 `sources.lock.json` 固定版本与哈希；下载后再次校验。许可证文本同样固定
  到完整 commit 与 SHA-256，下载后校验。
- 压缩包内时间戳、属主、权限被归一化；tar.gz 的 gzip 头 `mtime=0`。
- `manifest.json` 记录每个组件的上游版本、完整 commit（mpv 必有）、资产名和
  SHA-256，以及每个许可证文本的 SHA-256；`platforms.<platform>` 另记录该平台
  实际使用的 `mpv_version` / `mpv_commit`。还记录配置、脚本、构建代码、安装器
  和随包许可证文档的 `inputs_hash` 与 `sources.lock.json` 的内容哈希。
- 压缩包内时间戳、属主、权限被归一化，因此归档条目本身不随构建时间变化。
- `playkit-version.json` / `manifest.json` 中的 `built_at` 默认取当前 UTC 时间，
  会随每次构建变化；需要可复现的字节级产物时，设置 `SOURCE_DATE_EPOCH` 或向
  `build.assemble` / `build.release` 传入 `--built-at`。日常发布以 `SHA256SUMS`
  和 `inputs_hash` 校验来源与完整性，不依赖“不同时间构建字节相同”。

## 输入哈希

`build/inputs.py` 对以下路径按相对路径排序后计算 SHA-256：

```text
config/
scripts/local/
updater/
manifests/
installer/
build/
.github/workflows/build.yml
VERSION
LICENSE
NOTICE
THIRD_PARTY.md
LICENSE_PROVENANCE.md
```

`build/` 包含会改变最终发行包内容、结构、uosc 补丁、许可证索引和发布元数据的
构建源码；`.github/workflows/build.yml` 决定实际执行的组装/打包命令和参数，
因此也参与哈希。`updater/` 同时包含 Lua 和 Rust helper 源码（`Cargo.toml`、
`Cargo.lock`、`src/`、`tests/`）；`updater/helper/target/` 是本地编译缓存，
被 `build/inputs.py` 和 `.gitignore` 排除。`tests/`、`__pycache__/`、`.pyc/.pyo`
与 Python 工具缓存不参与哈希：纯测试或本地缓存变化不会触发正式 Release。任何
上述输入的变化都会改变 `inputs_hash`，从而触发下一次月度发布，即使上游没有变化。

## uosc UI 缩放补丁

`build/assemble.py` 在安装完上游 `uosc.zip` 后调用 `build/patch_uosc.py`，在
`scripts/uosc/main.lua` 中插入 `scale=0` 自动缩放模式：按当前显示器较短边
除以 1080 计算倍率（1080p 为 100%），并观察 `display-width` / `display-height`，
窗口拖到另一块显示器时重新计算。正数 `scale` 仍保持上游“OS DPI 因子 × 用户值”
的语义。补丁锚点在上游更新后找不到时构建会明确失败；被打补丁的文件带有本项目
名称与修改日期的说明，其余 uosc 文件保持上游原样。

## 缩略图引擎

uosc 5.13 固定向名为 `thumbfast` 的脚本发送 `thumb` / `clear` 消息。本项目的
`scripts/local/thumbfast.lua` 是原创实现：

1. 文件加载后按视频宽高比计算不超过 270×270 的预览尺寸，并向 uosc 发送
   `thumbfast-info`。
2. 收到 `thumb` 后按 `precise` 秒分桶；缓存未命中时启动随包的 mpv 子进程，
   使用 `--start=<bucket> --vo=null --vf=scale=...` 解码一帧，经
   `screenshot-raw` 写成 BGRA 原始帧和尺寸元数据。
3. 主进程用 `overlay-add` 显示该帧，`clear` 时移除。
4. 默认跳过短于 30 秒的视频；可通过 `script-opts/thumbfast.conf` 调整。
5. 子进程复用包内 mpv，不额外依赖 ffmpeg；Windows 和 Linux 共用同一实现。

## 运行时状态隔离

冒烟测试把 `saved-props.json`、缩略图缓存、watch-later、ICC/着色器缓存全部重定向
到临时目录，避免把测试产生的状态文件打进发布包。`build/verify` 也会拒绝
`_cache/`、`saved-props.json`、`update-check-state.json`、`.playkit/update-status.json`
和 `.playkit/updater.log`。

`.playkit/state.json`、`.playkit/official/`、`.playkit/generated/` 和
`playkit-inventory.json` 是**有意随包发布**的更新状态：全新安装的官方基线与生成
配置字节相同，用户层为空。更新时 helper 保留 `saved-props.json`、
`update-check-state.json`、`watch_later/`、`_cache/watch_later/` 和
`_cache/command_history.txt`，其余运行缓存丢弃。

## 平台差异

### Windows

- 从 shinchiro 的 `mpv-x86_64-YYYYMMDD-git-<commit>.7z` 中只取
  `mpv.exe`、`mpv.com`、`d3dcompiler_43.dll`。
- 包根目录即播放器目录；`portable_config/` 与 `mpv.exe` 同级。
- `yt-dlp.exe` 放在包根目录。
- 更新器为安装目录根部的 `updater/playkit-updater.exe`，使用 MSVC 静态 CRT
  构建，不依赖系统 VC++ 运行库。

### Linux

- 下载 pkgforge-dev 的 `mpv-<version>-anylinux-x86_64.AppImage`。
- 优先用 `unsquashfs` 按 squashfs magic 偏移解包；失败时调用 AppImage 自身的
  `--appimage-extract`。
- 只删除 `self-updater` 与 `get-yt-dlp` 两类 hook，避免 AppImage 运行时自动更新
  或再次下载 yt-dlp；CA 证书、路径映射、命名空间等上游兼容 hook 保留。
- 生成的 `mpv-playkit` 启动器设置 `DISABLE_AUTO_UPDATES=1`，用
  `--config-dir` 指向包内 `portable_config/`，并执行 `mpv/AppRun`。
- `yt-dlp` 放在包根目录并加入启动器的 `PATH`。
- 更新器为安装目录根部的 `updater/playkit-updater`，使用
  `x86_64-unknown-linux-musl` 静态链接；打包时设置可执行位并在压缩包中保留。

## 校验门禁

`build.verify` 与 `build.assemble` 会在发布前检查：

1. 必需文件存在（两个平台的播放器、配置、脚本、字体、manifest、许可证索引、
   `playkit-inventory.json`、`.playkit/state.json`、helper）。
2. 禁用路径不存在：基线目录、shader、VapourSynth、历史 `thumb_engine`、模型、
   AppImage 自更新/yt-dlp 下载 hook、运行时缓存和状态文件。
3. `mpv.conf` 中 `~~/` 引用存在。
4. `update-check.lua` 的 `enabled=no` 门禁位于任何 subprocess 调用之前。
5. 更新器元数据：`protocol=1`、`patch_schema=1`、固定的 `user_layer` /
   `state_dir`、helper 资产名/版本/SHA-256/大小；Linux 上 helper 必须可执行。
6. `playkit-inventory.json` 覆盖包内全部官方文件，哈希匹配；官方基线与生成配置
   相同；每个文件都可通过路径安全检查。
7. 压缩包顶层目录正确、无路径穿越、必需成员齐全、SHA-256 匹配。
8. `manifest.json` 的版本和平台与构建参数一致（Release manifest schema 2；
   旧版 schema 1 仍可解析，新增字段是可选扩展）。

任何一步抛错都会让工作流失败，后续 release 作业不会运行。

## 工作流

| 工作流 | 触发 | 行为 |
| --- | --- | --- |
| `monthly-update.yml` | 每月 1 日 05:17 UTC；手动 | resolve → 候选 artifact → build → commit-lock → release |
| `build.yml` | push、PR、手动、被调用 | Python 测试 + Rust 更新器测试（Ubuntu/Windows）+ Windows/Linux 构建、冒烟、打包、校验；被调用时先安装并校验候选 lock |
| `release.yml` | 被 `monthly-update.yml` 调用 | 校验候选 lock 字节，从 Actions 缓存取回两个平台产物，生成发布元数据并在最终提交上创建 Release |

候选 lock 通过 workflow artifact 在 resolve、两个平台构建、commit-lock 与
release 之间传递；每个消费者都校验 resolve 记录的 `lock_sha256`。两个平台的
构建产物通过 Actions 缓存（不是 artifact 存储）传递给 release 作业，避免 artifact
存储配额限制。缓存键包含 `run_id`、`run_attempt` 与候选 lock 的 SHA-256。

`build.yml` 中两个平台作业并行；`release.yml` 只在两者都成功后运行。Rust 更新器
在两个操作系统上执行 `cargo test --all-targets`、`cargo fmt --check` 和
`cargo clippy -D warnings`；任一失败会阻止平台构建与发布。

## 失败语义

- 解析失败、许可证无法取得、资产缺失、哈希不符、解包失败、uosc 补丁锚点失效、
  冒烟失败、结构校验失败、发布资产不完整：工作流失败，不创建 Release。
- 任一平台构建或校验失败：`commit-lock` 不运行，main 中的 `sources.lock.json`
  保持上一次已验证的版本；候选提交只留在临时分支，不影响发布。
- 已存在同名 tag 或 Release：`release.yml` 明确拒绝覆盖。
- 无可发布变化：不创建空 Release，也不提交新 lock（仅 `generated_at` 变化不算变化）。
- 仅有 lock 变化、而上游组件与最新 Release 完全一致时（例如修复过期 lock），
  只把 lock 提交到 main，不制造空 Release；该组合此前已经通过发布验证。
