//! Process detachment, PID waiting, the directory swap transaction and rollback.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use fs2::FileExt;

use crate::util::{ensure_dir, random_token, remove_dir_all_retry, rename_retry, Failure, Result};

#[derive(Debug, Clone)]
pub struct Layout {
    pub root: PathBuf,
    pub parent: PathBuf,
    pub name: String,
    pub previous: PathBuf,
}

impl Layout {
    pub fn new(root: &Path) -> Result<Self> {
        let root = fs::canonicalize(root).map_err(|error| {
            Failure::failed(
                "找不到 PlayKit 安装目录。",
                format!("{}: {error}", root.display()),
            )
        })?;
        if !root.is_dir() {
            return Err(Failure::failed(
                "PlayKit 安装路径不是目录。",
                format!("{}", root.display()),
            ));
        }
        let parent = root.parent().ok_or_else(|| {
            Failure::blocked(
                "不能直接在文件系统根目录更新 PlayKit。",
                format!("{} has no parent directory", root.display()),
            )
        })?;
        let name = root
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                Failure::blocked("PlayKit 安装目录名无效。", format!("{}", root.display()))
            })?
            .to_string();
        if name.is_empty() || name == "." || name == ".." {
            return Err(Failure::blocked(
                "PlayKit 安装目录名无效。",
                format!("{}", root.display()),
            ));
        }
        let previous = parent.join(format!("{name}.previous"));
        let parent_path = parent.to_path_buf();
        Ok(Self {
            root,
            parent: parent_path,
            name,
            previous,
        })
    }

    pub fn staging_dir(&self) -> PathBuf {
        self.parent
            .join(format!(".playkit-staging-{}-{}", self.name, random_token()))
    }

    pub fn lock_path(&self) -> PathBuf {
        self.parent
            .join(format!(".playkit-update-{}.lock", self.name))
    }

    pub fn status_path(&self) -> PathBuf {
        self.root
            .join("portable_config")
            .join(".playkit")
            .join("update-status.json")
    }

    pub fn log_path(&self) -> PathBuf {
        self.root
            .join("portable_config")
            .join(".playkit")
            .join("updater.log")
    }

    pub fn failed_path(&self) -> PathBuf {
        self.parent
            .join(format!(".playkit-failed-{}-{}", self.name, random_token()))
    }
}

#[derive(Debug)]
pub struct InstallLock {
    _file: File,
    path: PathBuf,
}

impl Drop for InstallLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

pub fn acquire_lock(layout: &Layout) -> Result<InstallLock> {
    let path = layout.lock_path();
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| {
            Failure::blocked(
                "无法创建更新锁文件。",
                format!("{}: {error}", path.display()),
            )
        })?;
    file.try_lock_exclusive().map_err(|error| {
        Failure::blocked(
            "另一个更新正在进行，请稍后重试。",
            format!("{}: {error}", path.display()),
        )
    })?;
    let _ = file.set_len(0);
    let _ = writeln!(file, "{}", std::process::id());
    Ok(InstallLock { _file: file, path })
}

pub fn prepare_staging(layout: &Layout) -> Result<PathBuf> {
    let staging = layout.staging_dir();
    ensure_dir(&staging)?;
    Ok(staging)
}

pub fn check_writable(path: &Path) -> Result<()> {
    let probe = path.join(format!(".playkit-write-test-{}", random_token()));
    match fs::write(&probe, b"probe") {
        Ok(()) => {
            let _ = fs::remove_file(&probe);
            Ok(())
        }
        Err(error) => Err(Failure::blocked(
            "PlayKit 安装目录不可写，请把程序放在有写权限的位置或手动更新。",
            format!("{}: {error}", path.display()),
        )),
    }
}

/// Copy the running helper to a private temp directory and start it detached.
/// The returned PID belongs to the detached worker.
pub fn spawn_detached_worker(args: &[OsString]) -> Result<u32> {
    let exe = std::env::current_exe()
        .map_err(|error| Failure::internal(format!("current_exe: {error}")))?;
    let temp_root = std::env::temp_dir().join(format!(
        "playkit-updater-{}-{}",
        std::process::id(),
        random_token()
    ));
    ensure_dir(&temp_root)?;
    let name = if cfg!(windows) {
        "playkit-updater.exe"
    } else {
        "playkit-updater"
    };
    let worker = temp_root.join(name);
    fs::copy(&exe, &worker).map_err(|error| {
        let _ = remove_dir_all_retry(&temp_root);
        Failure::failed(
            "无法启动更新器。",
            format!("copy {} -> {}: {error}", exe.display(), worker.display()),
        )
    })?;
    crate::util::make_executable(&worker)?;
    let mut command = Command::new(&worker);
    command
        .arg("--worker")
        .args(args)
        .current_dir(&temp_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    match command.spawn() {
        Ok(child) => Ok(child.id()),
        Err(error) => {
            let _ = remove_dir_all_retry(&temp_root);
            Err(Failure::failed(
                "无法启动更新器。",
                format!("{}: {error}", worker.display()),
            ))
        }
    }
}

pub fn cleanup_stale_temp_dirs() {
    let Ok(entries) = fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with("playkit-updater-") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_dir() {
            continue;
        }
        let age = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .unwrap_or_default();
        if age > Duration::from_secs(7 * 24 * 3600) {
            let _ = remove_dir_all_retry(&entry.path());
        }
    }
}

