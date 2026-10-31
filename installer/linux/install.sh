#!/bin/sh
# mpv PlayKit Linux installer - reserved interface, not implemented yet.
#
# This file defines the option names and defaults that a future Linux
# installer should honour.  It deliberately performs no changes: the portable
# package already runs with ./mpv-playkit from the extracted directory.
#
# Original code, licensed GPL-3.0-or-later.

set -eu

PACKAGE_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
PREFIX="${XDG_DATA_HOME:-$HOME/.local/share}/mpv-playkit"
DESKTOP=1
UNINSTALL=0

usage() {
	cat <<'EOF'
用法: installer/linux/install.sh [选项]

Linux 安装器接口已预留，但尚未实现。当前版本请直接运行包根目录的
./mpv-playkit（便携模式），无需安装。

预留选项（实现后生效）:
  --prefix <目录>   安装目录（默认 $XDG_DATA_HOME/mpv-playkit）
  --no-desktop      不安装 .desktop 菜单项
  --uninstall       卸载
  -h, --help        显示本帮助
EOF
}

while [ "$#" -gt 0 ]; do
	case "$1" in
		--prefix)
			[ "$#" -ge 2 ] || { echo "缺少 --prefix 的参数" >&2; exit 2; }
			PREFIX=$2
			shift 2
			;;
		--no-desktop)
			DESKTOP=0
			shift
			;;
		--uninstall)
			UNINSTALL=1
			shift
			;;
		-h|--help)
			usage
			exit 0
			;;
		*)
			echo "未知选项: $1" >&2
			usage >&2
			exit 2
			;;
	esac
done

cat >&2 <<EOF
mpv PlayKit Linux 安装器尚未实现。

已预留的接口参数：
  包根目录   $PACKAGE_ROOT
  安装目录   $PREFIX
  菜单项     $([ "$DESKTOP" -eq 1 ] && echo 是 || echo 否)
  卸载模式   $([ "$UNINSTALL" -eq 1 ] && echo 是 || echo 否)

请直接运行：
  "$PACKAGE_ROOT/mpv-playkit"

如需实现安装器，请参考同目录的 mpv-playkit.desktop.in、mpv-playkit.svg
以及 installer/README.md 中的 TODO 说明。
EOF
exit 1
