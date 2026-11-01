//! Small shared helpers: errors, hashing, atomic files, paths, versions, time.

use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The update was stopped on purpose; the current installation is intact.
    Blocked,
    /// A download, verification or filesystem step failed; current is intact.
    Failed,
    /// Bad command line.
    Usage,
    /// A bug or an unexpected platform condition.
    Internal,
}

#[derive(Debug, Clone)]
pub struct Failure {
    pub kind: ErrorKind,
    /// Short, user-facing Chinese text. Never contains a traceback.
    pub user: String,
    /// Technical detail for the log; may contain English text and paths.
    pub detail: String,
}

impl Failure {
    pub fn new(kind: ErrorKind, user: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            kind,
            user: user.into(),
            detail: detail.into(),
        }
    }

    pub fn blocked(user: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new(ErrorKind::Blocked, user, detail)
    }

    pub fn failed(user: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new(ErrorKind::Failed, user, detail)
    }

    pub fn usage(detail: impl Into<String>) -> Self {
        Self::new(ErrorKind::Usage, "更新器参数错误。", detail)
    }

    pub fn internal(detail: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, "更新器内部错误。", detail)
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.user, self.detail)
    }
}

impl std::error::Error for Failure {}

pub type Result<T> = std::result::Result<T, Failure>;

impl From<io::Error> for Failure {
    fn from(value: io::Error) -> Self {
        Failure::failed("文件操作失败。", format!("io error: {value}"))
    }
}

impl From<serde_json::Error> for Failure {
    fn from(value: serde_json::Error) -> Self {
        Failure::failed("更新元数据格式错误。", format!("json error: {value}"))
    }
}

impl From<zip::result::ZipError> for Failure {
    fn from(value: zip::result::ZipError) -> Self {
        Failure::failed("压缩包解析失败。", format!("zip error: {value}"))
    }
}

impl From<url::ParseError> for Failure {
    fn from(value: url::ParseError) -> Self {
        Failure::failed("下载地址无效。", format!("url error: {value}"))
    }
}

pub fn io_context(context: &str, error: io::Error) -> Failure {
    Failure::failed("文件操作失败。", format!("{context}: {error}"))
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        File::open(path).map_err(|e| io_context(&format!("open {}", path.display()), e))?;
    let mut hasher = Sha256::new();
    // Keep large buffers on the heap: the Windows main thread has a 1 MiB
    // stack, and hashing an 80+ MiB archive must not overflow it.
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| io_context(&format!("read {}", path.display()), e))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

pub fn sha256_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex(&hasher.finalize())
}

pub fn hex(data: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(data.len() * 2);
    for byte in data {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

pub fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn atomic_write(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| io_context(&format!("create {}", parent.display()), e))?;
    }
    let mut temp = None;
    for _ in 0..32 {
        let candidate = path.with_extension(format!(
            "{}.tmp-{}",
            path.extension().and_then(OsStr::to_str).unwrap_or("tmp"),
            random_token()
        ));
        match OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&candidate)
        {
            Ok(file) => {
                temp = Some((candidate, file));
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(io_context("create atomic temporary file", error)),
        }
    }
    let (temp_path, mut file) = temp
        .ok_or_else(|| Failure::failed("无法创建安全的临时文件。", "temporary name collisions"))?;
    let write_result = file
        .write_all(data)
        .and_then(|_| file.sync_all())
        .map_err(|e| io_context(&format!("write {}", temp_path.display()), e));
    drop(file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    replace_file(&temp_path, path).map_err(|e| {
        let _ = fs::remove_file(&temp_path);
        io_context(&format!("replace {}", path.display()), e)
    })
}

#[cfg(not(windows))]
fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

#[cfg(windows)]
fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
    }
    let mut source: Vec<u16> = from.as_os_str().encode_wide().collect();
    let mut target: Vec<u16> = to.as_os_str().encode_wide().collect();
    source.push(0);
    target.push(0);
    let ok = unsafe {
        MoveFileExW(
            source.as_ptr(),
            target.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let data = fs::read(path).map_err(|e| io_context(&format!("read {}", path.display()), e))?;
    serde_json::from_slice(&data)
        .map_err(|e| Failure::failed("更新元数据格式错误。", format!("{}: {e}", path.display())))
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let mut data = serde_json::to_vec_pretty(value)?;
    data.push(b'\n');
    atomic_write(path, &data)
}

pub fn read_optional_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    read_json(path).map(Some)
}

