# 播放器内自动更新

`updater/helper/` 是一个用 Rust 编写的小型更新器，构建到安装目录根部的
`updater/`（与 `mpv.exe` 同级，`portable_config` 之外）：

```text
updater/playkit-updater.exe   (Windows)
updater/playkit-updater       (Linux)
```

它负责下载、校验、解压、应用用户 patch 和事务式替换安装目录。
`updater/update-check.lua` 只负责界面、检查 Release 和启动 helper，不直接
删除或覆盖任何播放器文件。

## 设计模型

每次更新都安装一个完整、干净、经 GitHub Actions 验证的新 PlayKit：

```text
新官方 PlayKit + 用户定制 → 新定制 PlayKit
```

不会在旧安装目录上做增量补丁，也不会把旧配置当作新版本的基础。更新器先把
新归档完整解压到同级 staging 目录，在新文件上重新应用 `user/` 层，全部校验
通过后才交换目录。

`enabled=no` 时 Lua 在读取配置后立即返回：不联网、不启动 HTTP 子进程、不读
Release、不启动 helper。`install_enabled=no` 可以只关闭安装，保留检查。

## 更新流程

1. Lua 已检测到一个比当前版本新的 Release，用户选择“下载并安装更新”。
2. helper 将自身复制到安装目录之外的临时目录并脱离播放器进程；原始 helper
   可以在交换时被替换，运行中的副本不受影响。
3. 下载 `manifest.json`、`SHA256SUMS` 和平台归档，全部通过 HTTPS 从固定的
   GitHub 仓库和 Release tag 下载；重定向只允许受信任的 GitHub 主机。
4. 校验 `SHA256SUMS` 中的归档 SHA-256，解析并校验发布 manifest：
   版本、tag、仓库、平台资产名、大小、`updater` 协议版本。
5. 在临时目录解压，拒绝绝对路径、`..`、超长路径、链接逃逸和不受支持的条目。
6. 校验包结构、`playkit-version.json`、包内 `manifest.json`、
   `playkit-inventory.json` 中每个文件的 SHA-256，以及 `.playkit/state.json`
   记录的官方基线。
7. 等待当前 mpv 退出（`--wait-pid`，默认最多 600 秒）。
8. 在 current 的同级 staging 目录组装新安装：复制新包、保留运行时状态、
   迁移 `user/` 子层、应用手动和自动 patch、复制用户脚本/字体/script-opts。
9. 再次确认 staging 完整后，用重命名完成交换：
   `current → current.previous`，`staging → current`。
   任一步失败会尝试恢复原目录。
10. 按 `--relaunch` 策略启动新版本（`auto` 只在原本由安装目录中的 mpv 运行时
    重启）。

任何下载、校验、解压、patch 或退出等待失败都会中止更新，current 保持不变。
失败原因写入 `.playkit/updater.log`，Lua 只显示简短 OSD，不显示 traceback。

## 事务目录

```text
<安装目录>/                          current
<安装目录>.previous/                 上一版本（最多保留一个）
.playkit-staging-<name>-<token>/     正在进行的新安装
.playkit-update-<name>.lock          防止两个 helper 同时更新
```

`.previous` 是 current 的同级目录，不是 `portable_config` 内部，因此交换不会
受卸载器的“保留用户数据”逻辑影响。更新成功后再执行一次更新时，旧
`.previous` 会被替换；磁盘上最多保留 current 和 previous 两份。

## 状态与所有权

安装状态位于 `portable_config/.playkit/`：

| 路径 | 内容 |
| --- | --- |
| `state.json` | 当前版本、平台、官方与生成文件的 SHA-256、已应用的 patch |
| `official/` | 本版本官方配置的只读基线 |
| `generated/` | 应用用户 patch 后最后一次写入磁盘的配置快照 |
| `update-status.json` | helper 写入的进度/结果（`schema`、`state`、`message`） |
| `updater.log` | 轮转日志，上限 1 MiB |

`portable_config/user/` 永远不被更新器整体删除。更新时保留并重建：

- `user/patches/`（手动 patch 与 `auto/` 捕获的 patch）
- `user/scripts/`（用户 Lua，复制到 `scripts/`）
- `user/fonts/`（复制到 `fonts/`）
- `user/script-opts/`（用户覆盖项，复制到 `script-opts/`）
- `user/backup/`（捕获/迁移前的备份）

运行时状态中保留 `saved-props.json`、`update-check-state.json`、
`watch_later/`、`_cache/watch_later/`、`_cache/command_history.txt`；
其余 `_cache/` 缓存丢弃。包内 `user/README.md` 等空目录占位文件不会被复制进
用户层覆盖用户已有文件。

## 命令行

