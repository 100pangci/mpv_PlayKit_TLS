//! Command-line parsing and dispatch.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use crate::update::{
    self, CaptureOptions, InstallOptions, RelaunchOptions, RelaunchPolicy, RepairOptions,
    RollbackOptions, UnmanagedPolicy,
};
use crate::util::{Failure, Result};

const USAGE: &str = "\
playkit-updater {version}

用法:
  playkit-updater install  --root <安装目录> --repository <owner/repo> --tag <tag>
                            [--wait-pid <pid>] [--unmanaged capture|discard|abort]
                            [--relaunch auto|yes|no] [--detach] [--quiet]
  playkit-updater capture  --root <安装目录> [--dry-run] [--quiet]
  playkit-updater repair   --root <安装目录> [--assume-official] [--if-needed] [--quiet]
  playkit-updater relaunch --root <安装目录> [--wait-pid <pid>] [--wait-timeout <秒>]
                            [--detach] [--quiet]
  playkit-updater rollback --root <安装目录> [--wait-pid <pid>]
                            [--relaunch auto|yes|no] [--detach] [--quiet]
  playkit-updater verify   --root <安装目录>
  playkit-updater status   --root <安装目录>
  playkit-updater --version

更新器只从固定的 GitHub Release 下载完整安装包，校验 SHA-256 后在临时目录
完成全部准备，最后才交换安装目录。当前安装不会在验证完成前被修改。
";

