# mpv PlayKit TLS

面向 Windows x64 与 Linux x64 的便携式 mpv 发行版。本仓库只保存配置、原创 Lua、
平台适配、构建脚本和上游来源定义；mpv、yt-dlp、uosc、字体等第三方文件在构建时
从上游下载并校验，不长期入库。

功能目标：除 VapourSynth/AI 滤镜外，与基线 `mpv-lazy-20260510.exe` 保持一致的用户
体验，包括 uosc 界面与菜单、完整快捷键、时间轴缩略图、属性保存、统计/控制台、
profiles、截图、yt-dlp 在线播放和播放器内更新检测。所有历史 PlayKit Lua 均已由
原创实现或官方上游替代，不复制来源不明的代码。

包含：

- mpv（Windows: shinchiro 构建；Linux: pkgforge-dev AppImage）
- yt-dlp
- uosc（构建时仅对 `main.lua` 应用自动 UI 缩放补丁，其余文件保持上游）与
  LXGW WenKai Mono Lite 字体
- 播放器配置、profiles、按键菜单与原创本地脚本
- 构建时从真正上游下载并校验的 GLSL 着色器：Anime4K v4/GAN、ACNet/ARNet、
  ArtCNN、CuNNy、AiUpscale、NNEDI3/RAVU、CfL、JointBilateral、PixelClipper、
  AN3223 nlmeans/guided/hdeband、Adaptive sharpen、FSRCNNX（代码与模型/数据
  许可证分别记录，逐文件见 `docs/shader-license-audit.md`）
- Rust `umpv` 单实例启动器：多个文件交给同一个 mpv 窗口，Windows/Linux 共用一套源码
- 播放历史与书签：构建时固定上游提交的 SimpleHistory / SimpleBookmark，数据保存在
  用户层并在更新、回滚后保留
