//! The install/capture/repair/rollback pipelines.
//!
//! Design rule: a complete, verified package is staged first; the current
//! installation is untouched until the very last rename.  User intent lives in
//! semantic patches and is re-applied to the official configuration of the new
//! version, never carried over as a frozen copy of an old config.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::archive;
use crate::config::{capture_patch, managed_kind, verify_capture, CapturedPatch, ConfigDoc};
use crate::download::{
    fetch_metadata, sha256_of, verify_downloaded, DirFetcher, Fetcher, HttpFetcher, ReleaseLocation,
};
use crate::manifest::{
    expected_asset_name, normalize_tag, parse_sha256sums, ReleaseManifest, MAX_ARCHIVE_BYTES,
    MAX_METADATA_BYTES,
};
use crate::patch::{PatchDoc, PatchItem, PatchOp};
use crate::state::{
    auto_patch_dir, auto_v2_patch_dir, discover_managed, generated_dir, managed_targets,
    new_package_state, official_dir, patch_dir, patch_targets, state_path, user_layer_fingerprint,
    v2_patch_dir, State, STATE_SCHEMA,
};
use crate::status::{Logger, Reporter};
use crate::swap::{
    acquire_lock, check_writable, cleanup_stale_temp_dirs, perform_rollback, perform_swap,
    prepare_staging, relaunch, try_acquire_lock, wait_for_process, Layout,
};
use crate::util::{
    atomic_write, compare_versions, copy_file, copy_tree_merge, ensure_dir, human_size, io_context,
    is_valid_repository, read_optional_json, remove_dir_all_retry, sha256_file, utc_now_iso,
    Failure, Result,
};

const MAX_PRESERVED_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnmanagedPolicy {
    Capture,
    Discard,
    Abort,
}

