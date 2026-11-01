//! umpv: a small single-instance launcher for the portable mpv PlayKit.
//!
//! The first invocation starts mpv with `--input-ipc-server`; later invocations
//! connect to that IPC endpoint and send `loadfile` commands instead of opening
//! a second player.  The protocol is mpv's JSON IPC: one JSON object per line.
//!
//! Windows uses a named pipe (`\\.\pipe\umpv` by default), Unix uses a Unix
//! domain socket.  Configuration is read from `umpv.conf` next to the binary
//! (or `portable_config/umpv.conf`), with command-line overrides for testing.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

const CONFIG_NAME: &str = "umpv.conf";
const DEFAULT_WINDOWS_PIPE: &str = r"\\.\pipe\umpv";
const DEFAULT_FLAG: LoadFlag = LoadFlag::Replace;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadFlag {
    Replace,
    Append,
    AppendPlay,
}

impl LoadFlag {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "replace" => Some(Self::Replace),
            "append" => Some(Self::Append),
            "append-play" | "append_play" => Some(Self::AppendPlay),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::Append => "append",
            Self::AppendPlay => "append-play",
        }
    }

    /// Later files in the same batch must not replace the first one.
    fn following(self) -> Self {
        match self {
            Self::Replace => Self::Append,
            other => other,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub socket: String,
    pub loadfile_flag: LoadFlag,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            socket: default_socket(),
            loadfile_flag: DEFAULT_FLAG,
        }
    }
}

