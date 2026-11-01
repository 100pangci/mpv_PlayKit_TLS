# 上游来源与更新规则

## 组件表

| id | 上游 | Release | 资产匹配 | 版本模板 | 安装结果 |
| --- | --- | --- | --- | --- | --- |
| `mpv-windows` | `shinchiro/mpv-winbuild-cmake` | latest | `^mpv-x86_64-(?P<date>\d{8})-git-(?P<commit>[0-9a-f]+)\.7z$` | `{date}-git-{commit}` | `mpv.exe`, `mpv.com`, `d3dcompiler_43.dll` |
| `mpv-linux` | `pkgforge-dev/mpv-AppImage` | tag `nightly` | `^mpv-(?P<version>v[0-9][^-]*(?:-\d+-g(?P<commit>[0-9a-f]+))?)-anylinux-x86_64\.AppImage$` | `{version}` | `mpv/`（AppDir） |
| `yt-dlp-windows` | `yt-dlp/yt-dlp` | latest | `^yt-dlp\.exe$` | `{release_tag}` | `yt-dlp.exe` |
| `yt-dlp-linux` | `yt-dlp/yt-dlp` | latest | `^yt-dlp_linux$` | `{release_tag}` | `yt-dlp` |
| `uosc` | `tomasklaen/uosc` | latest | `^uosc\.zip$` | `{release_tag}` | `portable_config/scripts/uosc/`, `portable_config/fonts/` |
| `lxgw-wenkai-mono-lite` | `lxgw/LxgwWenKai-Lite` | latest | `^LXGWWenKaiMonoLite-Regular\.ttf$` | `{release_tag}` | `portable_config/fonts/` |
| `anime4k-restore` | `bloc97/Anime4K` | raw `v4.0.1` | 固定文件路径 + SHA-256 | `v4.0.1` | `portable_config/shaders/Anime4K/` |
| `anime4k-upscale-gan` | `bloc97/Anime4K` | raw commit `8e39551` | 固定文件路径 + SHA-256 | `v4.0-GAN-8e39551` | `portable_config/shaders/Anime4K/` |
| `fsrcnnx` | `igv/FSRCNN-TensorFlow` | tag `1.1` | `^FSRCNNX_x2_16-0-4-1\.glsl$` | `1.1` | `portable_config/shaders/FSRCNNX/` |
| `nnedi3` | `bjin/mpv-prescalers` | raw commit `b3f0a59` | 固定文件路径 + SHA-256 | `2024-01-11` | `portable_config/shaders/EDI/` |
| `k7-modernanime` | `hooke007/mpv_PlayKit` | raw commit `4921c67` | 固定文件路径 + SHA-256 | `2026-05-30` | `portable_config/shaders/ESRGAN/` |
| `adaptive-sharpen-lite` | `hooke007/mpv_PlayKit` | raw commit `4921c67` | 固定文件路径 + SHA-256 | `2026-05-30` | `portable_config/shaders/Adaptive_sharpen/` |

所有 Release 资产匹配都要求 **恰好一个** 资产。零个或多个都会让解析失败并停止发布。

## Release 选择

- `mode: latest`：`GET /repos/{repo}/releases/latest`，不会把 prerelease 当作最新。
- `mode: tag`：`GET /repos/{repo}/releases/tags/{tag}`。Linux mpv 固定跟随
  `nightly`，因为 pkgforge 的 nightly 资产包含当前 mpv master 构建。
- 若上游改变资产命名，解析会失败而不是悄悄使用旧文件；需要更新
  `manifests/*.yml` 的正则后重新生成 lock。

## 原始文件（GLSL）

`kind: raw-files` 用于没有 Release 资产、但需要固定到某个上游提交的文件：

```yaml
- id: anime4k-restore
  kind: raw-files
  upstream: bloc97/Anime4K
  ref: v4.0.1
  files:
    - source: glsl/Restore/Anime4K_Restore_CNN_L.glsl
      target: portable_config/shaders/Anime4K/Anime4K_Restore_CNN_L.glsl
      sha256: <64 位小写十六进制>
      license: MIT
```

`build.resolve` 把 `ref` 解析成完整 commit，下载 `source` 并核对声明的 SHA-256；
lock 只保存完整 commit、`raw.githubusercontent.com/<repo>/<commit>/<source>` 和
哈希。`build.fetch` 再下载一次并校验，`build.assemble` 按 `target` 安装。
仓库不保存 shader 文件本身。

`manifests/shaders.yml` 同时列出排除项：Ani4Kv2/AniSD 的模型权重是
CC BY-NC 4.0，QCOM 的来源与许可证无法确认。

## 哈希与 lock

`build.resolve` 解析所有组件并写入候选 `sources.lock.json`：

