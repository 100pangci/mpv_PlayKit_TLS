# mpv PlayKit TLS

面向 Windows x64 与 Linux x64 的便携式 mpv 发行版。本仓库只保存配置、原创 Lua、
平台适配、构建脚本和上游来源定义；mpv、yt-dlp、uosc、字体等第三方文件在构建时
从上游下载并校验，不长期入库。

功能目标：除滤镜外，与基线 `mpv-lazy-20260510.exe` 保持一致的用户体验，包括
uosc 界面与菜单、完整快捷键、时间轴缩略图、属性保存、统计/控制台、profiles、
截图、yt-dlp 在线播放和播放器内更新检测。所有历史 PlayKit Lua 均已由原创实现或
官方上游替代，不复制来源不明的代码。

包含：

- mpv（Windows: shinchiro 构建；Linux: pkgforge-dev AppImage）
- yt-dlp
- uosc 与 LXGW WenKai Mono Lite 字体
- 播放器配置、profiles、按键菜单与原创本地脚本
- 播放器内更新检测器
- GitHub Actions 每月自动检查、构建与发布

不包含 GLSL shader、VapourSynth、AI 超分/补帧、CUDA、TensorRT、DirectML 或模型。
这是唯一有意偏离基线的功能范围；菜单中的滤镜/着色器条目也已移除。

## 下载与使用

发布页：<https://github.com/100pangci/mpv_PlayKit_TLS/releases>

- Windows：解压 `mpv-PlayKit-<version>-windows-x64.zip`，双击 `mpv.exe`。
- Linux：解压 `mpv-PlayKit-<version>-linux-x64.tar.gz`，在终端运行：

  ```sh
  ./mpv-PlayKit-<version>-linux-x64/mpv-playkit
  ```

`portable_config/` 是完整配置目录。第三方组件的许可证原文位于包内 `licenses/`，
`licenses/INDEX.json` 记录每个文件的来源和 SHA-256。

## 更新策略

- GitHub Actions 每月 1 日 05:17 UTC 检查上游。
- 上游和仓库输入都没有变化时不创建空 Release。
- 有变化时才构建两个平台，生成 `manifest.json` 与 `SHA256SUMS`，然后发布。
- 任一组件下载、哈希校验、解包、冒烟测试或结构校验失败，整个发布流程停止。
- 版本号采用 `年.月`，同月多次发布时追加 `.1`、`.2`。
- `sources.lock.json` 固定每个上游资产的版本、URL 与 SHA-256。

播放器内的更新检测器只提示新版本，不会下载或覆盖播放器。关闭方法见
[更新检测器文档](docs/update-checker.md)；`enabled=no` 时脚本不会发起任何网络请求。
仓库为私有时，GitHub 的匿名 Release API 不可用，检测器只记录日志；菜单中的
“打开 Release 页面”仍可通过浏览器登录状态访问最新发布页。仓库公开后会自动恢复
版本检测。

## 仓库结构

```text
config/
  common/            两个平台共用的 mpv 配置、按键、原生菜单、profiles、script-opts
  windows/           Windows 平台配置
  linux/             Linux 平台配置
scripts/local/       本项目原创或重写的 Lua（构建时复制到 scripts/ 根目录）
updater/             播放器内更新检测器
manifests/           上游来源、资产匹配规则、安装规则
sources.lock.json    当前固定的上游版本与哈希
build/               解析、下载、解包、组装、打包、校验与发布工具
tests/               单元测试与冒烟测试素材
docs/                架构、上游、更新检测器与许可证处置说明
.github/workflows/   月度检查、双平台构建、发布
```

## 本地构建

要求 Python 3.11+、7-Zip；Linux 还需要 `squashfs-tools`（解包 AppImage）。
Windows 上可用 `SEVEN_ZIP_PATH` 指定 `7z.exe`。

```sh
python -m pip install -r requirements.txt

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

发布前会检查：必需文件、禁用路径（基线文件、shader、VapourSynth、模型、
自更新/yt-dlp 下载 hook）、`update-check` 的联网门禁顺序、压缩包顶层目录与
SHA-256。任何一项失败都不会发布。

## 许可证

- 本仓库原创代码：MIT，见 [LICENSE](LICENSE)。
- 第三方组件：各自保留上游许可证，构建时下载到包内 `licenses/`；汇总见
  [THIRD_PARTY.md](THIRD_PARTY.md)。
- 基线审计与历史 PlayKit 材料的处置：见 [LICENSE_PROVENANCE.md](LICENSE_PROVENANCE.md)。
- 本仓库不包含也不授权任何未解决许可证的历史 PlayKit Lua 或 GLSL 文件。

## 文档

- [架构与构建流程](docs/architecture.md)
- [上游来源与更新规则](docs/upstreams.md)
- [播放器内更新检测器](docs/update-checker.md)
- [Lua 处置清单](docs/lua-disposition.md)
- [基线审计摘要](docs/baseline-audit.md)
