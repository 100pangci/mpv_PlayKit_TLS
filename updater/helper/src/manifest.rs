//! Release and package metadata, plus the SHA256SUMS verification chain.

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;

use crate::util::{
    compare_versions, is_safe_asset_name, is_sha256, is_valid_repository, Failure, Result,
};

pub const SUPPORTED_UPDATER_PROTOCOL: u32 = 1;
pub const SUPPORTED_PATCH_SCHEMA: u32 = 1;
pub const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_METADATA_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct PlatformEntry {
    pub asset: String,
    pub sha256: String,
    pub size: u64,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub mpv_version: Option<String>,
    #[serde(default)]
    pub mpv_commit: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct UpdaterBlock {
    #[serde(default)]
    pub protocol: Option<u32>,
    #[serde(default)]
    pub min_helper_version: Option<String>,
    #[serde(default)]
    pub patch_schema: Option<u32>,
    #[serde(default)]
    pub user_layer: Option<String>,
    #[serde(default)]
    pub state_dir: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseManifest {
    pub schema: u32,
    #[serde(rename = "playkit_version")]
    pub version: String,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub built_at: Option<String>,
    #[serde(default)]
    pub source_commit: Option<String>,
    #[serde(default)]
    pub inputs_hash: Option<String>,
    #[serde(default)]
    pub platforms: BTreeMap<String, PlatformEntry>,
    #[serde(default)]
    pub updater: Option<UpdaterBlock>,
}

impl ReleaseManifest {
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() as u64 > MAX_METADATA_BYTES {
            return Err(Failure::failed(
                "Release 元数据过大。",
                format!("manifest.json is {} bytes", data.len()),
            ));
        }
        let manifest: Self = serde_json::from_slice(data).map_err(|error| {
            Failure::failed(
                "Release 元数据格式错误。",
                format!("manifest.json: {error}"),
            )
        })?;
        manifest.validate()?;
        Ok(manifest)
    }

    fn validate(&self) -> Result<()> {
        if self.schema == 0 || self.schema > 2 {
            return Err(Failure::failed(
                "此版本不支持自动安装。",
                format!("release manifest schema {}", self.schema),
            ));
        }
        if crate::util::version_parts(&self.version).is_none() {
            return Err(Failure::failed(
                "Release 版本号无效。",
                format!("version {:?}", self.version),
            ));
        }
        if let Some(tag) = &self.tag {
            if normalize_tag(tag) != self.version {
                return Err(Failure::failed(
                    "Release 元数据中的 tag 与版本不一致。",
                    format!("tag {tag:?}, version {:?}", self.version),
                ));
            }
        }
        if let Some(repository) = &self.repository {
            if !is_valid_repository(repository) {
                return Err(Failure::failed(
                    "Release 元数据中的仓库无效。",
                    format!("repository {repository:?}"),
                ));
            }
        }
        if let Some(updater) = &self.updater {
            if let Some(protocol) = updater.protocol {
                if protocol != SUPPORTED_UPDATER_PROTOCOL {
                    return Err(Failure::blocked(
                        "此更新需要更新的更新器，请手动下载新版本。",
                        format!("updater protocol {protocol}"),
                    ));
                }
            }
            if let Some(patch_schema) = updater.patch_schema {
                if patch_schema != SUPPORTED_PATCH_SCHEMA {
                    return Err(Failure::blocked(
                        "此更新使用不兼容的用户 patch 格式。",
                        format!("patch schema {patch_schema}"),
                    ));
                }
            }
            if let Some(minimum) = &updater.min_helper_version {
                if compare_versions(minimum, env!("CARGO_PKG_VERSION"))
                    == Some(std::cmp::Ordering::Greater)
                {
                    return Err(Failure::blocked(
                        "此更新需要更新的更新器，请手动下载新版本。",
                        format!(
                            "requires updater {minimum}, this is {}",
                            env!("CARGO_PKG_VERSION")
                        ),
                    ));
                }
            }
        }
        for (platform, entry) in &self.platforms {
            if !is_safe_asset_name(&entry.asset) {
                return Err(Failure::failed(
                    "Release 元数据中的资产名无效。",
                    format!("{platform}: {:?}", entry.asset),
                ));
            }
            if !is_sha256(&entry.sha256) {
                return Err(Failure::failed(
                    "Release 元数据中的 SHA-256 无效。",
                    format!("{platform}: {:?}", entry.sha256),
                ));
            }
            if entry.size == 0 || entry.size > MAX_ARCHIVE_BYTES {
                return Err(Failure::failed(
                    "Release 元数据中的资产大小无效。",
                    format!("{platform}: {}", entry.size),
                ));
            }
        }
        Ok(())
    }

    pub fn platform(&self, platform: &str) -> Result<&PlatformEntry> {
        self.platforms.get(platform).ok_or_else(|| {
            Failure::failed(
                "Release 中没有当前平台的安装包。",
                format!(
                    "platform {platform} not in {:?}",
                    self.platforms.keys().collect::<Vec<_>>()
                ),
            )
        })
    }

    pub fn check_identity(&self, repository: &str, tag: &str) -> Result<()> {
        let version = normalize_tag(tag);
        if version != self.version {
            return Err(Failure::failed(
                "Release 版本与请求的 tag 不一致。",
                format!("tag {tag:?} -> {version}, manifest {:?}", self.version),
            ));
        }
        if let Some(found) = &self.repository {
            if !found.eq_ignore_ascii_case(repository) {
                return Err(Failure::failed(
                    "Release 属于其他仓库。",
                    format!("manifest repository {found:?}, expected {repository:?}"),
                ));
            }
        }
        if let Some(found_tag) = &self.tag {
            if found_tag != tag {
                return Err(Failure::failed(
                    "Release tag 不一致。",
                    format!("manifest tag {found_tag:?}, requested {tag:?}"),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PackageBlock {
    #[serde(default)]
    pub asset: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub root: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PackageManifest {
    pub schema: u32,
    #[serde(rename = "playkit_version")]
    pub version: String,
    pub platform: String,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub source_commit: Option<String>,
    #[serde(default)]
    pub inputs_hash: Option<String>,
    #[serde(default)]
    pub package: Option<PackageBlock>,
    #[serde(default)]
    pub components: BTreeMap<String, serde_json::Value>,
}

impl PackageManifest {
    pub fn parse(data: &[u8], source: &str) -> Result<Self> {
        if data.len() as u64 > MAX_METADATA_BYTES {
            return Err(Failure::failed(
                "包内元数据过大。",
                format!("{source}: {} bytes", data.len()),
            ));
        }
        serde_json::from_slice(data)
            .map_err(|error| Failure::failed("包内元数据格式错误。", format!("{source}: {error}")))
    }

    pub fn check(&self, release: &ReleaseManifest, platform: &str, source: &str) -> Result<()> {
        if self.schema != 1 {
            return Err(Failure::failed(
                "包内元数据 schema 不受支持。",
                format!("{source}: schema {}", self.schema),
            ));
        }
        if self.version != release.version {
            return Err(Failure::failed(
                "包内版本与 Release 不一致。",
                format!("{source}: {:?} != {:?}", self.version, release.version),
            ));
        }
        if self.platform != platform {
            return Err(Failure::failed(
                "包内平台与当前平台不一致。",
                format!("{source}: {:?} != {platform:?}", self.platform),
            ));
        }
        if let (Some(expected), Some(actual)) = (&release.repository, &self.repository) {
            if !expected.eq_ignore_ascii_case(actual) {
                return Err(Failure::failed(
                    "包内仓库与 Release 不一致。",
                    format!("{source}: {actual:?} != {expected:?}"),
                ));
            }
        }
        if let (Some(expected), Some(actual)) = (&release.source_commit, &self.source_commit) {
            if expected != actual {
                return Err(Failure::failed(
                    "包内提交与 Release 不一致。",
                    format!("{source}: {actual:?} != {expected:?}"),
                ));
            }
        }
        if let (Some(expected), Some(actual)) = (&release.inputs_hash, &self.inputs_hash) {
            if expected != actual {
                return Err(Failure::failed(
                    "包内输入哈希与 Release 不一致。",
                    format!("{source}: {actual:?} != {expected:?}"),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionInfo {
    pub schema: u32,
    #[serde(rename = "playkit_version")]
    pub version: String,
    pub platform: String,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub source_commit: Option<String>,
    #[serde(default)]
    pub inputs_hash: Option<String>,
}

impl VersionInfo {
    pub fn parse(data: &[u8], source: &str) -> Result<Self> {
        if data.len() as u64 > MAX_METADATA_BYTES {
            return Err(Failure::failed(
                "包内版本信息过大。",
                format!("{source}: {} bytes", data.len()),
            ));
        }
        let info: Self = serde_json::from_slice(data).map_err(|error| {
            Failure::failed("包内版本信息格式错误。", format!("{source}: {error}"))
        })?;
        if info.schema != 1 {
            return Err(Failure::failed(
                "包内版本信息 schema 不受支持。",
                format!("{source}: schema {}", info.schema),
            ));
        }
        Ok(info)
    }

    pub fn check(&self, release: &ReleaseManifest, platform: &str, source: &str) -> Result<()> {
        if self.version != release.version {
            return Err(Failure::failed(
                "包内版本与 Release 不一致。",
                format!("{source}: {:?} != {:?}", self.version, release.version),
            ));
        }
        if self.platform != platform {
            return Err(Failure::failed(
                "包内平台与当前平台不一致。",
                format!("{source}: {:?} != {platform:?}", self.platform),
            ));
        }
        if let (Some(expected), Some(actual)) = (&release.source_commit, &self.source_commit) {
            if expected != actual {
                return Err(Failure::failed(
                    "包内提交与 Release 不一致。",
                    format!("{source}: {actual:?} != {expected:?}"),
                ));
            }
        }
        if let (Some(expected), Some(actual)) = (&release.inputs_hash, &self.inputs_hash) {
            if expected != actual {
                return Err(Failure::failed(
                    "包内输入哈希与 Release 不一致。",
                    format!("{source}: {actual:?} != {expected:?}"),
                ));
            }
        }
        Ok(())
    }
}

/// Parse the release `SHA256SUMS` file.  The format is the standard
/// `sha256sum` output; names are validated as plain asset file names.
pub fn parse_sha256sums(text: &str) -> Result<HashMap<String, String>> {
    let mut result: HashMap<String, String> = HashMap::new();
    for (index, raw) in text.lines().enumerate() {
        let line_number = index + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let digest = parts.next().unwrap_or("").trim();
        let mut name = parts.next().unwrap_or("").trim();
        if let Some(stripped) = name.strip_prefix('*') {
            name = stripped;
        }
        if let Some(stripped) = name.strip_prefix("./") {
            name = stripped;
        }
        if !is_sha256(digest) {
            return Err(Failure::failed(
                "SHA256SUMS 格式错误。",
                format!("line {line_number}: invalid digest"),
            ));
        }
        if !is_safe_asset_name(name) {
            return Err(Failure::failed(
                "SHA256SUMS 格式错误。",
                format!("line {line_number}: invalid file name {name:?}"),
            ));
        }
        if let Some(existing) = result.get(name) {
            if existing != digest {
                return Err(Failure::failed(
                    "SHA256SUMS 包含冲突的校验和。",
                    format!("line {line_number}: {name}"),
                ));
            }
            continue;
        }
        result.insert(name.to_string(), digest.to_ascii_lowercase());
    }
    if result.is_empty() {
        return Err(Failure::failed("SHA256SUMS 为空。", "no checksum lines"));
    }
    Ok(result)
}

pub fn expected_asset_name(version: &str, platform: &str) -> Result<String> {
    let extension = match platform {
        "windows-x64" => "zip",
        "linux-x64" => "tar.gz",
        other => {
            return Err(Failure::failed(
                "不支持的平台。",
                format!("platform {other:?}"),
            ))
        }
    };
    Ok(format!("mpv-PlayKit-{version}-{platform}.{extension}"))
}

pub fn normalize_tag(tag: &str) -> String {
    tag.strip_prefix('v').unwrap_or(tag).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release_json() -> String {
        r#"{
          "schema": 2,
          "playkit_version": "2026.11",
          "tag": "v2026.11",
          "repository": "owner/repo",
          "source_commit": "abc",
          "inputs_hash": "def",
          "platforms": {
            "windows-x64": {
              "asset": "mpv-PlayKit-2026.11-windows-x64.zip",
              "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "size": 123,
              "format": "zip"
            }
          },
          "updater": {"protocol": 1, "patch_schema": 1, "min_helper_version": "1.0.0"}
        }"#
        .to_string()
    }

    #[test]
    fn parses_release_manifest() {
        let manifest = ReleaseManifest::parse(release_json().as_bytes()).unwrap();
        assert_eq!(manifest.version, "2026.11");
        assert!(manifest.platform("windows-x64").is_ok());
        manifest.check_identity("owner/repo", "v2026.11").unwrap();
    }

    #[test]
    fn rejects_wrong_repository() {
        let manifest = ReleaseManifest::parse(release_json().as_bytes()).unwrap();
        assert!(manifest.check_identity("other/repo", "v2026.11").is_err());
    }

    #[test]
    fn rejects_newer_protocol() {
        let text = release_json().replace("\"protocol\": 1", "\"protocol\": 2");
        let error = ReleaseManifest::parse(text.as_bytes()).unwrap_err();
        assert_eq!(error.kind, crate::util::ErrorKind::Blocked);
    }

    #[test]
    fn rejects_bad_hash() {
        let text = release_json().replace(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "not-a-hash",
        );
        assert!(ReleaseManifest::parse(text.as_bytes()).is_err());
    }

    #[test]
    fn sha256sums_parsing() {
        let text = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  mpv-PlayKit-2026.11-windows-x64.zip\n\
                    bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb *manifest.json\n";
        let sums = parse_sha256sums(text).unwrap();
        assert!(sums.contains_key("manifest.json"));
        assert!(sums.contains_key("mpv-PlayKit-2026.11-windows-x64.zip"));
    }

    #[test]
    fn sha256sums_reject_traversal_names() {
        assert!(parse_sha256sums(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  ../evil\n"
        )
        .is_err());
    }

    #[test]
    fn sha256sums_reject_conflicts() {
        assert!(parse_sha256sums(
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  a.zip\n\
             bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  a.zip\n"
        )
        .is_err());
    }

    #[test]
    fn expected_names() {
        assert_eq!(
            expected_asset_name("2026.11", "windows-x64").unwrap(),
            "mpv-PlayKit-2026.11-windows-x64.zip"
        );
        assert_eq!(
            expected_asset_name("2026.11", "linux-x64").unwrap(),
            "mpv-PlayKit-2026.11-linux-x64.tar.gz"
        );
        assert!(expected_asset_name("2026.11", "macos-arm64").is_err());
    }
}
