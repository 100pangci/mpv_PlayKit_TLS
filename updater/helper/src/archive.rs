//! Safe extraction of the release archive and structural verification of the
//! extracted package.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use zip::ZipArchive;

use crate::manifest::{PackageManifest, ReleaseManifest, VersionInfo};
use crate::util::{
    ensure_dir, io_context, is_sha256, is_within, lexical_join, make_executable, relative_path,
    sha256_file, Failure, Result,
};

const MAX_UNCOMPRESSED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ENTRY_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_COMPRESSION_RATIO: u64 = 500;
const MIN_RATIO_CHECK_BYTES: u64 = 1024 * 1024;

pub fn expected_top(version: &str, platform: &str) -> String {
    format!("mpv-PlayKit-{version}-{platform}")
}

pub fn extract_package(
    archive: &Path,
    dest: &Path,
    version: &str,
    platform: &str,
) -> Result<PathBuf> {
    let top_name = expected_top(version, platform);
    ensure_dir(dest)?;
    match platform {
        "windows-x64" => extract_zip(archive, dest, &top_name)?,
        "linux-x64" => extract_tar_gz(archive, dest, &top_name)?,
        other => {
            return Err(Failure::failed(
                "不支持的平台。",
                format!("platform {other:?}"),
            ))
        }
    }
    let top = dest.join(&top_name);
    if !top.is_dir() {
        return Err(Failure::failed(
            "压缩包缺少顶层目录。",
            format!("expected {top_name}/ in {}", archive.display()),
        ));
    }
    // Reject a second top-level entry; a valid package has exactly one.
    let mut top_entries = Vec::new();
    for entry in fs::read_dir(dest)
        .map_err(|error| io_context(&format!("read {}", dest.display()), error))?
    {
        top_entries
            .push(entry.map_err(|error| io_context(&format!("read {}", dest.display()), error))?);
    }
    if top_entries.len() != 1 {
        return Err(Failure::failed(
            "压缩包包含多个顶层条目。",
            format!(
                "{}: {:?}",
                archive.display(),
                top_entries
                    .iter()
                    .map(|entry| entry.file_name())
                    .collect::<Vec<_>>()
            ),
        ));
    }
    Ok(top)
}

fn extract_zip(archive: &Path, dest: &Path, top_name: &str) -> Result<()> {
    let file = File::open(archive)
        .map_err(|error| io_context(&format!("open {}", archive.display()), error))?;
    let mut zip = ZipArchive::new(file)?;
    let mut seen: HashSet<String> = HashSet::new();
    let mut total: u64 = 0;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let name = entry.name().to_string();
        let relative = safe_zip_name(&name, top_name)?;
        let key = relative.to_string_lossy().to_ascii_lowercase();
        if !seen.insert(key) {
            return Err(Failure::failed(
                "压缩包包含重复路径。",
                format!("{}: {name:?}", archive.display()),
            ));
        }
        let mode = entry.unix_mode().unwrap_or(0);
        if mode & 0o170000 == 0o120000 {
            return Err(Failure::failed(
                "压缩包包含符号链接，更新已中止。",
                format!("{}: {name:?}", archive.display()),
            ));
        }
        if entry.is_dir() {
            ensure_dir(&dest.join(&relative))?;
            continue;
        }
        let declared = entry.size();
        if declared > MAX_ENTRY_BYTES {
            return Err(Failure::failed(
                "压缩包中的文件过大。",
                format!("{}: {name:?} ({declared} bytes)", archive.display()),
            ));
        }
        if declared >= MIN_RATIO_CHECK_BYTES
            && entry.compressed_size() > 0
            && declared / entry.compressed_size() > MAX_COMPRESSION_RATIO
        {
            return Err(Failure::failed(
                "压缩包压缩比异常，更新已中止。",
                format!("{}: {name:?}", archive.display()),
            ));
        }
        total = total.saturating_add(declared);
        if total > MAX_UNCOMPRESSED_BYTES {
            return Err(Failure::failed(
                "压缩包解压后过大。",
                format!("{}: {total} bytes", archive.display()),
            ));
        }
        let target = dest.join(&relative);
        if !is_within(dest, &target) {
            return Err(Failure::failed(
                "压缩包路径不安全。",
                format!("{}: {name:?}", archive.display()),
            ));
        }
        if let Some(parent) = target.parent() {
            ensure_dir(parent)?;
        }
        let mut output = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&target)
            .map_err(|error| io_context(&format!("create {}", target.display()), error))?;
        let copied = io::copy(&mut entry.by_ref().take(declared + 1), &mut output)
            .map_err(|error| io_context(&format!("extract {}", target.display()), error))?;
        if copied != declared {
            return Err(Failure::failed(
                "压缩包条目大小不一致。",
                format!("{}: {name:?} ({copied} != {declared})", archive.display()),
            ));
        }
        output
            .flush()
            .map_err(|error| io_context(&format!("flush {}", target.display()), error))?;
        drop(output);
        apply_unix_mode(&target, mode)?;
    }
    Ok(())
}

