//! End-to-end updater tests against a synthetic, fully valid release.
//!
//! These tests deliberately exercise the real archive verifier, manifest
//! chain, state handling, staging transaction and rollback.  Only the network
//! transport is replaced by `--source-dir` (DirFetcher).

use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};
use tempfile::tempdir;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use playkit_updater::update::{
    capture, install, read_status_file, rollback, CaptureOptions, InstallOptions, RelaunchPolicy,
    RollbackOptions, UnmanagedPolicy,
};
use playkit_updater::util::{sha256_bytes, sha256_file};

const REPOSITORY: &str = "owner/repo";
const OLD_VERSION: &str = "2026.10";
const NEW_VERSION: &str = "2026.11";
const SOURCE_COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";
const INPUTS_HASH: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

fn platform() -> &'static str {
    if cfg!(windows) {
        "windows-x64"
    } else {
        "linux-x64"
    }
}

fn other_platform() -> &'static str {
    if cfg!(windows) {
        "linux-x64"
    } else {
        "windows-x64"
    }
}

fn marker_name() -> &'static str {
    if cfg!(windows) {
        "mpv.exe"
    } else {
        "mpv-playkit"
    }
}

fn helper_name() -> &'static str {
    if cfg!(windows) {
        "playkit-updater.exe"
    } else {
        "playkit-updater"
    }
}

fn asset_name(version: &str, platform: &str) -> String {
    let extension = if platform == "windows-x64" {
        "zip"
    } else {
        "tar.gz"
    };
    format!("mpv-PlayKit-{version}-{platform}.{extension}")
}

fn managed_configs(new_version: bool) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    files.insert(
        "mpv.conf".to_string(),
        if new_version {
            "volume=100\nosc=yes\nnew-option=yes\n".to_string()
        } else {
            "volume=100\nosc=yes\n".to_string()
        },
    );
    files.insert(
        "input.conf".to_string(),
        "MBTN_LEFT cycle pause #! 播放 > 暂停\n".to_string(),
    );
    files.insert(
        "profiles.conf".to_string(),
        "[gpu-hq]\nscale=bilinear\n".to_string(),
    );
    files.insert("platform.conf".to_string(), "hwdec=auto\n".to_string());
    files.insert(
        "script-opts/update-check.conf".to_string(),
        "enabled=no\n".to_string(),
    );
    files
}

fn write_file(path: impl AsRef<Path>, data: impl AsRef<[u8]>) {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, data).unwrap();
}

fn write_json(path: impl AsRef<Path>, value: &Value) {
    let mut data = serde_json::to_vec_pretty(value).unwrap();
    data.push(b'\n');
    write_file(path, data);
}