pub fn wait_for_process(pid: u32, timeout: Duration) -> Result<()> {
    if pid == 0 {
        return Ok(());
    }
    let identity = process_identity(pid);
    let started = Instant::now();
    loop {
        if !process_alive(pid, identity) {
            return Ok(());
        }
        if started.elapsed() >= timeout {
            return Err(Failure::blocked(
                "播放器仍在运行，更新尚未安装。请退出播放器后重试。",
                format!("pid {pid} still alive after {}s", timeout.as_secs()),
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

#[cfg(windows)]
fn process_alive(pid: u32, identity: Option<u64>) -> bool {
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, FILETIME, HANDLE, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    const SYNCHRONIZE: u32 = 0x0010_0000;
    unsafe {
        let handle: HANDLE = OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            // Access denied means the process exists but belongs to someone
            // else; err on the side of not swapping underneath it.
            return GetLastError() == 5;
        }
        if let Some(expected) = identity {
            let mut created = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };
            let mut exit = created;
            let mut kernel = created;
            let mut user = created;
            let ok = GetProcessTimes(handle, &mut created, &mut exit, &mut kernel, &mut user);
            if ok != 0 {
                let value = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
                if value != expected {
                    CloseHandle(handle);
                    return false;
                }
            }
        }
        let result = WaitForSingleObject(handle, 0);
        CloseHandle(handle);
        result != WAIT_OBJECT_0
    }
}

#[cfg(windows)]
fn process_identity(pid: u32) -> Option<u64> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    unsafe {
        let handle: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut created = FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        };
        let mut exit = created;
        let mut kernel = created;
        let mut user = created;
        let ok = GetProcessTimes(handle, &mut created, &mut exit, &mut kernel, &mut user);
        CloseHandle(handle);
        if ok == 0 {
            return None;
        }
        Some(((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64)
    }
}

#[cfg(unix)]
fn process_alive(pid: u32, identity: Option<u64>) -> bool {
    if let Some(expected) = identity {
        if let Some(actual) = process_identity(pid) {
            if actual != expected {
                return false;
            }
        }
    }
    if is_zombie(pid) {
        return false;
    }
    let result = unsafe { libc::kill(pid as i32, 0) };
    if result == 0 {
        return true;
    }
    matches!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EPERM)
    )
}

#[cfg(unix)]
fn process_identity(pid: u32) -> Option<u64> {
    let text = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let end = text.rfind(')')?;
    let rest = text.get(end + 1..)?.trim_start();
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // Field 22 (starttime) is index 19 after the comm field.
    fields.get(19)?.parse().ok()
}

#[cfg(unix)]
fn is_zombie(pid: u32) -> bool {
    let Ok(text) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let Some(end) = text.rfind(')') else {
        return false;
    };
    text.get(end + 2..end + 3) == Some("Z")
}

#[cfg(not(any(unix, windows)))]
fn process_alive(_pid: u32, _identity: Option<u64>) -> bool {
    false
}

#[cfg(not(any(unix, windows)))]
fn process_identity(_pid: u32) -> Option<u64> {
    None
}

/// Replace the current installation with the fully staged package.
pub fn perform_swap(layout: &Layout, staged_package: &Path) -> Result<()> {
    if !staged_package.is_dir() {
        return Err(Failure::internal(format!(
            "staged package is not a directory: {}",
            staged_package.display()
        )));
    }
    let marker = staged_package.join("playkit-version.json");
    if !marker.is_file() {
        return Err(Failure::internal(
            "staged package is incomplete (missing playkit-version.json)",
        ));
    }
    if layout.previous.exists() {
        if layout.previous.is_dir() {
            remove_dir_all_retry(&layout.previous).map_err(|error| {
                Failure::blocked("无法清理上一次更新保留的备份目录。", error.detail)
            })?;
        } else {
            fs::remove_file(&layout.previous).map_err(|error| {
                Failure::blocked(
                    "无法清理上一次更新保留的备份文件。",
                    format!("{}: {error}", layout.previous.display()),
                )
            })?;
        }
    }
    rename_retry(&layout.root, &layout.previous).map_err(|error| {
        Failure::failed(
            "无法替换当前安装，更新已中止。",
            format!("rename current -> previous: {}", error.detail),
        )
    })?;
    if let Err(error) = rename_retry(staged_package, &layout.root) {
        let restore = rename_retry(&layout.previous, &layout.root);
        return Err(Failure::failed(
            "无法安装新版本，已尝试恢复原安装。",
            match restore {
                Ok(()) => format!(
                    "rename staged -> current failed: {}; current restored",
                    error.detail
                ),
                Err(restore_error) => format!(
                    "rename staged -> current failed: {}; restore also failed: {}; \
                     the previous installation is at {}",
                    error.detail,
                    restore_error.detail,
                    layout.previous.display()
                ),
            },
        ));
    }
    sync_parent(&layout.parent);
    Ok(())
}

/// Roll back to the retained previous installation.
pub fn perform_rollback(layout: &Layout) -> Result<()> {
    if !layout.previous.is_dir() {
        return Err(Failure::blocked(
            "没有可回滚的上一个版本。",
            format!("{} does not exist", layout.previous.display()),
        ));
    }
    if !layout.previous.join("playkit-version.json").is_file() {
        return Err(Failure::blocked(
            "上一个版本的备份不完整，无法回滚。",
            format!("{}", layout.previous.display()),
        ));
    }
    let failed = layout.failed_path();
    rename_retry(&layout.root, &failed).map_err(|error| {
        Failure::failed(
            "无法回滚，当前安装保持不变。",
            format!("rename current -> failed: {}", error.detail),
        )
    })?;
    if let Err(error) = rename_retry(&layout.previous, &layout.root) {
        let restore = rename_retry(&failed, &layout.root);
        return Err(Failure::failed(
            "回滚失败。",
            match restore {
                Ok(()) => format!(
                    "rename previous -> current failed: {}; current restored",
                    error.detail
                ),
                Err(restore_error) => format!(
                    "rename previous -> current failed: {}; restore also failed: {}; \
                     current is at {}",
                    error.detail,
                    restore_error.detail,
                    failed.display()
                ),
            },
        ));
    }
    sync_parent(&layout.parent);
    let _ = remove_dir_all_retry(&failed);
    Ok(())
}

pub fn relaunch(layout: &Layout, platform: &str) -> Result<()> {
    let executable = match platform {
        "windows-x64" => layout.root.join("mpv.exe"),
        "linux-x64" => layout.root.join("mpv-playkit"),
        other => {
            return Err(Failure::internal(format!(
                "unknown platform for relaunch: {other}"
            )))
        }
    };
    if !executable.is_file() {
        return Err(Failure::failed(
            "找不到播放器程序，请手动启动。",
            format!("{}", executable.display()),
        ));
    }
    let mut command = Command::new(&executable);
    command
        .current_dir(&layout.root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(CREATE_NEW_PROCESS_GROUP);
    }
    command.spawn().map_err(|error| {
        Failure::failed(
            "无法重新启动播放器，请手动启动。",
            format!("{}: {error}", executable.display()),
        )
    })?;
    Ok(())
}

fn sync_parent(parent: &Path) {
    #[cfg(unix)]
    {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn layout_rejects_filesystem_root() {
        #[cfg(unix)]
        assert!(Layout::new(Path::new("/")).is_err());
        #[cfg(windows)]
        assert!(Layout::new(Path::new("C:\\")).is_err());
    }

    #[test]
    fn lock_is_exclusive() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("PlayKit");
        fs::create_dir_all(&root).unwrap();
        let layout = Layout::new(&root).unwrap();
        let first = acquire_lock(&layout).unwrap();
        let error = acquire_lock(&layout).unwrap_err();
        assert_eq!(error.kind, crate::util::ErrorKind::Blocked);
        drop(first);
        let _second = acquire_lock(&layout).unwrap();
    }

    #[test]
    fn swap_and_rollback() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("PlayKit");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("playkit-version.json"), b"old").unwrap();
        fs::write(root.join("old.txt"), b"old").unwrap();
        let layout = Layout::new(&root).unwrap();
        let staging = prepare_staging(&layout).unwrap();
        let staged = staging.join("package");
        fs::create_dir_all(&staged).unwrap();
        fs::write(staged.join("playkit-version.json"), b"new").unwrap();
        fs::write(staged.join("new.txt"), b"new").unwrap();
        perform_swap(&layout, &staged).unwrap();
        assert_eq!(fs::read(root.join("playkit-version.json")).unwrap(), b"new");
        assert!(root.join("new.txt").is_file());
        assert!(layout.previous.join("old.txt").is_file());
        perform_rollback(&layout).unwrap();
        assert_eq!(fs::read(root.join("playkit-version.json")).unwrap(), b"old");
        assert!(root.join("old.txt").is_file());
        assert!(!layout.previous.exists());
    }

    #[test]
    fn rollback_without_previous_is_blocked() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("PlayKit");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("playkit-version.json"), b"old").unwrap();
        let layout = Layout::new(&root).unwrap();
        let error = perform_rollback(&layout).unwrap_err();
        assert_eq!(error.kind, crate::util::ErrorKind::Blocked);
        assert!(root.join("playkit-version.json").is_file());
    }

    #[test]
    fn wait_for_dead_pid_returns_immediately() {
        let started = Instant::now();
        wait_for_process(0, Duration::from_secs(1)).unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
