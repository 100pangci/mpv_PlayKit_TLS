# 播放器内更新检测器

`updater/update-check.lua` 检测 GitHub 上是否存在更新的 mpv PlayKit Release，
通过 OSD 和 uosc 菜单提示用户，并在用户确认后启动随包的 Rust 更新器
（安装目录根部的 `updater/playkit-updater`）。Lua 脚本本身不下载、不解压、
不覆盖播放器文件；实际替换由 helper 完成。helper 的事务、校验链和回滚见
[自动更新文档](updater.md)。

构建时安装为：

```text
portable_config/scripts/update-check.lua
portable_config/script-opts/update-check.conf
updater/playkit-updater[.exe]          （与 mpv.exe 同级）
```

mpv 自动加载 `portable_config/scripts/` 下的脚本；不要为它添加显式 `script=`
配置，否则会重复加载并可能导致 uosc 的相对 `require()` 失败。

## 配置

`config/common/script-opts/update-check.conf`：

```ini
enabled=yes
repository=100pangci/mpv_PlayKit_TLS
check_interval_hours=24
timeout_seconds=12
snooze_hours=24
install_enabled=yes
unmanaged=capture
relaunch=auto
wait_timeout=600
```

| 键 | 默认 | 说明 |
| --- | --- | --- |
| `enabled` | `yes` | `no`/`false`/`0` 时脚本在读取配置后立即返回 |
| `repository` | `100pangci/mpv_PlayKit_TLS` | 必须是 `owner/name` |
| `check_interval_hours` | `24` | 两次自动检查的最小间隔 |
| `timeout_seconds` | `12` | curl/wget 超时 |
| `snooze_hours` | `24` | “稍后提醒”的默认时长 |
| `install_enabled` | `yes` | `no` 时隐藏/拒绝播放器内安装，但仍可检查 |
| `unmanaged` | `capture` | 直接编辑生成配置时的处理：`capture`、`discard` 或 `abort` |
| `relaunch` | `auto` | 更新后重启：`auto`、`yes` 或 `no` |
| `wait_timeout` | `600` | 等待 mpv 退出的最长秒数（1–3600） |

`relaunch=auto` 只在 helper 能确认 mpv 是从当前安装目录启动时重启；`yes`
强制重启，`no` 只更新不重启。

## 联网门禁

脚本按以下顺序执行：

1. `mp.options.read_options()` 读取本地配置。
2. 若 `enabled` 为 `no`/`false`/`0`，打印
   `update-check is disabled by configuration; no network request will be made`
   并 `return`。
3. 只有通过门禁后才展开路径、读取 `playkit-version.json` 与
   `update-check-state.json`。
4. 若状态文件中 `disabled=true`，不注册启动定时器，自动检查不会发生；脚本仍
   保持加载，菜单中的“恢复更新检查”可用。`run_check()` 自身也会在
   `state.disabled` 为真时立即返回。
5. 若 `playkit-version.json` 缺失或版本号不是发布版本，同样只跳过自动检查。
6. 只有以上条件都满足，才在启动 4 秒后或用户手动触发时调用 curl/wget。

因此 `enabled=no` 时不会创建或修改状态文件，也不会读取版本文件之外的路径，
更不会发起任何网络请求。`install_enabled=no` 是在此门禁之后、启动 helper 之前
检查的：检查仍可进行，但不会下载或替换任何文件。`build/verify.py` 会静态检查
门禁字符串出现在 `name = "subprocess"` 之前。

检查本身不强制后台安装：只有自动检查是定时的，安装、捕获和回滚都必须由用户
在菜单中选择。

## 状态文件

`portable_config/update-check-state.json` 由脚本在需要时写入：

```json
{
  "schema": 1,
  "last_check": 0,
  "latest_version": "",
  "latest_tag": "",
  "latest_url": "",
  "snoozed_until": 0,
  "dismissed_version": "",
  "disabled": false
}
```

- 写入采用临时文件 + 替换；先尝试原子重命名，Windows 上目标已存在时再删除
  目标并重试。
- 删除该文件会清除“已关闭/已推迟”状态；菜单中的“恢复更新检查”也会重置
  `disabled`、清除推迟并立即检查一次（即使当前会话处于关闭状态）。
- `latest_tag` 是 Release 的 tag，安装时传给 helper；不在客户端拼接下载 URL。

## 用户操作

