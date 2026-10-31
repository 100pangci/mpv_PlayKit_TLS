# Third-party components

本仓库不提交下列二进制或未修改的上游文件。构建时由
`manifests/*.yml` + `sources.lock.json` 下载、校验并安装；包内保留许可证原文。

| 组件 | 上游 | 用途 | 许可证 | 包内位置 |
| --- | --- | --- | --- | --- |
| mpv (Windows) | [shinchiro/mpv-winbuild-cmake](https://github.com/shinchiro/mpv-winbuild-cmake) | 播放器 | GPL-2.0-or-later / LGPL-2.1-or-later（按组件） | `mpv.exe`, `mpv.com`, `d3dcompiler_43.dll` |
| mpv (Linux) | [pkgforge-dev/mpv-AppImage](https://github.com/pkgforge-dev/mpv-AppImage) | 播放器运行时 | GPL-3.0-or-later AND GPL-2.0-or-later；其中 mpv 本体为 GPL-2.0-or-later / LGPL-2.1-or-later | `mpv/` |
| yt-dlp | [yt-dlp/yt-dlp](https://github.com/yt-dlp/yt-dlp) | 在线视频解析 | Unlicense | `yt-dlp.exe` / `yt-dlp` |
| uosc | [tomasklaen/uosc](https://github.com/tomasklaen/uosc) | UI 与菜单 | LGPL-2.1-or-later；内含 MIT（`lib/fzy.lua`）、MPL-2.0（`lib/utils.lua` 的 thumbfast FFI 片段）、Apache-2.0（图标字体） | `portable_config/scripts/uosc/` |
| uosc / Material Icons | [google/material-design-icons](https://github.com/google/material-design-icons) | uosc 图标字体 | Apache-2.0 | `portable_config/fonts/uosc_icons.otf` |
| fzy (Lua) | [swarn/fzy-lua](https://github.com/swarn/fzy-lua) | uosc 菜单模糊匹配 | MIT | `portable_config/scripts/uosc/lib/fzy.lua` |
| thumbfast FFI 片段 | [po5/thumbfast](https://github.com/po5/thumbfast) | uosc 缩略图 FFI 声明 | MPL-2.0 | `portable_config/scripts/uosc/lib/utils.lua` |
| LXGW WenKai Mono Lite | [lxgw/LxgwWenKai-Lite](https://github.com/lxgw/LxgwWenKai-Lite) | 中文字幕字体 | OFL-1.1 | `portable_config/fonts/LXGWWenKaiMonoLite-Regular.ttf` |

`d3dcompiler_43.dll` 是 Microsoft DirectX 的可再分发组件，不是 mpv 或本项目的
GPL 代码；它由 shinchiro 的 Windows 构建附带，按 Microsoft 的 DirectX
可再分发条款随包提供。

本项目自己的 Lua（`idle-brand`、`save_global_props`、`input_plus`、`thumbfast`、
`update-check`）为原创实现，按仓库 **GPL-3.0-or-later** 分发，不属于第三方组件。
缩略图功能不包含历史 `thumb_engine` 或第三方 thumbfast 的代码。

空闲画面的 mpv 徽标造型与配色来自 mpv 的 `player/lua/osc.lua`（mpv 项目，
GPL-2.0-or-later / LGPL-2.1-or-later）；`scripts/local/idle-brand.lua` 用原创
绘图代码重新生成该造型，未复制基线 `Logo.lua` 的代码。mpv 的许可证原文随包
提供。

## 包内许可证文件

构建系统下载每个组件声明的许可证，写入：

```text
licenses/
  INDEX.json                              # 组件、版本、来源 URL、SPDX、SHA-256
  lxgw-wenkai-mono-lite/OFL.txt
  mpv-windows/LICENSE.GPL                 # Windows 包
  mpv-windows/LICENSE.LGPL
  mpv-linux/LICENSE                       # Linux 包
  mpv-linux/LICENSE.GPL
  mpv-linux/LICENSE.LGPL
  uosc/LICENSE.LGPL
  uosc/Material-Icons-Apache-2.0.txt
  uosc/MIT-fzy.txt
  uosc/MPL-2.0.txt
  yt-dlp-windows/LICENSE                  # Windows 包
  yt-dlp-windows/THIRD_PARTY_LICENSES.txt
  yt-dlp-linux/LICENSE                    # Linux 包
  yt-dlp-linux/THIRD_PARTY_LICENSES.txt
```

具体文件名以 `licenses/INDEX.json` 为准。`INDEX.json` 的 `license` 字段是**逐文件
SPDX**（例如 `LICENSE.LGPL` 标为 `LGPL-2.1-or-later`，`LICENSE.GPL` 标为
`GPL-2.0-or-later`）；`manifest.json` 中的 `components.*.license` 是组件级聚合。
`SHA256SUMS` 覆盖发布资产，`manifest.json` 同时记录上游资产哈希和
`sources.lock.json` 的内容哈希。

## 分发注意

- mpv 的 GPL/LGPL 义务由对应上游构建与许可证原文承担；再分发时不要删除
  `licenses/` 或修改二进制。
- uosc 以 LGPL-2.1 发布；本项目不修改 uosc 源码，直接使用上游 `uosc.zip`。
  uosc 的 `lib/fzy.lua`（MIT）和 `lib/utils.lua` 内的 MPL-2.0 片段保留原文件头；
  对应的完整许可证文本也放在 `licenses/uosc/`。uosc 附带的 `ziggy-*` 辅助程序
  源码位于上游仓库的同一 Release 标签，包内保留其许可证与来源记录。
- 字体按 OFL-1.1 分发；不要单独售卖字体。
- `yt-dlp` 的 Unlicense 不覆盖其运行时下载的第三方内容；`THIRD_PARTY_LICENSES.txt`
  随包提供。
- `d3dcompiler_43.dll` 按 Microsoft DirectX 可再分发条款提供，不适用本项目的
  GPL 许可证。

历史 mpv_PlayKit 基线中许可证不明的 Lua/GLSL 文件没有被复制进本仓库或构建产物。
详见 [LICENSE_PROVENANCE.md](LICENSE_PROVENANCE.md) 与
[docs/lua-disposition.md](docs/lua-disposition.md)。