fn collect_files(root: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, current: &Path, out: &mut BTreeMap<String, String>) {
        for entry in fs::read_dir(current).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.is_dir() {
                walk(root, &path, out);
            } else if metadata.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(relative, sha256_file(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn write_portable(portable: &Path, version: &str, new_version: bool) {
    let managed = managed_configs(new_version);
    for (relative, contents) in &managed {
        write_file(portable.join(relative), contents);
        write_file(portable.join(".playkit/official").join(relative), contents);
        write_file(portable.join(".playkit/generated").join(relative), contents);
    }
    write_file(portable.join("menu.conf"), "# menu\n");
    write_file(portable.join("scripts/update-check.lua"), "-- stub\n");
    write_file(portable.join("scripts/uosc/main.lua"), "-- stub\n");
    write_file(portable.join("user/README.md"), "user layer\n");
    write_file(portable.join("user/scripts/README.md"), "user scripts\n");
    write_file(portable.join("user/fonts/README.md"), "user fonts\n");
    write_file(
        portable.join("user/script-opts/README.md"),
        "user script opts\n",
    );

    let hashes: BTreeMap<String, String> = managed
        .iter()
        .map(|(relative, contents)| (relative.clone(), sha256_bytes(contents.as_bytes())))
        .collect();
    let state = json!({
        "schema": 1,
        "installed_version": version,
        "platform": platform(),
        "installed_at": "2026-01-01T00:00:00Z",
        "previous_version": Value::Null,
        "updated_by": Value::Null,
        "official": hashes,
        "generated": hashes,
        "patches": [],
        "patch_warnings": [],
        "user_scripts": [],
        "user_fonts": [],
        "user_script_opts": [],
    });
    write_json(portable.join(".playkit/state.json"), &state);
}

fn inventory_hashes(portable: &Path) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    for (relative, digest) in collect_files(portable) {
        let parts: Vec<&str> = relative.split('/').collect();
        if parts.first() == Some(&"user") || parts.first() == Some(&".playkit") {
            continue;
        }
        if relative == "saved-props.json" || relative == "update-check-state.json" {
            continue;
        }
        result.insert(relative, digest);
    }
    result
}

fn write_package(root: &Path, version: &str, new_version: bool) {
    fs::create_dir_all(root).unwrap();
    let portable = root.join("portable_config");
    write_portable(&portable, version, new_version);
    write_file(root.join(marker_name()), b"binary");
    if cfg!(windows) {
        write_file(root.join("yt-dlp.exe"), b"yt-dlp");
        write_file(root.join("updater").join(helper_name()), b"helper");
    } else {
        write_file(root.join("mpv/AppRun"), b"apprun");
        write_file(root.join("yt-dlp"), b"yt-dlp");
        write_file(root.join("updater").join(helper_name()), b"helper");
    }
    for name in [
        "README.txt",
        "LICENSE",
        "NOTICE",
        "THIRD_PARTY.md",
        "LICENSE_PROVENANCE.md",
    ] {
        write_file(root.join(name), format!("{name}\n"));
    }
    let version_info = json!({
        "schema": 1,
        "playkit_version": version,
        "platform": platform(),
        "repository": REPOSITORY,
        "built_at": "2026-01-01T00:00:00Z",
        "source_commit": SOURCE_COMMIT,
        "inputs_hash": INPUTS_HASH,
    });
    write_json(root.join("playkit-version.json"), &version_info);

    let asset = asset_name(version, platform());
    let package_manifest = json!({
        "schema": 1,
        "playkit_version": version,
        "platform": platform(),
        "repository": REPOSITORY,
        "built_at": "2026-01-01T00:00:00Z",
        "source_commit": SOURCE_COMMIT,
        "inputs_hash": INPUTS_HASH,
        "package": {
            "asset": asset,
            "platform": platform(),
            "format": if cfg!(windows) { "zip" } else { "tar.gz" },
            "root": format!("mpv-PlayKit-{version}-{}", platform()),
        },
    });
    write_json(root.join("manifest.json"), &package_manifest);
    write_json(
        root.join("playkit-inventory.json"),
        &json!({"schema": 1, "files": inventory_hashes(&portable)}),
    );
}

fn archive_bytes(package: &Path, version: &str, extra: Option<(&str, &[u8])>) -> Vec<u8> {
    let top = format!("mpv-PlayKit-{version}-{}", platform());
    if cfg!(windows) {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut cursor);
            writer
                .add_directory(
                    format!("{top}/"),
                    SimpleFileOptions::default().unix_permissions(0o755),
                )
                .unwrap();
            for (relative, _) in collect_files(package) {
                let data = fs::read(package.join(&relative)).unwrap();
                writer
                    .start_file(
                        format!("{top}/{relative}"),
                        SimpleFileOptions::default()
                            .compression_method(CompressionMethod::Deflated)
                            .unix_permissions(0o755),
                    )
                    .unwrap();
                writer.write_all(&data).unwrap();
            }
            if let Some((name, data)) = extra {
                writer
                    .start_file(
                        format!("{top}/{name}"),
                        SimpleFileOptions::default()
                            .compression_method(CompressionMethod::Deflated),
                    )
                    .unwrap();
                writer.write_all(data).unwrap();
            }
            writer.finish().unwrap();
        }
        cursor.into_inner()
    } else {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        let mut root_header = tar::Header::new_gnu();
        root_header.set_entry_type(tar::EntryType::Directory);
        root_header.set_mode(0o755);
        root_header.set_size(0);
        root_header.set_cksum();
        builder
            .append_data(&mut root_header, format!("{top}/"), std::io::empty())
            .unwrap();
        for (relative, _) in collect_files(package) {
            let data = fs::read(package.join(&relative)).unwrap();
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Regular);
            header.set_mode(0o755);
            header.set_size(data.len() as u64);
            header.set_cksum();
            builder
                .append_data(&mut header, format!("{top}/{relative}"), data.as_slice())
                .unwrap();
        }
        if let Some((name, data)) = extra {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Regular);
            header.set_mode(0o644);
            header.set_size(data.len() as u64);
            // `append_data` rejects `..` before writing.  Build a malicious
            // header directly instead so the extractor itself is tested.
            let raw = header.as_mut_bytes();
            raw[..100].fill(0);
            raw[..name.len()].copy_from_slice(name.as_bytes());
            header.set_cksum();
            builder.append(&header, data).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }
}

