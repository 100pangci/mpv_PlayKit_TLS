# mpv PlayKit 安装器

本目录包含可选安装脚本。播放器本身是便携的：解压后直接运行 `mpv.exe`
（Windows）或 `./mpv-playkit`（Linux）即可，不安装也能使用。

所有安装器代码均为本项目原创，按 GPL-3.0-or-later 分发。

## Windows

双击 `windows\install.cmd` 打开图形界面。安装程序只写入当前用户
（`HKCU`），不需要管理员权限，也不会修改系统级 `HKLM` 或文件默认关联。

默认安装位置：`%LOCALAPPDATA%\Programs\mpv-playkit`。

图形界面可选项：

- 创建桌面快捷方式
- 创建开始菜单快捷方式（含“卸载 mpv PlayKit”）
- 添加“用 mpv PlayKit 播放”右键菜单（文件与文件夹）
- 在“打开方式”中注册常见音视频格式

静默安装：

```bat
installer\windows\install.cmd -Silent
installer\windows\install.cmd -Silent -InstallDir "D:\Apps\mpv-playkit" -NoDesktopShortcut
installer\windows\install.cmd -Silent -NoContextMenu -NoFileAssociations
```

可用参数：`-Silent`、`-InstallDir <目录>`、`-NoDesktopShortcut`、
`-NoStartMenuShortcut`、`-NoContextMenu`、`-NoFileAssociations`、`-Launch`。

卸载：开始菜单中的“卸载 mpv PlayKit”，或

```bat
installer\windows\uninstall.cmd
installer\windows\uninstall.cmd -Silent [-KeepUserData]
```

卸载会删除快捷方式、`HKCU` 下的 ProgID / “打开方式” / 右键菜单 / 卸载项，
以及安装目录。`-KeepUserData` 只保留 `portable_config`（配置与个人数据）。

实现说明：

- 安装时把当前包复制到安装目录；运行过播放器产生的
  `portable_config\_cache`、`saved-props.json`、`update-check-state.json`
  以及 `.playkit\update-status.json` / `.playkit\updater.log` 不会复制。
- 覆盖安装前，如果目标已有 `playkit-version.json`，安装器会先调用旧安装目录中的
  `updater\playkit-updater.exe capture`，把用户对生成配置的直接
  编辑保存为 `portable_config\user\patches\auto\` 下的 patch；捕获失败会中止
  安装，不会静默覆盖配置。旧版本没有 helper 时只记录警告，并保留
  `portable_config\user`。
- 复制完成后调用新包的 `repair`，以新官方配置重建 `.playkit\official` 与
  `.playkit\generated`，重新应用手动/自动 patch 和用户脚本、字体、script-opts。
  因此安装器覆盖安装与播放器内自动更新的结果一致。
- 升级时重新运行安装程序即可覆盖程序文件；`portable_config\user` 与用户数据
  不会被删除。安装目录中额外的用户文件不会被删除。
- 注册内容对应原版 `mpv.com --register` / `umpv-install.bat` 的用户级部分：
  ProgID（默认动词 `play`）、`Applications\mpv.exe`、逐扩展名
  `OpenWithProgids`、`SupportedTypes`、`SystemFileAssociations\video/audio`
  的“打开方式”列表、App Paths（`mpv.exe` / `mpv.com`）、Default Programs
  的 Capabilities 与 `RegisteredApplications`、开始菜单快捷方式，以及
  “应用和功能”中的卸载项。
- 与原版安装脚本的**有意差异**：只写当前用户 `HKCU`，不需要管理员权限，
  不写 `HKLM`；不注册 `mpv://` URL 协议、DVD/蓝光/CD 自动播放处理程序，
  也不修改系统的 `LongPathsEnabled`。这些系统级改动容易影响其他播放器，
  需要时请自行用原版 `mpv.com --register` 注册。
- “打开方式”注册使用逐扩展名的 `OpenWithProgids`，不抢占系统默认程序；
  Windows 10/11 的默认关联由用户在系统设置中选择。Default Programs 项会
  让 mpv PlayKit 出现在“默认应用”列表中，便于手动选择。
- 与原版 `mpv.com --register` 共用同名的 `Applications\mpv.exe` 和
  `App Paths\mpv.exe`（可执行文件都叫 `mpv.exe`）：后安装者会覆盖先安装者
  的用户级注册，卸载会移除本安装写入的这些项。ProgID 使用独立的
  `mpv-playkit`，不会覆盖原版的 `io.mpv.*` ProgID。
- 文件关联的扩展名列表从 `portable_config\script-opts\uosc.conf` 的
  `video_types` / `audio_types` / `image_types` / `subtitle_types` 读取。

## Linux（预留接口）

Linux 安装器尚未实现。`linux\install.sh` 只定义了参数与默认值，运行时会说明
接口并退出，不会修改系统。当前请直接使用包根目录的 `./mpv-playkit`。

预留文件：

- `linux/install.sh`：参数骨架（`--prefix`、`--no-desktop`、`--uninstall`）
- `linux/mpv-playkit.desktop.in`：桌面菜单项模板（`@EXEC@`、`@ICON@` 占位符）
- `linux/mpv-playkit.svg`：图标

TODO（未来实现时）：

1. 将包复制到 `$XDG_DATA_HOME/mpv-playkit`（或 `--prefix` 指定目录）。
2. 由 `mpv-playkit.desktop.in` 生成
   `$XDG_DATA_HOME/applications/mpv-playkit.desktop`，替换 `@EXEC@`、`@ICON@`。
3. 安装图标到 `$XDG_DATA_HOME/icons/hicolor/scalable/apps/`。
4. `--uninstall` 删除上述文件，不触碰用户自己的 `portable_config` 修改。
