//! HTTPS downloads with proxy support, redirect pinning, size limits and
//! streaming SHA-256.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use url::Url;

use crate::util::{hex, is_safe_asset_name, is_sha256, is_valid_repository, Failure, Result};

const MAX_REDIRECTS: usize = 6;
const USER_AGENT: &str = concat!("playkit-updater/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone)]
pub struct ReleaseLocation {
    pub repository: String,
    pub tag: String,
}

impl ReleaseLocation {
    pub fn new(repository: &str, tag: &str) -> Result<Self> {
        if !is_valid_repository(repository) {
            return Err(Failure::failed(
                "更新仓库无效。",
                format!("repository {repository:?}"),
            ));
        }
        if !valid_tag(tag) {
            return Err(Failure::failed("更新 tag 无效。", format!("tag {tag:?}")));
        }
        Ok(Self {
            repository: repository.to_string(),
            tag: tag.to_string(),
        })
    }

    pub fn asset_url(&self, asset: &str) -> Result<Url> {
        if !is_safe_asset_name(asset) {
            return Err(Failure::failed(
                "更新资产名无效。",
                format!("asset {asset:?}"),
            ));
        }
        let mut url = Url::parse("https://github.com/")?;
        // `repository` has already been validated as exactly `owner/name`.
        // Push the two components separately: pushing the combined string
        // would percent-encode the slash into `%2F`, which GitHub does not
        // treat as a repository separator.
        let (owner, name) = self
            .repository
            .split_once('/')
            .ok_or_else(|| Failure::internal("repository was validated without a slash"))?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| Failure::internal("github.com URL cannot have path segments"))?;
            segments.pop_if_empty();
            segments.push(owner);
            segments.push(name);
            segments.push("releases");
            segments.push("download");
            segments.push(&self.tag);
            segments.push(asset);
        }
        Ok(url)
    }
}

fn valid_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 100
        && tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

pub trait Fetcher {
    /// Fetch a release asset into `dest`, enforcing `max_bytes`.
    fn fetch(
        &self,
        asset: &str,
        dest: &Path,
        max_bytes: u64,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<()>;
    fn describe(&self, asset: &str) -> String;
}

pub struct HttpFetcher {
    direct: ureq::Agent,
    proxied: Option<ureq::Agent>,
    no_proxy: String,
    location: ReleaseLocation,
    proxy_note: Option<String>,
}

impl HttpFetcher {
    pub fn new(location: ReleaseLocation) -> Result<Self> {
        let (proxy, proxy_note, no_proxy) = proxy_from_env()?;
        let builder = || {
            ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(20))
                .timeout_read(Duration::from_secs(60))
                .timeout_write(Duration::from_secs(60))
                .redirects(0)
                .user_agent(USER_AGENT)
        };
        let direct = builder().build();
        let proxied = proxy.map(|proxy| builder().proxy(proxy).build());
        Ok(Self {
            direct,
            proxied,
            no_proxy,
            location,
            proxy_note,
        })
    }

    pub fn proxy_note(&self) -> Option<&str> {
        self.proxy_note.as_deref()
    }

    fn agent_for(&self, url: &Url) -> &ureq::Agent {
        match &self.proxied {
            Some(agent)
                if !no_proxy_matches(
                    url.host_str().unwrap_or(""),
                    url.port_or_known_default().unwrap_or(443),
                    &self.no_proxy,
                ) =>
            {
                agent
            }
            _ => &self.direct,
        }
    }

