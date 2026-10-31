from __future__ import annotations

import argparse
import gzip
import ntpath
import os
import posixpath
import stat
import tarfile
import zipfile
from pathlib import Path

from .common import PLATFORM_ARCHIVE, fail, human_size, log, make_executable, package_basename, package_filename, sha256_file

FIXED_TIME = (2020, 1, 1, 0, 0, 0)


def iter_stage(stage: Path):
    for path in sorted(stage.rglob("*"), key=lambda item: str(item.relative_to(stage)).replace("\\", "/")):
        yield path


def safe_link_target(relative: str, linkname: str) -> str:
    """Reject symlink targets that escape the package root."""
    if (
        not linkname
        or posixpath.isabs(linkname)
        or ntpath.isabs(linkname)
        or ntpath.splitdrive(linkname)[0]
    ):
        fail(f"unsafe symlink target in stage: {relative} -> {linkname}")
    target = posixpath.normpath(posixpath.join(posixpath.dirname(relative), linkname))
    if target == ".." or target.startswith("../"):
        fail(f"symlink target escapes the package: {relative} -> {linkname}")
    return linkname


def zip_stage(stage: Path, output: Path, top: str) -> None:
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        root_info = zipfile.ZipInfo(f"{top}/", date_time=FIXED_TIME)
        root_info.external_attr = (stat.S_IFDIR | 0o755) << 16
        root_info.external_attr |= 0x10
        archive.writestr(root_info, b"")
        for path in iter_stage(stage):
            relative = str(path.relative_to(stage)).replace("\\", "/")
            archive_name = f"{top}/{relative}"
            if path.is_symlink():
                fail(f"zip staging tree contains a symlink: {relative}")
            if path.is_dir():
                info = zipfile.ZipInfo(archive_name + "/", date_time=FIXED_TIME)
                info.external_attr = (stat.S_IFDIR | 0o755) << 16
                info.external_attr |= 0x10
                archive.writestr(info, b"")
                continue
            mode = path.stat().st_mode
            executable = bool(mode & (stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH))
            info = zipfile.ZipInfo(archive_name, date_time=FIXED_TIME)
            info.external_attr = (stat.S_IFREG | (0o755 if executable else 0o644)) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            with path.open("rb") as handle:
                archive.writestr(info, handle.read())


def tar_stage(stage: Path, output: Path, top: str) -> None:
    with output.open("wb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0, compresslevel=9) as compressed:
            with tarfile.open(fileobj=compressed, mode="w") as archive:
                root_info = tarfile.TarInfo(top)
                root_info.type = tarfile.DIRTYPE
                root_info.mode = 0o755
                root_info.mtime = 0
                root_info.uid = root_info.gid = 0
                root_info.uname = root_info.gname = ""
                archive.addfile(root_info)

                for path in iter_stage(stage):
                    relative = str(path.relative_to(stage)).replace("\\", "/")
                    archive_name = f"{top}/{relative}"
                    if path.is_symlink():
                        linkname = safe_link_target(relative, os.readlink(path))
                        info = tarfile.TarInfo(archive_name)
                        info.type = tarfile.SYMTYPE
                        info.linkname = linkname
                        info.mode = 0o777
                        info.mtime = 0
                        info.uid = info.gid = 0
                        info.uname = info.gname = ""
                        archive.addfile(info)
                        continue
                    info = archive.gettarinfo(str(path), arcname=archive_name)
                    info.mtime = 0
                    info.uid = info.gid = 0
                    info.uname = info.gname = ""
                    if info.isdir():
                        info.mode = 0o755
                        archive.addfile(info)
                    elif info.isfile():
                        executable = bool(path.stat().st_mode & (stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH))
                        info.mode = 0o755 if executable else 0o644
                        with path.open("rb") as handle:
                            archive.addfile(info, handle)
                    else:
                        archive.addfile(info)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Package an assembled platform tree")
    parser.add_argument("--stage", type=Path, required=True)
    parser.add_argument("--platform", required=True, choices=sorted(PLATFORM_ARCHIVE))
    parser.add_argument("--version", required=True)
    parser.add_argument("--out-dir", type=Path, default=Path("dist"))
    args = parser.parse_args(argv)

    stage = args.stage.resolve()
    if not stage.is_dir():
        fail(f"missing stage directory: {stage}")
    args.out_dir.mkdir(parents=True, exist_ok=True)
    top = package_basename(args.version, args.platform)
    output = args.out_dir.resolve() / package_filename(args.version, args.platform)

    if args.platform.startswith("windows"):
        zip_stage(stage, output, top)
    else:
        tar_stage(stage, output, top)

    digest = sha256_file(output)
    checksum = output.with_suffix(output.suffix + ".sha256")
    checksum.write_text(f"{digest}  {output.name}\n", encoding="utf-8", newline="\n")
    log(f"packaged {output} ({human_size(output.stat().st_size)}, sha256 {digest})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