pub fn run(args: Vec<OsString>) -> Result<i32> {
    let args: Vec<String> = args
        .into_iter()
        .map(|arg| {
            arg.into_string()
                .map_err(|_| Failure::usage("命令行参数必须是有效的 UTF-8。".to_string()))
        })
        .collect::<Result<Vec<_>>>()?;
    let args: Vec<String> = args.into_iter().filter(|arg| arg != "--worker").collect();
    let Some(command) = args.first().map(String::as_str) else {
        print!("{}", USAGE.replace("{version}", env!("CARGO_PKG_VERSION")));
        return Ok(0);
    };
    match command {
        "--version" | "version" => {
            println!("playkit-updater {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        "--help" | "-h" | "help" => {
            print!("{}", USAGE.replace("{version}", env!("CARGO_PKG_VERSION")));
            Ok(0)
        }
        "install" => install(&args[1..]),
        "capture" => capture(&args[1..]),
        "repair" => repair(&args[1..], false),
        // Internal alias used by update-check.lua.  A helper older than the
        // fingerprint feature rejects this command with a usage error, which
        // lets the Lua script skip automatic repair instead of repeatedly
        // running a full repair and restarting the player.
        "repair-auto" => repair(&args[1..], true),
        "relaunch" => relaunch(&args[1..]),
        "rollback" => rollback(&args[1..]),
        "verify" => verify(&args[1..]),
        "status" => status(&args[1..]),
        other => Err(Failure::usage(format!("未知命令：{other}"))),
    }
}

fn install(args: &[String]) -> Result<i32> {
    let root = required_path(args, "--root")?;
    let repository = required_value(args, "--repository")?;
    let tag = required_value(args, "--tag")?;
    let platform = optional_value(args, "--platform");
    let wait_pid = optional_u32(args, "--wait-pid")?.unwrap_or(0);
    let wait_timeout = Duration::from_secs(
        optional_u32(args, "--wait-timeout")?
            .unwrap_or(600)
            .clamp(1, 3600) as u64,
    );
    let unmanaged = UnmanagedPolicy::parse(
        &optional_value(args, "--unmanaged").unwrap_or_else(|| "capture".to_string()),
    )?;
    let relaunch = RelaunchPolicy::parse(
        &optional_value(args, "--relaunch").unwrap_or_else(|| "auto".to_string()),
    )?;
    let quiet = has_flag(args, "--quiet");
    let detach = has_flag(args, "--detach");
    let source_dir = optional_path(args, "--source-dir");
    if source_dir.is_some() && !cfg!(debug_assertions) {
        return Err(Failure::usage(
            "此构建不接受 --source-dir；正式更新只允许从 GitHub HTTPS 下载。",
        ));
    }
    if detach {
        return detach_worker("install", args);
    }
    let options = InstallOptions {
        root,
        repository,
        tag,
        platform,
        wait_pid,
        wait_timeout,
        unmanaged,
        relaunch,
        source_dir,
        quiet,
    };
    let report = update::install(&options)?;
    if !quiet {
        println!(
            "已更新到 {}（原版本 {}）。",
            report.version, report.previous_version
        );
        for warning in &report.warnings {
            println!("警告：{warning}");
        }
    }
    Ok(0)
}

fn capture(args: &[String]) -> Result<i32> {
    let root = required_path(args, "--root")?;
    let dry_run = has_flag(args, "--dry-run");
    let quiet = has_flag(args, "--quiet");
    let report = update::capture(&CaptureOptions {
        root,
        dry_run,
        quiet,
    })?;
    if !quiet {
        let json = serde_json::to_string_pretty(&report)
            .map_err(|error| Failure::internal(format!("serialize capture report: {error}")))?;
        println!("{json}");
    }
    Ok(0)
}

fn repair(args: &[String], force_if_needed: bool) -> Result<i32> {
    let root = required_path(args, "--root")?;
    let assume_official = has_flag(args, "--assume-official");
    let if_needed = force_if_needed || has_flag(args, "--if-needed");
    let quiet = has_flag(args, "--quiet");
    let report = update::repair(&RepairOptions {
        root,
        assume_official,
        if_needed,
        report_status: !force_if_needed,
        quiet,
    })?;
    if !quiet {
        if report.repaired {
            println!("配置 patch 已重新应用。");
        } else {
            println!("配置 patch 无需重新应用。");
        }
    }
    if !if_needed {
        return Ok(0);
    }
    // update-check.lua distinguishes these results without parsing output:
    // 0 = configuration changed and a restart will load it, 3 = nothing to do,
    // 4 = repair ran but produced identical generated files.
    Ok(if !report.repaired {
        3
    } else if report.changed {
        0
    } else {
        4
    })
}

fn relaunch(args: &[String]) -> Result<i32> {
    let root = required_path(args, "--root")?;
    let wait_pid = optional_u32(args, "--wait-pid")?.unwrap_or(0);
    let wait_timeout = Duration::from_secs(
        optional_u32(args, "--wait-timeout")?
            .unwrap_or(600)
            .clamp(1, 3600) as u64,
    );
    let quiet = has_flag(args, "--quiet");
    if has_flag(args, "--detach") {
        return detach_worker("relaunch", args);
    }
    update::relaunch_player(&RelaunchOptions {
        root,
        wait_pid,
        wait_timeout,
        quiet,
    })?;
    Ok(0)
}

fn rollback(args: &[String]) -> Result<i32> {
    let root = required_path(args, "--root")?;
    let wait_pid = optional_u32(args, "--wait-pid")?.unwrap_or(0);
    let wait_timeout = Duration::from_secs(
        optional_u32(args, "--wait-timeout")?
            .unwrap_or(600)
            .clamp(1, 3600) as u64,
    );
    let relaunch = RelaunchPolicy::parse(
        &optional_value(args, "--relaunch").unwrap_or_else(|| "auto".to_string()),
    )?;
    let quiet = has_flag(args, "--quiet");
    if has_flag(args, "--detach") {
        return detach_worker("rollback", args);
    }
    update::rollback(&RollbackOptions {
        root,
        wait_pid,
        wait_timeout,
        relaunch,
        quiet,
    })?;
    Ok(0)
}

fn verify(args: &[String]) -> Result<i32> {
    let root = required_path(args, "--root")?;
    let warnings = update::verify_install(&root)?;
    for warning in &warnings {
        println!("警告：{warning}");
    }
    if warnings.is_empty() {
        println!("安装状态校验通过。");
    }
    Ok(0)
}

fn status(args: &[String]) -> Result<i32> {
    let root = required_path(args, "--root")?;
    match update::read_status_file(&root)? {
        Some(status) => {
            let json = serde_json::to_string_pretty(&status)
                .map_err(|error| Failure::internal(format!("serialize status: {error}")))?;
            println!("{json}");
        }
        None => println!("{{\"state\":\"unknown\"}}"),
    }
    Ok(0)
}

/// The initial helper copies itself outside the installation and starts the
/// real worker, so the swap cannot delete the executable that is running.
fn detach_worker(command: &str, args: &[String]) -> Result<i32> {
    let mut forwarded: Vec<OsString> = Vec::with_capacity(args.len() + 1);
    forwarded.push(OsString::from(command));
    for arg in args {
        if arg == "--detach" {
            continue;
        }
        forwarded.push(OsString::from(arg));
    }
    crate::swap::spawn_detached_worker(&forwarded)?;
    Ok(0)
}

fn has_flag(args: &[String], name: &str) -> bool {
    args.iter().any(|arg| arg == name)
}

fn required_value(args: &[String], name: &str) -> Result<String> {
    optional_value(args, name).ok_or_else(|| Failure::usage(format!("缺少 {name}")))
}

fn optional_value(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
}

fn required_path(args: &[String], name: &str) -> Result<PathBuf> {
    let value = required_value(args, name)?;
    let path = PathBuf::from(&value);
    if !path.is_absolute() {
        return Err(Failure::usage(format!(
            "{name} 必须是绝对路径（收到 {value:?}）"
        )));
    }
    Ok(path)
}

fn optional_path(args: &[String], name: &str) -> Option<PathBuf> {
    optional_value(args, name).map(PathBuf::from)
}

fn optional_u32(args: &[String], name: &str) -> Result<Option<u32>> {
    match optional_value(args, name) {
        None => Ok(None),
        Some(value) => value
            .parse::<u32>()
            .map(Some)
            .map_err(|_| Failure::usage(format!("{name} 必须是非负整数（收到 {value:?}）"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_available() {
        assert_eq!(run(vec![OsString::from("--version")]).unwrap(), 0);
    }

    #[test]
    fn relative_root_is_rejected() {
        let error = run(vec![
            OsString::from("verify"),
            OsString::from("--root"),
            OsString::from("relative/path"),
        ])
        .unwrap_err();
        assert_eq!(error.kind, crate::util::ErrorKind::Usage);
    }

    #[test]
    fn unknown_command_is_usage_error() {
        assert!(run(vec![OsString::from("nope")]).is_err());
    }
}