    fn request(&self, url: &Url) -> Result<ureq::Response> {
        let mut current = url.clone();
        validate_url(&current)?;
        for hop in 0..=MAX_REDIRECTS {
            let result = self
                .agent_for(&current)
                .get(current.as_str())
                .set("Accept", "application/octet-stream")
                .set("User-Agent", USER_AGENT)
                .call();
            let response = match result {
                Ok(response) => response,
                Err(ureq::Error::Status(code, response)) => {
                    if (300..400).contains(&code) {
                        response
                    } else {
                        return Err(status_failure(code, &current));
                    }
                }
                Err(ureq::Error::Transport(transport)) => {
                    return Err(transport_failure(&transport, &current));
                }
            };
            let status = response.status();
            if (300..400).contains(&status) {
                if hop == MAX_REDIRECTS {
                    return Err(Failure::failed(
                        "下载重定向次数过多。",
                        format!("{url}: too many redirects"),
                    ));
                }
                let location = response.header("location").ok_or_else(|| {
                    Failure::failed(
                        "下载重定向缺少目标地址。",
                        format!("{url}: HTTP {status} without Location"),
                    )
                })?;
                let next = current.join(location).map_err(|error| {
                    Failure::failed(
                        "下载重定向地址无效。",
                        format!("{url}: {location:?}: {error}"),
                    )
                })?;
                validate_url(&next)?;
                current = next;
                continue;
            }
            if status != 200 {
                return Err(status_failure(status, &current));
            }
            return Ok(response);
        }
        Err(Failure::failed("下载重定向次数过多。", format!("{url}")))
    }
}

impl Fetcher for HttpFetcher {
    fn fetch(
        &self,
        asset: &str,
        dest: &Path,
        max_bytes: u64,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<()> {
        let url = self.location.asset_url(asset)?;
        let response = self.request(&url)?;
        let total = response
            .header("content-length")
            .and_then(|value| value.parse::<u64>().ok());
        if let Some(total) = total {
            if total > max_bytes {
                return Err(Failure::failed(
                    "下载内容超过大小限制。",
                    format!("{url}: Content-Length {total} > {max_bytes}"),
                ));
            }
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                Failure::failed(
                    "创建下载目录失败。",
                    format!("{}: {error}", parent.display()),
                )
            })?;
        }
        let part = part_path(dest);
        let _ = fs::remove_file(&part);
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&part)
            .map_err(|error| {
                Failure::failed("创建下载文件失败。", format!("{}: {error}", part.display()))
            })?;
        let mut reader = response.into_reader();
        let mut hasher = Sha256::new();
        let mut downloaded: u64 = 0;
        let mut buffer = [0u8; 64 * 1024];
        let mut last_report = Instant::now();
        let mut last_bytes = 0u64;
        progress(0, total);
        loop {
            let read = reader.read(&mut buffer).map_err(|error| {
                let _ = fs::remove_file(&part);
                Failure::failed("读取下载内容失败。", format!("{url}: {error}"))
            })?;
            if read == 0 {
                break;
            }
            downloaded += read as u64;
            if downloaded > max_bytes {
                let _ = fs::remove_file(&part);
                return Err(Failure::failed(
                    "下载内容超过大小限制。",
                    format!("{url}: limit {max_bytes} bytes"),
                ));
            }
            file.write_all(&buffer[..read]).map_err(|error| {
                let _ = fs::remove_file(&part);
                Failure::failed("写入下载文件失败。", format!("{}: {error}", part.display()))
            })?;
            hasher.update(&buffer[..read]);
            if last_report.elapsed() >= Duration::from_millis(400)
                || downloaded - last_bytes >= 8 * 1024 * 1024
            {
                progress(downloaded, total);
                last_report = Instant::now();
                last_bytes = downloaded;
            }
        }
        file.sync_all().map_err(|error| {
            let _ = fs::remove_file(&part);
            Failure::failed("同步下载文件失败。", format!("{}: {error}", part.display()))
        })?;
        drop(file);
        if let Some(total) = total {
            if downloaded != total {
                let _ = fs::remove_file(&part);
                return Err(Failure::failed(
                    "下载不完整。",
                    format!("{url}: got {downloaded} of {total} bytes"),
                ));
            }
        }
        if downloaded == 0 {
            let _ = fs::remove_file(&part);
            return Err(Failure::failed("下载内容为空。", format!("{url}")));
        }
        fs::rename(&part, dest).map_err(|error| {
            let _ = fs::remove_file(&part);
            Failure::failed("保存下载文件失败。", format!("{}: {error}", dest.display()))
        })?;
        progress(downloaded, total);
        Ok(())
    }

    fn describe(&self, asset: &str) -> String {
        self.location
            .asset_url(asset)
            .map(|url| url.to_string())
            .unwrap_or_else(|_| asset.to_string())
    }
}