`input.conf` 中的绑定（uosc 菜单项；直接按键仍为 `Ctrl+u` 检查）：

| 操作 | 绑定 |
| --- | --- |
| 立即检查 | `Ctrl+u` |
| 下载并安装更新 | uosc 菜单“更新 > 下载并安装更新” |
| 保存当前配置修改 | uosc 菜单“更新 > 保存当前配置修改” |
| 回滚到上一版本 | uosc 菜单“更新 > 回滚到上一版本” |
| 打开 Release 页面 | uosc 菜单“更新 > 打开 Release 页面” |
| 稍后提醒 | uosc 菜单“更新 > 稍后提醒” |
| 关闭更新检查 | uosc 菜单“更新 > 关闭更新检查” |
| 恢复更新检查 | uosc 菜单“更新 > 恢复更新检查” |

安装前会再次检查：helper 存在、当前版本可解析、已知的新版本比当前版本严格
更新、tag 合法。任一条件不满足只显示 OSD，不启动子进程。

打开链接时 Windows 使用 `explorer.exe`，Linux 使用 `xdg-open`。只接受
`https://github.com/` 开头的 URL。尚未成功检测到新版本时，“打开 Release 页面”
会回退到 `https://github.com/<repository>/releases/latest`。

## 安装、进度与失败

选择安装后，Lua 删除旧的 `update-status.json`，每秒读取一次 helper 写入的
状态文件，并把 `message` 显示在 OSD 上（下载、校验、准备、等待退出等）。
`installed`、`rolled-back`、`repaired`、`captured` 为终态并停止轮询。

失败时 OSD 只显示简短原因，并提示：

```text
详情见 portable_config/.playkit/updater.log
```

完整错误、校验失败的字段和路径只写入日志与状态文件的 `detail`，不会把 Rust
错误直接弹给用户。helper 无法启动时同样只给出一行提示和日志位置。

`state` 的取值包括 `starting`、`downloading`、`verifying`、`ready`、`waiting`、
`staging`、`installing`、`installed`、`capturing`、`captured`、`repairing`、
`repaired`、`rolling-back`、`rolled-back`、`blocked`、`failed`。

## 仓库可见性

GitHub 的 Release API 对匿名请求只返回公开仓库的数据。本仓库公开时，检测器可
直接读取最新 Release。若以后改为私有仓库，匿名检测会得到 404，脚本只在日志中
记录 `update-check: no working HTTP client (curl or wget) or request failed`，
不会弹出错误提示，也不影响播放；每 24 小时会按间隔重试。菜单中的“打开 Release
页面”仍会打开 `releases/latest`，由浏览器使用用户自己的登录状态访问。脚本不会
内置任何访问令牌，避免把凭据分发到每个安装包。

## 手动验证

1. 完全关闭联网检查：

   ```sh
   mpv --config-dir=portable_config \
       --script-opts-append=update-check-enabled=no \
       --msg-level=all=info
   ```

   应看到 `update-check is disabled by configuration; no network request will be
   made`，并且没有 curl/wget 子进程，也没有 updater helper 进程。

2. 启用检查（会访问 GitHub API）：

   ```sh
   mpv --config-dir=portable_config --msg-level=all=info
   ```

   首次启动 4 秒后检查一次。若本地版本是 `0.0.0-dev` 或没有
   `playkit-version.json`，脚本会跳过比较。

3. 模拟新版本：把 `portable_config/playkit-version.json` 的
   `playkit_version` 改成旧版本（如 `2026.01`），启动后应出现 OSD 提示。
   选择“下载并安装更新”后观察 `update-status.json` 和 `updater.log`。

4. 只关闭安装：

   ```sh
   mpv --config-dir=portable_config \
       --script-opts-append=update-check-install_enabled=no
   ```

   检查仍会进行，但安装菜单项只给出“已关闭”提示，不启动 helper。

## 安全边界

- `enabled=no` 时完全不联网、不启动子进程。
- 不执行下载到的任何内容；检测阶段只解析 GitHub Release JSON。
- 安装只把结构化参数（安装目录、仓库、tag、PID）传给 helper，不拼接 shell。
- 不把 Release 页面 URL 之外的地址传给浏览器。
- 输出长度限制为 256 KiB，避免异常响应占用内存。
- 检测失败只记录警告，不影响播放；安装失败不影响当前安装。