fn write_release(
    release_dir: &Path,
    version: &str,
    archive: Vec<u8>,
    manifest_platform: &str,
) -> PathBuf {
    fs::create_dir_all(release_dir).unwrap();
    let asset = asset_name(version, manifest_platform);
    fs::write(release_dir.join(&asset), &archive).unwrap();
    let release_manifest = json!({
        "schema": 2,
        "playkit_version": version,
        "tag": format!("v{version}"),
        "repository": REPOSITORY,
        "built_at": "2026-01-01T00:00:00Z",
        "source_commit": SOURCE_COMMIT,
        "inputs_hash": INPUTS_HASH,
        "platforms": {
            manifest_platform: {
                "asset": asset,
                "format": if manifest_platform == "windows-x64" { "zip" } else { "tar.gz" },
                "sha256": sha256_bytes(&archive),
                "size": archive.len(),
            }
        },
        "updater": {
            "protocol": 1,
            "min_helper_version": "1.0.0",
            "patch_schema": 1,
            "user_layer": "portable_config/user",
            "state_dir": "portable_config/.playkit",
        },
    });
    let manifest_bytes = serde_json::to_vec_pretty(&release_manifest).unwrap();
    fs::write(release_dir.join("manifest.json"), &manifest_bytes).unwrap();
    let sums = format!(
        "{}  manifest.json\n{}  {}\n",
        sha256_bytes(&manifest_bytes),
        sha256_bytes(&archive),
        asset
    );
    fs::write(release_dir.join("SHA256SUMS"), sums).unwrap();
    release_dir.to_path_buf()
}

fn make_release(release_dir: &Path, version: &str) -> PathBuf {
    let package = release_dir.join("package");
    write_package(&package, version, true);
    let archive = archive_bytes(&package, version, None);
    write_release(release_dir, version, archive, platform())
}

fn install_options(root: &Path, release: &Path, policy: UnmanagedPolicy) -> InstallOptions {
    InstallOptions {
        root: root.to_path_buf(),
        repository: REPOSITORY.to_string(),
        tag: format!("v{NEW_VERSION}"),
        platform: Some(platform().to_string()),
        wait_pid: 0,
        wait_timeout: Duration::from_secs(5),
        unmanaged: policy,
        relaunch: RelaunchPolicy::No,
        source_dir: Some(release.to_path_buf()),
        quiet: true,
    }
}

fn previous_path(root: &Path) -> PathBuf {
    let name = root.file_name().unwrap().to_string_lossy();
    root.parent().unwrap().join(format!("{name}.previous"))
}

fn read_version(root: &Path) -> String {
    let data = fs::read(root.join("playkit-version.json")).unwrap();
    let value: Value = serde_json::from_slice(&data).unwrap();
    value["playkit_version"].as_str().unwrap().to_string()
}

