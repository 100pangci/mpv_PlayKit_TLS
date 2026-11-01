# umpv 单实例启动器

包根目录的 `umpv.exe`（Windows）或 `umpv`（Linux）是本项目用 Rust 编写的
单实例启动器。它不会自己解码媒体：第一次启动时拉起随包的 mpv，之后的调用
把文件交给同一个 mpv 进程，因此双击多个文件只会出现一个播放器窗口。

它不是上游 mpv 自带的 Python `umpv`，也不依赖 Python 运行时；源码位于
`updater/helper/src/umpv.rs`，与更新器一起由 `build.helper` 构建。

## 行为

1. 读取 `umpv.conf`（程序目录，其次 `portable_config/umpv.conf`，最后当前
   目录）。
2. 尝试连接配置中的 IPC 端点。
3. 连接成功：按 `Loadfile_Flag` 发送 `loadfile` 命令。多个文件时，第一个
   使用配置的 flag；若为 `replace`，后续文件改用 `append`，避免互相替换。
4. 端点不存在：清理 Linux 上的陈旧 socket，然后启动 mpv：
   `--input-ipc-server=<端点> --idle=yes --force-window=yes -- <文件...>`。
   启动后 umpv 立即退出，播放器在后台运行。
5. 端点存在但拒绝连接或其他错误：报告错误，不启动第二个播放器。

Windows 上首次启动会用 `CREATE_NO_WINDOW` 拉起控制台子系统的 `mpv.exe`，
避免弹出额外终端窗口；Linux 上 mpv 放入独立进程组，关闭终端不会带走播放器。

## 配置

`umpv.conf` 与二进制同级，格式为简单的 `键=值`，`#`/`;` 开头为注释：

```ini
Socket_Path=\\.\pipe\umpv
Loadfile_Flag=replace
```

- `Socket_Path`：Windows 的命名管道（短名称 `umpv` 会补全为
  `\\.\pipe\umpv`），或 Linux 的 Unix domain socket 绝对路径。
  Linux 留空时使用 `$XDG_RUNTIME_DIR/umpv.sock`；没有该变量时使用
  `/tmp/umpv-<uid>.sock`。`~/` 会展开到用户主目录。
- `Loadfile_Flag`：`replace`（默认）、`append` 或 `append-play`。

命令行可临时覆盖配置：

```text
umpv [--config <文件>] [--socket <名称>] [--flag replace|append|append-play] [--] <文件...>
```

`umpv --version` 与 `umpv --help` 输出后立即退出。`--` 之后的参数一律视为
文件名，包括以 `-` 开头的路径。

## Windows 文件关联

`installer\windows\umpv-register.cmd` 把 `umpv.exe` 注册为当前用户（仅
`HKCU`）的“打开方式”候选，使用专属 ProgID `mpv-playkit.umpv`。它不会覆盖
主安装器的 `mpv-playkit` 关联，也不会修改系统默认程序；用户仍需在
“默认应用”中自行选择。扩展名列表读取 `portable_config\script-opts\uosc.conf`
的 `*_types`，读取失败时回退到常见音视频扩展名。

```bat
installer\windows\umpv-register.cmd
installer\windows\umpv-unregister.cmd
```

卸载主安装器时，`uninstall.ps1` 也会清理 umpv 的 ProgID、`Applications\umpv.exe`、
App Paths、Capabilities、RegisteredApplications 和逐扩展名 `OpenWithProgids`。

## Linux

包内没有单独的桌面注册脚本。可以让 `umpv` 成为文件管理器的自定义打开命令，
或者：

```sh
./umpv video.mkv
```

若要让桌面环境的“打开方式”使用它，需要自行创建 `.desktop` 文件；本项目
暂不写入系统目录。AppImage 运行时的 `mpv-playkit` 启动器会被优先使用，因此
`portable_config` 仍然生效。

## 构建与校验

`python -m build.helper --platform <platform>` 同时产出更新器和 umpv：

- Windows：`dist/updater/windows-x64/umpv.exe`（GUI 子系统，静态 CRT）
- Linux：`dist/updater/linux-x64/umpv`（musl 静态链接）

`build.assemble` 把二进制放到包根目录，并把 `config/umpv/<os>.conf` 复制为
包根 `umpv.conf`。`manifest.json` / `playkit-version.json` 记录 `umpv` 的
版本、大小和 SHA-256；`build.verify` 校验这些字段以及 Linux 可执行位，
`build.smoke` 在所有平台上运行 `umpv --version`。

## 安全边界

mpv 的 JSON IPC 没有额外的身份验证，能连接该端点的本机进程就可以发送播放
命令。请把 socket/管道端点视为当前用户权限范围内的接口：

- Linux 的默认 socket 位于 `$XDG_RUNTIME_DIR`（通常是仅当前用户可访问的
  目录）；使用 `/tmp` 回退时文件名包含 uid，但 `/tmp` 本身是共享目录。
- Windows 命名管道的访问控制由 mpv 创建管道时的默认安全描述符决定。
- umpv 只发送 `loadfile` 命令，不读取 IPC 的响应，也不执行配置文件中的
  任意命令。`umpv.conf` 只是数据，不是脚本。

如果本机存在不可信用户或恶意进程，应改用仅自己可读写的运行目录，或不要
启用文件关联。