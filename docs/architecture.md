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
| `updater/update-check.lua` | 播放器内更新检测器；构建时同样复制到 scripts 根目录 |
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
build.resolve ── GitHub Releases API ──► sources.lock.json
      │                                   (版本、URL、sha256、许可证 URL)
      ▼
build.decide ── 比较最新 Release 的 manifest.json
      │          + 当前 lock + inputs_hash
      ├── 无变化 ──► 结束，不创建 Release
      ▼ 有变化
build.fetch ── 下载资产与许可证，校验 SHA-256
      │
      ▼
build.assemble ── 按平台解包、安装、生成 playkit-version.json / manifest.json
      │            （Linux 额外解包 AppImage、删除自更新/yt-dlp 下载 hook、生成启动器）
      ▼
build.smoke ── mpv --version + 配置加载播放 + yt-dlp --version
      │
      ▼
build.package ── 确定性 zip / tar.gz + .sha256
      │
      ▼
build.verify ── 结构、禁用路径、引用、门禁顺序、压缩包校验
      │
      ▼
build.release ── 汇总两个平台的 manifest 与哈希，生成发布 manifest/SHA256SUMS
      │
      ▼
gh release create ── 仅在此前全部成功时执行
```

## 确定性

- 资产由 `sources.lock.json` 固定版本与哈希；下载后再次校验。
- 压缩包内时间戳、属主、权限被归一化；tar.gz 的 gzip 头 `mtime=0`。
- `manifest.json` 记录每个组件的上游版本、资产名和 SHA-256，以及配置、脚本、
  构建代码、安装器和随包许可证文档的 `inputs_hash` 与 `sources.lock.json`
  的内容哈希。
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
因此也参与哈希。`tests/`、`__pycache__/`、`.pyc/.pyo` 与 Python 工具缓存不参与
哈希：纯测试或本地缓存变化不会触发正式 Release。任何上述输入的变化
都会改变 `inputs_hash`，从而触发下一次月度发布，即使上游没有变化。

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
`_cache/`、`saved-props.json` 和 `update-check-state.json`。

## 平台差异

### Windows

- 从 shinchiro 的 `mpv-x86_64-YYYYMMDD-git-<commit>.7z` 中只取
  `mpv.exe`、`mpv.com`、`d3dcompiler_43.dll`。
- 包根目录即播放器目录；`portable_config/` 与 `mpv.exe` 同级。
- `yt-dlp.exe` 放在包根目录。

### Linux

- 下载 pkgforge-dev 的 `mpv-<version>-anylinux-x86_64.AppImage`。
- 优先用 `unsquashfs` 按 squashfs magic 偏移解包；失败时调用 AppImage 自身的
  `--appimage-extract`。
- 只删除 `self-updater` 与 `get-yt-dlp` 两类 hook，避免 AppImage 运行时自动更新
  或再次下载 yt-dlp；CA 证书、路径映射、命名空间等上游兼容 hook 保留。
- 生成的 `mpv-playkit` 启动器设置 `DISABLE_AUTO_UPDATES=1`，用
  `--config-dir` 指向包内 `portable_config/`，并执行 `mpv/AppRun`。
- `yt-dlp` 放在包根目录并加入启动器的 `PATH`。

## 校验门禁

`build.verify` 与 `build.assemble` 会在发布前检查：

1. 必需文件存在（两个平台的播放器、配置、脚本、字体、manifest、许可证索引）。
2. 禁用路径不存在：基线目录、shader、VapourSynth、历史 `thumb_engine`、模型、
   AppImage 自更新/yt-dlp 下载 hook、运行时缓存和状态文件。
3. `mpv.conf` 中 `~~/` 引用存在。
4. `update-check.lua` 的 `enabled=no` 门禁位于任何 subprocess 调用之前。
5. 压缩包顶层目录正确、无路径穿越、必需成员齐全、SHA-256 匹配。
6. `manifest.json` 的版本和平台与构建参数一致。

任何一步抛错都会让工作流失败，后续 release 作业不会运行。

## 工作流

| 工作流 | 触发 | 行为 |
| --- | --- | --- |
| `monthly-update.yml` | 每月 1 日 05:17 UTC；手动 | resolve → decide → 有变化才调用 build → release |
| `build.yml` | push、PR、手动、被调用 | 单元测试 + Windows/Linux 构建、冒烟、打包、校验 |
| `release.yml` | 被 `monthly-update.yml` 调用 | 从 Actions 缓存取回两个平台产物，生成发布元数据并创建 Release |

两个平台的构建产物通过 Actions 缓存（不是 artifact 存储）传递给 release 作业，
避免 artifact 存储配额限制。缓存键包含 `run_id` 与 `run_attempt`。

`build.yml` 中两个平台作业并行；`release.yml` 只在两者都成功后运行。

## 失败语义

- 解析失败、资产缺失、哈希不符、解包失败、冒烟失败、结构校验失败、发布资产不完整：
  工作流失败，不创建 Release。
- 已存在同名 tag：`release.yml` 明确拒绝覆盖。
- 无可发布变化：不创建空 Release，也不提交新 lock（仅 `generated_at` 变化不算变化）。
