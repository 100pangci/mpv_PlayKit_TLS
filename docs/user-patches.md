# 用户配置 Patch

自动更新会用新版本的官方配置整体替换 `portable_config/mpv.conf`、
`input.conf`、`profiles.conf`、`platform.conf` 和 `script-opts/*.conf`。
因此不要直接把修改留在这些文件里；把“配置意图”写进
`portable_config/user/patches/`，更新器会在新官方配置上重新应用。

## 为什么不是旧配置副本

用户维护的是差异，不是某个历史版本的完整文件。新版本修改了官方默认值时，
没有写进 patch 的键会自然跟随新官方值；只有 patch 里明确写出的内容才覆盖。
这样既不会冻结旧配置，也不会在每次更新时丢失个人设置。

`.playkit/official/` 是本次安装的官方配置基线，`.playkit/generated/` 是上次
应用用户 patch 后真正写入磁盘的结果。两者都由更新器维护，用户不需要编辑。

## 目录

```text
portable_config/user/
  patches/
    mpv.conf.patch              # schema 1 手动 patch（旧格式，仍支持）
    input.conf.patch
    profiles.conf.patch
    script-opts/
      uosc.conf.patch
    v2/
      input.conf.patch          # schema 2 手动 patch（推荐）
      mpv.conf.patch
    auto/
      mpv.conf.patch            # 旧版自动捕获（会迁移到 auto/v2/）
      v2/
        mpv.conf.patch          # 自动捕获的直接编辑
  scripts/                      # 用户 Lua，更新时复制到 portable_config/scripts/
  fonts/                        # 用户字体，复制到 portable_config/fonts/
  script-opts/                  # 用户覆盖项，复制到 portable_config/script-opts/
  backup/                       # 捕获/迁移前的备份
```

patch 文件名是 `<目标路径>.patch`。目标路径使用 `/` 分隔，例如
`user/patches/v2/script-opts/uosc.conf.patch`。文件内的 `target=` 可以显式
指定目标；未指定时使用文件名推断。未知目标的 patch 会被忽略并在日志中警告。

`patches/` 根目录放 schema 1 手动 patch，`patches/v2/` 放 schema 2 手动 patch。
自动捕获的 patch 位于 `patches/auto/v2/`。旧 helper 只认识 schema 1，所以
schema 2 文件放在 `v2/` 子目录中；旧 helper 会忽略整个子目录，不会因为读不懂
新格式而中止更新。helper 1.1.0 同时读取两套格式，并在第一次修复时把旧的
`auto/*.patch` 合并进 `auto/v2/`。

## 应用时机

- **正常更新**：下载并校验新版本后，helper 自动把 `user/patches/`（包括
  `v2/`）和 `auto/v2/` 中的 patch 应用到新官方配置，不需要在 GUI 里点任何
  “应用”按钮。
- **启动时自动检测**：`update-check.lua` 在启动约 6 秒后运行
  `playkit-updater repair-auto`（`repair --if-needed` 的别名）。helper 比较
  patch 文件、用户脚本、字体和 `user/script-opts/` 的指纹；与上次安装、
  修复或捕获时一致就立即退出，不重写任何配置。有变化时才重新应用 patch。
- **自动重启**：如果自动修复改变了生成配置，且启动时还没有加载媒体，
  `auto_repair_relaunch=auto`（默认）会重启播放器一次，让新配置在本次启动
  就生效。已经加载媒体时不会自动退出，只提示手动重启，避免丢失播放。
  `auto_repair_relaunch=no` 关闭自动重启；`yes` 强制重启（不会重新打开当前
  文件）。
- **保存当前配置修改**：菜单项 `update-check/capture-now`，只把直接编辑捕获成
  `auto/v2/*.patch`，不更新版本，也不是应用 patch 的步骤。捕获后会同步更新
  generated 快照和用户层指纹，因此下一次启动不会把这次编辑误判为未托管修改。
- **立即重新应用配置 patch**：菜单项 `update-check/repair-now`，用于不等待
  下一次更新就强制重放 patch，或手动迁移。写出的配置在下次启动时生效；
  正常更新和重启都不需要它。

`auto_repair=no` 关闭启动自动检测；`install_enabled=no` 或状态中的
`disabled=true` 也会关闭它。`enabled=no` 时脚本在读取配置后立即返回，不会
启动 helper。第一次启动由旧 helper 安装的版本、或包内 state 还没有指纹时，
helper 会做一次初始化修复；如果生成配置没有变化，不会自动重启。

