from __future__ import annotations

import argparse
import json
import shutil
import tarfile
import zipfile
from pathlib import Path
from typing import Any

from .common import (
    DEFAULT_LOCK,
    fail,
    human_size,
    log,
    now_iso,
    package_filename,
    read_json,
    sha256_file,
    write_json,
)
from .upstream import lock_hash

# Files that are generated next to the published assets but are not themselves
# release assets.  SHA256SUMS must only list files users can download.
CHECKSUM_EXCLUDED = frozenset({"SHA256SUMS", "release-notes.md"})


def checksum_lines(out_dir: Path) -> list[str]:
    lines = []
    for path in sorted(out_dir.iterdir()):
        if path.name in CHECKSUM_EXCLUDED or not path.is_file():
            continue
        lines.append(f"{sha256_file(path)}  {path.name}")
    return lines


def find_assets(dist: Path, version: str) -> dict[str, Path]:
    result: dict[str, Path] = {}
    for platform in ("windows-x64", "linux-x64"):
        name = package_filename(version, platform)
        matches = sorted(dist.rglob(name))
        if len(matches) != 1:
            fail(f"expected exactly one {name} under {dist}, found {len(matches)}")
        result[platform] = matches[0]
    return result


def embedded_manifest(asset: Path, version: str, platform: str) -> dict[str, Any]:
    top = f"mpv-PlayKit-{version}-{platform}"
    member = f"{top}/manifest.json"
    if asset.suffix == ".zip":
        with zipfile.ZipFile(asset) as archive:
            try:
                payload = archive.read(member)
            except KeyError:
                fail(f"{asset}: missing {member}")
    else:
        with tarfile.open(asset, "r:gz") as archive:
            try:
                handle = archive.extractfile(member)
            except KeyError:
                fail(f"{asset}: missing {member}")
            if handle is None:
                fail(f"{asset}: could not read {member}")
            payload = handle.read()
    data = json.loads(payload.decode("utf-8"))
    if data.get("playkit_version") != version:
        fail(f"{asset}: embedded version mismatch")
    if data.get("platform") != platform:
        fail(f"{asset}: embedded platform mismatch")
    return data


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Create release manifest, notes and checksums")
    parser.add_argument("--version", required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--dist", type=Path, required=True)
    parser.add_argument("--lock", type=Path, default=DEFAULT_LOCK)
    parser.add_argument("--repository", required=True)
    parser.add_argument("--out-dir", type=Path, default=Path("dist/release"))
    parser.add_argument("--built-at", default=None)
    parser.add_argument("--source-sha", default="local")
    args = parser.parse_args(argv)

    lock = read_json(args.lock)
    assets = find_assets(args.dist, args.version)
    out_dir = args.out_dir.resolve()
    out_dir.mkdir(parents=True, exist_ok=True)

    platforms: dict[str, Any] = {}
    embedded: dict[str, dict[str, Any]] = {}
    for platform, asset in assets.items():
        manifest = embedded_manifest(asset, args.version, platform)
        embedded[platform] = manifest
        digest = sha256_file(asset)
        target = out_dir / asset.name
        shutil.copy2(asset, target)
        platforms[platform] = {
            "asset": asset.name,
            "format": "zip" if platform.startswith("windows") else "tar.gz",
            "sha256": digest,
            "size": asset.stat().st_size,
        }
        log(f"release asset {asset.name}: {human_size(asset.stat().st_size)} sha256={digest}")

    inputs_hashes = {manifest.get("inputs_hash") for manifest in embedded.values()}
    if len(inputs_hashes) != 1:
        fail(f"platform packages disagree on inputs_hash: {inputs_hashes}")
    inputs_hash = inputs_hashes.pop()

    components: dict[str, Any] = {}
    for identifier, locked in sorted(lock.get("components", {}).items()):
        components[identifier] = {
            "version": locked.get("version"),
            "upstream": locked.get("upstream"),
            "release_tag": locked.get("release_tag"),
            "release_url": locked.get("release_url"),
            "asset_name": locked.get("asset_name"),
            "sha256": locked.get("sha256"),
            "license": (locked.get("license") or {}).get("spdx"),
        }
        for platform, manifest in embedded.items():
            packaged = (manifest.get("components") or {}).get(identifier)
            if packaged and packaged.get("sha256") != locked.get("sha256"):
                fail(f"{identifier}: packaged hash differs from lock on {platform}")

    manifest = {
        "schema": 1,
        "playkit_version": args.version,
        "tag": args.tag,
        "repository": args.repository,
        "built_at": args.built_at or now_iso(),
        "source_commit": args.source_sha,
        "inputs_hash": inputs_hash,
        "upstream_lock_sha256": lock_hash(lock),
        "components": components,
        "platforms": platforms,
    }
    write_json(out_dir / "manifest.json", manifest)

    notes_lines = [
        f"# mpv PlayKit {args.version}",
        "",
        f"Automated build of `{args.repository}` at `{args.source_sha}`.",
        "",
        "## Components",
        "",
        "| Component | Version | Upstream |",
        "| --- | --- | --- |",
    ]
    for identifier, component in sorted(components.items()):
        notes_lines.append(
            f"| {identifier} | {component.get('version')} | {component.get('upstream')} |"
        )
    notes_lines.extend(
        [
            "",
            "## Assets",
            "",
            f"- `{platforms['windows-x64']['asset']}`",
            f"- `{platforms['linux-x64']['asset']}`",
            "",
            "See `manifest.json` and `licenses/` for exact hashes and license notices.",
            "",
        ]
    )
    (out_dir / "release-notes.md").write_text("\n".join(notes_lines), encoding="utf-8", newline="\n")

    checksums = checksum_lines(out_dir)
    (out_dir / "SHA256SUMS").write_text("\n".join(checksums) + "\n", encoding="utf-8", newline="\n")
    log(f"wrote release metadata to {out_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
