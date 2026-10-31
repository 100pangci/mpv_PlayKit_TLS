# Third-party components

本仓库不提交下列二进制或未修改的上游文件。构建时由
`manifests/*.yml` + `sources.lock.json` 下载、校验并安装；包内保留许可证原文。

| 组件 | 上游 | 用途 | 许可证 | 包内位置 |
| --- | --- | --- | --- | --- |
| mpv (Windows) | [shinchiro/mpv-winbuild-cmake](https://github.com/shinchiro/mpv-winbuild-cmake) | 播放器 | GPL-2.0-or-later / LGPL-2.1-or-later（按组件） | `mpv.exe`, `mpv.com`, `d3dcompiler_43.dll` |
| mpv (Linux) | [pkgforge-dev/mpv-AppImage](https://github.com/pkgforge-dev/mpv-AppImage) | 播放器运行时 | GPL-3.0-or-later AND GPL-2.0-or-later | `mpv/` |
| yt-dlp | [yt-dlp/yt-dlp](https://github.com/yt-dlp/yt-dlp) | 在线视频解析 | Unlicense | `yt-dlp.exe` / `yt-dlp` |
| uosc | [tomasklaen/uosc](https://github.com/tomasklaen/uosc) | UI 与菜单 | LGPL-2.1-or-later | `portable_config/scripts/uosc/` |
| uosc / Material Icons | [google/material-design-icons](https://github.com/google/material-design-icons) | uosc 图标字体 | Apache-2.0 | `portable_config/fonts/uosc_icons.otf` |
| LXGW WenKai Mono Lite | [lxgw/LxgwWenKai-Lite](https://github.com/lxgw/LxgwWenKai-Lite) | 中文字幕字体 | OFL-1.1 | `portable_config/fonts/LXGWWenKaiMonoLite-Regular.ttf` |

本项目自己的 Lua（`idle-brand`、`save_global_props`、`input_plus`、`thumbfast`、
`update-check`）为原创实现，按仓库 MIT 许可证分发，不属于第三方组件。缩略图功能
不包含历史 `thumb_engine` 或第三方 thumbfast 的代码。

## 包内许可证文件

构建系统下载每个组件声明的许可证，写入：

```text
licenses/
  INDEX.json                              # 组件、版本、来源 URL、SPDX、SHA-256
  lxgw-wenkai-mono-lite/OFL.txt
  mpv-windows/LICENSE.GPL                 # Windows 包
  mpv-windows/LICENSE.LGPL
  mpv-linux/LICENSE                       # Linux 包
  uosc/LICENSE.LGPL
  uosc/Material-Icons-Apache-2.0.txt
  yt-dlp-windows/LICENSE                  # Windows 包
  yt-dlp-windows/THIRD_PARTY_LICENSES.txt
  yt-dlp-linux/LICENSE                    # Linux 包
  yt-dlp-linux/THIRD_PARTY_LICENSES.txt
```

具体文件名以 `licenses/INDEX.json` 为准。`SHA256SUMS` 覆盖发布资产，`manifest.json`
同时记录上游资产哈希和 `sources.lock.json` 的内容哈希。

## 分发注意

- mpv 的 GPL/LGPL 义务由对应上游构建与许可证原文承担；再分发时不要删除
  `licenses/` 或修改二进制。
- uosc 以 LGPL-2.1 发布；本项目不修改 uosc 源码，直接使用上游 `uosc.zip`。
- 字体按 OFL-1.1 分发；不要单独售卖字体。
- `yt-dlp` 的 Unlicense 不覆盖其运行时下载的第三方内容；`THIRD_PARTY_LICENSES.txt`
  随包提供。

历史 mpv_PlayKit 基线中许可证不明的 Lua/GLSL 文件没有被复制进本仓库或构建产物。
详见 [LICENSE_PROVENANCE.md](LICENSE_PROVENANCE.md) 与
[docs/lua-disposition.md](docs/lua-disposition.md)。