## 语法

patch 是纯文本，不是 unified diff。第一行必须是：

```text
schema=2
```

也接受旧的 `schema=1`；schema 1 只支持键操作，不支持下面的行操作。可选第二行
指定目标：

```text
target=input.conf
```

### 键操作（schema 1 和 2）

| 写法 | 含义 |
| --- | --- |
| `key=value` | 设置键；键不存在时插入，存在时替换该作用域内的值 |
| `-key` | 删除该作用域内的键；键不存在时记录警告 |
| `!key` | 与 `-key` 相同（便于避免把行首 `-` 误认为其他语法） |
| `+key=value` | 追加一行，即使同名键已存在；用于需要重复键的配置 |
| `# ...` | 注释，空行忽略 |

`[global]` 表示文件顶部的全局作用域；`[profile-name]` 只作用于同名配置段。
`profiles.conf` 支持任意段名；`mpv.conf`、`input.conf`、`platform.conf` 和
`script-opts` 使用 `[global]`。段名区分大小写，`global` 不区分大小写。

### 行操作（schema 2）

行操作用于注释、空行、uosc 菜单项以及任何不是 `key=value` 的原始文本。
内容块以 `|` 开头，以 `@end` 结束；`|` 单独一行表示空行，`||foo` 表示内容
`|foo`。

```text
schema=2
target=input.conf

@after command:script-binding update-check/check-now
|#                script-binding my-ui/refresh #! 更新 > 刷新订阅
@end
```

支持的操作：

| 操作 | 含义 |
| --- | --- |
| `@after <选择器>` | 在目标行之后插入内容块 |
| `@before <选择器>` | 在目标行之前插入内容块 |
| `@replace <选择器>` | 用内容块替换目标行 |
| `@delete <选择器>` | 删除目标行；目标不存在时只记录警告 |
| `@append` | 在当前作用域末尾追加内容块 |
| `@prepend` | 在当前作用域开头插入内容块 |
| `@splice remove=N insert=M` | 精确匹配 N 行旧文本，替换为 M 行新文本 |
| `@optional <操作>` | 目标不存在时跳过并警告，而不是中止更新 |

选择器：

| 选择器 | 匹配对象 |
| --- | --- |
| `menu:更新 > 立即检查更新` | uosc 菜单标签路径 |
| `command:script-binding update-check/check-now` | input.conf 绑定命令 |
| `key:Ctrl+u` | 配置键或按键 |
| `section:gpu-hq` | `[gpu-hq]` 段头 |
| `line:精确的整行文本` | 去除首尾空白后完全相同的原始行 |

同一选择器匹配到多处时，在类型后加 `[n]` 指定第 n 个匹配，例如
`menu[2]:更新 > 立即检查更新`、`command[2]:cycle pause`。没有 `[n]` 且匹配
多处时，插入/替换操作会中止更新，避免改错位置；删除和 `@optional` 也不会
猜测。

### 为什么优先锚定命令

菜单标签是展示文本，后续做 i18n 时可能整体变化；绑定命令是稳定的行为标识。
因此插入菜单项时优先用 `command:` 作为锚点，让新标签只出现在插入的内容里：

```text
@after command:script-binding update-check/check-now
|#                script-binding my-ui/refresh #! 更新 > 刷新订阅
@end
```

如果确实要跟随菜单层级和显示文本，再用 `menu:`。`menu:` 会规范化 `>` 两侧
空格，所以 `更新>立即检查更新` 和 `更新 > 立即检查更新` 等价。

### 任意文本块

当没有稳定语义锚点，或要替换一整段文本时，用 `@splice`：

```text
schema=2
target=mpv.conf

@splice remove=2 insert=1
|# old line 1
|# old line 2
|# replacement
@end
```

默认按原始行精确匹配。加 `match=trim` 可忽略每行首尾空白；`occurrence=N`
选择第 N 处相同旧块。`insert=0` 表示只删除。应用后再次应用会识别新块并跳过，
所以 `repair` 是幂等的。

### mpv.conf

```text
schema=2
target=mpv.conf

[global]
volume=70
keep-open=yes
-sub-font-size
```

### profiles.conf

```text
schema=2
target=profiles.conf

[gpu-hq]
scale=ewa_lanczossharp

[low-power]
profile=gpu-hq
scale=bilinear
```

只修改 `[gpu-hq]` 中的 `scale`，不会触碰全局或其他 profile 里的同名键。