- 截图增强：原创的截图模式、区域裁剪、连拍与图片拼接
- 画中画：原创的窗口角落缩放与完整窗口状态恢复
- 动态裁切：构建时固定 [Ashyni/mpv-scripts](https://github.com/Ashyni/mpv-scripts)
  的 `dynamic-crop.lua`（MIT），默认按需启用
- 纯净/测试/跑分模式启动器（`installer/modes/`），用于隔离配置问题和基准测试
- 播放器内自动更新：Rust 更新器负责下载、校验、patch 和事务式替换
- 用户配置 patch 层（`portable_config/user/`），更新时自动重新应用
- GitHub Actions 每月自动检查、构建与发布

不包含 VapourSynth、CUDA、TensorRT、DirectML 和不透明模型文件，也不包含许可证
不允许再分发的 shader（Ani4Kv2/AniSD 的模型为 CC BY-NC 4.0，AnimeJaNai 为
CC BY-NC-SA 4.0，k7/QCOM/ESPCN/LUT/Shadertoy 等来源或授权不明）。这些文件不会
进入仓库或构建产物；随包的 AI shader 权重内嵌在 GLSL/.hook 中，并单独记录模型
来源与许可证。

## 来源声明

本项目源自 [hooke007/mpv_PlayKit](https://github.com/hooke007/mpv_PlayKit)，在其基线
版本 `mpv-lazy-20260510` 的基础上重新构建。感谢原作者 hooke007 的付出与开源分享。

本项目仅参考其功能与配置结构，历史 PlayKit Lua 未复制进仓库；凡入选的功能均由
本项目原创实现，或改用官方上游实现（逐项见 `docs/lua-disposition.md`、
`LICENSE_PROVENANCE.md`）。

播放历史与书签直接复用 [Eisa01/mpv-scripts](https://github.com/Eisa01/mpv-scripts)
的 SimpleHistory / SimpleBookmark，版权归 Eisa AlAwadhi 所有，许可证为
BSD-2-Clause；完整许可证文本随包放在 `licenses/eisa01-mpv-scripts/LICENSE`。
本项目未修改上游脚本源码，只添加了 `script-opts` 选项覆盖。

动态裁切直接复用 [Ashyni/mpv-scripts](https://github.com/Ashyni/mpv-scripts) 的
`dynamic-crop.lua`（MIT）。它是 dyphire/mpv-config 中同名脚本的原始上游，构建时
按固定提交下载并校验 SHA-256，完整许可证文本随包放在
`licenses/ashyni-dynamic-crop/LICENSE`。截图增强与画中画为本项目原创实现。

功能规划还参考了 [dyphire/mpv-config](https://github.com/dyphire/mpv-config) 等社区
配置；动态裁切已确认其原始上游为 Ashyni/mpv-scripts。感谢这些项目及其作者的
开源分享。

## 下载与使用

发布页：<https://github.com/100pangci/mpv_PlayKit_TLS/releases>

- Windows：解压 `mpv-PlayKit-<version>-windows-x64.zip`，双击 `mpv.exe`。
  也可以双击 `installer\windows\install.cmd` 安装到当前用户目录，创建开始菜单、
  桌面快捷方式、右键菜单和“打开方式”注册（不需要管理员权限，详见
  [installer/README.md](installer/README.md)）。
  单实例启动：把媒体文件交给包根目录的 `umpv.exe`，或运行
  `installer\windows\umpv-register.cmd` 把 `umpv.exe` 加入“打开方式”。
  详见 [docs/umpv.md](docs/umpv.md)。
- Linux：解压 `mpv-PlayKit-<version>-linux-x64.tar.gz`，在终端运行：

  ```sh
  ./mpv-PlayKit-<version>-linux-x64/mpv-playkit
  ```

  Linux 安装器接口已预留但尚未实现（`installer/linux/`）。

`portable_config/` 是完整配置目录。第三方组件的许可证原文位于包内 `licenses/`，
`licenses/INDEX.json` 记录每个文件的来源和 SHA-256。

## 播放增强

### 播放历史与书签

播放历史与书签使用上游 [Eisa01/mpv-scripts](https://github.com/Eisa01/mpv-scripts)
的 `SimpleHistory` 与 `SimpleBookmark`（BSD-2-Clause）。两者在构建时按
`sources.lock.json` 固定的提交下载并校验 SHA-256，不复制进本仓库；PlayKit 只
提供 `config/common/script-opts/` 下的选项覆盖。数据写入
`portable_config/user/data/`，自动更新和回滚都会保留。

| 按键 | 功能 |
| --- | --- |
| `Ctrl+Alt+H` | 打开播放历史列表 |
| `Ctrl+Alt+R` | 从历史记录继续上次进度 |
| `Alt+R` | 打开上次播放的文件 |
| `Ctrl+Shift+I` | 切换隐私模式（本次会话不写入历史） |
| `Ctrl+B` | 为当前位置添加书签 |
| `Alt+B` | 添加文件书签（不带时间点） |
| `B` | 打开书签列表 |
| `Ctrl+Alt+1` | 打开“收藏”书签分组 |

历史的自动恢复已关闭（`resume_option=none`），默认仍由 mpv 的 `watch_later`
恢复进度，避免重复恢复；需要从历史恢复时使用上表的手动命令。书签加载也不会
写入 watch-later（`preserve_video_settings=no`），因此不会与 `watch_later` 和
`save_global_props` 的进度/属性恢复互相覆盖。历史列表支持
搜索、删除条目和跳转；路径失效的条目仍保留在日志中，打开时由 mpv 给出错误，
可在列表中删除。上游脚本没有 `enabled` 选项；如需完全关闭，删除或重命名
`portable_config/scripts/SimpleHistory.lua`（或 `SimpleBookmark.lua`）。历史与
书签的列表、过滤和隐私选项见对应的 `script-opts` 文件及上游文档。

### 截图增强

`scripts/local/screenshot_tools.lua` 是原创实现，不依赖基线代码。原生的
`Ctrl+S`（窗口-无字幕/OSD）和 `Ctrl+Shift+S`（窗口）保持不变，新增：

| 按键 | 功能 |
| --- | --- |
| `Alt+s` | 区域截图（区域来自 `region=x:y:w:h`，可覆盖字幕区域） |
| `Ctrl+Alt+s` | 连续截图（默认 5 张、间隔 1 秒） |
| `Ctrl+Alt+t` | 把最近一次连拍拼接成一张长图 |
| `Ctrl+Alt+c` | 取消当前截图、连拍或拼接任务 |

截图模式为 `video`（不含字幕/OSD）、`subtitles`（含字幕、不含 OSD）或
`window`（含字幕与 OSD）。默认输出到 `portable_config/user/screenshots/`，
支持 png/jpg/webp，质量沿用 mpv 当前的 `screenshot-*` 属性；区域截图通过临时
`video-crop` 实现并在完成后恢复用户原值。连拍记录写入
`portable_config/user/screenshot-tools/queue.jsonl`。拼接优先调用随包 mpv
子进程，失败或缺失时回退到 PATH 中的 `ffmpeg`；两者都不可用时只提示失败，
不会留下半成品。截图与队列位于用户层，自动更新和回滚会保留。全部选项见
`script-opts/screenshot_tools.conf`。

### 画中画

`scripts/local/pip.lua` 是原创实现，只使用 mpv 自身的窗口属性，不调用平台窗口
API，Windows 与 Linux 共用同一份逻辑。按 `Alt+p` 或选择原生菜单
“视频 > 切换 画中画”即可进入或退出。

进入时默认把窗口缩放到屏幕右下角，宽度为屏幕的 25%，与边缘间距 16 像素，临时
置顶并去掉边框；退出时按相反顺序恢复进入前的 `geometry`、`autofit`、
`autofit-larger`、`autofit-smaller`、`window-scale`、`window-maximized`、
`keepaspect-window`、`border`、`ontop` 和 `fullscreen`。若进入前是全屏，默认先
退出全屏，退出画中画后再恢复全屏。

配置位于 `script-opts/pip.conf`：`position`（四个角落）、`width_percent`、
`height_percent`（0 表示按视频宽高比计算）、`margin`、`min_width`、`ontop`、
`border`、`exit_fullscreen`、`follow_display`（显示器/DPI 变化时重新定位）、
`keep_on_new_file`（切换视频时重新计算大小）和 `toggle_keybind`。写入失败或某些
VO 不支持定位时会跳过对应属性，不会把窗口留在半配置状态。Wayland 合成器可能
忽略绝对窗口位置，此时仍会应用大小与置顶设置，但角落位置由合成器决定。

### 动态裁切

动态裁切使用上游 [Ashyni/mpv-scripts](https://github.com/Ashyni/mpv-scripts) 的
`dynamic-crop.lua`（MIT），构建时按 `sources.lock.json` 固定的提交下载到
`portable_config/scripts/` 并校验 SHA-256，不复制进本仓库。PlayKit 只提供
`script-opts/dynamic_crop.conf` 选项覆盖，默认 `mode=1`（按需启用），加载后不插入
滤镜、不修改 `video-crop`。

按 `Alt+c`（上游自身的 `Shift+C` 也保留）或选择原生菜单
“视频 > 切换 动态裁切”：第一次切换启用 cropdetect 并在结果稳定后应用裁切，
第二次移除裁切，第三次恢复上次裁切并继续检测。`crop_method=1` 使用 mpv 的
`video-crop` 属性，不向用户 `vf` 链插入滤镜，因此 GLSL shader、字幕和手动滤镜
保持原样；切换过程中不会删除用户的 `vf`。检测要求 `hwdec=no` 或 `*-copy`，本
发行版默认的 `auto-copy` 满足要求。`detect_limit`、`fast_change_timer`、
`ratio_timer`、`prevent_change_*` 等参数见 `dynamic_crop.conf` 与上游文件头说明。

## 更新策略

GitHub Actions 每月 1 日 05:17 UTC 检查上游，采用事务式流程：

1. **Resolve**：解析上游，生成候选 `sources.lock.json`，并上传为本次运行的
   workflow artifact。
2. **Build candidate**：Windows 与 Linux 下载同一份候选 lock，校验其 SHA-256
   后覆盖工作区里的 `sources.lock.json`，再分别构建、冒烟测试、打包与校验。
3. **Verify**：单元测试、两个平台的 smoke test、stage 校验与压缩包校验必须全部
   成功。
4. **Promote**：全部成功后才把候选 lock 提交到 main。候选提交先存在于临时分支，
   验证通过后以 fast-forward 方式推进 main；失败时 main 完全不变。
5. **Release**：Release 绑定到这个最终提交，tag、发布 manifest 的
   `source_commit` 与包内 `playkit-version.json` 指向同一个提交。

因此 main 中的 `sources.lock.json` 永远代表**最后一个已经通过完整双平台构建验证的
上游组合**，而不是未经验证的自动解析结果。任一环节失败都会停止发布。

- 上游和仓库输入都没有变化时不创建空 Release；`force` 手动触发仍可强制发布。
- 手动触发月度工作流时可勾选 `draft`，本次 Release 以草稿形式创建，便于先检查
  两个平台的资产和发布说明，再在 Releases 页面手动发布；定时任务仍直接发布。
- 版本号采用 `年.月`，同月多次发布时追加 `.1`、`.2`。
- `sources.lock.json` 固定每个上游资产的版本、URL 与 SHA-256；许可证文本也固定到
  上游仓库的完整 commit 与 SHA-256，下载 URL 不使用 `main`/`master`/`latest`。
- `manifest.json` 的 `platforms.<platform>` 分别记录该平台的 `mpv_version` 与
  `mpv_commit`。Windows（shinchiro）与 Linux（pkgforge-dev）可能使用不同的 mpv
  提交，发布说明中按平台分列，不再只写一个模糊的 mpv 版本。

播放器内的更新器会检测新版本，并在用户确认后下载完整安装包、校验 SHA-256、
展开到同级 staging 目录、重新应用 `user/` 层的配置 patch，等 mpv 退出后再
事务式交换目录。旧版本保留为 `.previous`，可从菜单回滚。更新只安装完整的
新 PlayKit，不会对旧安装目录做增量补丁。

`enabled=no` 时脚本不会发起任何网络请求，也不会启动更新器；`install_enabled=no`
可以只关闭安装而保留检查。用户配置以语义 patch 保存在
`portable_config/user/patches/`（schema 2 行操作放在 `v2/`），新官方配置整体
替换后 patch 会自动重新应用；正常更新不需要手动“应用 patch”。改完 patch 后
重启播放器也会自动检测并重新应用：启动时尚未加载媒体的情况下，播放器会自动
重启一次以加载新配置；已经加载媒体时只提示手动重启，避免丢失播放。
`auto_repair=no` 可以关闭自动修复，`auto_repair_relaunch=no` 可以关闭自动重启。
菜单中的“立即重新应用配置 patch”只用于改完 patch 后立即生效或手动迁移。
详见 [自动更新文档](docs/updater.md) 与 [用户配置 Patch](docs/user-patches.md)。

检查器通过 GitHub 公开 API 读取最新 Release；若仓库以后改为私有，匿名检测会
失败并只记录日志，菜单中的“打开 Release 页面”仍会回退到最新发布页。

## 仓库结构

```text
config/
  common/            两个平台共用的 mpv 配置、按键、原生菜单、profiles、script-opts
  windows/           Windows 平台配置
  linux/             Linux 平台配置
  umpv/              umpv 单实例启动器配置（Windows 命名管道 / Linux socket）
scripts/local/       本项目原创或重写的 Lua（构建时复制到 scripts/ 根目录）
installer/           可选的 Windows 安装/卸载与 umpv 注册脚本、Linux 安装接口占位
installer/modes/     纯净/测试/跑分模式启动器
updater/            播放器内更新检测 Lua 与 Rust 更新器/umpv（updater/helper/）
manifests/           上游来源、资产匹配规则、安装规则
sources.lock.json    当前固定的上游版本与哈希
build/               解析、下载、解包、组装、打包、校验与发布工具
tests/               单元测试与冒烟测试素材
docs/                架构、上游、更新检测器与许可证处置说明
.github/workflows/   月度检查、双平台构建、发布
```

## 本地构建

要求 Python 3.11+、7-Zip、Rust 1.85+（含 Cargo）；Linux 还需要
`squashfs-tools`（解包 AppImage）。Windows 上可用 `SEVEN_ZIP_PATH` 指定
`7z.exe`。Linux 发行包使用 `x86_64-unknown-linux-musl` 静态构建，需先
`rustup target add x86_64-unknown-linux-musl` 并安装 `musl-tools`。

```sh
python -m pip install -r requirements.txt

# 0. 构建 Rust 更新器与 umpv（Windows 产物为 playkit-updater.exe / umpv.exe）
python -m build.helper --platform windows-x64

# 1. 解析上游并写 lock（也可先用仓库中已有的 sources.lock.json）
python -m build.resolve --output sources.lock.json

# 2. 下载并校验
python -m build.fetch --lock sources.lock.json --work .work --platform windows-x64

# 3. 组装
python -m build.assemble \
  --platform windows-x64 \
  --version 2026.11 \
  --lock sources.lock.json \
  --work .work \
  --out dist/stage/windows-x64 \
  --helper dist/updater/windows-x64/playkit-updater.exe \
  --repository 100pangci/mpv_PlayKit_TLS

# 4. 冒烟测试、打包、校验
python -m build.smoke --stage dist/stage/windows-x64 --platform windows-x64
python -m build.package --stage dist/stage/windows-x64 --platform windows-x64 --version 2026.11 --out-dir dist
python -m build.verify --stage dist/stage/windows-x64 --platform windows-x64 --version 2026.11
python -m build.verify --asset dist/mpv-PlayKit-2026.11-windows-x64.zip --platform windows-x64 --version 2026.11
```

Linux 构建把 `windows-x64` 换成 `linux-x64`，产物是 `.tar.gz`。PowerShell 中把
续行的 `\` 换成反引号，并按需设置：

```powershell
$env:SEVEN_ZIP_PATH = "D:\Software\7-Zip\7z.exe"
```

Linux 的 AppImage 解包依赖 `unsquashfs`；没有该工具时构建脚本会尝试用 AppImage
自身的 `--appimage-extract`，但在无 FUSE 的环境中仍建议使用 GitHub Actions 的
Ubuntu runner。

## 发布与维护

- 自动：`Monthly update` 工作流按 cron 运行。
- 手动强制发布：`Actions` → `Monthly update` → `Run workflow`，勾选 `force`。
- 只构建不发布：运行 `Build` 工作流，产物在 Actions 运行结束后通过缓存保留，
  可在运行日志中查看构建结果。
- 固定到新的上游版本：运行 `build.resolve` 生成新的 `sources.lock.json` 并提交；
  下一次检查会因 lock/inputs 变化触发构建。

发布前会检查：必需文件、禁用路径（基线文件、未声明的 shader、VapourSynth、模型、
自更新/yt-dlp 下载 hook）、`update-check` 的联网门禁顺序、压缩包顶层目录与
SHA-256，以及更新器与 umpv 清单（`playkit-inventory.json`、`.playkit/state.json`、
helper/umpv 版本/大小/SHA-256、Linux 可执行位）。任何一项失败都不会发布。

## 许可证

- 本仓库原创代码：**GPL-3.0-or-later**，见 [LICENSE](LICENSE)；版权与第三方
  组件边界、GPL 源码提供说明见 [NOTICE](NOTICE)。
- 第三方组件：各自保留上游许可证，构建时下载到包内 `licenses/`；汇总见
  [THIRD_PARTY.md](THIRD_PARTY.md)。本项目不改变这些组件的许可证。
- 基线审计与历史 PlayKit 材料的处置：见 [LICENSE_PROVENANCE.md](LICENSE_PROVENANCE.md)。
- 本仓库不包含也不授权任何未解决许可证的历史 PlayKit Lua 文件。入选的 GLSL
  着色器只从作者或项目上游按固定提交下载，逐文件记录 SHA-256 和许可证；
  模型权重为 CC BY-NC 或来源不明的 shader 不导入。

## 文档

- [架构与构建流程](docs/architecture.md)
- [上游来源与更新规则](docs/upstreams.md)
- [播放器内更新检测器](docs/update-checker.md)
- [自动更新与事务式安装](docs/updater.md)
- [用户配置 Patch](docs/user-patches.md)
- [umpv 单实例启动器](docs/umpv.md)
- [Lua 处置清单](docs/lua-disposition.md)
- [基线审计摘要](docs/baseline-audit.md)
