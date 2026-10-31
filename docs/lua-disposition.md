# 基线 Lua 处置清单

基线为 `mpv-lazy-20260510.exe`（SHA-256
`f03e288d5c41b00604a64d30bb81e5f76e7f125c0447926ff39561b32d44fef4`）。
许可证审计结论见 [LICENSE_PROVENANCE.md](../LICENSE_PROVENANCE.md)。

目标：保留基线的用户可见功能，同时不复制任何许可证不明或无法逐文件确认的 Lua
代码。能重写的功能使用本项目的原创实现；能由官方上游提供的功能使用上游原版；
滤镜、着色器和 VapourSynth 相关内容不进入本发行版。

## 9 个未解决文件

| 基线文件 | 处置 | 当前实现 |
| --- | --- | --- |
| `uosc/elements/Logo.lua` | 重写 | `scripts/local/idle-brand.lua`：原创空闲画面文字标签，不使用基线的矢量 Logo 代码 |
| `uosc/lib/lang.lua` | 由上游替代 | 使用 uosc 5.13.0 官方 `lib/intl.lua`（LGPL-2.1），不复制基线的部分副本；`uosc.conf` 设置 `languages=slang,zh-hans,zh,en` |
| `input_plus.lua` | 重写 | `scripts/local/input_plus.lua`：原创按键名检测和文档链接；不包含基线代码 |
| `save_global_props.lua` | 重写 | `scripts/local/save_global_props.lua`：原创属性保存/恢复，默认 `volume,mute` |
| `thumb_engine/batch.lua` | 合并重写 | 统一由 `scripts/local/thumbfast.lua` 实现，不再存在 `thumb_engine/` 目录 |
| `thumb_engine/helper.lua` | 合并重写 | 同上 |
| `thumb_engine/main.lua` | 合并重写 | 同上；uosc 5.13 通过脚本名 `thumbfast` 调用 |
| `thumb_engine/process.lua` | 合并重写 | 同上；使用随包的 mpv 子进程解码单帧 |
| `thumb_engine/winapi.lua` | 合并重写 | 同上；不依赖 Win32 API，Windows/Linux 共用 |

## 当前本地脚本

| 脚本 | 功能 | 来源 |
| --- | --- | --- |
| `idle-brand.lua` | 空闲画面文字标签 | 本项目原创 |
| `save_global_props.lua` | 保存/恢复音量与静音 | 本项目原创 |
| `input_plus.lua` | 按键名检测、文档链接 | 本项目原创 |
| `thumbfast.lua` | uosc 时间轴缩略图 | 本项目原创，干净实现 |
| `update-check.lua` | GitHub Release 更新提示 | 本项目原创 |

`thumbfast.lua` 不包含历史 `thumb_engine` 或第三方 thumbfast 的代码。它按 uosc
5.13 的公开消息协议工作：

- 启动/换文件时向 uosc 发送 `thumbfast-info`；
- 收到 `thumb` 时用 `mpv --start=<time>` 子进程解码一帧，缩放到最大 270×270，
  以 BGRA 原始帧缓存；
- 用 `overlay-add` 显示，收到 `clear` 时移除；
- 默认对短于 30 秒的文件禁用，可在 `script-opts/thumbfast.conf` 调整。

## uosc

- 基线包含 25 个 uosc Lua 文件，其中 16 个与上游逐字节一致，9 个被修改；这 9 个
  修改版的许可证状态无法逐文件确认。
- 本仓库不导入这些修改版，而是从 `tomasklaen/uosc` 的当前 Release 下载
  `uosc.zip`，保持 LGPL-2.1 原样。
- 本项目只通过 `input.conf` 的 `#!` 菜单注释、`script-opts/uosc.conf` 和
  `thumbfast.lua` 做配置层/协议层定制，不修改 uosc 源码。

## 其他基线内容

- 429 个 GLSL shader：全部不导入。
- VapourSynth、AI 超分/补帧、CUDA、TensorRT、DirectML、模型：全部不导入。
- 基线二进制、脚本包、字体（除另行按许可证获取的 LXGW 字体外）：不导入。
- 基线 `mpv.conf` 的选项值只作为功能参考；本仓库配置重新编写。

## 构建期保护

`build/verify.py` 拒绝：

- `baseline/`、`shaders/`、`vs/`、`thumb_engine/`、`mpv-lazy/` 路径；
- `.vpy`、`.onnx`、`.glsl` 文件；
- `self-updater`、`get-yt-dlp` 两类 AppImage hook；
- `_cache/`、`saved-props.json`、`update-check-state.json` 等运行时状态。

`input_plus.lua`、`thumbfast.lua`、`idle-brand.lua` 和 `save_global_props.lua`
是本项目允许的本地脚本，不属于禁用路径。

## 新增本地脚本的规则

1. 新脚本放 `scripts/local/`，文件名即 mpv 脚本名。
2. 不使用基线中许可证不明的代码；如需参考，只参考公开 API 用法并重写。
3. 构建时脚本被复制到包内 `portable_config/scripts/` 根目录，由 mpv 自动加载。
4. 不要修改 `scripts/uosc/`；uosc 由构建系统从上游获取。
5. 新增脚本后运行 `python -m build.verify`，确认没有落入禁用路径。
