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
use crate::config::{capture_diff, managed_kind, ConfigDoc, ParsedLine};
use crate::download::{
    fetch_metadata, sha256_of, verify_downloaded, DirFetcher, Fetcher, HttpFetcher, ReleaseLocation,
};
use crate::manifest::{
    expected_asset_name, normalize_tag, parse_sha256sums, ReleaseManifest, MAX_ARCHIVE_BYTES,
    MAX_METADATA_BYTES,
};
use crate::patch::{PatchDoc, PatchEntry};
use crate::state::{
    discover_managed, generated_dir, managed_targets, new_package_state, official_dir, state_path,
    State, STATE_SCHEMA,
};
use crate::status::{Logger, Reporter};
use crate::swap::{
    acquire_lock, check_writable, cleanup_stale_temp_dirs, perform_rollback, perform_swap,
    prepare_staging, relaunch, wait_for_process, Layout,
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
    write_auto_patches(&staged_portable.join("user"), &captured)?;
    preserve_runtime(
        &current_portable,
        &staged_portable,
        old_inventory.as_ref(),
        new_inventory.as_ref(),
        &mut warnings,
    )?;
    let user_assets = install_user_assets(&staged_portable, &mut warnings)?;
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
) -> Result<BTreeMap<String, Vec<PatchEntry>>> {
    let mut targets: BTreeSet<String> =
        managed_targets(context.current_portable, context.state_old)
            .into_iter()
            .collect();
    targets.extend(discover_managed(context.fallback_portable));
    let mut captured: BTreeMap<String, Vec<PatchEntry>> = BTreeMap::new();
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
        if context.state_old.is_none() && !migration_warned {
            warnings.push(
                "没有历史配置基线，已按当前配置与新版官方配置的差异生成用户 patch，并备份原配置。"
                    .to_string(),
            );
            migration_warned = true;
        }
        let unrepresentable = unrepresentable_lines(&current_doc, &baseline_doc);
        if !unrepresentable.is_empty() {
            return Err(Failure::blocked(
                "检测到无法转换为用户 patch 的配置行，更新已停止。",
                format!("{target}: {}", unrepresentable.join(" | ")),
            ));
        }
        let entries = capture_diff(&current_doc, &baseline_doc);
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
        if entries.is_empty() {
            warnings.push(format!(
                "{target}: 检测到格式或注释变化，但没有可表达的配置意图；原文件已备份。"
            ));
            continue;
        }
        captured.insert(target, entries);
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

fn unrepresentable_lines(current: &ConfigDoc, baseline: &ConfigDoc) -> Vec<String> {
    fn collect(doc: &ConfigDoc) -> BTreeMap<String, usize> {
        let mut map = BTreeMap::new();
        for line in &doc.lines {
            if matches!(line.parsed, ParsedLine::Other) {
                *map.entry(line.raw.trim().to_string()).or_insert(0) += 1;
            }
        }
        map
    }
    let current_lines = collect(current);
    let baseline_lines = collect(baseline);
    let mut result = Vec::new();
    for (line, count) in &current_lines {
        if baseline_lines.get(line).copied().unwrap_or(0) < *count {
            result.push(format!("+{line}"));
        }
    }
    for (line, count) in &baseline_lines {
        if current_lines.get(line).copied().unwrap_or(0) < *count {
            result.push(format!("-{line}"));
        }
    }
    result.truncate(10);
    result
}

fn write_auto_patches(user_dir: &Path, captured: &BTreeMap<String, Vec<PatchEntry>>) -> Result<()> {
    let auto = user_dir.join("patches").join("auto");
    remove_dir_all_retry(&auto)?;
    ensure_dir(&auto)?;
    for (target, entries) in captured {
        if entries.is_empty() {
            continue;
        }
        let doc = PatchDoc {
            schema: crate::patch::PATCH_SCHEMA,
            target: Some(target.clone()),
            source: format!("auto:{target}"),
            entries: entries.clone(),
        };
        let path = auto.join(format!("{target}.patch"));
        atomic_write(&path, doc.render().as_bytes())?;
    }
    Ok(())
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
}

fn install_user_assets(portable: &Path, warnings: &mut Vec<String>) -> Result<UserAssets> {
    let user = portable.join("user");
    Ok(UserAssets {
        scripts: copy_user_tree(
            &user.join("scripts"),
            &portable.join("scripts"),
            "用户脚本",
            warnings,
        )?,
        fonts: copy_user_tree(
            &user.join("fonts"),
            &portable.join("fonts"),
            "用户字体",
            warnings,
        )?,
        script_opts: copy_user_tree(
            &user.join("script-opts"),
            &portable.join("script-opts"),
            "用户 script-opts",
            warnings,
        )?,
    })
}

fn copy_user_tree(
    source: &Path,
    destination: &Path,
    label: &str,
    warnings: &mut Vec<String>,
) -> Result<Vec<String>> {
    let mut result = Vec::new();
    if !source.is_dir() {
        return Ok(result);
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
            let target = destination.join(relative);
            if target.exists() {
                return Err(Failure::blocked(
                    format!("{label}与官方文件同名，更新已停止。"),
                    format!("{} conflicts with {}", path.display(), target.display()),
                ));
            }
            copy_file(&path, &target)?;
            result.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    result.sort();
    Ok(result)
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
        let manual = user_dir.join("patches").join(format!("{target}.patch"));
        let auto = user_dir
            .join("patches")
            .join("auto")
            .join(format!("{target}.patch"));
        let mut had_patch = false;
        // Automatic patches capture direct edits to generated files, while
        // manual patches describe the user's long-term intent.  Apply the
        // automatic one first so an explicit manual patch always wins when
        // both touch the same key.
        for patch_path in [&auto, &manual] {
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
        doc.write(&config_path)?;
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
    for root in [patches.clone(), auto.clone()] {
        if !root.is_dir() {
            continue;
        }
        let skip_auto = root == patches;
        let mut pending = vec![root.clone()];
        while let Some(directory) = pending.pop() {
            let entries = fs::read_dir(&directory)
                .map_err(|error| io_context(&format!("read {}", directory.display()), error))?;
            for entry in entries {
                let entry = entry
                    .map_err(|error| io_context(&format!("read {}", directory.display()), error))?;
                let path = entry.path();
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    // `auto` is walked as its own root, with its own relative
                    // paths; skip it while walking manual patches.
                    if skip_auto && path == auto {
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
        write_auto_patches(&portable.join("user"), &captured)?;
    }
    Ok(CaptureReport {
        targets: captured
            .iter()
            .map(|(target, entries)| (target.clone(), entries.len()))
            .collect(),
        warnings,
        dry_run: options.dry_run,
    })
}

#[derive(Debug, Clone)]
pub struct RepairOptions {
    pub root: PathBuf,
    pub assume_official: bool,
    pub quiet: bool,
}

pub fn repair(options: &RepairOptions) -> Result<()> {
    let layout = Layout::new(&options.root)?;
    let logger = Logger::new(layout.log_path(), options.quiet);
    let mut reporter = Reporter::new(&layout.root, None, logger);
    reporter.set("repairing", "正在重新应用用户配置。", None);
    let result = repair_inner(options, &layout);
    match &result {
        Ok(()) => reporter.finish("repaired", "用户配置已重新应用。", None),
        Err(error) => reporter.finish("failed", error.user.clone(), Some(error.detail.clone())),
    }
    result
}

fn repair_inner(options: &RepairOptions, layout: &Layout) -> Result<()> {
    let _lock = acquire_lock(layout)?;
    let platform = detect_platform(&layout.root, None)?;
    let version = read_install_version(&layout.root, platform)?;
    let portable = layout.root.join("portable_config");
    let old_state = State::load(&portable)?;
    let mut warnings = Vec::new();
    let timestamp = utc_now_iso().replace([':', '-'], "");
    let mut targets: BTreeSet<String> = discover_managed(&portable).into_iter().collect();
    if let Some(state) = &old_state {
        targets.extend(state.official.keys().cloned());
    }
    for target in targets {
        let Some(kind) = managed_kind(&target) else {
            continue;
        };
        let config_path = portable.join(&target);
        let official_path = official_dir(&portable).join(&target);
        if !config_path.is_file() || !official_path.is_file() {
            continue;
        }
        let current = ConfigDoc::read(&config_path, kind)?;
        let official = ConfigDoc::read(&official_path, kind)?;
        let differs_from_official = current.render() != official.render();
        let differs_from_generated = old_state
            .as_ref()
            .and_then(|state| state.generated.get(&target))
            .map(|hash| sha256_file(&config_path).map(|actual| &actual != hash))
            .transpose()?
            .unwrap_or(false);
        if differs_from_official && differs_from_generated {
            let backup = portable
                .join("user")
                .join("backup")
                .join(format!("repair-{timestamp}"))
                .join(&target);
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
    let assets = install_user_assets(&portable, &mut warnings)?;
    let applied = apply_patches(&portable, &portable.join("user"), &mut warnings)?;
    let previous = old_state
        .as_ref()
        .map(|state| state.installed_version.clone())
        .unwrap_or_else(|| version.clone());
    finalize_state(
        &portable, &version, &previous, platform, &applied, &assets, &warnings,
    )?;
    Ok(())
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

    fn doc(text: &str) -> ConfigDoc {
        ConfigDoc::parse(text, ConfigKind::Mpv).unwrap()
    }

    #[test]
    fn unrepresentable_detects_unknown_lines() {
        let current = doc("volume=70\nsome command without equals\n");
        let baseline = doc("volume=100\n");
        let lines = unrepresentable_lines(&current, &baseline);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].starts_with('+'));
    }

    #[test]
    fn comments_are_not_unrepresentable() {
        let current = doc("# my note\nvolume=70\n");
        let baseline = doc("# official note\nvolume=100\n");
        assert!(unrepresentable_lines(&current, &baseline).is_empty());
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