impl Config {
    pub fn apply_text(&mut self, text: &str) -> Result<(), UmpvError> {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            let Some((key, raw_value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim().to_ascii_lowercase();
            let value = unquote(raw_value.trim());
            match key.as_str() {
                "socket_path" | "socket" => {
                    if !value.is_empty() {
                        self.socket = normalize_socket(&expand_path(&value));
                    }
                }
                "loadfile_flag" | "flag" => {
                    self.loadfile_flag = LoadFlag::parse(&value).ok_or_else(|| {
                        UmpvError::Failed(format!("无效的 Loadfile_Flag：{value}"))
                    })?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum UmpvError {
    Usage(String),
    Failed(String),
}

impl fmt::Display for UmpvError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Usage(message) | Self::Failed(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for UmpvError {}

struct ParsedArgs {
    config: Option<PathBuf>,
    socket: Option<String>,
    flag: Option<LoadFlag>,
    files: Vec<OsString>,
}

pub fn main_entry() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let exe = std::env::current_exe().ok();
    let exe_dir = exe
        .as_deref()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."));
    match run(args, exe_dir) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            report(&error.to_string());
            match error {
                UmpvError::Usage(_) => ExitCode::from(2),
                UmpvError::Failed(_) => ExitCode::from(1),
            }
        }
    }
}

pub fn run(args: Vec<OsString>, exe_dir: &Path) -> Result<(), UmpvError> {
    if has_flag(&args, &["--version", "-V"]) {
        println!("umpv {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if has_flag(&args, &["--help", "-h"]) {
        print!("{}", usage());
        return Ok(());
    }

    let parsed = parse_args(args)?;
    let mut config = load_config(parsed.config.as_deref(), exe_dir)?;
    if let Some(socket) = parsed.socket {
        config.socket = normalize_socket(&expand_path(&socket));
    }
    if let Some(flag) = parsed.flag {
        config.loadfile_flag = flag;
    }

    let files = parsed
        .files
        .iter()
        .map(|value| {
            value.to_str().map(str::to_owned).ok_or_else(|| {
                UmpvError::Usage(format!(
                    "文件名不是有效的 UTF-8：{}",
                    value.to_string_lossy()
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let commands = loadfile_commands(&files, config.loadfile_flag);

    match connect(&config.socket) {
        Ok(mut stream) => {
            write_commands(&mut stream, &commands)
                .map_err(|error| UmpvError::Failed(format!("无法向 mpv IPC 发送命令：{error}")))?;
        }
        Err(error) if is_missing(&error) => {
            cleanup_stale_socket(&config.socket);
            spawn_player(exe_dir, &config.socket, &files)?;
        }
        Err(error) => {
            return Err(UmpvError::Failed(format!(
                "无法连接 mpv IPC {}：{error}",
                config.socket
            )));
        }
    }
    Ok(())
}

fn usage() -> String {
    format!(
        "umpv {version}\n\
         \n\
         用法:\n\
           umpv [--config <文件>] [--socket <名称>] [--flag replace|append|append-play] [--] <文件>...\n\
         \n\
         把文件交给正在运行的 mpv PlayKit；没有实例时启动一个新的单实例播放器。\n\
         配置文件默认读取程序目录下的 {CONFIG_NAME}（其次 portable_config/{CONFIG_NAME}）。\n",
        version = env!("CARGO_PKG_VERSION")
    )
}

fn has_flag(args: &[OsString], names: &[&str]) -> bool {
    for arg in args {
        if arg == "--" {
            return false;
        }
        if names.iter().any(|name| arg == name) {
            return true;
        }
    }
    false
}

fn parse_args(args: Vec<OsString>) -> Result<ParsedArgs, UmpvError> {
    let mut parsed = ParsedArgs {
        config: None,
        socket: None,
        flag: None,
        files: Vec::new(),
    };
    let mut index = 0;
    while index < args.len() {
        let text = args[index].to_string_lossy().into_owned();
        if text == "--" {
            parsed.files.extend(args[index + 1..].iter().cloned());
            break;
        }
        if let Some(value) = text.strip_prefix("--config=") {
            parsed.config = Some(PathBuf::from(value));
        } else if text == "--config" {
            index += 1;
            let value = args
                .get(index)
                .ok_or_else(|| UmpvError::Usage("--config 缺少路径".to_string()))?;
            parsed.config = Some(PathBuf::from(value));
        } else if let Some(value) = text.strip_prefix("--socket=") {
            parsed.socket = Some(value.to_string());
        } else if text == "--socket" {
            index += 1;
            parsed.socket = Some(
                args.get(index)
                    .and_then(|value| value.to_str())
                    .ok_or_else(|| UmpvError::Usage("--socket 缺少名称".to_string()))?
                    .to_string(),
            );
        } else if let Some(value) = text.strip_prefix("--flag=") {
            parsed.flag = Some(parse_flag(value)?);
        } else if text == "--flag" {
            index += 1;
            let value = args
                .get(index)
                .and_then(|value| value.to_str())
                .ok_or_else(|| UmpvError::Usage("--flag 缺少取值".to_string()))?;
            parsed.flag = Some(parse_flag(value)?);
        } else if text.starts_with('-') && text != "-" {
            return Err(UmpvError::Usage(format!("未知参数：{text}")));
        } else {
            parsed.files.push(args[index].clone());
        }
        index += 1;
    }
    Ok(parsed)
}

fn parse_flag(value: &str) -> Result<LoadFlag, UmpvError> {
    LoadFlag::parse(value).ok_or_else(|| {
        UmpvError::Usage(format!(
            "无效的 --flag：{value}（可选 replace、append、append-play）"
        ))
    })
}

fn load_config(explicit: Option<&Path>, exe_dir: &Path) -> Result<Config, UmpvError> {
    let path = match explicit {
        Some(path) => Some(path.to_path_buf()),
        None => find_config(exe_dir),
    };
    let mut config = Config::default();
    if let Some(path) = path {
        let text = fs::read_to_string(&path).map_err(|error| {
            UmpvError::Failed(format!("无法读取配置文件 {}：{error}", path.display()))
        })?;
        config.apply_text(&text)?;
    }
    Ok(config)
}

pub fn find_config(exe_dir: &Path) -> Option<PathBuf> {
    let mut candidates = vec![
        exe_dir.join(CONFIG_NAME),
        exe_dir.join("portable_config").join(CONFIG_NAME),
    ];
    if let Ok(current) = std::env::current_dir() {
        let candidate = current.join(CONFIG_NAME);
        if !candidates.iter().any(|path| path == &candidate) {
            candidates.push(candidate);
        }
    }
    candidates.into_iter().find(|path| path.is_file())
}

pub fn default_socket() -> String {
    #[cfg(windows)]
    {
        DEFAULT_WINDOWS_PIPE.to_string()
    }
    #[cfg(not(windows))]
    {
        let name = "umpv.sock";
        if let Some(directory) = std::env::var_os("XDG_RUNTIME_DIR") {
            let directory = PathBuf::from(directory);
            if !directory.as_os_str().is_empty() {
                return directory.join(name).to_string_lossy().into_owned();
            }
        }
        let uid = unsafe { libc::getuid() };
        format!("/tmp/umpv-{uid}.sock")
    }
}

#[cfg(windows)]
pub fn normalize_socket(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return default_socket();
    }
    if raw.starts_with(r"\\.\pipe\") || raw.starts_with(r"\\?\pipe\") {
        raw.to_string()
    } else {
        format!(r"\\.\pipe\{}", raw.trim_start_matches('\\'))
    }
}

#[cfg(not(windows))]
pub fn normalize_socket(raw: &str) -> String {
    if raw.trim().is_empty() {
        default_socket()
    } else {
        raw.trim().to_string()
    }
}

fn expand_path(value: &str) -> String {
    let rest = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"));
    if let Some(rest) = rest {
        if let Some(home) = home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    }
    value.to_string()
}

#[cfg(windows)]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}

#[cfg(not(windows))]
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

pub fn loadfile_commands(files: &[String], flag: LoadFlag) -> Vec<serde_json::Value> {
    files
        .iter()
        .enumerate()
        .map(|(index, path)| {
            let current = if index == 0 { flag } else { flag.following() };
            serde_json::json!({
                "command": ["loadfile", path, current.as_str()]
            })
        })
        .collect()
}

#[cfg(windows)]
type Stream = std::fs::File;
#[cfg(not(windows))]
type Stream = std::os::unix::net::UnixStream;

#[cfg(windows)]
fn connect_once(socket: &str) -> io::Result<Stream> {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(normalize_socket(socket))
}

#[cfg(not(windows))]
fn connect_once(socket: &str) -> io::Result<Stream> {
    Stream::connect(socket)
}

fn connect(socket: &str) -> io::Result<Stream> {
    const ATTEMPTS: usize = 20;
    let mut last = None;
    for attempt in 0..ATTEMPTS {
        match connect_once(socket) {
            Ok(stream) => return Ok(stream),
            Err(error) => {
                if is_busy(&error) && attempt + 1 < ATTEMPTS {
                    std::thread::sleep(Duration::from_millis(25));
                    last = Some(error);
                    continue;
                }
                return Err(error);
            }
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("mpv IPC 一直处于忙状态")))
}

fn is_missing(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    ) || matches!(error.raw_os_error(), Some(2) | Some(3) | Some(111))
}

fn is_busy(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::WouldBlock || error.raw_os_error() == Some(231)
}

fn write_commands<W: Write>(writer: &mut W, commands: &[serde_json::Value]) -> io::Result<()> {
    for command in commands {
        serde_json::to_writer(&mut *writer, command).map_err(io::Error::other)?;
        writer.write_all(b"\n")?;
    }
    writer.flush()
}

/// Send one batch to an IPC endpoint.  Kept public for tests and diagnostics.
pub fn send_files(socket: &str, files: &[String], flag: LoadFlag) -> io::Result<()> {
    let mut stream = connect(socket)?;
    write_commands(&mut stream, &loadfile_commands(files, flag))
}

pub fn player_executable(exe_dir: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let candidate = exe_dir.join("mpv.exe");
        if candidate.is_file() {
            candidate
        } else {
            PathBuf::from("mpv.exe")
        }
    }
    #[cfg(not(windows))]
    {
        for candidate in [
            exe_dir.join("mpv-playkit"),
            exe_dir.join("mpv").join("AppRun"),
        ] {
            if candidate.is_file() {
                return candidate;
            }
        }
        PathBuf::from("mpv")
    }
}

fn player_args(socket: &str, files: &[String]) -> Vec<OsString> {
    let mut args = vec![
        OsString::from(format!("--input-ipc-server={socket}")),
        OsString::from("--idle=yes"),
        OsString::from("--force-window=yes"),
    ];
    if !files.is_empty() {
        args.push(OsString::from("--"));
        args.extend(files.iter().map(OsString::from));
    }
    args
}

fn spawn_player(exe_dir: &Path, socket: &str, files: &[String]) -> Result<(), UmpvError> {
    let executable = player_executable(exe_dir);
    let mut command = Command::new(&executable);
    command
        .args(player_args(socket, files))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // umpv itself is a GUI process; without this flag the console mpv.exe
        // would open an extra terminal window on every fresh launch.
        command.creation_flags(0x0800_0000);
    }
    let child = command.spawn().map_err(|error| {
        UmpvError::Failed(format!("无法启动 mpv（{}）：{error}", executable.display()))
    })?;
    drop(child);
    Ok(())
}

#[cfg(not(windows))]
fn cleanup_stale_socket(socket: &str) {
    use std::os::unix::fs::FileTypeExt;
    let path = Path::new(socket);
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_socket() {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(windows)]
fn cleanup_stale_socket(_socket: &str) {}

#[cfg(all(windows, not(debug_assertions)))]
fn report(message: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let text: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
    let title: Vec<u16> = "mpv PlayKit"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(any(not(windows), debug_assertions))]
fn report(message: &str) {
    eprintln!("umpv: {message}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_are_usable() {
        let config = Config::default();
        assert!(!config.socket.is_empty());
        assert_eq!(config.loadfile_flag, LoadFlag::Replace);
    }

    #[test]
    fn config_accepts_quotes_and_case() {
        let mut config = Config::default();
        config
            .apply_text(
                "# comment\n\
                 SOCKET_PATH = \"custom-pipe\"\n\
                 loadfile_flag = append-play\n",
            )
            .expect("config");
        assert_eq!(config.socket, normalize_socket("custom-pipe"));
        assert_eq!(config.loadfile_flag, LoadFlag::AppendPlay);
    }

    #[test]
    fn config_rejects_unknown_flag() {
        let mut config = Config::default();
        assert!(config.apply_text("Loadfile_Flag=everything\n").is_err());
    }

    #[test]
    fn multiple_files_replace_then_append() {
        let files = vec!["a.mkv".to_string(), "b.mkv".to_string()];
        let commands = loadfile_commands(&files, LoadFlag::Replace);
        assert_eq!(
            commands[0],
            serde_json::json!({"command": ["loadfile", "a.mkv", "replace"]})
        );
        assert_eq!(
            commands[1],
            serde_json::json!({"command": ["loadfile", "b.mkv", "append"]})
        );
    }

    #[test]
    fn append_play_is_kept_for_later_files() {
        let files = vec!["a.mkv".to_string(), "b.mkv".to_string()];
        let commands = loadfile_commands(&files, LoadFlag::AppendPlay);
        assert_eq!(
            commands[1],
            serde_json::json!({"command": ["loadfile", "b.mkv", "append-play"]})
        );
    }

    #[test]
    fn empty_socket_falls_back_to_the_platform_default() {
        assert_eq!(normalize_socket("   "), default_socket());
    }

    #[cfg(unix)]
    #[test]
    fn unix_player_prefers_the_portable_launcher() {
        let directory = tempfile::tempdir().expect("tempdir");
        let app_run = directory.path().join("mpv").join("AppRun");
        fs::create_dir_all(app_run.parent().expect("parent")).expect("mkdir");
        fs::write(&app_run, "#!/bin/sh\n").expect("write");
        let launcher = directory.path().join("mpv-playkit");
        fs::write(&launcher, "#!/bin/sh\n").expect("write");
        assert_eq!(player_executable(directory.path()), launcher);
        fs::remove_file(&launcher).expect("remove");
        assert_eq!(player_executable(directory.path()), app_run);
    }

    #[test]
    fn arguments_split_options_and_files() {
        let args = vec![
            OsString::from("--socket=test"),
            OsString::from("--flag"),
            OsString::from("append"),
            OsString::from("--"),
            OsString::from("-leading-dash.mkv"),
        ];
        let parsed = parse_args(args).expect("arguments");
        assert_eq!(parsed.socket.as_deref(), Some("test"));
        assert_eq!(parsed.flag, Some(LoadFlag::Append));
        assert_eq!(parsed.files, vec![OsString::from("-leading-dash.mkv")]);
    }

    #[test]
    fn unknown_option_is_usage_error() {
        assert!(matches!(
            parse_args(vec![OsString::from("--bogus")]),
            Err(UmpvError::Usage(_))
        ));
    }

    #[test]
    fn config_lookup_prefers_the_binary_directory() {
        let directory = tempfile::tempdir().expect("tempdir");
        let first = directory.path().join(CONFIG_NAME);
        let second = directory.path().join("portable_config").join(CONFIG_NAME);
        fs::create_dir_all(second.parent().expect("parent")).expect("mkdir");
        fs::write(&second, "Loadfile_Flag=append\n").expect("write");
        assert_eq!(find_config(directory.path()), Some(second.clone()));
        fs::write(&first, "Loadfile_Flag=append-play\n").expect("write");
        assert_eq!(find_config(directory.path()), Some(first));
    }

    #[test]
    fn player_arguments_put_files_after_double_dash() {
        let args = player_args("test-pipe", &["a.mkv".to_string()]);
        assert_eq!(args[0], OsString::from("--input-ipc-server=test-pipe"));
        assert_eq!(args[3], OsString::from("--"));
        assert_eq!(args[4], OsString::from("a.mkv"));
    }

    #[cfg(unix)]
    #[test]
    fn sends_json_lines_to_a_unix_socket() {
        use std::io::{BufRead, BufReader};
        use std::os::unix::net::UnixListener;

        let directory = tempfile::tempdir().expect("tempdir");
        let socket = directory.path().join("ipc.sock");
        let listener = UnixListener::bind(&socket).expect("bind");
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept");
            let mut lines = BufReader::new(stream);
            let mut line = String::new();
            lines.read_line(&mut line).expect("read");
            line
        });
        send_files(
            socket.to_str().expect("socket"),
            &["file.mkv".to_string()],
            LoadFlag::Replace,
        )
        .expect("send");
        let line = server.join().expect("join");
        let value: serde_json::Value = serde_json::from_str(&line).expect("json");
        assert_eq!(
            value,
            serde_json::json!({"command": ["loadfile", "file.mkv", "replace"]})
        );
    }
}