/// Test/CLI source that reads release assets from a local directory.  It is
/// never selected automatically; the updater only uses it when the caller
/// explicitly passes `--source-dir`.
pub struct DirFetcher {
    dir: PathBuf,
}

impl DirFetcher {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }
}

impl Fetcher for DirFetcher {
    fn fetch(
        &self,
        asset: &str,
        dest: &Path,
        max_bytes: u64,
        progress: &mut dyn FnMut(u64, Option<u64>),
    ) -> Result<()> {
        if !is_safe_asset_name(asset) {
            return Err(Failure::failed(
                "更新资产名无效。",
                format!("asset {asset:?}"),
            ));
        }
        let source = self.dir.join(asset);
        let metadata = fs::metadata(&source).map_err(|error| {
            Failure::failed(
                "读取本地更新资产失败。",
                format!("{}: {error}", source.display()),
            )
        })?;
        if !metadata.is_file() || metadata.len() > max_bytes {
            return Err(Failure::failed(
                "本地更新资产无效。",
                format!("{}: {} bytes", source.display(), metadata.len()),
            ));
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&source, dest)?;
        progress(metadata.len(), Some(metadata.len()));
        Ok(())
    }

    fn describe(&self, asset: &str) -> String {
        self.dir.join(asset).display().to_string()
    }
}

pub fn fetch_metadata(
    fetcher: &dyn Fetcher,
    asset: &str,
    dest: &Path,
    max: u64,
) -> Result<Vec<u8>> {
    fetcher.fetch(asset, dest, max, &mut |_, _| {})?;
    let data = fs::read(dest).map_err(|error| {
        Failure::failed(
            "读取更新元数据失败。",
            format!("{}: {error}", dest.display()),
        )
    })?;
    if data.len() as u64 > max {
        return Err(Failure::failed(
            "更新元数据过大。",
            format!("{asset}: {} bytes", data.len()),
        ));
    }
    Ok(data)
}

pub fn verify_downloaded(
    path: &Path,
    expected_sha256: &str,
    expected_size: Option<u64>,
) -> Result<u64> {
    if !is_sha256(expected_sha256) {
        return Err(Failure::failed(
            "期望的 SHA-256 无效。",
            format!("{expected_sha256:?}"),
        ));
    }
    let metadata = fs::metadata(path).map_err(|error| {
        Failure::failed("读取下载文件失败。", format!("{}: {error}", path.display()))
    })?;
    if let Some(expected) = expected_size {
        if metadata.len() != expected {
            return Err(Failure::failed(
                "下载文件大小与 Release 元数据不一致。",
                format!(
                    "{}: {} bytes, expected {expected}",
                    path.display(),
                    metadata.len()
                ),
            ));
        }
    }
    let actual = crate::util::sha256_file(path)?;
    if !actual.eq_ignore_ascii_case(expected_sha256) {
        return Err(Failure::failed(
            "下载文件校验失败，更新已中止。",
            format!(
                "{}: sha256 {actual}, expected {expected_sha256}",
                path.display()
            ),
        ));
    }
    Ok(metadata.len())
}

fn part_path(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "download".to_string());
    dest.with_file_name(format!(".{name}.part-{}", std::process::id()))
}

fn status_failure(code: u16, url: &Url) -> Failure {
    let message = match code {
        401 | 403 => "GitHub 拒绝了下载请求（可能触发了速率限制）。",
        404 => "下载地址不存在，Release 可能尚未发布完整。",
        429 => "GitHub 请求过于频繁，请稍后重试。",
        _ => "下载失败。",
    };
    Failure::failed(message, format!("{url}: HTTP {code}"))
}