```text
playkit-updater install  --root <安装目录> --repository <owner/repo> --tag <tag>
                          [--wait-pid <pid>] [--wait-timeout <秒>]
                          [--unmanaged capture|discard|abort]
                          [--relaunch auto|yes|no] [--detach] [--quiet]
playkit-updater capture  --root <安装目录> [--dry-run] [--quiet]
playkit-updater repair   --root <安装目录> [--assume-official] [--quiet]
playkit-updater rollback --root <安装目录> [--wait-pid <pid>]
                          [--relaunch auto|yes|no] [--detach] [--quiet]
playkit-updater verify   --root <安装目录>
playkit-updater status   --root <安装目录>
playkit-updater --version
```

- `install`：完整更新。`--source-dir` 只在 debug 构建中可用，用于本地测试；
  正式构建拒绝它，只允许从 GitHub HTTPS 下载。
- `capture`：把当前配置与上次生成快照的差异写成 `user/patches/auto/` 下的
  patch，并更新 `generated/` 哈希。不联网。
- `repair`：安装器复制完文件后重建 official/generated 状态并重新应用用户层，
  使新目录与自动更新后的结果一致。
- `rollback`：把 `.previous` 换回 current；没有 previous 时明确失败。
- `verify`：只检查本地状态和配置哈希，不修改文件。
- `status`：输出 `update-status.json` 的 JSON，供脚本或人工排查。

`--detach` 由 Lua 使用：helper 再复制自身到系统临时目录并启动真正的 worker，
初始进程立即返回，避免播放器退出时把 helper 一起结束。

## 用户未托管修改

直接编辑 `portable_config/mpv.conf` 等生成文件而不写 patch，称为“未托管修改”。
更新前会比较 current 与 `.playkit/generated/`：

| 策略 | 行为 |
| --- | --- |
| `capture`（默认） | 把差异转成 `user/patches/auto/<target>.patch`，然后继续更新 |
| `discard` | 丢弃差异（会记录警告），继续更新 |
| `abort` | 视为严重冲突，停止更新，current 不变 |

无法用 patch 语义表达的行（例如 input.conf 中无法识别的命令形式）也会中止
更新，而不是静默丢失。手动 patch 永远优先于自动 patch，并且在清理
`auto/` 后仍会保留。

## 失败与恢复

- 校验失败：staging 与临时下载全部删除；current、previous、user 层不变。
- 交换前失败：同上，Lua 显示“更新失败，当前安装未修改”。
- 交换中段失败：helper 尝试把 `.previous` 重命名回 current；若恢复也失败，
  日志会记录两个目录的位置，用户可手动重命名。
- 启动新版本失败：不会自动反向交换，但 `.previous` 保留，可通过菜单回滚。
- 回滚会把 `.previous` 换回 current，并删除刚才被替换的新版本目录；回滚是
  单向操作，不会保留新版本供再次切换。回滚后的安装没有 `.previous`，下一次
  成功更新才会重新生成。

## 安全边界

- 只接受 `https://github.com/<固定仓库>/releases/...` 及 GitHub 的发布资产主机；
  重定向逐跳校验，不允许跳转到任意主机。
- 下载、manifest、包内版本和文件清单使用 SHA-256 校验链；不执行 Release 中的
  任何脚本。
- 解压拒绝 zip-slip、tar 路径穿越、绝对路径、链接逃逸和异常条目。
- helper 来自当前已安装包（由 CI 构建并随包分发），或来自已通过校验的新包；
  不接受 Release 页面上任意可执行文件。
- 不拼接 shell 命令；所有外部 URL 和路径都通过结构化参数传递。
- 临时目录使用随机后缀，下载文件先写临时名再原子重命名。

## 构建与测试

```sh
python -m build.helper --platform windows-x64
python -m build.helper --platform linux-x64
```

`build.assemble` 通过 `--helper` 接收构建好的二进制并放进安装目录根部的
`updater/`；`build.verify` 校验其版本、大小、SHA-256 和 Linux 可执行位。CI 在
Ubuntu 与 Windows 上运行 `cargo test`、`cargo fmt --check` 和
`cargo clippy -D warnings`，然后为两个平台交叉构建（Windows MSVC 静态 CRT，
Linux musl 静态链接）。

## 本地手工验证

```sh
# 校验安装状态
updater/playkit-updater verify --root .

# 查看最近一次更新状态
updater/playkit-updater status --root .

# 只捕获当前直接编辑，不更新
updater/playkit-updater capture --root . --dry-run
```

在开发构建中可以使用 `install --source-dir <本地 release 目录>` 模拟完整更新，
目录中需要包含 `manifest.json`、`SHA256SUMS` 和平台归档；正式构建会拒绝该参数。