```json
{
  "schema": 1,
  "generated_at": "2026-10-31T00:00:00Z",
  "repository": "100pangci/mpv_PlayKit_TLS",
  "components": {
    "mpv-windows": {
      "release_tag": "20261021",
      "version": "20261021-git-e471496c56",
      "commit": "e471496c56f0f49c133bd88b50aed62b845f1909",
      "provides": "mpv",
      "asset_name": "mpv-x86_64-20261021-git-e471496c56.7z",
      "url": "https://github.com/...",
      "sha256": "...",
      "license": {
        "spdx": "GPL-2.0-or-later",
        "files": [
          {
            "path": "LICENSE.GPL",
            "license": "GPL-2.0-or-later",
            "repository": "mpv-player/mpv",
            "commit": "e471496c56f0f49c133bd88b50aed62b845f1909",
            "source_path": "LICENSE.GPL",
            "url": "https://raw.githubusercontent.com/mpv-player/mpv/e471496c56f0f49c133bd88b50aed62b845f1909/LICENSE.GPL",
            "sha256": "...",
            "size": 18092,
            "scope": "mpv Windows build as a whole"
          }
        ]
      }
    }
  }
}
```

- GitHub Release API 提供 `digest` 时直接使用；缺失时下载资产计算 SHA-256。
- 资产名只带 mpv 短 commit 时，`asset.commit_repository` 指定的仓库会把短哈希解析
  成完整 commit；`commit` 字段始终记录完整 40 位 SHA。
- `provides: mpv` 标记该平台的播放器组件；发布 manifest 据此写入每个平台各自的
  `mpv_version` / `mpv_commit`，不会把 Windows 与 Linux 的 mpv 混为一个版本。
- `build.fetch` 再校验一次资产与许可证的 SHA-256；不匹配立即失败并删除临时文件。
- `lock_hash` 只哈希 `components`，忽略 `generated_at`，因此“仅时间变化”不会
  触发发布或提交。
- 月度工作流只在完整双平台构建成功后把新 lock 提交到 main；判断同样基于忽略
  `generated_at` 的内容哈希。

## 许可证固定

`manifests/*.yml` 中的许可证文件不再写死 URL，而是声明：

```yaml
- path: MIT-fzy.txt              # 包内 licenses/<component>/ 下的文件名
  license: MIT
  repository: swarn/fzy-lua
  ref: main                      # 解析输入：可以是 tag、分支或短 commit
  source_path: LICENSE           # 仓库内的真实路径，默认等于 path
```

`build.resolve` 会：

1. 用 GitHub Commits API 把 `ref` 解析成完整 commit；
2. 生成 `https://raw.githubusercontent.com/<repository>/<commit>/<source_path>`；
3. 下载许可证文本并记录 `sha256`、`size` 和对应 `scope`。

lock 与包内 `licenses/INDEX.json` 因此只包含完整 commit 和固定 URL，不会出现
`main`、`master` 或 `latest`。`build.fetch` 会校验许可证 SHA-256；
`build.verify` 会拒绝缺少完整 commit、SHA-256 或 URL 中不含该 commit 的许可证
条目。上游许可证无法下载或哈希不符时，构建失败而不是继续发布。

## API 认证与访问

`build.upstream` 通过 `GITHUB_TOKEN` 或 `GH_TOKEN` 认证。GitHub Actions 中
`monthly-update.yml` 把 `secrets.GITHUB_TOKEN` 注入环境；本地运行时如遇速率限制，
可设置 `GITHUB_TOKEN`。下载 Release 资产使用 API URL + `Accept:
application/octet-stream`，公开仓库和私有仓库都能工作。仓库公开后匿名请求也可
读取 Release，但构建仍使用 token 以避免 API 速率限制。

## uosc 与缩略图

- uosc 从上游 `uosc.zip` 获取。当前锁定版本为 5.13.0，其控件名使用
  `play-pause`，缩略图消息固定发给名为 `thumbfast` 的脚本。
- 构建阶段 `build/patch_uosc.py` 只修改
  `portable_config/scripts/uosc/main.lua`，加入 `scale=0` 自动 UI 缩放并观察
  显示器尺寸变化；补丁带有明确修改说明，上游锚点变化时构建失败。其余 uosc
  文件保持上游版本。
- 构建时只保留当前平台的 `scripts/uosc/bin/ziggy-*`，删除其他平台 helper。
- uosc 菜单通过 `input.conf` 末尾带 `#!` 的注释配置；这项菜单定制本身不需要
  修改 uosc 源码。
- 缩略图由本仓库原创的 `scripts/local/thumbfast.lua` 实现：它按 uosc 的公开消息
  协议响应 `thumbfast-info`、`thumb`、`clear`，用随包 mpv 子进程解码单帧，不依赖
  `thumb_engine/` 或第三方 thumbfast。

## 新增或替换上游

1. 在对应 `manifests/*.yml` 增加组件：`upstream`、`release`、`asset.pattern`、
   `asset.version`、`license`、`extract`、`install`；shader 使用 `raw-files` 的
   `ref`/`files` 结构。
2. 确认资产在目标 Release 中唯一匹配；raw 文件填写实际 SHA-256。
3. 运行 `python -m build.resolve --output sources.lock.json`。只更新部分组件时
   使用 `--only <id>`（可重复），它会保留 lock 中其他组件的现有版本。
4. 运行本地 fetch/assemble/smoke/package/verify，或直接跑 `Build` 工作流。
5. 提交 manifest 与新的 lock。

不要手工编辑 lock 中的版本或哈希；让解析器生成。