fn setup_old_install(root: &Path) {
    write_package(root, OLD_VERSION, false);
    write_file(
        root.join("portable_config/user/scripts/mine.lua"),
        "-- user script\n",
    );
    // Simulate a previously installed user layer: the previous version had
    // already copied this script into scripts/.
    write_file(
        root.join("portable_config/scripts/mine.lua"),
        "-- user script\n",
    );
    write_file(
        root.join("portable_config/update-check-state.json"),
        "{\"schema\":1,\"disabled\":false}\n",
    );
    write_file(
        root.join("portable_config/saved-props.json"),
        "{\"volume\":70}\n",
    );
    write_file(
        root.join("portable_config/_cache/watch_later/movie.state"),
        "position=1\n",
    );
}

#[test]
fn full_install_captures_edits_preserves_user_layer_and_rolls_back() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("playkit");
    fs::create_dir(&root).unwrap();
    setup_old_install(&root);
    // A direct edit to a managed file: no manual patch.
    write_file(
        root.join("portable_config/mpv.conf"),
        "volume=70\nosc=yes\n",
    );
    let release = make_release(&dir.path().join("release"), NEW_VERSION);

    let report = install(&install_options(&root, &release, UnmanagedPolicy::Capture)).unwrap();
    assert_eq!(report.version, NEW_VERSION);
    assert_eq!(report.previous_version, OLD_VERSION);

    assert_eq!(read_version(&root), NEW_VERSION);
    let mpv_conf = fs::read_to_string(root.join("portable_config/mpv.conf")).unwrap();
    assert!(mpv_conf.contains("volume=70"), "{mpv_conf}");
    assert!(mpv_conf.contains("new-option=yes"), "{mpv_conf}");
    assert!(!mpv_conf.contains("volume=100"), "{mpv_conf}");
    assert!(root
        .join("portable_config/user/patches/auto/v2/mpv.conf.patch")
        .is_file());
    assert!(root.join("portable_config/scripts/mine.lua").is_file());
    assert!(root
        .join("portable_config/update-check-state.json")
        .is_file());
    assert!(root.join("portable_config/saved-props.json").is_file());
    assert!(root
        .join("portable_config/_cache/watch_later/movie.state")
        .is_file());
    assert!(previous_path(&root).is_dir());
    let status = read_status_file(&root).unwrap().unwrap();
    assert_eq!(status.state, "installed");

    rollback(&RollbackOptions {
        root: root.clone(),
        wait_pid: 0,
        wait_timeout: Duration::from_secs(5),
        relaunch: RelaunchPolicy::No,
        quiet: true,
    })
    .unwrap();
    assert_eq!(read_version(&root), OLD_VERSION);
    let old_conf = fs::read_to_string(root.join("portable_config/mpv.conf")).unwrap();
    assert_eq!(old_conf, "volume=70\nosc=yes\n");
    assert!(!previous_path(&root).exists());
    assert!(root.join("portable_config/scripts/mine.lua").is_file());
}

#[test]
fn bad_archive_checksum_aborts_without_touching_current() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("playkit");
    fs::create_dir(&root).unwrap();
    setup_old_install(&root);
    let release = make_release(&dir.path().join("release"), NEW_VERSION);
    let archive = release.join(asset_name(NEW_VERSION, platform()));
    let mut bytes = fs::read(&archive).unwrap();
    bytes.extend_from_slice(b"tampered");
    fs::write(&archive, bytes).unwrap();

    let error = install(&install_options(&root, &release, UnmanagedPolicy::Capture)).unwrap_err();
    assert_eq!(error.kind, playkit_updater::util::ErrorKind::Failed);
    assert_eq!(read_version(&root), OLD_VERSION);
    assert!(!previous_path(&root).exists());
    assert!(fs::read_to_string(root.join("portable_config/mpv.conf"))
        .unwrap()
        .contains("volume=100"));
}