fn safe_zip_name(name: &str, top_name: &str) -> Result<PathBuf> {
    if name.contains('\0') {
        return Err(Failure::failed("压缩包路径不安全。", format!("{name:?}")));
    }
    let normalized = name.replace('\\', "/");
    let path = relative_path(&normalized)?;
    let mut components = path.components();
    match components.next() {
        Some(Component::Normal(first)) if first.to_string_lossy() == top_name => {}
        _ => {
            return Err(Failure::failed(
                "压缩包顶层目录不正确。",
                format!("{name:?}, expected {top_name}/"),
            ))
        }
    }
    Ok(path)
}

fn extract_tar_gz(archive: &Path, dest: &Path, top_name: &str) -> Result<()> {
    let file = File::open(archive)
        .map_err(|error| io_context(&format!("open {}", archive.display()), error))?;
    let decoder = GzDecoder::new(file);
    let mut tar = tar::Archive::new(decoder);
    tar.set_preserve_permissions(false);
    tar.set_preserve_mtime(false);
    tar.set_unpack_xattrs(false);
    let mut seen: HashSet<String> = HashSet::new();
    let mut total: u64 = 0;
    let entries = tar.entries()?;
    for entry in entries {
        let mut entry = entry?;
        let raw_path = entry.path()?.to_path_buf();
        let name = raw_path.to_string_lossy().replace('\\', "/");
        let relative = safe_tar_name(&name, top_name)?;
        let key = relative.to_string_lossy().to_string();
        if !seen.insert(key) {
            return Err(Failure::failed(
                "压缩包包含重复路径。",
                format!("{}: {name:?}", archive.display()),
            ));
        }
        let entry_type = entry.header().entry_type();
        let target = dest.join(&relative);
        if !is_within(dest, &target) {
            return Err(Failure::failed(
                "压缩包路径不安全。",
                format!("{}: {name:?}", archive.display()),
            ));
        }
        if entry_type.is_dir() {
            ensure_dir(&target)?;
            continue;
        }
        if entry_type.is_symlink() {
            let link = entry.link_name()?.ok_or_else(|| {
                Failure::failed(
                    "压缩包中的符号链接无效。",
                    format!("{}: {name:?}", archive.display()),
                )
            })?;
            let resolved = lexical_join(target.parent().unwrap_or(dest), &link);
            if link.is_absolute() || !is_within(&dest.join(top_name), &resolved) {
                return Err(Failure::failed(
                    "压缩包中的符号链接指向包外，更新已中止。",
                    format!("{}: {name:?} -> {link:?}", archive.display()),
                ));
            }
            if let Some(parent) = target.parent() {
                ensure_dir(parent)?;
            }
            #[cfg(unix)]
            {
                let _ = fs::remove_file(&target);
                std::os::unix::fs::symlink(&link, &target)
                    .map_err(|error| io_context(&format!("symlink {}", target.display()), error))?;
                continue;
            }
            #[cfg(not(unix))]
            {
                return Err(Failure::failed(
                    "当前平台不支持压缩包中的符号链接。",
                    format!("{}: {name:?}", archive.display()),
                ));
            }
        }
        if entry_type.is_hard_link() {
            let link = entry.link_name()?.ok_or_else(|| {
                Failure::failed(
                    "压缩包中的硬链接无效。",
                    format!("{}: {name:?}", archive.display()),
                )
            })?;
            let resolved = lexical_join(dest, &link);
            if link.is_absolute() || !is_within(&dest.join(top_name), &resolved) {
                return Err(Failure::failed(
                    "压缩包中的硬链接指向包外，更新已中止。",
                    format!("{}: {name:?} -> {link:?}", archive.display()),
                ));
            }
            if !resolved.exists() {
                return Err(Failure::failed(
                    "压缩包中的硬链接目标不存在。",
                    format!("{}: {name:?} -> {link:?}", archive.display()),
                ));
            }
            if let Some(parent) = target.parent() {
                ensure_dir(parent)?;
            }
            fs::hard_link(&resolved, &target)
                .map_err(|error| io_context(&format!("hardlink {}", target.display()), error))?;
            continue;
        }
        if !entry_type.is_file() {
            return Err(Failure::failed(
                "压缩包包含不支持的文件类型，更新已中止。",
                format!("{}: {name:?}", archive.display()),
            ));
        }
        let declared = entry.header().size()?;
        if declared > MAX_ENTRY_BYTES {
            return Err(Failure::failed(
                "压缩包中的文件过大。",
                format!("{}: {name:?} ({declared} bytes)", archive.display()),
            ));
        }
        total = total.saturating_add(declared);
        if total > MAX_UNCOMPRESSED_BYTES {
            return Err(Failure::failed(
                "压缩包解压后过大。",
                format!("{}: {total} bytes", archive.display()),
            ));
        }
        if let Some(parent) = target.parent() {
            ensure_dir(parent)?;
        }
        let mut output = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&target)
            .map_err(|error| io_context(&format!("create {}", target.display()), error))?;
        let copied = io::copy(&mut entry.by_ref().take(declared + 1), &mut output)
            .map_err(|error| io_context(&format!("extract {}", target.display()), error))?;
        if copied != declared {
            return Err(Failure::failed(
                "压缩包条目大小不一致。",
                format!("{}: {name:?} ({copied} != {declared})", archive.display()),
            ));
        }
        output
            .flush()
            .map_err(|error| io_context(&format!("flush {}", target.display()), error))?;
        drop(output);
        let mode = entry.header().mode().unwrap_or(0o644);
        apply_unix_mode(&target, mode)?;
    }
    Ok(())
}

