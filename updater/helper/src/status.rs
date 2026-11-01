//! Durable, user-readable status for the in-player UI and a rotating log.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::util::{atomic_write, ensure_dir, human_size, utc_now_iso, Result};

const MAX_LOG_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct Logger {
    path: PathBuf,
    quiet: bool,
}

impl Logger {
    pub fn new(path: PathBuf, quiet: bool) -> Self {
        Self { path, quiet }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn info(&self, message: impl AsRef<str>) {
        self.write("info", message.as_ref());
    }

    pub fn warn(&self, message: impl AsRef<str>) {
        self.write("warn", message.as_ref());
    }

    pub fn error(&self, message: impl AsRef<str>) {
        self.write("error", message.as_ref());
    }

    fn write(&self, level: &str, message: &str) {
        let line = format!(
            "[{}] [{}] {}\n",
            utc_now_iso(),
            level,
            message.replace(['\r', '\n'], " ")
        );
        if let Some(parent) = self.path.parent() {
            let _ = ensure_dir(parent);
        }
        self.rotate_if_needed();
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = file.write_all(line.as_bytes());
        }
        if !self.quiet && level != "info" {
            eprint!("{line}");
        }
    }

    fn rotate_if_needed(&self) {
        let Ok(metadata) = fs::metadata(&self.path) else {
            return;
        };
        if metadata.len() < MAX_LOG_BYTES {
            return;
        }
        let backup = self.path.with_extension("log.1");
        let _ = fs::remove_file(&backup);
        let _ = fs::rename(&self.path, &backup);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Status {
    pub schema: u32,
    pub state: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<u32>,
    pub updated_at: String,
    pub pid: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

pub struct Reporter {
    path: PathBuf,
    logger: Logger,
    status: Status,
    last_write: std::time::Instant,
    last_progress: Option<u32>,
}

impl Reporter {
    pub fn new(root: &Path, version: Option<String>, logger: Logger) -> Self {
        Self {
            path: root
                .join("portable_config")
                .join(".playkit")
                .join("update-status.json"),
            logger,
            status: Status {
                schema: 1,
                state: "starting".to_string(),
                message: "更新器正在启动。".to_string(),
                detail: None,
                version,
                progress: None,
                updated_at: utc_now_iso(),
                pid: std::process::id(),
                warnings: Vec::new(),
            },
            last_write: std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(10))
                .unwrap_or_else(std::time::Instant::now),
            last_progress: None,
        }
    }

    pub fn logger(&self) -> &Logger {
        &self.logger
    }

    pub fn set(
        &mut self,
        state: &str,
        message: impl Into<String>,
        detail: impl Into<Option<String>>,
    ) {
        let message = message.into();
        let detail = detail.into();
        self.status.state = state.to_string();
        self.status.message = message.clone();
        self.status.detail = detail.clone();
        self.status.updated_at = utc_now_iso();
        match detail {
            Some(detail) => self.logger.info(format!("{state}: {message} ({detail})")),
            None => self.logger.info(format!("{state}: {message}")),
        }
        self.flush(true);
    }

    pub fn progress(&mut self, downloaded: u64, total: Option<u64>) {
        let percent = total
            .filter(|total| *total > 0)
            .map(|total| ((downloaded.saturating_mul(100) / total).min(100)) as u32);
        let message = match (percent, total) {
            (Some(percent), Some(total)) => format!(
                "正在下载更新 {percent}%（{} / {}）",
                human_size(downloaded),
                human_size(total)
            ),
            _ => format!("正在下载更新（{}）", human_size(downloaded)),
        };
        let changed = self.last_progress != percent;
        self.status.state = "downloading".to_string();
        self.status.message = message;
        self.status.progress = percent;
        self.status.detail = None;
        self.status.updated_at = utc_now_iso();
        self.last_progress = percent;
        self.flush(changed || self.last_write.elapsed().as_millis() >= 900);
    }

    pub fn warnings(&mut self, warnings: &[String]) {
        self.status.warnings = warnings.to_vec();
        for warning in warnings {
            self.logger.warn(warning);
        }
        self.flush(false);
    }

    pub fn finish(
        &mut self,
        state: &str,
        message: impl Into<String>,
        detail: impl Into<Option<String>>,
    ) {
        self.set(state, message, detail);
        self.flush(true);
    }

    pub fn flush(&mut self, force: bool) {
        if !force && self.last_write.elapsed().as_millis() < 250 {
            return;
        }
        let mut status = self.status.clone();
        status.updated_at = utc_now_iso();
        let data = match serde_json::to_vec_pretty(&status) {
            Ok(mut data) => {
                data.push(b'\n');
                data
            }
            Err(error) => {
                self.logger
                    .error(format!("status serialization failed: {error}"));
                return;
            }
        };
        if let Err(error) = atomic_write(&self.path, &data) {
            self.logger
                .warn(format!("could not write status file: {}", error.detail));
        }
        self.last_write = std::time::Instant::now();
    }

    pub fn status_path(&self) -> &Path {
        &self.path
    }
}

pub fn read_status(path: &Path) -> Result<Option<Status>> {
    if !path.exists() {
        return Ok(None);
    }
    let data = fs::read(path)
        .map_err(|error| crate::util::io_context(&format!("read {}", path.display()), error))?;
    let status: Status = serde_json::from_slice(&data).map_err(|error| {
        crate::util::Failure::failed(
            "更新状态文件格式错误。",
            format!("{}: {error}", path.display()),
        )
    })?;
    Ok(Some(status))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn writes_and_reads_status() {
        let dir = tempdir().unwrap();
        let logger = Logger::new(dir.path().join("updater.log"), true);
        let mut reporter = Reporter::new(dir.path(), Some("2026.11".to_string()), logger);
        reporter.set("downloading", "下载中", None);
        reporter.progress(50, Some(100));
        let status = read_status(reporter.status_path()).unwrap().unwrap();
        assert_eq!(status.state, "downloading");
        assert_eq!(status.progress, Some(50));
        assert_eq!(status.version.as_deref(), Some("2026.11"));
    }

    #[test]
    fn log_rotates() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("updater.log");
        fs::write(&path, vec![b'x'; (MAX_LOG_BYTES + 1) as usize]).unwrap();
        let logger = Logger::new(path.clone(), true);
        logger.info("hello");
        assert!(path.with_extension("log.1").is_file());
        assert!(path.is_file());
    }
}