#[test]
fn abort_policy_blocks_on_direct_edits() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("playkit");
    fs::create_dir(&root).unwrap();
    setup_old_install(&root);
    write_file(
        root.join("portable_config/mpv.conf"),
        "volume=70\nosc=yes\n",
    );
    let release = make_release(&dir.path().join("release"), NEW_VERSION);

    let error = install(&install_options(&root, &release, UnmanagedPolicy::Abort)).unwrap_err();
    assert_eq!(error.kind, playkit_updater::util::ErrorKind::Blocked);
    assert_eq!(read_version(&root), OLD_VERSION);
    assert!(fs::read_to_string(root.join("portable_config/mpv.conf"))
        .unwrap()
        .contains("volume=70"));
    assert!(!previous_path(&root).exists());
}

#[test]
fn discard_policy_starts_from_the_new_official_configuration() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("playkit");
    fs::create_dir(&root).unwrap();
    setup_old_install(&root);
    write_file(
        root.join("portable_config/mpv.conf"),
        "volume=70\nosc=yes\n",
    );
    let release = make_release(&dir.path().join("release"), NEW_VERSION);

    install(&install_options(&root, &release, UnmanagedPolicy::Discard)).unwrap();
    let mpv_conf = fs::read_to_string(root.join("portable_config/mpv.conf")).unwrap();
    assert!(mpv_conf.contains("volume=100"), "{mpv_conf}");
    assert!(mpv_conf.contains("new-option=yes"), "{mpv_conf}");
    assert!(!mpv_conf.contains("volume=70"), "{mpv_conf}");
    assert!(!root
        .join("portable_config/user/patches/auto/mpv.conf.patch")
        .is_file());
}

#[test]
fn install_without_state_uses_new_official_and_backs_up_edits() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("playkit");
    fs::create_dir(&root).unwrap();
    setup_old_install(&root);
    write_file(
        root.join("portable_config/mpv.conf"),
        "volume=70\nosc=yes\nuser-option=1\n",
    );
    fs::remove_dir_all(root.join("portable_config/.playkit")).unwrap();
    fs::remove_file(root.join("playkit-inventory.json")).unwrap();
    let release = make_release(&dir.path().join("release"), NEW_VERSION);

    install(&install_options(&root, &release, UnmanagedPolicy::Capture)).unwrap();
    let mpv_conf = fs::read_to_string(root.join("portable_config/mpv.conf")).unwrap();
    assert!(mpv_conf.contains("volume=100"), "{mpv_conf}");
    assert!(mpv_conf.contains("new-option=yes"), "{mpv_conf}");
    assert!(!mpv_conf.contains("user-option=1"), "{mpv_conf}");
    assert!(!root
        .join("portable_config/user/patches/auto/mpv.conf.patch")
        .is_file());
    let backup =
        fs::read_to_string(root.join("portable_config/user/backup/pre-update-2026.10/mpv.conf"))
            .unwrap();
    assert!(backup.contains("user-option=1"), "{backup}");
}

#[test]
fn same_version_is_blocked_before_downloading() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("playkit");
    fs::create_dir(&root).unwrap();
    setup_old_install(&root);
    let release = make_release(&dir.path().join("release"), OLD_VERSION);
    let mut options = install_options(&root, &release, UnmanagedPolicy::Capture);
    options.tag = format!("v{OLD_VERSION}");

    let error = install(&options).unwrap_err();
    assert_eq!(error.kind, playkit_updater::util::ErrorKind::Blocked);
    assert!(error.user.contains("最新版本"), "{}", error.user);
    assert_eq!(read_version(&root), OLD_VERSION);
}

#[test]
fn archive_path_traversal_is_rejected() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("playkit");
    fs::create_dir(&root).unwrap();
    setup_old_install(&root);
    let release_dir = dir.path().join("release");
    let package = release_dir.join("package");
    write_package(&package, NEW_VERSION, true);
    let archive = archive_bytes(&package, NEW_VERSION, Some(("../../evil", b"evil")));
    let release = write_release(&release_dir, NEW_VERSION, archive, platform());

    let error = install(&install_options(&root, &release, UnmanagedPolicy::Capture)).unwrap_err();
    assert_eq!(error.kind, playkit_updater::util::ErrorKind::Failed);
    assert_eq!(read_version(&root), OLD_VERSION);
    assert!(!dir.path().join("evil").exists());
}