fn safe_tar_name(name: &str, top_name: &str) -> Result<PathBuf> {
    if name.contains('\0') {
        return Err(Failure::failed("压缩包路径不安全。", format!("{name:?}")));
    }
    let path = relative_path(name)?;
    let mut components = path.components();
    match components.next() {
        Some(Component::Normal(first)) if first.to_string_lossy() == top_name => {}
        _ => {
            return Err(Failure::failed(
                "压缩包顶层目录不正确。",
                format!("{name:?}, expected {top_name}/"),
            ))
        }
    }
    Ok(path)
}

fn apply_unix_mode(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = mode & 0o777;
        if mode != 0 {
            let permissions = fs::Permissions::from_mode(mode);
            fs::set_permissions(path, permissions)
                .map_err(|error| io_context(&format!("chmod {}", path.display()), error))?;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
    Ok(())
}

pub fn verify_package_tree(
    top: &Path,
    release: &ReleaseManifest,
    platform: &str,
    archive_asset: &str,
) -> Result<PackageManifest> {
    let version = &release.version;
    let required = [
        "playkit-version.json",
        "manifest.json",
        "playkit-inventory.json",
        "README.txt",
        "LICENSE",
        "NOTICE",
        "THIRD_PARTY.md",
        "LICENSE_PROVENANCE.md",
        "portable_config/mpv.conf",
        "portable_config/input.conf",
        "portable_config/profiles.conf",
        "portable_config/platform.conf",
        "portable_config/menu.conf",
        "portable_config/scripts/update-check.lua",
        "portable_config/scripts/uosc/main.lua",
        "portable_config/.playkit/state.json",
        "portable_config/.playkit/official/mpv.conf",
        "portable_config/.playkit/official/input.conf",
        "portable_config/.playkit/official/profiles.conf",
        "portable_config/.playkit/official/platform.conf",
        "portable_config/user/README.md",
    ];
    for relative in required {
        let path = top.join(relative);
        if !path.is_file() {
            return Err(Failure::failed(
                "更新包结构不完整。",
                format!("missing {}", path.display()),
            ));
        }
    }
    if platform == "windows-x64" {
        for relative in ["mpv.exe", "yt-dlp.exe", "updater/playkit-updater.exe"] {
            if !top.join(relative).is_file() {
                return Err(Failure::failed(
                    "更新包结构不完整。",
                    format!("missing {}", top.join(relative).display()),
                ));
            }
        }
    } else if platform == "linux-x64" {
        for relative in [
            "mpv-playkit",
            "mpv/AppRun",
            "yt-dlp",
            "updater/playkit-updater",
        ] {
            let path = top.join(relative);
            if !path.is_file() {
                return Err(Failure::failed(
                    "更新包结构不完整。",
                    format!("missing {}", path.display()),
                ));
            }
            make_executable(&path)?;
        }
        make_executable(&top.join("mpv-playkit"))?;
        make_executable(&top.join("mpv").join("AppRun"))?;
    } else {
        return Err(Failure::failed(
            "不支持的平台。",
            format!("platform {platform:?}"),
        ));
    }

    let version_bytes = fs::read(top.join("playkit-version.json"))
        .map_err(|error| io_context("read playkit-version.json", error))?;
    let version_info = VersionInfo::parse(&version_bytes, "playkit-version.json")?;
    version_info.check(release, platform, "playkit-version.json")?;
    if version_info.version != *version {
        return Err(Failure::failed(
            "包内版本不正确。",
            format!("{} != {version}", version_info.version),
        ));
    }
    let manifest_bytes = fs::read(top.join("manifest.json"))
        .map_err(|error| io_context("read manifest.json", error))?;
    let package_manifest = PackageManifest::parse(&manifest_bytes, "manifest.json")?;
    package_manifest.check(release, platform, "manifest.json")?;
    if let Some(package) = &package_manifest.package {
        if let Some(root) = &package.root {
            let expected = expected_top(version, platform);
            if root != &expected {
                return Err(Failure::failed(
                    "包内顶层目录声明不一致。",
                    format!("manifest root {root:?}, expected {expected:?}"),
                ));
            }
        }
        if let Some(asset) = &package.asset {
            if asset != archive_asset {
                return Err(Failure::failed(
                    "包内资产名与 Release 不一致。",
                    format!("manifest asset {asset:?}, expected {archive_asset:?}"),
                ));
            }
        }
    }

    // Runtime state must never be shipped.  It would leak one user's cache or
    // update state into every installation.
    for forbidden in [
        "portable_config/_cache",
        "portable_config/saved-props.json",
        "portable_config/update-check-state.json",
        "portable_config/.playkit/update-status.json",
        "portable_config/.playkit/updater.log",
    ] {
        if top.join(forbidden).exists() {
            return Err(Failure::failed(
                "更新包包含运行时状态。",
                format!("unexpected {}", top.join(forbidden).display()),
            ));
        }
    }
    verify_state(&top.join("portable_config").join(".playkit"), release)?;
    verify_inventory(top)?;
    Ok(package_manifest)
}

/// The inventory lets the updater distinguish official files from user files
/// that happened to live under portable_config.  Verify every recorded hash and
/// require every packaged portable_config file to be listed, so a tampered or
/// incomplete inventory cannot cause a user file to be deleted.
fn verify_inventory(top: &Path) -> Result<()> {
    #[derive(serde::Deserialize)]
    struct Inventory {
        schema: u32,
        #[serde(default)]
        files: BTreeMap<String, String>,
    }
    let path = top.join("playkit-inventory.json");
    let data =
        fs::read(&path).map_err(|error| io_context(&format!("read {}", path.display()), error))?;
    let inventory: Inventory = serde_json::from_slice(&data).map_err(|error| {
        Failure::failed(
            "包内文件清单格式错误。",
            format!("{}: {error}", path.display()),
        )
    })?;
    if inventory.schema != 1 {
        return Err(Failure::failed(
            "包内文件清单 schema 不受支持。",
            format!("{}: schema {}", path.display(), inventory.schema),
        ));
    }
    if inventory.files.is_empty() {
        return Err(Failure::failed(
            "包内文件清单为空。",
            format!("{}", path.display()),
        ));
    }
    let portable = top.join("portable_config");
    for (relative, expected) in &inventory.files {
        if !is_sha256(expected) {
            return Err(Failure::failed(
                "包内文件清单包含无效哈希。",
                format!("{relative}: {expected:?}"),
            ));
        }
        let path = portable.join(relative_path(relative)?);
        if !path.is_file() {
            return Err(Failure::failed(
                "包内文件清单列出的文件缺失。",
                format!("{}", path.display()),
            ));
        }
        let actual = sha256_file(&path)?;
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(Failure::failed(
                "包内文件清单校验失败。",
                format!("{relative}: {actual} != {expected}"),
            ));
        }
    }
    let mut pending = vec![portable];
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|error| io_context(&format!("read {}", directory.display()), error))?;
        for entry in entries {
            let entry = entry
                .map_err(|error| io_context(&format!("read {}", directory.display()), error))?;
            let path = entry.path();
            let relative = path
                .strip_prefix(top.join("portable_config"))
                .map_err(|error| Failure::internal(format!("inventory path: {error}")))?
                .to_string_lossy()
                .replace('\\', "/");
            if relative == "user"
                || relative.starts_with("user/")
                || relative == ".playkit"
                || relative.starts_with(".playkit/")
                || relative == "_cache"
                || relative.starts_with("_cache/")
                || relative == "saved-props.json"
                || relative == "update-check-state.json"
            {
                continue;
            }
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| io_context(&format!("stat {}", path.display()), error))?;
            if metadata.is_dir() {
                pending.push(path);
                continue;
            }
            if !metadata.is_file() {
                return Err(Failure::failed(
                    "包内包含不支持的文件类型。",
                    format!("{}", path.display()),
                ));
            }
            if !inventory.files.contains_key(&relative) {
                return Err(Failure::failed(
                    "包内文件未登记在文件清单中。",
                    relative.to_string(),
                ));
            }
        }
    }
    Ok(())
}