pub fn remove_dir_all_retry(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut last = None;
    for attempt in 0..10 {
        match fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last = Some(error);
                if attempt < 9 {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
            }
        }
    }
    Err(io_context(
        &format!("remove {}", path.display()),
        last.unwrap_or_else(|| io::Error::other("unknown remove error")),
    ))
}

pub fn remove_file_retry(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut last = None;
    for attempt in 0..10 {
        match fs::remove_file(path) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last = Some(error);
                if attempt < 9 {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
            }
        }
    }
    Err(io_context(
        &format!("remove {}", path.display()),
        last.unwrap_or_else(|| io::Error::other("unknown remove error")),
    ))
}

pub fn rename_retry(from: &Path, to: &Path) -> Result<()> {
    let mut last = None;
    for attempt in 0..20 {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last = Some(error);
                if attempt < 19 {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
            }
        }
    }
    Err(io_context(
        &format!("rename {} -> {}", from.display(), to.display()),
        last.unwrap_or_else(|| io::Error::other("unknown rename error")),
    ))
}

pub fn copy_file(source: &Path, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| io_context(&format!("create {}", parent.display()), e))?;
    }
    fs::copy(source, destination).map_err(|e| {
        io_context(
            &format!("copy {} -> {}", source.display(), destination.display()),
            e,
        )
    })?;
    Ok(())
}

/// Copy a file or directory tree, merging into an existing destination.
/// Existing destination files are replaced. Symlinks are recreated only when
/// their target stays inside `source_root`; unsafe links are skipped.
pub fn copy_tree_merge(source: &Path, destination: &Path, source_root: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)
        .map_err(|e| io_context(&format!("stat {}", source.display()), e))?;
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(source)
            .map_err(|e| io_context(&format!("readlink {}", source.display()), e))?;
        let resolved = lexical_join(source.parent().unwrap_or(source_root), &target);
        if !is_within(source_root, &resolved) {
            return Ok(());
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| io_context(&format!("create {}", parent.display()), e))?;
        }
        #[cfg(unix)]
        {
            let _ = fs::remove_file(destination);
            std::os::unix::fs::symlink(&target, destination)
                .map_err(|e| io_context(&format!("symlink {}", destination.display()), e))?;
        }
        #[cfg(not(unix))]
        {
            let _ = (&target, destination);
        }
        return Ok(());
    }
    if metadata.is_dir() {
        fs::create_dir_all(destination)
            .map_err(|e| io_context(&format!("create {}", destination.display()), e))?;
        let mut entries: Vec<PathBuf> = fs::read_dir(source)
            .map_err(|e| io_context(&format!("read {}", source.display()), e))?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect();
        entries.sort();
        for entry in entries {
            let relative = entry
                .strip_prefix(source)
                .map_err(|e| Failure::internal(format!("strip prefix: {e}")))?;
            copy_tree_merge(&entry, &destination.join(relative), source_root)?;
        }
        return Ok(());
    }
    copy_file(source, destination)
}

pub fn lexical_join(base: &Path, relative: &Path) -> PathBuf {
    let mut result = base.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(part) => result.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            Component::RootDir | Component::Prefix(_) => {
                result = PathBuf::from(relative);
                return result;
            }
        }
    }
    result
}

pub fn is_within(root: &Path, candidate: &Path) -> bool {
    let root = normalize_lexical(root);
    let candidate = normalize_lexical(candidate);
    candidate == root || candidate.starts_with(&root)
}

pub fn normalize_lexical(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

/// Convert a package-relative path with `/` separators into a safe local path.
pub fn relative_path(value: &str) -> Result<PathBuf> {
    if value.is_empty() {
        return Err(Failure::failed("包内路径为空。", "empty relative path"));
    }
    let normalized = value.replace('\\', "/");
    if normalized.starts_with('/') {
        return Err(Failure::failed(
            "包内路径不安全。",
            format!("absolute path in {value:?}"),
        ));
    }
    let mut result = PathBuf::new();
    for part in normalized.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(Failure::failed(
                "包内路径不安全。",
                format!("path traversal in {value:?}"),
            ));
        }
        if part.contains('\0') || part.contains(':') {
            return Err(Failure::failed(
                "包内路径不安全。",
                format!("unsafe path component in {value:?}"),
            ));
        }
        result.push(part);
    }
    if result.as_os_str().is_empty() {
        return Err(Failure::failed("包内路径为空。", "empty relative path"));
    }
    Ok(result)
}