#[test]
fn missing_required_package_file_is_rejected() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("playkit");
    fs::create_dir(&root).unwrap();
    setup_old_install(&root);
    let release_dir = dir.path().join("release");
    let package = release_dir.join("package");
    write_package(&package, NEW_VERSION, true);
    fs::remove_file(package.join("THIRD_PARTY.md")).unwrap();
    let archive = archive_bytes(&package, NEW_VERSION, None);
    let release = write_release(&release_dir, NEW_VERSION, archive, platform());

    let error = install(&install_options(&root, &release, UnmanagedPolicy::Capture)).unwrap_err();
    assert_eq!(error.kind, playkit_updater::util::ErrorKind::Failed);
    assert!(error.detail.contains("THIRD_PARTY.md"), "{}", error.detail);
    assert_eq!(read_version(&root), OLD_VERSION);
}

#[test]
fn manual_patches_are_preserved_and_applied_to_the_new_official_config() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("playkit");
    fs::create_dir(&root).unwrap();
    setup_old_install(&root);
    write_file(
        root.join("portable_config/user/patches/mpv.conf.patch"),
        "schema=1\ntarget=mpv.conf\n\n[global]\nosc=no\n",
    );
    let release = make_release(&dir.path().join("release"), NEW_VERSION);

    install(&install_options(&root, &release, UnmanagedPolicy::Capture)).unwrap();
    let mpv_conf = fs::read_to_string(root.join("portable_config/mpv.conf")).unwrap();
    assert!(mpv_conf.contains("osc=no"), "{mpv_conf}");
    assert!(mpv_conf.contains("volume=100"), "{mpv_conf}");
    assert!(mpv_conf.contains("new-option=yes"), "{mpv_conf}");
    assert!(root
        .join("portable_config/user/patches/mpv.conf.patch")
        .is_file());
}

#[test]
fn schema_two_manual_patch_inserts_a_uosc_menu_item() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("playkit");
    fs::create_dir(&root).unwrap();
    setup_old_install(&root);
    write_file(
        root.join("portable_config/user/patches/v2/input.conf.patch"),
        "schema=2\ntarget=input.conf\n\n@after menu:播放 > 暂停\n|# script-binding my-ui/refresh #! 工具 > 刷新订阅\n@end\n",
    );
    let release = make_release(&dir.path().join("release"), NEW_VERSION);

    install(&install_options(&root, &release, UnmanagedPolicy::Capture)).unwrap();
    let input = fs::read_to_string(root.join("portable_config/input.conf")).unwrap();
    assert_eq!(
        input,
        "MBTN_LEFT cycle pause #! 播放 > 暂停\n# script-binding my-ui/refresh #! 工具 > 刷新订阅\n"
    );
    // Repair must be idempotent: the menu item is not inserted twice.
    playkit_updater::update::repair(&playkit_updater::update::RepairOptions {
        root: root.clone(),
        assume_official: false,
        if_needed: false,
        report_status: false,
        quiet: true,
    })
    .unwrap();
    let input = fs::read_to_string(root.join("portable_config/input.conf")).unwrap();
    assert_eq!(input.matches("工具 > 刷新订阅").count(), 1, "{input}");
}