### input.conf

input.conf 支持 `key command` 和 `key=command` 两种写法；替换时保留原行的
分隔风格，新增行使用空格。删除绑定用 `-key` 或 `!key`；不带命令的 `key=`
是无效 patch。

```text
schema=2
target=input.conf

MBTN_LEFT cycle pause
Ctrl+Shift+u script-binding update-check/install-now
-Ctrl+u

@after command:cycle pause
|Ctrl+Alt+x script-binding my-ui/x #! 工具 > 我的工具
@end
```

`#!` 菜单标签会被保留：替换一个已有 uosc 菜单项时，如果没有在新命令里写
`#! 新标签`，原标签会继续沿用。

### script-opts

```text
schema=2
target=script-opts/uosc.conf

[global]
background-color=#000000C0
```

### 删除官方绑定

官方 `input.conf` 中已有的绑定可以直接覆盖（同一按键写新命令），也可以用
`-key` 删除。删除后播放器不再响应该按键；如果希望保留按键但禁用行为，可绑定
到 `ignore`：

```text
Ctrl+u ignore
```

## 冲突与警告

- 插入/替换的目标不存在：**中止更新**，当前安装不变。
- 选择器匹配多处但没有 `[n]`：**中止更新**。
- `@delete` 目标不存在、`@optional` 目标不存在、删除不存在的键：记录警告，
  继续更新。
- 在 `profiles.conf` 中写入一个不存在的段：在文件末尾创建该段，这是添加
  profile 的正常方式。
- 手动 patch 无法解析、schema 不支持、目标路径不安全、编码不是 UTF-8：
  视为严重冲突，**中止更新**，当前安装不变。
- 用户 `script-opts/` 中的文件与官方文件同名：内容相同则跳过；内容不同则
  **中止更新**，避免静默覆盖官方脚本配置。
- 用户脚本、字体复制时遇到同名文件：内容相同则跳过；内容不同则**中止更新**，
  避免覆盖官方文件。

所有警告写入 `.playkit/updater.log` 并保存到 `state.json` 的
`patch_warnings`；Lua 只显示简短的“更新完成（有警告）”。

## 捕获直接编辑

如果直接编辑了 `portable_config/mpv.conf` 或 `input.conf`，更新器会比较它和
`.playkit/generated/` 中上次生成的快照：

- `unmanaged=capture`（默认）：把差异写成 `patches/auto/v2/<target>.patch`。
- `unmanaged=discard`：忽略这些修改。
- `unmanaged=abort`：停止更新，等待用户先处理。

键和值的变化写成键操作；注释、菜单项和空行等文本变化写成行操作。捕获插入的
菜单项时，如果锚点行有菜单标签或绑定命令，会自动使用 `menu:`/`command:`
选择器，而不是行号或整行文本。

同一处连续修改同时包含配置键和注释时，更新器无法安全判断意图，会中止捕获并
要求写成手动 patch，而不是静默丢掉其中一部分。捕获完成后会在内存中重放并
校验结果，校验失败同样中止更新。

捕获的 patch 只包含差异，不包含整个文件；之后可以把它移动到
`user/patches/v2/` 作为长期维护的手动 patch，或直接编辑。自动 patch 会保留
旧内容并与新捕获合并：同一个键以最后一次捕获为准，行操作按顺序保留，因此
多次更新不会因为 `auto/` 被清空而丢失之前的直接编辑意图。

也可以在菜单中手动执行“保存当前配置修改”（`update-check/capture-now`），
它只写 patch，不更新版本。

## 用户脚本与字体

- 把 Lua 放进 `user/scripts/`，更新时复制到 `portable_config/scripts/`；
  与官方脚本同名且内容不同会中止更新。
- 字体放进 `user/fonts/`，复制到 `portable_config/fonts/`。
- `user/script-opts/` 中的文件复制到 `portable_config/script-opts/`；与官方
  文件同名且内容不同会中止更新。
- 更新器只复制这些目录，不会删除 `user/` 中的任何内容；删除用户脚本后，
  下一次更新不会再把旧副本装回去（但旧的 `portable_config/scripts/` 副本会在
  整目录替换时消失）。

## 回滚

菜单中的“回滚到上一版本”会把 `.previous` 换回当前安装。回滚恢复的是整个
安装目录，包括当时的 `user/` 层和 patch。回滚会删除被替换的新版本目录，
是单向操作；回滚后的安装没有 `.previous`，直到下一次成功更新。