fn transport_failure(error: &ureq::Transport, url: &Url) -> Failure {
    let text = error.to_string();
    let message = if text.contains("timed out") || text.contains("timeout") {
        "连接 GitHub 超时。"
    } else if text.contains("certificate") || text.contains("tls") {
        "无法建立安全连接，请检查系统证书或代理设置。"
    } else if text.contains("proxy") {
        "无法通过代理连接 GitHub，请检查代理设置。"
    } else {
        "无法连接 GitHub，请检查网络。"
    };
    Failure::failed(message, format!("{url}: {text}"))
}

fn validate_url(url: &Url) -> Result<()> {
    if url.scheme() != "https" {
        return Err(Failure::failed(
            "下载地址必须使用 HTTPS。",
            format!("{url}"),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Failure::failed(
            "下载地址包含无效的认证信息。",
            format!("{url}"),
        ));
    }
    let host = url.host_str().unwrap_or("");
    if !allowed_host(host) {
        return Err(Failure::failed(
            "下载被重定向到了非 GitHub 地址，更新已中止。",
            format!("{url}"),
        ));
    }
    if let Some(port) = url.port() {
        if port != 443 {
            return Err(Failure::failed(
                "下载地址使用了非标准端口。",
                format!("{url}"),
            ));
        }
    }
    Ok(())
}

fn allowed_host(host: &str) -> bool {
    host == "github.com"
        || host == "objects.githubusercontent.com"
        || host == "release-assets.githubusercontent.com"
        || host == "github-releases.githubusercontent.com"
        || host.ends_with(".githubusercontent.com")
}

fn env_first(names: &[&str]) -> Option<String> {
    for name in names {
        if let Ok(value) = std::env::var(name) {
            if !value.trim().is_empty() {
                return Some(value);
            }
        }
    }
    None
}

/// Select a proxy exactly as the user configured it.  We do not silently fall
/// back to a direct connection when a proxy is configured but unusable.
fn proxy_from_env() -> Result<(Option<ureq::Proxy>, Option<String>, String)> {
    let no_proxy = env_first(&["NO_PROXY", "no_proxy"]).unwrap_or_default();
    if no_proxy.trim() == "*" {
        return Ok((None, Some("NO_PROXY=*".to_string()), no_proxy));
    }
    let value = env_first(&["HTTPS_PROXY", "https_proxy"])
        .or_else(|| env_first(&["ALL_PROXY", "all_proxy"]))
        .or_else(|| env_first(&["HTTP_PROXY", "http_proxy"]));
    let Some(value) = value else {
        return Ok((None, None, no_proxy));
    };
    let normalized = normalize_proxy(&value);
    let proxy = ureq::Proxy::new(&normalized).map_err(|error| {
        Failure::failed(
            "代理设置无效，更新已中止。",
            format!("proxy {value:?}: {error}"),
        )
    })?;
    Ok((Some(proxy), Some(redact_proxy(&value)), no_proxy))
}

fn normalize_proxy(value: &str) -> String {
    let trimmed = value.trim();
    if let Some(rest) = trimmed.strip_prefix("socks5h://") {
        return format!("socks5://{rest}");
    }
    if let Some(rest) = trimmed.strip_prefix("socks://") {
        return format!("socks5://{rest}");
    }
    if !trimmed.contains("://") {
        return format!("http://{trimmed}");
    }
    trimmed.to_string()
}

fn redact_proxy(value: &str) -> String {
    match Url::parse(&normalize_proxy(value)) {
        Ok(mut url) => {
            if !url.username().is_empty() {
                let _ = url.set_username("***");
            }
            if url.password().is_some() {
                let _ = url.set_password(Some("***"));
            }
            url.to_string()
        }
        Err(_) => "<invalid proxy>".to_string(),
    }
}

/// `NO_PROXY` handling is deliberately conservative: an entry matches the
/// host exactly, as a domain suffix, or as a wildcard suffix.  A port in the
/// entry restricts the match to that port.
pub fn no_proxy_matches(host: &str, port: u16, no_proxy: &str) -> bool {
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    for raw in no_proxy.split([',', ' ', '\t', '\n']) {
        let entry = raw.trim();
        if entry.is_empty() {
            continue;
        }
        if entry == "*" {
            return true;
        }
        let entry = entry.to_ascii_lowercase();
        let (pattern, pattern_port) = split_host_port(&entry);
        if let Some(pattern_port) = pattern_port {
            if pattern_port != port {
                continue;
            }
        }
        let pattern = pattern.trim_start_matches('.');
        let pattern = pattern.strip_prefix("*.").unwrap_or(pattern);
        if host == pattern || host.ends_with(&format!(".{pattern}")) {
            return true;
        }
    }
    false
}

fn split_host_port(value: &str) -> (&str, Option<u16>) {
    if value.starts_with('[') {
        if let Some(end) = value.find(']') {
            let host = &value[..=end];
            let rest = &value[end + 1..];
            if let Some(port) = rest.strip_prefix(':').and_then(|p| p.parse().ok()) {
                return (host, Some(port));
            }
            return (host, None);
        }
    }
    if value.matches(':').count() == 1 {
        if let Some((host, port)) = value.rsplit_once(':') {
            if let Ok(port) = port.parse::<u16>() {
                return (host, Some(port));
            }
        }
    }
    (value, None)
}

pub fn sha256_of(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex(&hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_url_is_pinned() {
        let location = ReleaseLocation::new("owner/repo", "v2026.11").unwrap();
        let url = location
            .asset_url("mpv-PlayKit-2026.11-windows-x64.zip")
            .unwrap();
        assert_eq!(
            url.as_str(),
            "https://github.com/owner/repo/releases/download/v2026.11/mpv-PlayKit-2026.11-windows-x64.zip"
        );
    }

    #[test]
    fn unsafe_asset_is_rejected() {
        let location = ReleaseLocation::new("owner/repo", "v2026.11").unwrap();
        assert!(location.asset_url("../evil.zip").is_err());
        assert!(location.asset_url("a/b.zip").is_err());
        assert!(location.asset_url(".hidden").is_err());
    }

    #[test]
    fn only_github_hosts_are_allowed() {
        assert!(allowed_host("github.com"));
        assert!(allowed_host("objects.githubusercontent.com"));
        assert!(allowed_host("release-assets.githubusercontent.com"));
        assert!(!allowed_host("evil.example.com"));
        assert!(!allowed_host("github.com.evil.example"));
        assert!(validate_url(&Url::parse("http://github.com/x").unwrap()).is_err());
        assert!(validate_url(&Url::parse("https://github.com:8443/x").unwrap()).is_err());
        assert!(validate_url(&Url::parse("https://user@github.com/x").unwrap()).is_err());
    }

    #[test]
    fn no_proxy_suffixes() {
        assert!(no_proxy_matches("example.com", 443, "example.com"));
        assert!(no_proxy_matches("a.example.com", 443, ".example.com"));
        assert!(no_proxy_matches("a.example.com", 443, "*.example.com"));
        assert!(!no_proxy_matches("notexample.com", 443, "example.com"));
        assert!(!no_proxy_matches("example.com", 443, "example.com:8080"));
        assert!(no_proxy_matches("example.com", 8080, "example.com:8080"));
        assert!(no_proxy_matches("anything", 443, "*"));
    }

    #[test]
    fn proxy_normalization() {
        assert_eq!(normalize_proxy("127.0.0.1:8080"), "http://127.0.0.1:8080");
        assert_eq!(
            normalize_proxy("socks5h://127.0.0.1:1080"),
            "socks5://127.0.0.1:1080"
        );
        assert_eq!(normalize_proxy("http://u:p@host:1"), "http://u:p@host:1");
    }

    #[test]
    fn proxy_redaction() {
        let url = format!("http://{}:{}@127.0.0.1:8080", "alice", "pw123");
        let redacted = redact_proxy(&url);
        assert!(!redacted.contains("pw123"));
        assert!(!redacted.contains("alice"));
    }
}