#[test]
fn repair_if_needed_tracks_patch_changes_and_skips_an_unchanged_layer() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("playkit");
    fs::create_dir(&root).unwrap();
    setup_old_install(&root);
    let patch = root.join("portable_config/user/patches/v2/mpv.conf.patch");
    write_file(&patch, "schema=2\ntarget=mpv.conf\n\n[global]\nvolume=55\n");

    let options = |if_needed: bool| playkit_updater::update::RepairOptions {
        root: root.clone(),
        assume_official: false,
        if_needed,
        report_status: false,
        quiet: true,
    };

    let first = playkit_updater::update::repair(&options(false)).unwrap();
    assert!(first.repaired);
    assert!(first.changed);
    let mpv_conf = fs::read_to_string(root.join("portable_config/mpv.conf")).unwrap();
    assert!(mpv_conf.contains("volume=55"), "{mpv_conf}");

    let unchanged = playkit_updater::update::repair(&options(true)).unwrap();
    assert!(!unchanged.repaired);
    assert!(!unchanged.changed);

    write_file(&patch, "schema=2\ntarget=mpv.conf\n\n[global]\nvolume=56\n");
    let changed = playkit_updater::update::repair(&options(true)).unwrap();
    assert!(changed.repaired);
    assert!(changed.changed);
    let mpv_conf = fs::read_to_string(root.join("portable_config/mpv.conf")).unwrap();
    assert!(mpv_conf.contains("volume=56"), "{mpv_conf}");

    let unchanged_again = playkit_updater::update::repair(&options(true)).unwrap();
    assert!(!unchanged_again.repaired);

    fs::remove_file(&patch).unwrap();
    let removed = playkit_updater::update::repair(&options(true)).unwrap();
    assert!(removed.repaired);
    assert!(removed.changed);
    let mpv_conf = fs::read_to_string(root.join("portable_config/mpv.conf")).unwrap();
    assert!(!mpv_conf.contains("volume=56"), "{mpv_conf}");
    assert!(mpv_conf.contains("volume=100"), "{mpv_conf}");
}

#[test]
fn capture_refreshes_the_generated_snapshot_for_if_needed_repair() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("playkit");
    fs::create_dir(&root).unwrap();
    setup_old_install(&root);
    write_file(
        root.join("portable_config/mpv.conf"),
        "volume=55\nosc=yes\n",
    );

    capture(&CaptureOptions {
        root: root.clone(),
        dry_run: false,
        quiet: true,
    })
    .unwrap();

    let state: Value = serde_json::from_str(
        &fs::read_to_string(root.join("portable_config/.playkit/state.json")).unwrap(),
    )
    .unwrap();
    let generated = state["generated"]["mpv.conf"].as_str().unwrap();
    assert_eq!(
        generated,
        sha256_file(&root.join("portable_config/mpv.conf")).unwrap()
    );
    assert!(state["user_fingerprint"].as_str().is_some());

    let skipped = playkit_updater::update::repair(&playkit_updater::update::RepairOptions {
        root: root.clone(),
        assume_official: false,
        if_needed: true,
        report_status: false,
        quiet: true,
    })
    .unwrap();
    assert!(!skipped.repaired);

    // A forced repair must not mistake the just-captured edit for an
    // unmanaged modification.
    let repaired = playkit_updater::update::repair(&playkit_updater::update::RepairOptions {
        root,
        assume_official: false,
        if_needed: false,
        report_status: false,
        quiet: true,
    })
    .unwrap();
    assert!(repaired.repaired);
}

#[test]
fn first_repair_of_a_fresh_package_does_not_count_placeholders_as_changes() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("playkit");
    fs::create_dir(&root).unwrap();
    setup_old_install(&root);

    let report = playkit_updater::update::repair(&playkit_updater::update::RepairOptions {
        root,
        assume_official: false,
        if_needed: true,
        report_status: false,
        quiet: true,
    })
    .unwrap();
    assert!(report.repaired);
    assert!(!report.changed);
}

