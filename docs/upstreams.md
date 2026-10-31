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

所有匹配都要求 **恰好一个** 资产。零个或多个都会让解析失败并停止发布。

## Release 选择

- `mode: latest`：`GET /repos/{repo}/releases/latest`，不会把 prerelease 当作最新。
- `mode: tag`：`GET /repos/{repo}/releases/tags/{tag}`。Linux mpv 固定跟随
  `nightly`，因为 pkgforge 的 nightly 资产包含当前 mpv master 构建。
- 若上游改变资产命名，解析会失败而不是悄悄使用旧文件；需要更新
  `manifests/*.yml` 的正则后重新生成 lock。

## 哈希与 lock

`build.resolve` 解析所有组件并写入 `sources.lock.json`：

```json
{
  "schema": 1,
  "generated_at": "2026-10-31T00:00:00Z",
  "repository": "100pangci/mpv_PlayKit_TLS",
  "components": {
    "mpv-windows": {
      "release_tag": "...",
      "version": "20261021-git-e471496c56",
      "asset_name": "mpv-x86_64-20261021-git-e471496c56.7z",
      "url": "https://github.com/...",
      "sha256": "...",
      "license": { "spdx": "GPL-2.0-or-later", "files": [...] }
    }
  }
}
```

- GitHub Release API 提供 `digest` 时直接使用；缺失时下载资产计算 SHA-256。
- `build.fetch` 再校验一次；不匹配立即失败并删除临时文件。
- `lock_hash` 只哈希 `components`，忽略 `generated_at`，因此“仅时间变化”不会
  触发发布或提交。
- 发布工作流只在 `lock_changed=true` 时把新 lock 提交回仓库；这个判断同样基于
  忽略 `generated_at` 的内容哈希。

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
   `asset.version`、`license`、`extract`、`install`。
2. 确认资产在目标 Release 中唯一匹配。
3. 运行 `python -m build.resolve --output sources.lock.json`。
4. 运行本地 fetch/assemble/smoke/package/verify，或直接跑 `Build` 工作流。
5. 提交 manifest 与新的 lock。

不要手工编辑 lock 中的版本或哈希；让解析器生成。
