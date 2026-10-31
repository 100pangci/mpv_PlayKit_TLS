# 基线审计摘要

本文件记录为什么仓库以“重新实现 + 构建时获取上游”的方式重建，而不是直接
再分发历史 mpv_PlayKit 便携包。

## 基线

- 文件：`mpv-lazy-20260510.exe`
- SHA-256：`f03e288d5c41b00604a64d30bb81e5f76e7f125c0447926ff39561b32d44fef4`
- 大小：132,331,618 字节
- 审计日期：2026-10-29

## 许可证结论

固定提交 `hooke007/mpv_PlayKit@4921c6796620a13c2c03266f368d59d354a67b72`
的 `LICENSE.MD`：

- 未列出的文件默认 **UNLICENSED**。
- 便携脚本和 shader 通常把条款写在文件内。
- 该策略不授予复制文件的再分发权；缺失许可证不等于获得许可。

因此不能把基线包整体再分发，也不能把其中许可证不明的 Lua/GLSL 直接复制进
新仓库。

## Lua 审计结果

- 34 个 Lua 文件中，25 个属于 uosc：16 个与上游逐字节一致，9 个被修改。
- uosc 仓库许可证为 LGPL-2.1；`uosc/lib/fzy.lua` 含 MIT 声明；
  `uosc/lib/utils.lua` 的 thumbfast FFI 片段标注 MPL-2.0。
- 9 个文件无法在固定 PlayKit 策略下解决许可证，或无法映射到已审计的 uosc 提交。
  处置见 [lua-disposition.md](lua-disposition.md)。

## Shader 审计结果

基线目录含 429 个 GLSL 文件：

- 138 个按固定策略无许可证；
- 79 个有许可证引用但未对修改后的作品逐项核实；
- 31 个有继承条款但未核实；
- 181 个有内嵌条款但范围仍需审查。

部分文件同时包含代码许可证（如 MIT）和模型条款（如 CC BY-NC 4.0），不能合并
成单一仓库许可证。本发行版完全不包含 shader，因此不涉及这些文件。

## 本仓库的处理

| 基线内容 | 处理 |
| --- | --- |
| 基线中未修改的 uosc 文件 | 改为构建时从上游下载当前 Release（仅 `main.lua` 应用自动 UI 缩放补丁） |
| 基线中修改过的 uosc | 不导入；本项目只通过配置层、构建期 `main.lua` 补丁和缩略图消息协议定制 |
| 9 个未解决 Lua | 全部由原创实现或官方上游替代：`idle-brand`、`input_plus`、`save_global_props`、`thumbfast` 重写；`lang` 使用 uosc 官方 `intl.lua` |
| shader / VapourSynth / AI 组件 | 不导入 |
| 基线二进制 | 不导入；改为从各上游 Release 获取并校验 |
| 配置思路 | 作为需求参考，重新编写 |

## 结果

- 仓库中不包含历史 PlayKit 的未授权文件。
- 每个第三方组件都能在 `THIRD_PARTY.md`、`sources.lock.json` 和包内
  `licenses/INDEX.json` 中追溯。
- `build/verify.py` 对已知的高风险路径做硬性拒绝，避免后续维护时误加。
