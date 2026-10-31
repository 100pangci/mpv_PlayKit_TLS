# 播放器内更新检测器

`updater/update-check.lua` 只检测 GitHub 上是否存在更新的 mpv PlayKit Release，
并通过 OSD 和 uosc 菜单提示用户。它不会下载、解压或覆盖播放器文件。

构建时脚本被安装为：

```text
portable_config/scripts/update-check.lua
portable_config/script-opts/update-check.conf
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
```

可选项：

| 键 | 默认 | 说明 |
| --- | --- | --- |
| `enabled` | `yes` | `no`/`false`/`0` 时脚本在读取配置后立即返回 |
| `repository` | `100pangci/mpv_PlayKit_TLS` | 必须是 `owner/name` |
| `check_interval_hours` | `24` | 两次自动检查的最小间隔 |
| `timeout_seconds` | `12` | curl/wget 超时 |
| `snooze_hours` | `24` | “稍后提醒”的默认时长 |

## 联网门禁

脚本按以下顺序执行：

1. `mp.options.read_options()` 读取本地配置。
2. 若 `enabled` 为 `no`/`false`/`0`，打印
   `update-check is disabled by configuration; no network request will be made`
   并 `return`。
3. 只有通过门禁后才展开路径、读取 `playkit-version.json` 与
   `update-check-state.json`。
4. 若状态文件中 `disabled=true`，同样直接返回。
5. 只有以上条件都满足，才在启动 4 秒后或用户手动触发时调用 curl/wget。

因此 `enabled=no` 时不会创建或修改状态文件，也不会读取版本文件之外的路径，
更不会发起任何网络请求。`build/verify.py` 会静态检查门禁字符串出现在
`name = "subprocess"` 之前。

## 状态文件

`portable_config/update-check-state.json` 由脚本在需要时写入：

```json
{
  "schema": 1,
  "last_check": 0,
  "latest_version": "",
  "latest_url": "",
  "snoozed_until": 0,
  "dismissed_version": "",
  "disabled": false
}
```

- 写入采用临时文件 + 替换；Windows 上先删除目标再重命名。
- 删除该文件会清除“已关闭/已推迟”状态；菜单中的“恢复更新检查”也会重置并立即
  检查一次。

## 用户操作

`input.conf` 中的绑定：

| 操作 | 绑定 |
| --- | --- |
| 立即检查 | `Ctrl+u` |
| 打开 Release 页面 | uosc 菜单“更新 > 打开 Release 页面” |
| 稍后提醒 | uosc 菜单“更新 > 稍后提醒” |
| 关闭更新检查 | uosc 菜单“更新 > 关闭更新检查” |
| 恢复更新检查 | uosc 菜单“更新 > 恢复更新检查” |

打开链接时 Windows 使用 `explorer.exe`，Linux 使用 `xdg-open`。只接受
`https://github.com/` 开头的 URL。尚未成功检测到新版本时，“打开 Release 页面”
会回退到 `https://github.com/<repository>/releases/latest`。

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
   made`，并且没有 curl/wget 子进程。

2. 启用检查（会访问 GitHub API）：

   ```sh
   mpv --config-dir=portable_config --msg-level=all=info
   ```

   首次启动 4 秒后检查一次。若本地版本是 `0.0.0-dev` 或没有
   `playkit-version.json`，脚本会跳过比较。

3. 模拟新版本：把 `portable_config/playkit-version.json` 的
   `playkit_version` 改成旧版本（如 `2026.01`），启动后应出现 OSD 提示。

## 安全边界

- 不执行下载到的任何内容；只解析 GitHub Release JSON。
- 不把 Release 页面 URL 之外的地址传给浏览器。
- 输出长度限制为 256 KiB，避免异常响应占用内存。
- 检测失败只记录警告，不影响播放。