impl UnmanagedPolicy {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "capture" => Ok(Self::Capture),
            "discard" => Ok(Self::Discard),
            "abort" => Ok(Self::Abort),
            _ => Err(Failure::usage(format!(
                "--unmanaged must be capture, discard or abort (got {value:?})"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelaunchPolicy {
    Auto,
    Yes,
    No,
}

impl RelaunchPolicy {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "auto" => Ok(Self::Auto),
            "yes" => Ok(Self::Yes),
            "no" => Ok(Self::No),
            _ => Err(Failure::usage(format!(
                "--relaunch must be auto, yes or no (got {value:?})"
            ))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub root: PathBuf,
    pub repository: String,
    pub tag: String,
    pub platform: Option<String>,
    pub wait_pid: u32,
    pub wait_timeout: Duration,
    pub unmanaged: UnmanagedPolicy,
    pub relaunch: RelaunchPolicy,
    pub source_dir: Option<PathBuf>,
    pub quiet: bool,
}

#[derive(Debug, Clone)]
pub struct InstallReport {
    pub version: String,
    pub previous_version: String,
    pub warnings: Vec<String>,
    pub relaunched: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inventory {
    pub schema: u32,
    #[serde(default)]
    pub files: BTreeMap<String, String>,
}

pub fn install(options: &InstallOptions) -> Result<InstallReport> {
    if !is_valid_repository(&options.repository) {
        return Err(Failure::usage(format!(
            "repository must be owner/name (got {:?})",
            options.repository
        )));
    }
    let layout = Layout::new(&options.root)?;
    let logger = Logger::new(layout.log_path(), options.quiet);
    let mut reporter = Reporter::new(&layout.root, None, logger);
    reporter.set("preparing", "正在准备更新。", None);
    let result = install_inner(options, &layout, &mut reporter);
    match &result {
        Ok(report) => {
            reporter.finish("installed", format!("已更新到 {}。", report.version), None);
        }
        Err(error) => {
            let state = if error.kind == crate::util::ErrorKind::Blocked {
                "blocked"
            } else {
                "failed"
            };
            reporter.finish(state, error.user.clone(), Some(error.detail.clone()));
        }
    }
    result
}

fn install_inner(
    options: &InstallOptions,
    layout: &Layout,
    reporter: &mut Reporter,
) -> Result<InstallReport> {
    check_writable(&layout.root)?;
    check_writable(&layout.parent)?;
    let _lock = acquire_lock(layout)?;
    cleanup_stale_temp_dirs();
    let platform = detect_platform(&layout.root, options.platform.as_deref())?;
    let current_version = read_install_version(&layout.root, platform)?;
    let target_version = normalize_tag(&options.tag);
    if crate::util::version_parts(&target_version).is_none() {
        return Err(Failure::failed(
            "更新版本号无效。",
            format!("tag {:?}", options.tag),
        ));
    }
    match compare_versions(&target_version, &current_version) {
        Some(std::cmp::Ordering::Equal) => {
            return Err(Failure::blocked(
                "已经是最新版本。",
                format!("installed {current_version}"),
            ))
        }
        Some(std::cmp::Ordering::Less) => {
            return Err(Failure::blocked(
                "自动更新不支持降级；如需回退请使用回滚功能。",
                format!("installed {current_version}, target {target_version}"),
            ))
        }
        Some(std::cmp::Ordering::Greater) => {}
        None => {
            return Err(Failure::blocked(
                "无法识别当前安装版本，自动更新已停止。",
                format!("installed version {current_version:?}"),
            ))
        }
    }
    reporter.logger().info(format!(
        "install {} -> {} on {platform} (repository {})",
        current_version, target_version, options.repository
    ));
    reporter.set(
        "preparing",
        format!("准备从 {current_version} 更新到 {target_version}。"),
        None,
    );
    let staging = prepare_staging(layout)?;
    let result = install_stage(
        options,
        layout,
        reporter,
        platform,
        &current_version,
        &target_version,
        &staging,
    );
    if result.is_err() {
        let _ = remove_dir_all_retry(&staging);
    }
    result
}

fn install_stage(
    options: &InstallOptions,
    layout: &Layout,
    reporter: &mut Reporter,
    platform: &str,
    current_version: &str,
    target_version: &str,
    staging: &Path,
) -> Result<InstallReport> {
    let location = ReleaseLocation::new(&options.repository, &options.tag)?;
    let fetcher: Box<dyn Fetcher> = match &options.source_dir {
        Some(dir) => Box::new(DirFetcher::new(dir.clone())),
        None => {
            let http = HttpFetcher::new(location)?;
            if let Some(note) = http.proxy_note() {
                reporter.logger().info(format!("using proxy {note}"));
            }
            Box::new(http)
        }
    };
    let downloads = staging.join("downloads");
    ensure_dir(&downloads)?;

    reporter.set("downloading", "正在获取 Release 元数据。", None);
    let manifest_bytes = fetch_metadata(
        fetcher.as_ref(),
        "manifest.json",
        &downloads.join("manifest.json"),
        MAX_METADATA_BYTES,
    )?;
    let manifest = ReleaseManifest::parse(&manifest_bytes)?;
    manifest.check_identity(&options.repository, &options.tag)?;
    if manifest.version != target_version {
        return Err(Failure::failed(
            "Release 版本与请求不一致。",
            format!("manifest {}, requested {target_version}", manifest.version),
        ));
    }
    let platform_entry = manifest.platform(platform)?.clone();
    let asset = expected_asset_name(target_version, platform)?;
    if platform_entry.asset != asset {
        return Err(Failure::failed(
            "Release 资产名与当前平台不一致。",
            format!("manifest {:?}, expected {asset:?}", platform_entry.asset),
        ));
    }
    let expected_format = if platform == "windows-x64" {
        "zip"
    } else {
        "tar.gz"
    };
    if let Some(format) = &platform_entry.format {
        if format != expected_format {
            return Err(Failure::failed(
                "Release 资产格式与当前平台不一致。",
                format!("manifest format {format:?}, expected {expected_format:?}"),
            ));
        }
    }

    reporter.set("downloading", "正在获取 SHA256SUMS。", None);
    let sums_bytes = fetch_metadata(
        fetcher.as_ref(),
        "SHA256SUMS",
        &downloads.join("SHA256SUMS"),
        MAX_METADATA_BYTES,
    )?;
    let sums_text = std::str::from_utf8(&sums_bytes)
        .map_err(|error| Failure::failed("SHA256SUMS 不是有效的 UTF-8。", format!("{error}")))?;
    let sums = parse_sha256sums(sums_text)?;
    let manifest_hash = sha256_of(&manifest_bytes);
    match sums.get("manifest.json") {
        Some(expected) if expected.eq_ignore_ascii_case(&manifest_hash) => {}
        Some(expected) => {
            return Err(Failure::failed(
                "Release 清单校验失败，更新已中止。",
                format!("manifest.json sha256 {manifest_hash}, SHA256SUMS {expected}"),
            ))
        }
        None => {
            return Err(Failure::failed(
                "SHA256SUMS 缺少 manifest.json。",
                "incomplete release metadata",
            ))
        }
    }
    match sums.get(&platform_entry.asset) {
        Some(expected) if expected.eq_ignore_ascii_case(&platform_entry.sha256) => {}
        Some(expected) => {
            return Err(Failure::failed(
                "Release 元数据中的安装包校验和不一致。",
                format!(
                    "{}: manifest {}, SHA256SUMS {expected}",
                    platform_entry.asset, platform_entry.sha256
                ),
            ))
        }
        None => {
            return Err(Failure::failed(
                "SHA256SUMS 缺少安装包校验和。",
                platform_entry.asset.to_string(),
            ))
        }
    }

    reporter.set(
        "downloading",
        format!(
            "正在下载 {}（{}）。",
            platform_entry.asset,
            human_size(platform_entry.size)
        ),
        None,
    );
    let archive_path = downloads.join(&platform_entry.asset);
    fetcher.fetch(
        &platform_entry.asset,
        &archive_path,
        MAX_ARCHIVE_BYTES,
        &mut |done, total| reporter.progress(done, total),
    )?;
    reporter.set("verifying", "正在校验下载的安装包。", None);
    verify_downloaded(
        &archive_path,
        &platform_entry.sha256,
        Some(platform_entry.size),
    )?;

    reporter.set("verifying", "正在检查安装包结构。", None);
    let package_parent = staging.join("package");
    let top = archive::extract_package(&archive_path, &package_parent, target_version, platform)?;
    archive::verify_package_tree(&top, &manifest, platform, &platform_entry.asset)?;

    reporter.set("ready", "更新已下载并校验，退出播放器后自动安装。", None);
    if options.wait_pid != 0 {
        reporter.set("waiting", "等待播放器退出后安装更新。", None);
        wait_for_process(options.wait_pid, options.wait_timeout)?;
    }

    reporter.set("staging", "正在准备新版本与用户配置。", None);
    let current_portable = layout.root.join("portable_config");
    let staged_portable = top.join("portable_config");
    let old_state = State::load(&current_portable)?;
    let old_inventory = read_inventory(&layout.root)?;
    let new_inventory = read_inventory(&top)?;
    let mut warnings: Vec<String> = Vec::new();

    let captured = capture_user_edits(
        &CaptureContext {
            current_portable: &current_portable,
            target_user_dir: &staged_portable.join("user"),
            fallback_portable: &staged_portable,
            state_old: old_state.as_ref(),
            policy: options.unmanaged,
            version_label: current_version,
        },
        &mut warnings,
    )?;
    preserve_runtime(
        &current_portable,
        &staged_portable,
        old_inventory.as_ref(),
        new_inventory.as_ref(),
        &mut warnings,
    )?;
    // Capture runs against the current installation, but the patch files must
    // be written after the user layer has been copied into staging.  Writing
    // earlier lets the copy overwrite the fresh capture with an older one.
    merge_auto_patches(&staged_portable.join("user"), &captured)?;
    let user_assets = install_user_assets(&staged_portable, None, &mut warnings)?;
    let applied = apply_patches(
        &staged_portable,
        &staged_portable.join("user"),
        &mut warnings,
    )?;
    finalize_state(
        &staged_portable,
        target_version,
        current_version,
        platform,
        &applied,
        &user_assets,
        &warnings,
    )?;
    if !warnings.is_empty() {
        reporter.warnings(&warnings);
    }

    // The user may have installed or repaired the directory while the download
    // was running.  The lock only covers our own helper processes.
    let recheck = read_install_version(&layout.root, platform)?;
    if recheck != current_version {
        return Err(Failure::blocked(
            "安装目录在更新过程中发生了变化，更新已停止。",
            format!("expected {current_version}, found {recheck}"),
        ));
    }

    copy_log_to_staged(layout, &staged_portable);
    reporter.set("installing", "正在替换安装目录。", None);
    perform_swap(layout, &top)?;
    let _ = remove_dir_all_retry(staging);
    reporter.finish("installed", format!("已更新到 {target_version}。"), None);

    let should_relaunch = match options.relaunch {
        RelaunchPolicy::Yes => true,
        RelaunchPolicy::No => false,
        RelaunchPolicy::Auto => options.wait_pid != 0,
    };
    let mut relaunched = false;
    if should_relaunch {
        match relaunch(layout, platform) {
            Ok(()) => relaunched = true,
            Err(error) => {
                warnings.push(error.user.clone());
                reporter.finish(
                    "installed",
                    "更新已安装，请手动启动播放器。",
                    Some(error.detail),
                );
            }
        }
    }
    Ok(InstallReport {
        version: target_version.to_string(),
        previous_version: current_version.to_string(),
        warnings,
        relaunched,
    })
}

fn detect_platform(root: &Path, requested: Option<&str>) -> Result<&'static str> {
    if std::env::consts::ARCH != "x86_64" {
        return Err(Failure::blocked(
            "当前 CPU 架构不受支持。",
            format!("arch {}", std::env::consts::ARCH),
        ));
    }
    let (platform, marker) = if cfg!(windows) {
        ("windows-x64", "mpv.exe")
    } else if cfg!(target_os = "linux") {
        ("linux-x64", "mpv-playkit")
    } else {
        return Err(Failure::blocked(
            "当前操作系统不受支持。",
            std::env::consts::OS.to_string(),
        ));
    };
    if !root.join(marker).is_file() {
        return Err(Failure::blocked(
            "无法识别 PlayKit 安装目录。",
            format!("{} is missing", root.join(marker).display()),
        ));
    }
    if let Some(requested) = requested {
        if requested != platform {
            return Err(Failure::blocked(
                "平台参数与当前系统不一致。",
                format!("requested {requested}, running {platform}"),
            ));
        }
    }
    Ok(platform)
}

fn read_install_version(root: &Path, platform: &str) -> Result<String> {
    let path = root.join("playkit-version.json");
    let data = fs::read(&path).map_err(|error| {
        Failure::blocked(
            "当前安装缺少版本信息。",
            format!("{}: {error}", path.display()),
        )
    })?;
    let info = crate::manifest::VersionInfo::parse(&data, "playkit-version.json")?;
    if info.platform != platform {
        return Err(Failure::blocked(
            "当前安装的平台与系统不一致。",
            format!("{} says {}", path.display(), info.platform),
        ));
    }
    if crate::util::version_parts(&info.version).is_none() {
        return Err(Failure::blocked(
            "当前安装的版本号无效。",
            format!("{}: {:?}", path.display(), info.version),
        ));
    }
    Ok(info.version)
}

fn read_inventory(root: &Path) -> Result<Option<Inventory>> {
    let path = root.join("playkit-inventory.json");
    if !path.is_file() {
        return Ok(None);
    }
    let inventory: Inventory = read_optional_json(&path)?
        .ok_or_else(|| Failure::failed("读取安装包清单失败。", format!("{}", path.display())))?;
    if inventory.schema != 1 {
        return Err(Failure::failed(
            "安装包清单版本不受支持。",
            format!("{}: schema {}", path.display(), inventory.schema),
        ));
    }
    Ok(Some(inventory))
}

struct CaptureContext<'a> {
    current_portable: &'a Path,
    target_user_dir: &'a Path,
    fallback_portable: &'a Path,
    state_old: Option<&'a State>,
    policy: UnmanagedPolicy,
    version_label: &'a str,
}

fn capture_user_edits(
    context: &CaptureContext<'_>,
    warnings: &mut Vec<String>,
) -> Result<BTreeMap<String, CapturedPatch>> {
    let mut targets: BTreeSet<String> =
        managed_targets(context.current_portable, context.state_old)
            .into_iter()
            .collect();
    targets.extend(discover_managed(context.fallback_portable));
    let mut captured: BTreeMap<String, CapturedPatch> = BTreeMap::new();
    let mut conflicts: Vec<String> = Vec::new();
    let mut migration_warned = false;

    for target in targets {
        let Some(kind) = managed_kind(&target) else {
            continue;
        };
        let current_path = context.current_portable.join(&target);
        let current_exists = current_path.is_file();
        let current_hash = if current_exists {
            Some(sha256_file(&current_path)?)
        } else {
            None
        };
        let generated_hash = context
            .state_old
            .and_then(|state| state.generated.get(&target))
            .cloned();
        let baseline_path = baseline_for_target(context, &target);
        let changed = match &generated_hash {
            Some(generated) => current_hash.as_deref() != Some(generated.as_str()),
            None => {
                if !current_exists {
                    false
                } else if let Some(baseline) = &baseline_path {
                    current_hash.as_deref() != Some(sha256_file(baseline)?.as_str())
                } else {
                    true
                }
            }
        };
        if !changed {
            continue;
        }
        if context.state_old.is_none() && context.policy == UnmanagedPolicy::Capture {
            // There is no generated/official snapshot to compare against, so
            // differences include both user edits and changes made by the new
            // official configuration.  Turning those into patches would freeze
            // old official values.  Preserve the old files as a backup and
            // start from the new official configuration instead.
            if current_exists {
                let backup = context
                    .target_user_dir
                    .join("backup")
                    .join(format!("pre-update-{}", context.version_label))
                    .join(&target);
                if !backup.exists() {
                    copy_file(&current_path, &backup)?;
                }
            }
            if !migration_warned {
                warnings.push(
                    "旧安装没有配置基线，无法区分官方变更与用户修改；已备份原配置并使用新版本官方配置。"
                        .to_string(),
                );
                migration_warned = true;
            }
            continue;
        }
        match context.policy {
            UnmanagedPolicy::Abort => {
                conflicts.push(target);
                continue;
            }
            UnmanagedPolicy::Discard => {
                warnings.push(format!("检测到未托管的配置修改，已按设置忽略：{target}"));
                continue;
            }
            UnmanagedPolicy::Capture => {}
        }
        let current_doc = if current_exists {
            ConfigDoc::read(&current_path, kind)?
        } else {
            ConfigDoc::parse("", kind)?
        };
        let baseline_doc = match &baseline_path {
            Some(path) => ConfigDoc::read(path, kind)?,
            None => ConfigDoc::parse("", kind)?,
        };
        let captured_patch = capture_patch(&current_doc, &baseline_doc)?;
        verify_capture(&current_doc, &baseline_doc, &captured_patch)?;
        if current_exists {
            let backup = context
                .target_user_dir
                .join("backup")
                .join(format!("pre-update-{}", context.version_label))
                .join(&target);
            if !backup.exists() {
                copy_file(&current_path, &backup)?;
            }
        }
        if captured_patch.is_empty() {
            warnings.push(format!(
                "{target}: 检测到格式变化，但没有可表达的配置意图；原文件已备份。"
            ));
            continue;
        }
        captured.insert(target, captured_patch);
    }

    if !conflicts.is_empty() {
        return Err(Failure::blocked(
            "检测到配置文件存在未托管修改，更新已停止。",
            format!(
                "changed configs: {} (use --unmanaged capture or discard)",
                conflicts.join(", ")
            ),
        ));
    }
    Ok(captured)
}

fn baseline_for_target(context: &CaptureContext<'_>, target: &str) -> Option<PathBuf> {
    if let Some(state) = context.state_old {
        let generated = state.generated_path(context.current_portable, target);
        if generated.is_file() {
            return Some(generated);
        }
        let official = state.official_path(context.current_portable, target);
        if official.is_file() {
            return Some(official);
        }
    }
    let fallback_official = context
        .fallback_portable
        .join(".playkit")
        .join("official")
        .join(target);
    if fallback_official.is_file() {
        return Some(fallback_official);
    }
    let fallback_config = context.fallback_portable.join(target);
    if fallback_config.is_file() {
        return Some(fallback_config);
    }
    None
}

/// Merge old automatic patches with the newly captured operations and write a
/// schema-2 patch set.  Old schema-1 automatic patches are folded in and then
/// removed so the new helper has a single source of truth; the previous
/// installation directory (used by rollback) still keeps its own copy.
fn merge_auto_patches(user_dir: &Path, captured: &BTreeMap<String, CapturedPatch>) -> Result<()> {
    let auto = user_dir.join("patches").join("auto");
    let auto_v2 = auto.join("v2");
    ensure_dir(&auto_v2)?;

    let mut documents: BTreeMap<String, Vec<PatchItem>> = BTreeMap::new();
    let mut old_schema1: Vec<PathBuf> = Vec::new();
    let mut old_schema1_files: Vec<(PathBuf, String)> = Vec::new();
    collect_auto_files(&auto, Some(&auto_v2), &mut old_schema1_files)?;
    for (path, target) in old_schema1_files {
        let Some(kind) = managed_kind(&target) else {
            continue;
        };
        let doc = PatchDoc::load(&path, &target, kind)?;
        documents.entry(target).or_default().extend(doc.items);
        old_schema1.push(path);
    }
    let mut old_v2_files: Vec<(PathBuf, String)> = Vec::new();
    collect_auto_files(&auto_v2, None, &mut old_v2_files)?;
    for (path, target) in old_v2_files {
        let Some(kind) = managed_kind(&target) else {
            continue;
        };
        let doc = PatchDoc::load(&path, &target, kind)?;
        documents.entry(target).or_default().extend(doc.items);
    }
    for (target, patch) in captured {
        documents
            .entry(target.clone())
            .or_default()
            .extend(patch.items());
    }

    for (target, items) in documents {
        let Some(kind) = managed_kind(&target) else {
            continue;
        };
        let items = compact_auto_items(items, kind);
        let path = auto_v2.join(format!("{target}.patch"));
        if items.is_empty() {
            if path.is_file() {
                fs::remove_file(&path)
                    .map_err(|error| io_context(&format!("remove {}", path.display()), error))?;
            }
            continue;
        }
        let doc = PatchDoc {
            schema: crate::patch::PATCH_SCHEMA,
            target: Some(target.clone()),
            source: format!("auto:{target}"),
            items,
        };
        atomic_write(&path, doc.render().as_bytes())?;
    }
    for path in old_schema1 {
        fs::remove_file(&path)
            .map_err(|error| io_context(&format!("remove {}", path.display()), error))?;
    }
    Ok(())
}

/// Collect `.patch` files under an automatic patch directory.  Targets such as
/// `script-opts/uosc.conf` are stored in nested directories, so a top-level
/// directory scan is not enough.  `skip` excludes the sibling `v2` subtree when
/// walking the schema-1 root.
fn collect_auto_files(
    root: &Path,
    skip: Option<&Path>,
    out: &mut Vec<(PathBuf, String)>,
) -> Result<()> {
    if !root.is_dir() {
        return Ok(());
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|error| io_context(&format!("read {}", directory.display()), error))?;
        for entry in entries {
            let entry = entry
                .map_err(|error| io_context(&format!("read {}", directory.display()), error))?;
            let path = entry.path();
            if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                if skip.map(|skip| path == skip).unwrap_or(false) {
                    continue;
                }
                pending.push(path);
                continue;
            }
            if !entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false)
            {
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("patch") {
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|error| Failure::internal(format!("auto patch path: {error}")))?;
            let text = relative.to_string_lossy().replace('\\', "/");
            let Some(target) = text.strip_suffix(".patch") else {
                continue;
            };
            if managed_kind(target).is_some() {
                out.push((path, target.to_string()));
            }
        }
    }
    out.sort_by(|left, right| left.1.cmp(&right.1));
    Ok(())
}

/// Automatic patches are generated, so repeated captures must not grow without
/// bound.  A later key operation fully determines the final value; keep only
/// the last one per key.  Line operations stay in order because they describe a
/// sequence of edits.  `+key` (append) is preserved verbatim.
fn compact_auto_items(items: Vec<PatchItem>, kind: crate::config::ConfigKind) -> Vec<PatchItem> {
    let mut last_key: BTreeMap<(Option<String>, String), usize> = BTreeMap::new();
    for (index, item) in items.iter().enumerate() {
        if let PatchItem::Key(entry) = item {
            if entry.op != PatchOp::Append {
                last_key.insert(
                    (entry.section.clone(), normalize_patch_key(kind, &entry.key)),
                    index,
                );
            }
        }
    }
    items
        .into_iter()
        .enumerate()
        .filter_map(|(index, item)| match &item {
            PatchItem::Key(entry) if entry.op != PatchOp::Append => {
                let lookup = (entry.section.clone(), normalize_patch_key(kind, &entry.key));
                (last_key.get(&lookup).copied() == Some(index)).then_some(item)
            }
            _ => Some(item),
        })
        .collect()
}

fn normalize_patch_key(kind: crate::config::ConfigKind, key: &str) -> String {
    if kind == crate::config::ConfigKind::Input {
        key.to_ascii_lowercase()
    } else {
        key.to_string()
    }
}

fn preserve_runtime(
    current_portable: &Path,
    staged_portable: &Path,
    old_inventory: Option<&Inventory>,
    new_inventory: Option<&Inventory>,
    warnings: &mut Vec<String>,
) -> Result<()> {
    // The new archive ships an empty user layer (documentation only).  Carry
    // the existing user-owned subtrees over before applying patches and
    // copying user scripts/fonts/script-opts into place.  `user/` is never
    // deleted by the updater.
    let current_user = current_portable.join("user");
    if current_user.is_dir() {
        let staged_user = staged_portable.join("user");
        for name in ["patches", "scripts", "fonts", "script-opts", "backup"] {
            let source = current_user.join(name);
            if source.is_dir() {
                copy_tree_merge(&source, &staged_user.join(name), current_portable)?;
            }
        }
    }
    for relative in ["update-check-state.json", "saved-props.json"] {
        let source = current_portable.join(relative);
        if source.is_file() {
            copy_file(&source, &staged_portable.join(relative))?;
        }
    }
    for relative in ["watch_later", "_cache/watch_later"] {
        let source = current_portable.join(relative);
        if source.is_dir() {
            copy_tree_merge(&source, &staged_portable.join(relative), current_portable)?;
        }
    }
    let history = current_portable.join("_cache").join("command_history.txt");
    if history.is_file() {
        copy_file(
            &history,
            &staged_portable.join("_cache/command_history.txt"),
        )?;
    }

    let Some(new_inventory) = new_inventory else {
        warnings.push("更新包缺少 playkit-inventory.json，跳过非官方文件的自动迁移。".to_string());
        return Ok(());
    };
    let old_files: BTreeSet<String> = old_inventory
        .map(|inventory| inventory.files.keys().cloned().collect())
        .unwrap_or_default();
    let new_files: BTreeSet<String> = new_inventory.files.keys().cloned().collect();
    let mut copied: u64 = 0;
    let mut pending = vec![current_portable.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|error| io_context(&format!("read {}", directory.display()), error))?;
        for entry in entries {
            let entry = entry
                .map_err(|error| io_context(&format!("read {}", directory.display()), error))?;
            let path = entry.path();
            let relative = path
                .strip_prefix(current_portable)
                .map_err(|error| Failure::internal(format!("relative path: {error}")))?
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
            if metadata.file_type().is_symlink() {
                warnings.push(format!("跳过符号链接：{relative}"));
                continue;
            }
            if metadata.is_dir() {
                pending.push(path);
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            if new_files.contains(&relative) || old_files.contains(&relative) {
                continue;
            }
            let destination = if let Some(rest) = relative.strip_prefix("scripts/") {
                staged_portable.join("user").join("scripts").join(rest)
            } else if let Some(rest) = relative.strip_prefix("fonts/") {
                staged_portable.join("user").join("fonts").join(rest)
            } else if let Some(rest) = relative.strip_prefix("script-opts/") {
                staged_portable.join("user").join("script-opts").join(rest)
            } else {
                staged_portable.join(&relative)
            };
            if destination.exists() {
                if destination.is_file()
                    && crate::util::sha256_file(&path)? == crate::util::sha256_file(&destination)?
                {
                    // Runtime state and user scripts were already copied above;
                    // an identical copy is not a conflict.
                    continue;
                }
                warnings.push(format!("保留非官方文件时发生同名冲突，已跳过：{relative}"));
                continue;
            }
            copied = copied.saturating_add(metadata.len());
            if copied > MAX_PRESERVED_BYTES {
                return Err(Failure::blocked(
                    "非官方文件过多，更新已停止。",
                    format!("more than {MAX_PRESERVED_BYTES} bytes under portable_config"),
                ));
            }
            copy_file(&path, &destination)?;
            warnings.push(format!(
                "保留了非官方文件：{relative} -> {}",
                destination
                    .strip_prefix(staged_portable)
                    .unwrap_or(&destination)
                    .display()
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Default)]
struct UserAssets {
    scripts: Vec<String>,
    fonts: Vec<String>,
    script_opts: Vec<String>,
    changed: bool,
}

fn install_user_assets(
    portable: &Path,
    old_state: Option<&State>,
    warnings: &mut Vec<String>,
) -> Result<UserAssets> {
    let user = portable.join("user");
    let scripts = old_state.map(|state| state.user_scripts.as_slice());
    let fonts = old_state.map(|state| state.user_fonts.as_slice());
    let script_opts = old_state.map(|state| state.user_script_opts.as_slice());
    let (scripts, scripts_changed) = copy_user_tree(
        &user.join("scripts"),
        &portable.join("scripts"),
        "用户脚本",
        scripts,
        warnings,
    )?;
    let (fonts, fonts_changed) = copy_user_tree(
        &user.join("fonts"),
        &portable.join("fonts"),
        "用户字体",
        fonts,
        warnings,
    )?;
    let (script_opts, script_opts_changed) = copy_user_tree(
        &user.join("script-opts"),
        &portable.join("script-opts"),
        "用户 script-opts",
        script_opts,
        warnings,
    )?;
    Ok(UserAssets {
        scripts,
        fonts,
        script_opts,
        changed: scripts_changed || fonts_changed || script_opts_changed,
    })
}

/// Copy user assets into the generated configuration.  During a fresh install
/// any existing target is an official file, so a same-name user file is a
/// conflict.  During in-place repair, a target listed in the previous state is
/// a copy of this same user asset and may be refreshed after the user edited
/// it.
fn copy_user_tree(
    source: &Path,
    destination: &Path,
    label: &str,
    replaceable: Option<&[String]>,
    warnings: &mut Vec<String>,
) -> Result<(Vec<String>, bool)> {
    let mut result = Vec::new();
    let mut changed = false;
    if !source.is_dir() {
        return Ok((result, changed));
    }
    let mut pending = vec![source.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory)
            .map_err(|error| io_context(&format!("read {}", directory.display()), error))?;
        for entry in entries {
            let entry = entry
                .map_err(|error| io_context(&format!("read {}", directory.display()), error))?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| io_context(&format!("stat {}", path.display()), error))?;
            if metadata.file_type().is_symlink() {
                warnings.push(format!("{label}中的符号链接已跳过：{}", path.display()));
                continue;
            }
            if metadata.is_dir() {
                pending.push(path);
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            let relative = path
                .strip_prefix(source)
                .map_err(|error| Failure::internal(format!("user asset path: {error}")))?;
            let relative_text = relative.to_string_lossy().replace('\\', "/");
            // The package ships README.md placeholders in the user asset
            // directories.  They are documentation, not scripts/fonts/options,
            // and copying them would make the first startup look like a user
            // change.
            if relative_text.eq_ignore_ascii_case("README.md") {
                continue;
            }
            let target = destination.join(relative);
            if target.exists() {
                let listed = replaceable
                    .map(|paths| paths.iter().any(|item| item == &relative_text))
                    .unwrap_or(false);
                // Repair runs against an installation where user assets were
                // already copied once.  An identical file is not a conflict.
                if target.is_file() && sha256_file(&path)? == sha256_file(&target)? {
                    if listed {
                        result.push(relative_text);
                    }
                    continue;
                }
                if listed {
                    copy_file(&path, &target)?;
                    changed = true;
                    result.push(relative_text);
                    continue;
                }
                return Err(Failure::blocked(
                    format!("{label}与官方文件同名，更新已停止。"),
                    format!("{} conflicts with {}", path.display(), target.display()),
                ));
            }
            copy_file(&path, &target)?;
            changed = true;
            result.push(relative_text);
        }
    }
    result.sort();
    Ok((result, changed))
}

fn apply_patches(
    portable: &Path,
    user_dir: &Path,
    warnings: &mut Vec<String>,
) -> Result<BTreeSet<String>> {
    let mut targets: BTreeSet<String> = discover_managed(portable).into_iter().collect();
    targets.extend(crate::state::patch_targets(portable)?);
    let mut applied = BTreeSet::new();
    warn_unknown_patch_files(user_dir, warnings)?;

    for target in targets {
        let Some(kind) = managed_kind(&target) else {
            warnings.push(format!("忽略无法识别的 patch 目标：{target}"));
            continue;
        };
        let config_path = portable.join(&target);
        let official_path = official_dir(portable).join(&target);
        if official_path.is_file() {
            // Always start from the verified official baseline.  This also
            // makes repair idempotent after an installer copied files.
            copy_file(&official_path, &config_path)?;
        } else if config_path.is_file() {
            copy_file(&config_path, &official_path)?;
        } else {
            atomic_write(&official_path, b"")?;
        }
        let mut doc = ConfigDoc::read(&official_path, kind)?;
        let manual = patch_dir(portable).join(format!("{target}.patch"));
        let automatic = auto_patch_dir(portable).join(format!("{target}.patch"));
        let manual_v2 = v2_patch_dir(portable).join(format!("{target}.patch"));
        let automatic_v2 = auto_v2_patch_dir(portable).join(format!("{target}.patch"));
        let mut had_patch = false;
        // Automatic patches capture direct edits to generated files, while
        // manual patches describe the user's long-term intent.  Apply every
        // automatic patch first so an explicit manual patch always wins when
        // both touch the same line or key.
        for patch_path in [&automatic, &automatic_v2, &manual, &manual_v2] {
            if !patch_path.is_file() {
                continue;
            }
            let patch = PatchDoc::load(patch_path, &target, kind)?;
            if patch.is_empty() {
                continue;
            }
            had_patch = true;
            for warning in doc.apply(&patch)? {
                warnings.push(format!("{target}: {warning}"));
            }
        }
        if had_patch {
            applied.insert(target.clone());
        }
        // Without a patch the file was just copied from the official baseline.
        // Avoid rewriting it, so formatting-only changes do not make a repair
        // look like a configuration change.  The fallback still creates a file
        // for an otherwise empty target.
        if had_patch || !config_path.is_file() {
            doc.write(&config_path)?;
        }
        let generated_path = generated_dir(portable).join(&target);
        copy_file(&config_path, &generated_path)?;
        if !official_path.is_file() {
            copy_file(&config_path, &official_path)?;
        }
    }
    Ok(applied)
}

fn warn_unknown_patch_files(user_dir: &Path, warnings: &mut Vec<String>) -> Result<()> {
    let patches = user_dir.join("patches");
    let auto = patches.join("auto");
    let v2 = patches.join("v2");
    let auto_v2 = auto.join("v2");
    for root in [patches.clone(), auto.clone(), v2.clone(), auto_v2.clone()] {
        if !root.is_dir() {
            continue;
        }
        let mut pending = vec![root.clone()];
        while let Some(directory) = pending.pop() {
            let entries = fs::read_dir(&directory)
                .map_err(|error| io_context(&format!("read {}", directory.display()), error))?;
            for entry in entries {
                let entry = entry
                    .map_err(|error| io_context(&format!("read {}", directory.display()), error))?;
                let path = entry.path();
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    // `auto` and both `v2` directories are walked as their own
                    // roots with their own relative paths.
                    if path == auto || path == v2 || path == auto_v2 {
                        continue;
                    }
                    pending.push(path);
                    continue;
                }
                if path.extension().and_then(|ext| ext.to_str()) != Some("patch") {
                    continue;
                }
                let relative = path
                    .strip_prefix(&root)
                    .map_err(|error| Failure::internal(format!("patch path: {error}")))?;
                let text = relative.to_string_lossy().replace('\\', "/");
                let Some(target) = text.strip_suffix(".patch") else {
                    continue;
                };
                if managed_kind(target).is_none() {
                    warnings.push(format!("忽略未知目标的 patch：{}", path.display()));
                }
            }
        }
    }
    Ok(())
}

fn finalize_state(
    portable: &Path,
    version: &str,
    previous_version: &str,
    platform: &str,
    applied: &BTreeSet<String>,
    assets: &UserAssets,
    warnings: &[String],
) -> Result<()> {
    let mut state = State::load(portable)?.unwrap_or_else(|| new_package_state(version, platform));
    state.schema = STATE_SCHEMA;
    state.installed_version = version.to_string();
    state.platform = platform.to_string();
    state.installed_at = utc_now_iso();
    state.previous_version = Some(previous_version.to_string());
    state.updated_by = Some(format!("playkit-updater/{}", env!("CARGO_PKG_VERSION")));
    state.patches = applied.iter().cloned().collect();
    state.patch_warnings = warnings.to_vec();
    state.user_scripts = assets.scripts.clone();
    state.user_fonts = assets.fonts.clone();
    state.user_script_opts = assets.script_opts.clone();

    let mut official_hashes = BTreeMap::new();
    let mut generated_hashes = BTreeMap::new();
    let mut targets: BTreeSet<String> = discover_managed(portable).into_iter().collect();
    targets.extend(state.official.keys().cloned());
    for target in targets {
        if managed_kind(&target).is_none() {
            continue;
        }
        let config_path = portable.join(&target);
        let official_path = official_dir(portable).join(&target);
        if !official_path.is_file() {
            if config_path.is_file() {
                copy_file(&config_path, &official_path)?;
            } else {
                continue;
            }
        }
        official_hashes.insert(target.clone(), sha256_file(&official_path)?);
        if config_path.is_file() {
            generated_hashes.insert(target.clone(), sha256_file(&config_path)?);
            copy_file(&config_path, &generated_dir(portable).join(&target))?;
        }
    }
    state.official = official_hashes;
    state.generated = generated_hashes;
    state.user_fingerprint = Some(user_layer_fingerprint(portable, version, platform)?);
    state.save(portable)
}

fn copy_log_to_staged(layout: &Layout, staged_portable: &Path) {
    let destination = staged_portable.join(".playkit").join("updater.log");
    let mut data = Vec::new();
    if let Ok(bytes) = fs::read(layout.log_path()) {
        data.extend_from_slice(&bytes);
    }
    data.extend_from_slice(
        format!(
            "[{}] [info] staged new installation; starting transaction\n",
            utc_now_iso()
        )
        .as_bytes(),
    );
    let _ = atomic_write(&destination, &data);
}

#[derive(Debug, Clone)]
pub struct CaptureOptions {
    pub root: PathBuf,
    pub dry_run: bool,
    pub quiet: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CaptureReport {
    pub targets: BTreeMap<String, usize>,
    pub warnings: Vec<String>,
    pub dry_run: bool,
}

pub fn capture(options: &CaptureOptions) -> Result<CaptureReport> {
    let layout = Layout::new(&options.root)?;
    let logger = Logger::new(layout.log_path(), options.quiet);
    let mut reporter = Reporter::new(&layout.root, None, logger);
    reporter.set("capturing", "正在捕获用户配置修改。", None);
    let result = capture_inner(options, &layout);
    match &result {
        Ok(report) => reporter.finish(
            "captured",
            format!("已捕获 {} 个配置文件的用户修改。", report.targets.len()),
            None,
        ),
        Err(error) => reporter.finish("failed", error.user.clone(), Some(error.detail.clone())),
    }
    result
}

fn capture_inner(options: &CaptureOptions, layout: &Layout) -> Result<CaptureReport> {
    let _lock = acquire_lock(layout)?;
    let platform = detect_platform(&layout.root, None)?;
    let version = read_install_version(&layout.root, platform)?;
    let portable = layout.root.join("portable_config");
    let state = State::load(&portable)?;
    let has_baseline = state.is_some()
        || portable.join(".playkit").join("official").is_dir()
        || portable.join(".playkit").join("generated").is_dir();
    if !has_baseline {
        return Err(Failure::blocked(
            "没有可用的历史基线，无法安全判断用户修改。",
            "missing .playkit state; reinstall this version once to create a baseline",
        ));
    }
    let mut warnings = Vec::new();
    let captured = capture_user_edits(
        &CaptureContext {
            current_portable: &portable,
            target_user_dir: &portable.join("user"),
            fallback_portable: &portable,
            state_old: state.as_ref(),
            policy: UnmanagedPolicy::Capture,
            version_label: &version,
        },
        &mut warnings,
    )?;
    if !options.dry_run {
        merge_auto_patches(&portable.join("user"), &captured)?;
        refresh_after_capture(&portable, &version, platform)?;
    }
    Ok(CaptureReport {
        targets: captured
            .iter()
            .map(|(target, patch)| (target.clone(), patch.len()))
            .collect(),
        warnings,
        dry_run: options.dry_run,
    })
}

/// Capture records the current files as the new generated baseline.  Without
/// this, the next repair would see a file that differs from both the official
/// baseline and the old generated snapshot and refuse to run, even though the
/// difference was just captured into an automatic patch.
fn refresh_after_capture(portable: &Path, version: &str, platform: &str) -> Result<()> {
    let Some(mut state) = State::load(portable)? else {
        return Ok(());
    };
    let mut generated_hashes = BTreeMap::new();
    for target in discover_managed(portable) {
        let config_path = portable.join(&target);
        if !config_path.is_file() {
            continue;
        }
        generated_hashes.insert(target.clone(), sha256_file(&config_path)?);
        copy_file(&config_path, &generated_dir(portable).join(&target))?;
    }
    state.generated = generated_hashes;
    state.patches = patch_targets(portable)?;
    state.user_fingerprint = Some(user_layer_fingerprint(portable, version, platform)?);
    state.save(portable)
}

#[derive(Debug, Clone)]
pub struct RepairOptions {
    pub root: PathBuf,
    pub assume_official: bool,
    /// Skip the whole repair when the user layer fingerprint still matches the
    /// state written by the last install, repair or capture.
    pub if_needed: bool,
    /// Write `update-status.json`.  Automatic startup repair disables this so
    /// it cannot overwrite the status of a user-initiated install/rollback.
    pub report_status: bool,
    pub quiet: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepairReport {
    pub repaired: bool,
    /// True when repair changed generated configuration files or copied a new
    /// version of a user asset.  The Lua script uses this to decide whether an
    /// automatic restart would load anything new.
    pub changed: bool,
}

pub fn repair(options: &RepairOptions) -> Result<RepairReport> {
    let layout = Layout::new(&options.root)?;
    let logger = Logger::new(layout.log_path(), options.quiet);
    if !options.report_status {
        let result = repair_inner(options, &layout);
        match &result {
            Ok(report) if report.repaired && report.changed => {
                logger.info("repair-auto: changes applied")
            }
            Ok(report) if report.repaired => logger.info("repair-auto: no changes applied"),
            Ok(_) => logger.info("repair-auto: up to date"),
            Err(error) => logger.error(format!(
                "repair-auto failed: {} ({})",
                error.user, error.detail
            )),
        }
        return result;
    }
    let mut reporter = Reporter::new(&layout.root, None, logger);
    reporter.set("repairing", "正在重新应用配置 patch。", None);
    let result = repair_inner(options, &layout);
    match &result {
        Ok(report) if !report.repaired => {
            reporter.finish("up-to-date", "配置 patch 无需重新应用。", None)
        }
        Ok(_) => reporter.finish(
            "repaired",
            "配置 patch 已重新应用；重启播放器后生效。",
            None,
        ),
        Err(error) => reporter.finish("failed", error.user.clone(), Some(error.detail.clone())),
    }
    result
}

fn repair_inner(options: &RepairOptions, layout: &Layout) -> Result<RepairReport> {
    let platform = detect_platform(&layout.root, None)?;
    let version = read_install_version(&layout.root, platform)?;
    let portable = layout.root.join("portable_config");
    if options.if_needed {
        if let Some(state) = State::load(&portable)? {
            let fingerprint = user_layer_fingerprint(&portable, &version, platform)?;
            if state.user_fingerprint.as_deref() == Some(fingerprint.as_str())
                && state.installed_version == version
                && state.platform == platform
                && state_has_snapshots(&portable, &state)
            {
                return Ok(RepairReport {
                    repaired: false,
                    changed: false,
                });
            }
        }
    }
    let _lock = match try_acquire_lock(layout)? {
        Some(lock) => lock,
        None if options.if_needed => {
            // An install or rollback owns the lock.  It will write a fresh
            // fingerprint itself; do not interfere or report a failure.
            return Ok(RepairReport {
                repaired: false,
                changed: false,
            });
        }
        None => {
            return Err(Failure::blocked(
                "另一个更新正在进行，请稍后重试。",
                format!("{}: lock is held", layout.lock_path().display()),
            ))
        }
    };
    let old_state = State::load(&portable)?;
    let mut warnings = Vec::new();
    let timestamp = utc_now_iso().replace([':', '-'], "");
    let mut targets: BTreeSet<String> = discover_managed(&portable).into_iter().collect();
    targets.extend(patch_targets(&portable)?);
    if let Some(state) = &old_state {
        targets.extend(state.official.keys().cloned());
        targets.extend(state.generated.keys().cloned());
    }
    let before = managed_config_hashes(&portable, &targets)?;
    for target in &targets {
        let Some(kind) = managed_kind(target) else {
            continue;
        };
        let config_path = portable.join(target);
        let official_path = official_dir(&portable).join(target);
        if !config_path.is_file() || !official_path.is_file() {
            continue;
        }
        let current = ConfigDoc::read(&config_path, kind)?;
        let official = ConfigDoc::read(&official_path, kind)?;
        let differs_from_official = current.render() != official.render();
        let differs_from_generated = old_state
            .as_ref()
            .and_then(|state| state.generated.get(target))
            .map(|hash| sha256_file(&config_path).map(|actual| &actual != hash))
            .transpose()?
            .unwrap_or(false);
        if differs_from_official && differs_from_generated {
            let backup = portable
                .join("user")
                .join("backup")
                .join(format!("repair-{timestamp}"))
                .join(target);
            copy_file(&config_path, &backup)?;
            if !options.assume_official {
                return Err(Failure::blocked(
                    "配置存在未托管修改，repair 已停止。",
                    format!(
                        "{target}: use --assume-official after backing it up, or run capture first"
                    ),
                ));
            }
            warnings.push(format!("repair 前已备份未托管修改：{target}"));
        }
    }
    let assets = install_user_assets(&portable, old_state.as_ref(), &mut warnings)?;
    // Migrate schema-1 automatic patches left by an older helper.  An empty
    // capture set preserves every existing automatic operation.
    merge_auto_patches(&portable.join("user"), &BTreeMap::new())?;
    let applied = apply_patches(&portable, &portable.join("user"), &mut warnings)?;
    let after = managed_config_hashes(&portable, &targets)?;
    let previous = old_state
        .as_ref()
        .map(|state| state.installed_version.clone())
        .unwrap_or_else(|| version.clone());
    finalize_state(
        &portable, &version, &previous, platform, &applied, &assets, &warnings,
    )?;
    Ok(RepairReport {
        repaired: true,
        changed: before != after || assets.changed,
    })
}

fn state_has_snapshots(portable: &Path, state: &State) -> bool {
    state
        .official
        .keys()
        .all(|target| official_dir(portable).join(target).is_file())
        && state
            .generated
            .keys()
            .all(|target| generated_dir(portable).join(target).is_file())
}

fn managed_config_hashes(
    portable: &Path,
    targets: &BTreeSet<String>,
) -> Result<BTreeMap<String, Option<String>>> {
    let mut result = BTreeMap::new();
    for target in targets {
        let path = portable.join(target);
        result.insert(
            target.clone(),
            if path.is_file() {
                Some(sha256_file(&path)?)
            } else {
                None
            },
        );
    }
    Ok(result)
}

#[derive(Debug, Clone)]
pub struct RelaunchOptions {
    pub root: PathBuf,
    pub wait_pid: u32,
    pub wait_timeout: Duration,
    pub quiet: bool,
}

/// Wait for the current player process to exit, then start it again.  Used by
/// `update-check.lua` after an automatic repair changed the generated
/// configuration: the current process has already read the old files.
pub fn relaunch_player(options: &RelaunchOptions) -> Result<()> {
    let layout = Layout::new(&options.root)?;
    let logger = Logger::new(layout.log_path(), options.quiet);
    logger.info(format!("relaunch: waiting for pid {}", options.wait_pid));
    let result = (|| -> Result<()> {
        let platform = detect_platform(&layout.root, None)?;
        if options.wait_pid != 0 {
            wait_for_process(options.wait_pid, options.wait_timeout)?;
        }
        relaunch(&layout, platform)
    })();
    match &result {
        Ok(()) => logger.info("relaunch: player started"),
        Err(error) => logger.error(format!(
            "relaunch failed: {} ({})",
            error.user, error.detail
        )),
    }
    result
}

#[derive(Debug, Clone)]
pub struct RollbackOptions {
    pub root: PathBuf,
    pub wait_pid: u32,
    pub wait_timeout: Duration,
    pub relaunch: RelaunchPolicy,
    pub quiet: bool,
}

pub fn rollback(options: &RollbackOptions) -> Result<()> {
    let layout = Layout::new(&options.root)?;
    let logger = Logger::new(layout.log_path(), options.quiet);
    let mut reporter = Reporter::new(&layout.root, None, logger);
    reporter.set("rolling-back", "正在回滚到上一个版本。", None);
    let result = rollback_inner(options, &layout, &mut reporter);
    match &result {
        Ok(()) => reporter.finish("rolled-back", "已回滚到上一个版本。", None),
        Err(error) => reporter.finish("failed", error.user.clone(), Some(error.detail.clone())),
    }
    result
}

fn rollback_inner(
    options: &RollbackOptions,
    layout: &Layout,
    reporter: &mut Reporter,
) -> Result<()> {
    let _lock = acquire_lock(layout)?;
    let platform = detect_platform(&layout.root, None)?;
    if options.wait_pid != 0 {
        reporter.set("waiting", "等待播放器退出后回滚。", None);
        wait_for_process(options.wait_pid, options.wait_timeout)?;
    }
    reporter.set("rolling-back", "正在交换安装目录。", None);
    perform_rollback(layout)?;
    let should_relaunch = match options.relaunch {
        RelaunchPolicy::Yes => true,
        RelaunchPolicy::No => false,
        RelaunchPolicy::Auto => options.wait_pid != 0,
    };
    if should_relaunch {
        if let Err(error) = relaunch(layout, platform) {
            reporter.finish(
                "rolled-back",
                "回滚完成，请手动启动播放器。",
                Some(error.detail),
            );
            return Ok(());
        }
    }
    Ok(())
}

pub fn verify_install(root: &Path) -> Result<Vec<String>> {
    let layout = Layout::new(root)?;
    let platform = detect_platform(&layout.root, None)?;
    let version = read_install_version(&layout.root, platform)?;
    let portable = layout.root.join("portable_config");
    let state = State::load(&portable)?.ok_or_else(|| {
        Failure::failed(
            "安装状态缺失。",
            format!("{}", state_path(&portable).display()),
        )
    })?;
    if state.installed_version != version {
        return Err(Failure::failed(
            "安装状态版本与 playkit-version.json 不一致。",
            format!("{} != {version}", state.installed_version),
        ));
    }
    let mut warnings = crate::state::verify_state(&portable)?;
    for (target, expected) in &state.generated {
        let path = portable.join(target);
        if !path.is_file() {
            warnings.push(format!("配置文件缺失：{target}"));
            continue;
        }
        let actual = sha256_file(&path)?;
        if &actual != expected {
            warnings.push(format!("配置文件哈希与安装状态不一致：{target}"));
        }
        if managed_kind(target).is_some() {
            ConfigDoc::read(&path, managed_kind(target).unwrap())?;
        }
    }
    for target in state.user_scripts.iter().map(|s| format!("scripts/{s}")) {
        if !portable.join(&target).is_file() {
            warnings.push(format!("用户脚本未安装：{target}"));
        }
    }
    for target in state.user_fonts.iter().map(|s| format!("fonts/{s}")) {
        if !portable.join(&target).is_file() {
            warnings.push(format!("用户字体未安装：{target}"));
        }
    }
    Ok(warnings)
}

pub fn read_status_file(root: &Path) -> Result<Option<crate::status::Status>> {
    let layout = Layout::new(root)?;
    crate::status::read_status(&layout.status_path())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigKind;
    use tempfile::tempdir;

    fn doc(text: &str) -> ConfigDoc {
        ConfigDoc::parse(text, ConfigKind::Mpv).unwrap()
    }

    #[test]
    fn compact_auto_items_keeps_last_key_operation() {
        let items = vec![
            PatchItem::Key(crate::patch::PatchEntry {
                section: None,
                key: "volume".into(),
                op: PatchOp::Set,
                value: "60".into(),
                line: 0,
            }),
            PatchItem::Key(crate::patch::PatchEntry {
                section: None,
                key: "volume".into(),
                op: PatchOp::Set,
                value: "70".into(),
                line: 0,
            }),
        ];
        let compacted = compact_auto_items(items, ConfigKind::Mpv);
        assert_eq!(compacted.len(), 1);
        match &compacted[0] {
            PatchItem::Key(entry) => assert_eq!(entry.value, "70"),
            _ => panic!("expected key operation"),
        }
    }

    #[test]
    fn merge_auto_patches_converts_schema_one_and_keeps_newest_key() {
        let dir = tempdir().unwrap();
        let user = dir.path().join("user");
        atomic_write(
            &user.join("patches/auto/mpv.conf.patch"),
            b"schema=1\ntarget=mpv.conf\nvolume=60\n",
        )
        .unwrap();
        atomic_write(
            &user.join("patches/auto/v2/mpv.conf.patch"),
            b"schema=2\ntarget=mpv.conf\n@append\n|# keep me\n@end\n",
        )
        .unwrap();
        atomic_write(
            &user.join("patches/auto/script-opts/uosc.conf.patch"),
            b"schema=1\ntarget=script-opts/uosc.conf\nbackground-color=#000000C0\n",
        )
        .unwrap();
        let mut captured = BTreeMap::new();
        captured.insert(
            "mpv.conf".to_string(),
            CapturedPatch {
                keys: vec![crate::patch::PatchEntry {
                    section: None,
                    key: "volume".into(),
                    op: PatchOp::Set,
                    value: "70".into(),
                    line: 0,
                }],
                lines: Vec::new(),
            },
        );

        merge_auto_patches(&user, &captured).unwrap();

        assert!(!user.join("patches/auto/mpv.conf.patch").exists());
        let text = fs::read_to_string(user.join("patches/auto/v2/mpv.conf.patch")).unwrap();
        assert!(text.contains("schema=2"), "{text}");
        assert!(text.contains("volume=70"), "{text}");
        assert!(!text.contains("volume=60"), "{text}");
        assert!(text.contains("# keep me"), "{text}");
        assert!(!user
            .join("patches/auto/script-opts/uosc.conf.patch")
            .exists());
        let uosc =
            fs::read_to_string(user.join("patches/auto/v2/script-opts/uosc.conf.patch")).unwrap();
        assert!(uosc.contains("background-color=#000000C0"), "{uosc}");
    }

    #[test]
    fn merge_auto_patches_keeps_existing_v2_when_nothing_changed() {
        let dir = tempdir().unwrap();
        let user = dir.path().join("user");
        atomic_write(
            &user.join("patches/auto/v2/mpv.conf.patch"),
            b"schema=2\ntarget=mpv.conf\n@append\n|# keep me\n@end\n",
        )
        .unwrap();

        merge_auto_patches(&user, &BTreeMap::new()).unwrap();

        let text = fs::read_to_string(user.join("patches/auto/v2/mpv.conf.patch")).unwrap();
        assert!(text.contains("# keep me"), "{text}");
    }

    #[test]
    fn comments_are_representable_as_line_operations() {
        let current = doc("# my note\nosc=yes\nvolume=70\n");
        let baseline = doc("# official note\nosc=yes\nvolume=100\n");
        let captured = crate::config::capture_patch(&current, &baseline).unwrap();
        assert_eq!(captured.keys.len(), 1);
        assert_eq!(captured.lines.len(), 1);
        crate::config::verify_capture(&current, &baseline, &captured).unwrap();
    }

    #[test]
    fn policy_parsing() {
        assert_eq!(
            UnmanagedPolicy::parse("capture").unwrap(),
            UnmanagedPolicy::Capture
        );
        assert!(UnmanagedPolicy::parse("other").is_err());
        assert_eq!(RelaunchPolicy::parse("auto").unwrap(), RelaunchPolicy::Auto);
    }
}