fn verify_state(state_dir: &Path, release: &ReleaseManifest) -> Result<()> {
    #[derive(serde::Deserialize)]
    struct State {
        schema: u32,
        #[serde(default)]
        installed_version: Option<String>,
        #[serde(default)]
        official: std::collections::BTreeMap<String, String>,
        #[serde(default)]
        generated: std::collections::BTreeMap<String, String>,
    }
    let path = state_dir.join("state.json");
    let data =
        fs::read(&path).map_err(|error| io_context(&format!("read {}", path.display()), error))?;
    let state: State = serde_json::from_slice(&data).map_err(|error| {
        Failure::failed(
            "包内更新状态格式错误。",
            format!("{}: {error}", path.display()),
        )
    })?;
    if state.schema != 1 {
        return Err(Failure::failed(
            "包内更新状态 schema 不受支持。",
            format!("{}: schema {}", path.display(), state.schema),
        ));
    }
    if let Some(version) = &state.installed_version {
        if version != &release.version {
            return Err(Failure::failed(
                "包内更新状态版本不一致。",
                format!("{}: {version:?} != {:?}", path.display(), release.version),
            ));
        }
    }
    if state.official.is_empty() || state.generated.is_empty() {
        return Err(Failure::failed(
            "包内更新状态缺少官方配置基线。",
            format!("{}", path.display()),
        ));
    }
    if state.official != state.generated {
        return Err(Failure::failed(
            "包内更新状态不应包含用户 patch。",
            format!("{}", path.display()),
        ));
    }
    for (relative, hash) in &state.official {
        let baseline = state_dir.join("official").join(relative_path(relative)?);
        if !baseline.is_file() {
            return Err(Failure::failed(
                "包内官方配置基线缺失。",
                format!("{}", baseline.display()),
            ));
        }
        let actual = crate::util::sha256_file(&baseline)?;
        if &actual != hash {
            return Err(Failure::failed(
                "包内官方配置基线校验失败。",
                format!("{}: {actual} != {hash}", baseline.display()),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;

    fn zip_with(entries: &[(&str, &[u8], Option<u32>)]) -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            for (name, data, mode) in entries {
                let mut options = SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated);
                if let Some(mode) = mode {
                    options = options.unix_permissions(*mode);
                }
                if name.ends_with('/') {
                    writer.add_directory(*name, options).unwrap();
                } else {
                    writer.start_file(*name, options).unwrap();
                    writer.write_all(data).unwrap();
                }
            }
            writer.finish().unwrap();
        }
        buffer.into_inner()
    }

    fn tar_with(entries: &[(&str, &[u8], u32, Option<&str>)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::default(),
        ));
        for (name, data, mode, link) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_mode(*mode);
            header.set_uid(0);
            header.set_gid(0);
            header.set_mtime(0);
            if let Some(target) = link {
                header.set_entry_type(tar::EntryType::Symlink);
                header.set_size(0);
                header.set_link_name(target).unwrap();
                header.set_cksum();
                builder
                    .append_data(&mut header, *name, std::io::empty())
                    .unwrap();
            } else {
                header.set_entry_type(tar::EntryType::Regular);
                header.set_size(data.len() as u64);
                header.set_cksum();
                builder
                    .append_data(&mut header, *name, Cursor::new(*data))
                    .unwrap();
            }
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn extracts_safe_zip() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        let data = zip_with(&[
            ("mpv-PlayKit-2026.11-windows-x64/", b"", None),
            (
                "mpv-PlayKit-2026.11-windows-x64/mpv.exe",
                b"exe",
                Some(0o755),
            ),
        ]);
        fs::write(&archive, data).unwrap();
        let out = dir.path().join("out");
        let top = extract_package(&archive, &out, "2026.11", "windows-x64").unwrap();
        assert!(top.join("mpv.exe").is_file());
    }

    #[test]
    fn rejects_zip_traversal() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        let data = zip_with(&[("mpv-PlayKit-2026.11-windows-x64/../../evil", b"x", None)]);
        fs::write(&archive, data).unwrap();
        let out = dir.path().join("out");
        assert!(extract_package(&archive, &out, "2026.11", "windows-x64").is_err());
        assert!(!dir.path().join("evil").exists());
    }

    #[test]
    fn rejects_zip_symlink() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        let mut buffer = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut buffer);
            writer
                .add_symlink(
                    "mpv-PlayKit-2026.11-windows-x64/link",
                    "/etc/passwd",
                    SimpleFileOptions::default(),
                )
                .unwrap();
            writer.finish().unwrap();
        }
        fs::write(&archive, buffer.into_inner()).unwrap();
        let out = dir.path().join("out");
        assert!(extract_package(&archive, &out, "2026.11", "windows-x64").is_err());
    }

    #[test]
    fn extracts_safe_tar() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("a.tar.gz");
        let data = tar_with(&[(
            "mpv-PlayKit-2026.11-linux-x64/mpv-playkit",
            b"#!/bin/sh\n",
            0o755,
            None,
        )]);
        fs::write(&archive, data).unwrap();
        let out = dir.path().join("out");
        let top = extract_package(&archive, &out, "2026.11", "linux-x64").unwrap();
        assert!(top.join("mpv-playkit").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn extracts_symlink_inside_package_on_unix() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("a.tar.gz");
        let data = tar_with(&[
            (
                "mpv-PlayKit-2026.11-linux-x64/mpv-playkit",
                b"#!/bin/sh\n",
                0o755,
                None,
            ),
            (
                "mpv-PlayKit-2026.11-linux-x64/link",
                b"",
                0o777,
                Some("mpv-playkit"),
            ),
        ]);
        fs::write(&archive, data).unwrap();
        let out = dir.path().join("out");
        let top = extract_package(&archive, &out, "2026.11", "linux-x64").unwrap();
        assert!(top.join("link").is_symlink());
    }

    #[test]
    fn rejects_tar_traversal() {
        let top = "mpv-PlayKit-2026.11-linux-x64";
        assert!(safe_tar_name(&format!("{top}/../../evil"), top).is_err());
        assert!(safe_tar_name("/etc/passwd", top).is_err());
        assert!(safe_tar_name("other/evil", top).is_err());
    }

    #[test]
    fn rejects_tar_escaping_symlink() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("a.tar.gz");
        let data = tar_with(&[(
            "mpv-PlayKit-2026.11-linux-x64/link",
            b"",
            0o777,
            Some("../../../etc/passwd"),
        )]);
        fs::write(&archive, data).unwrap();
        let out = dir.path().join("out");
        assert!(extract_package(&archive, &out, "2026.11", "linux-x64").is_err());
    }

    #[test]
    fn rejects_wrong_top_dir() {
        let dir = tempdir().unwrap();
        let archive = dir.path().join("a.zip");
        let data = zip_with(&[("other/mpv.exe", b"exe", None)]);
        fs::write(&archive, data).unwrap();
        let out = dir.path().join("out");
        assert!(extract_package(&archive, &out, "2026.11", "windows-x64").is_err());
    }
}