pub fn forward_slashes(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

pub fn relative_string(root: &Path, path: &Path) -> Result<String> {
    path.strip_prefix(root)
        .map(forward_slashes)
        .map_err(|e| Failure::internal(format!("path not under root: {e}")))
}

pub fn display_path(path: &Path) -> String {
    let text = path.display().to_string();
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
}

pub fn version_parts(value: &str) -> Option<Vec<u64>> {
    let text = value.strip_prefix('v').unwrap_or(value);
    if text.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    for part in text.split('.') {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        parts.push(part.parse::<u64>().ok()?);
    }
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    Some(parts)
}

pub fn compare_versions(left: &str, right: &str) -> Option<std::cmp::Ordering> {
    let a = version_parts(left)?;
    let b = version_parts(right)?;
    let length = a.len().max(b.len());
    for index in 0..length {
        let x = a.get(index).copied().unwrap_or(0);
        let y = b.get(index).copied().unwrap_or(0);
        if x != y {
            return Some(x.cmp(&y));
        }
    }
    Some(std::cmp::Ordering::Equal)
}

pub fn is_valid_repository(value: &str) -> bool {
    let mut parts = value.split('/');
    let owner = parts.next().unwrap_or("");
    let name = parts.next().unwrap_or("");
    parts.next().is_none()
        && !owner.is_empty()
        && !name.is_empty()
        && owner != "."
        && owner != ".."
        && name != "."
        && name != ".."
        && owner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

pub fn is_safe_asset_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && !value.starts_with('.')
}

pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn utc_now_iso() -> String {
    iso_from_unix(unix_now())
}

pub fn iso_from_unix(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rem = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year,
        month,
        day,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

// Howard Hinnant's days-from-civil inverse.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

pub fn random_token() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{}-{:x}", std::process::id(), nanos)
}

pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    for (index, unit) in UNITS.iter().enumerate() {
        if value < 1024.0 || index == UNITS.len() - 1 {
            return if index == 0 {
                format!("{value:.0} {unit}")
            } else {
                format!("{value:.1} {unit}")
            };
        }
        value /= 1024.0;
    }
    format!("{value:.1} GiB")
}

pub fn ensure_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|e| io_context(&format!("create {}", path.display()), e))
}

pub fn write_text(path: &Path, text: &str) -> Result<()> {
    atomic_write(path, text.replace("\r\n", "\n").as_bytes())
}

#[cfg(unix)]
pub fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)
        .map_err(|e| io_context(&format!("stat {}", path.display()), e))?
        .permissions();
    permissions.set_mode(permissions.mode() | 0o111);
    fs::set_permissions(path, permissions)
        .map_err(|e| io_context(&format!("chmod {}", path.display()), e))
}

#[cfg(not(unix))]
pub fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison() {
        assert_eq!(
            compare_versions("2026.11", "2026.10"),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            compare_versions("2026.11.1", "2026.11"),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            compare_versions("v2026.11", "2026.11.0"),
            Some(std::cmp::Ordering::Equal)
        );
        assert_eq!(compare_versions("not-a-version", "2026.11"), None);
    }

    #[test]
    fn repository_validation() {
        assert!(is_valid_repository("owner/repo"));
        assert!(is_valid_repository("a-b/c_d.e"));
        assert!(!is_valid_repository("owner"));
        assert!(!is_valid_repository("owner/repo/extra"));
        assert!(!is_valid_repository("../repo"));
        assert!(!is_valid_repository("owner/.."));
    }

    #[test]
    fn relative_paths_reject_traversal() {
        assert_eq!(
            relative_path("portable_config/mpv.conf").unwrap(),
            PathBuf::from("portable_config").join("mpv.conf")
        );
        assert!(relative_path("../evil").is_err());
        assert!(relative_path("/etc/passwd").is_err());
        assert!(relative_path("a/../../b").is_err());
        assert!(relative_path("C:/evil").is_err());
    }

    #[test]
    fn iso_time_is_well_formed() {
        assert_eq!(iso_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_from_unix(1_767_225_600), "2026-01-01T00:00:00Z");
    }

    #[test]
    fn lexical_containment() {
        assert!(is_within(Path::new("/a/b"), Path::new("/a/b/c/../d")));
        assert!(!is_within(Path::new("/a/b"), Path::new("/a/bc")));
    }
}
