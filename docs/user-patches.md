# 用户配置 Patch

自动更新会用新版本的官方配置整体替换 `portable_config/mpv.conf`、
`input.conf`、`profiles.conf`、`platform.conf` 和 `script-opts/*.conf`。
因此不要直接把修改留在这些文件里；把“配置意图”写进
`portable_config/user/patches/`，更新器会在新官方配置上重新应用。

## 为什么不是旧配置副本

用户维护的是差异，不是某个历史版本的完整文件。新版本修改了官方默认值时，
没有写进 patch 的键会自然跟随新官方值；只有 patch 里明确写出的键才覆盖。
这样既不会冻结旧配置，也不会在每次更新时丢失个人设置。

## 目录

```text
portable_config/user/
  patches/
    mpv.conf.patch
    input.conf.patch
    profiles.conf.patch
    platform.conf.patch
    script-opts/
      uosc.conf.patch
    auto/
      mpv.conf.patch          # 更新器捕获的直接编辑
  scripts/                    # 用户 Lua，更新时复制到 portable_config/scripts/
  fonts/                      # 用户字体，复制到 portable_config/fonts/
  script-opts/                # 用户覆盖项，复制到 portable_config/script-opts/
  backup/                     # 捕获/迁移前的备份
```

patch 文件名是 `<目标路径>.patch`。目标路径使用 `/` 分隔，例如
`user/patches/script-opts/uosc.conf.patch`。文件内的 `target=` 可以显式指定
目标；未指定时使用文件名推断。未知目标的 patch 会被忽略并在日志中警告。

手动 patch 位于 `patches/`，自动捕获的 patch 位于 `patches/auto/`。删除
`auto/` 下的文件不会影响手动 patch；两者同时存在时都会应用，手动 patch 后应用。

## 语法

patch 是纯文本，不是 unified diff。第一行必须是：

```text
schema=1
```

可选第二行指定目标：

```text
target=mpv.conf
```

之后是操作：

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

### mpv.conf

```text
schema=1
target=mpv.conf

[global]
volume=70
keep-open=yes
-sub-font-size
```

### profiles.conf

```text
schema=1
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
schema=1
target=input.conf

MBTN_LEFT cycle pause
Ctrl+Shift+u script-binding update-check/install-now
-Ctrl+u
```

`#!` 菜单标签会被保留：替换一个已有 uosc 菜单项时，如果没有在新命令里写
`#! 新标签`，原标签会继续沿用。

### script-opts

```text
schema=1
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

- 删除一个不存在的键：记录警告，继续更新。
- 在 `profiles.conf` 中写入一个不存在的段：在文件末尾创建该段，这是添加
  profile 的正常方式。
- 手动 patch 无法解析、schema 不支持、目标路径不安全、编码不是 UTF-8：
  视为严重冲突，**中止更新**，当前安装不变。
- 用户 `script-opts/` 中已有某个文件时，用户文件优先，并记录冲突警告。
- 用户脚本、字体复制时遇到同名文件：保留用户文件并记录警告。

所有警告写入 `.playkit/updater.log` 并保存到 `state.json` 的
`patch_warnings`；Lua 只显示简短的“更新完成（有警告）”。

## 捕获直接编辑

如果直接编辑了 `portable_config/mpv.conf`，更新器会比较它和
`.playkit/generated/` 中上次生成的快照：

- `unmanaged=capture`（默认）：把差异写成 `patches/auto/<target>.patch`。
- `unmanaged=discard`：忽略这些修改。
- `unmanaged=abort`：停止更新，等待用户先处理。

也可以在菜单中手动执行“保存当前配置修改”（`update-check/capture-now`），
它只写 patch，不更新版本。无法用 patch 语义表达的行会导致捕获失败，更新
中止，避免静默丢配置。

捕获的 patch 只包含差异，不包含整个文件；之后可以把它移动到
`user/patches/` 作为长期维护的手动 patch，或直接编辑。

## 用户脚本与字体

- 把 Lua 放进 `user/scripts/`，更新时复制到 `portable_config/scripts/`；
  同名官方脚本会被用户脚本覆盖并记录警告。
- 字体放进 `user/fonts/`，复制到 `portable_config/fonts/`。
- `user/script-opts/` 中的文件复制到 `portable_config/script-opts/`，优先于
  官方文件。
- 更新器只复制这些目录，不会删除 `user/` 中的任何内容；删除用户脚本后，
  下一次更新不会再把旧副本装回去（但旧的 `portable_config/scripts/` 副本会在
  整目录替换时消失）。

## 回滚

菜单中的“回滚到上一版本”会把 `.previous` 换回当前安装。回滚恢复的是整个
安装目录，包括当时的 `user/` 层和 patch。回滚会删除被替换的新版本目录，
是单向操作；回滚后的安装没有 `.previous`，直到下一次成功更新。