#[test]
fn repair_refreshes_edited_user_assets_recorded_in_state() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("playkit");
    fs::create_dir(&root).unwrap();
    setup_old_install(&root);

    let state_path = root.join("portable_config/.playkit/state.json");
    let mut state: Value = serde_json::from_str(&fs::read_to_string(&state_path).unwrap()).unwrap();
    state["user_scripts"] = json!(["mine.lua"]);
    write_json(&state_path, &state);
    write_file(
        root.join("portable_config/user/scripts/mine.lua"),
        "-- updated user script\n",
    );

    let report = playkit_updater::update::repair(&playkit_updater::update::RepairOptions {
        root: root.clone(),
        assume_official: false,
        if_needed: true,
        report_status: false,
        quiet: true,
    })
    .unwrap();
    assert!(report.repaired);
    assert!(report.changed);
    assert_eq!(
        fs::read_to_string(root.join("portable_config/scripts/mine.lua")).unwrap(),
        "-- updated user script\n"
    );
}

#[test]
fn user_script_opts_are_copied_but_a_same_name_official_file_blocks_the_update() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("playkit");
    fs::create_dir(&root).unwrap();
    setup_old_install(&root);
    write_file(
        root.join("portable_config/user/script-opts/mine.conf"),
        "custom=yes\n",
    );
    let release = make_release(&dir.path().join("release"), NEW_VERSION);

    install(&install_options(&root, &release, UnmanagedPolicy::Capture)).unwrap();
    assert_eq!(
        fs::read_to_string(root.join("portable_config/script-opts/mine.conf")).unwrap(),
        "custom=yes\n"
    );

    let dir = tempdir().unwrap();
    let root = dir.path().join("playkit");
    fs::create_dir(&root).unwrap();
    setup_old_install(&root);
    write_file(
        root.join("portable_config/user/script-opts/update-check.conf"),
        "enabled=yes\n",
    );
    let release = make_release(&dir.path().join("release"), NEW_VERSION);
    let error = install(&install_options(&root, &release, UnmanagedPolicy::Capture)).unwrap_err();
    assert_eq!(error.kind, playkit_updater::util::ErrorKind::Blocked);
    assert_eq!(read_version(&root), OLD_VERSION);
}

#[test]
fn capture_command_writes_auto_patches_without_swapping() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("playkit");
    fs::create_dir(&root).unwrap();
    setup_old_install(&root);
    write_file(
        root.join("portable_config/mpv.conf"),
        "volume=55\nosc=yes\n",
    );

    let report = capture(&CaptureOptions {
        root: root.clone(),
        dry_run: false,
        quiet: true,
    })
    .unwrap();
    assert_eq!(report.targets.get("mpv.conf"), Some(&1));
    assert_eq!(read_version(&root), OLD_VERSION);
    let patch =
        fs::read_to_string(root.join("portable_config/user/patches/auto/v2/mpv.conf.patch"))
            .unwrap();
    assert!(patch.contains("schema=2"), "{patch}");
    assert!(patch.contains("volume=55"), "{patch}");
    let status = read_status_file(&root).unwrap().unwrap();
    assert_eq!(status.state, "captured");
}

#[test]
fn rollback_without_previous_is_blocked() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("playkit");
    fs::create_dir(&root).unwrap();
    setup_old_install(&root);
    let error = rollback(&RollbackOptions {
        root: root.clone(),
        wait_pid: 0,
        wait_timeout: Duration::from_secs(5),
        relaunch: RelaunchPolicy::No,
        quiet: true,
    })
    .unwrap_err();
    assert_eq!(error.kind, playkit_updater::util::ErrorKind::Blocked);
    assert_eq!(read_version(&root), OLD_VERSION);
}

#[test]
fn release_without_the_running_platform_is_rejected() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("playkit");
    fs::create_dir(&root).unwrap();
    setup_old_install(&root);
    let release_dir = dir.path().join("release");
    let package = release_dir.join("package");
    write_package(&package, NEW_VERSION, true);
    let archive = archive_bytes(&package, NEW_VERSION, None);
    let release = write_release(&release_dir, NEW_VERSION, archive, other_platform());

    let error = install(&install_options(&root, &release, UnmanagedPolicy::Capture)).unwrap_err();
    assert_eq!(error.kind, playkit_updater::util::ErrorKind::Failed);
    assert_eq!(read_version(&root), OLD_VERSION);
    assert!(!previous_path(&root).exists());
}
