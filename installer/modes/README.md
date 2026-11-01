# 模式启动器

这些启动器用于隔离配置问题、做最小化测试和跑分。它们不修改任何文件，
只在启动 mpv 时追加命令行参数；正常播放请使用包根目录的 `mpv.exe` 或
`./mpv-playkit`。

| 模式 | Windows | Linux | 配置 | 日志（包根目录） |
| --- | --- | --- | --- | --- |
| 纯净 | `pure.cmd` | `pure.sh` | `--config=no`，不使用任何配置 | `mpv-pure.log` |
| 测试 | `test.cmd` | `test.sh` | `--config=no --include=.../test.conf` | `mpv-test.log` |
| 跑分 | `benchmark.cmd` | `benchmark.sh` | `--config=no --include=.../benchmark.conf` | `mpv-benchmark.log` |

## 用法

Windows 双击对应的 `.cmd`，或把媒体文件拖到 `.cmd` 上；也可以在命令行运行：

```bat
installer\modes\test.cmd "D:\video\sample.mkv"
installer\modes\benchmark.cmd --profile=bench_api_2 --profile=bench_render_2 "D:\video\sample.mkv"
```

Linux：

```sh
./installer/modes/test.sh video/sample.mkv
./installer/modes/benchmark.sh --profile=bench_api_2 video/sample.mkv
```

启动器会把额外参数原样传给 mpv，后出现的 `--profile=` 会覆盖文件里的默认值。
日志写在包根目录；移动包后日志不会跟着 `portable_config` 走。

## 跑分预设

`benchmark.conf` 定义了两组正交的 profile：

- `bench_api_1`：`gpu-next` + D3D11，硬解关闭（Windows 默认）
- `bench_api_2`：`gpu-next` + Vulkan，硬解关闭
- `bench_api_3`：`gpu-next` + OpenGL，硬解关闭（Linux 默认）
- `bench_render_1`：`display-resample` + sinc 插值（默认）
- `bench_render_2`：`display-resample` + Lanczos 插值

这些 profile 只定义在 `benchmark.conf` 中，不会影响正常播放。