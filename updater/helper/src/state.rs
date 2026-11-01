//! Persistent installation state: the official baseline, the generated output
//! of the last install, and the user layer inventory.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{managed_kind, ConfigKind};
use crate::util::{
    ensure_dir, hex, io_context, is_sha256, read_json, sha256_file, utc_now_iso, write_json,
    Failure, Result,
};

pub const STATE_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    pub schema: u32,
    pub installed_version: String,
    pub platform: String,
    #[serde(default)]
    pub installed_at: String,
    #[serde(default)]
    pub previous_version: Option<String>,
    #[serde(default)]
    pub updated_by: Option<String>,
    #[serde(default)]
    pub official: BTreeMap<String, String>,
    #[serde(default)]
    pub generated: BTreeMap<String, String>,
    #[serde(default)]
    pub patches: Vec<String>,
    /// Hash of every input that can change the generated configuration:
    /// patch files, user scripts, fonts and script options.  `repair
    /// --if-needed` compares it with the current user layer so an unchanged
    /// installation does not rewrite its configuration on every startup.
    #[serde(default)]
    pub user_fingerprint: Option<String>,
    #[serde(default)]
    pub patch_warnings: Vec<String>,
    #[serde(default)]
    pub user_scripts: Vec<String>,
    #[serde(default)]
    pub user_fonts: Vec<String>,
    #[serde(default)]
    pub user_script_opts: Vec<String>,
}

impl State {
    pub fn load(portable: &Path) -> Result<Option<Self>> {
        let path = state_path(portable);
        if !path.is_file() {
            return Ok(None);
        }
        let state: Self = read_json(&path).map_err(|error| {
            Failure::failed(
                "读取安装状态失败。",
                format!("{}: {}", path.display(), error.detail),
            )
        })?;
        if state.schema != STATE_SCHEMA {
            return Err(Failure::blocked(
                "安装状态版本不受支持，请重新安装 PlayKit。",
                format!("{}: schema {}", path.display(), state.schema),
            ));
        }
        Ok(Some(state))
    }

    pub fn save(&self, portable: &Path) -> Result<()> {
        let path = state_path(portable);
        write_json(&path, self)
    }

    pub fn official_path(&self, portable: &Path, relative: &str) -> PathBuf {
        portable
            .join(".playkit")
            .join("official")
            .join(relative.replace('\\', "/"))
    }

    pub fn generated_path(&self, portable: &Path, relative: &str) -> PathBuf {
        portable
            .join(".playkit")
            .join("generated")
            .join(relative.replace('\\', "/"))
    }
}

pub fn state_path(portable: &Path) -> PathBuf {
    portable.join(".playkit").join("state.json")
}

pub fn official_dir(portable: &Path) -> PathBuf {
    portable.join(".playkit").join("official")
}

pub fn generated_dir(portable: &Path) -> PathBuf {
    portable.join(".playkit").join("generated")
}

pub fn patch_dir(portable: &Path) -> PathBuf {
    portable.join("user").join("patches")
}

pub fn auto_patch_dir(portable: &Path) -> PathBuf {
    patch_dir(portable).join("auto")
}

/// Schema-2 manual patches live in a subdirectory so a helper that only
/// understands schema 1 ignores them instead of failing the whole update.
pub fn v2_patch_dir(portable: &Path) -> PathBuf {
    patch_dir(portable).join("v2")
}

/// Schema-2 automatic patches.  Kept separate from `auto/` for the same
/// compatibility reason.
pub fn auto_v2_patch_dir(portable: &Path) -> PathBuf {
    auto_patch_dir(portable).join("v2")
}

pub fn user_scripts_dir(portable: &Path) -> PathBuf {
    portable.join("user").join("scripts")
}

pub fn user_fonts_dir(portable: &Path) -> PathBuf {
    portable.join("user").join("fonts")
}

pub fn user_script_opts_dir(portable: &Path) -> PathBuf {
    portable.join("user").join("script-opts")
}

pub fn user_backup_dir(portable: &Path) -> PathBuf {
    portable.join("user").join("backup")
}

/// Fingerprint the complete user layer that feeds configuration generation.
///
/// Patch files, scripts and script options are hashed by content.  Fonts can be
/// large and are not part of the generated configuration, so they use size and
/// modification time instead.  The format tag and the helper/version strings
/// make a helper upgrade or PlayKit update force one repair pass.
pub(crate) fn user_layer_fingerprint(
    portable: &Path,
    version: &str,
    platform: &str,
) -> Result<String> {
    let mut hasher = Sha256::new();
    feed(&mut hasher, b"format", b"playkit-user-layer-v1");
    feed(&mut hasher, b"version", version.as_bytes());
    feed(&mut hasher, b"platform", platform.as_bytes());
    feed(&mut hasher, b"helper", env!("CARGO_PKG_VERSION").as_bytes());
    for (relative, content) in [
        ("user/patches", true),
        ("user/scripts", true),
        ("user/fonts", false),
        ("user/script-opts", true),
    ] {
        feed(&mut hasher, b"tree", relative.as_bytes());
        let root = portable.join(relative);
        hash_tree(&mut hasher, &root, "", content)?;
    }
    Ok(hex(&hasher.finalize()))
}

fn hash_tree(hasher: &mut Sha256, current: &Path, relative: &str, content: bool) -> Result<()> {
    let entries = match fs::read_dir(current) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            feed(hasher, b"missing", relative.as_bytes());
            return Ok(());
        }
        Err(error) => return Err(io_context(&format!("read {}", current.display()), error)),
    };
    let mut entries = entries
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|error| io_context(&format!("read {}", current.display()), error))?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let child = if relative.is_empty() {
            name.into_owned()
        } else {
            format!("{relative}/{name}")
        };
        let file_type = entry
            .file_type()
            .map_err(|error| io_context(&format!("stat {}", path.display()), error))?;
        if file_type.is_dir() {
            feed(hasher, b"dir", child.as_bytes());
            hash_tree(hasher, &path, &child, content)?;
        } else if file_type.is_file() {
            let metadata = entry
                .metadata()
                .map_err(|error| io_context(&format!("stat {}", path.display()), error))?;
            if content {
                let digest = sha256_file(&path)?;
                feed(hasher, b"file", child.as_bytes());
                feed(hasher, b"size", &metadata.len().to_le_bytes());
                feed(hasher, b"sha256", digest.as_bytes());
            } else {
                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map(|duration| duration.as_nanos())
                    .unwrap_or(0);
                feed(hasher, b"file-meta", child.as_bytes());
                feed(hasher, b"size", &metadata.len().to_le_bytes());
                feed(hasher, b"mtime", &modified.to_le_bytes());
            }
        } else if file_type.is_symlink() {
            let target = fs::read_link(&path)
                .map_err(|error| io_context(&format!("readlink {}", path.display()), error))?;
            feed(hasher, b"symlink", child.as_bytes());
            feed(hasher, b"target", target.to_string_lossy().as_bytes());
        } else {
            feed(hasher, b"other", child.as_bytes());
        }
    }
    Ok(())
}

fn feed(hasher: &mut Sha256, label: &[u8], data: &[u8]) {
    hasher.update((label.len() as u64).to_le_bytes());
    hasher.update(label);
    hasher.update((data.len() as u64).to_le_bytes());
    hasher.update(data);
}

/// All configuration files the updater owns and patches.
pub fn discover_managed(portable: &Path) -> Vec<String> {
    let mut result = BTreeSet::new();
    for name in ["mpv.conf", "input.conf", "profiles.conf", "platform.conf"] {
        if portable.join(name).is_file() {
            result.insert(name.to_string());
        }
    }
    if let Ok(entries) = fs::read_dir(portable.join("script-opts")) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let relative = format!("script-opts/{name}");
            if managed_kind(&relative).is_some() {
                result.insert(relative);
            }
        }
    }
    result.into_iter().collect()
}

/// Targets that need attention: official files plus everything recorded in the
/// previous state, so a deleted file can still be captured as a removal.
pub fn managed_targets(portable: &Path, state: Option<&State>) -> Vec<String> {
    let mut result: BTreeSet<String> = discover_managed(portable).into_iter().collect();
    if let Some(state) = state {
        result.extend(state.official.keys().cloned());
        result.extend(state.generated.keys().cloned());
    }
    result.into_iter().collect()
}

pub fn patch_targets(portable: &Path) -> Result<Vec<String>> {
    let mut result = BTreeSet::new();
    for directory in [patch_dir(portable), auto_patch_dir(portable)] {
        for root in [directory.clone(), directory.join("v2")] {
            if !root.is_dir() {
                continue;
            }
            collect_patch_targets(&root, &root, &mut result)?;
        }
    }
    Ok(result.into_iter().collect())
}

fn collect_patch_targets(root: &Path, current: &Path, result: &mut BTreeSet<String>) -> Result<()> {
    let entries = fs::read_dir(current)
        .map_err(|error| io_context(&format!("read {}", current.display()), error))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| io_context(&format!("read {}", current.display()), error))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| io_context(&format!("stat {}", path.display()), error))?;
        if file_type.is_dir() {
            collect_patch_targets(root, &path, result)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|error| Failure::internal(format!("patch path: {error}")))?;
        let text = relative.to_string_lossy().replace('\\', "/");
        let Some(target) = text.strip_suffix(".patch") else {
            continue;
        };
        if managed_kind(target).is_some() {
            result.insert(target.to_string());
        }
    }
    Ok(())
}

pub fn write_snapshot(source: &Path, destination: &Path) -> Result<String> {
    if let Some(parent) = destination.parent() {
        ensure_dir(parent)?;
    }
    fs::copy(source, destination).map_err(|error| {
        io_context(
            &format!("copy {} -> {}", source.display(), destination.display()),
            error,
        )
    })?;
    sha256_file(destination)
}

pub fn read_snapshot(path: &Path) -> Result<Option<String>> {
    if !path.is_file() {
        return Ok(None);
    }
    sha256_file(path).map(Some)
}

pub fn verify_state(portable: &Path) -> Result<Vec<String>> {
    let Some(state) = State::load(portable)? else {
        return Err(Failure::failed(
            "安装状态缺失。",
            format!("{}", state_path(portable).display()),
        ));
    };
    let mut warnings = Vec::new();
    for (relative, expected) in &state.official {
        if managed_kind(relative).is_none() {
            warnings.push(format!("状态中记录了非托管配置：{relative}"));
            continue;
        }
        let path = official_dir(portable).join(relative);
        if !path.is_file() {
            return Err(Failure::failed(
                "官方配置基线缺失。",
                format!("{}", path.display()),
            ));
        }
        let actual = sha256_file(&path)?;
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(Failure::failed(
                "官方配置基线校验失败。",
                format!("{relative}: {actual} != {expected}"),
            ));
        }
    }
    for (relative, expected) in &state.generated {
        if !is_sha256(expected) {
            return Err(Failure::failed(
                "安装状态中的哈希无效。",
                format!("{relative}: {expected}"),
            ));
        }
        let path = generated_dir(portable).join(relative);
        if path.is_file() {
            let actual = sha256_file(&path)?;
            if !actual.eq_ignore_ascii_case(expected) {
                warnings.push(format!("上次生成的配置副本已变化：{relative}"));
            }
        } else {
            warnings.push(format!("上次生成的配置副本缺失：{relative}"));
        }
    }
    for relative in &state.patches {
        let manual = patch_dir(portable).join(format!("{relative}.patch"));
        let automatic = auto_patch_dir(portable).join(format!("{relative}.patch"));
        let manual_v2 = v2_patch_dir(portable).join(format!("{relative}.patch"));
        let automatic_v2 = auto_v2_patch_dir(portable).join(format!("{relative}.patch"));
        if !manual.is_file()
            && !automatic.is_file()
            && !manual_v2.is_file()
            && !automatic_v2.is_file()
        {
            warnings.push(format!("patch 文件缺失：{relative}"));
        }
    }
    Ok(warnings)
}

pub fn new_package_state(version: &str, platform: &str) -> State {
    State {
        schema: STATE_SCHEMA,
        installed_version: version.to_string(),
        platform: platform.to_string(),
        installed_at: utc_now_iso(),
        previous_version: None,
        updated_by: None,
        official: BTreeMap::new(),
        generated: BTreeMap::new(),
        patches: Vec::new(),
        user_fingerprint: None,
        patch_warnings: Vec::new(),
        user_scripts: Vec::new(),
        user_fonts: Vec::new(),
        user_script_opts: Vec::new(),
    }
}

pub fn kind_for(relative: &str) -> Result<ConfigKind> {
    managed_kind(relative)
        .ok_or_else(|| Failure::blocked("不是可管理的配置文件。", relative.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write(path: &Path, data: impl AsRef<[u8]>) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, data).unwrap();
    }

    #[test]
    fn user_layer_fingerprint_tracks_inputs_and_is_deterministic() {
        let dir = tempdir().unwrap();
        let portable = dir.path();
        let empty = user_layer_fingerprint(portable, "2026.11", "windows-x64").unwrap();
        assert_eq!(
            empty,
            user_layer_fingerprint(portable, "2026.11", "windows-x64").unwrap()
        );

        let patch = portable.join("user/patches/v2/mpv.conf.patch");
        write(&patch, "schema=2\ntarget=mpv.conf\n\n[global]\nvolume=55\n");
        let with_patch = user_layer_fingerprint(portable, "2026.11", "windows-x64").unwrap();
        assert_ne!(empty, with_patch);

        write(&patch, "schema=2\ntarget=mpv.conf\n\n[global]\nvolume=56\n");
        let changed_patch = user_layer_fingerprint(portable, "2026.11", "windows-x64").unwrap();
        assert_ne!(with_patch, changed_patch);

        assert_ne!(
            changed_patch,
            user_layer_fingerprint(portable, "2026.12", "windows-x64").unwrap()
        );

        let font = portable.join("user/fonts/test.ttf");
        write(&font, b"font");
        let with_font = user_layer_fingerprint(portable, "2026.11", "windows-x64").unwrap();
        assert_ne!(changed_patch, with_font);
        write(&font, b"longer-font");
        assert_ne!(
            with_font,
            user_layer_fingerprint(portable, "2026.11", "windows-x64").unwrap()
        );

        let script = portable.join("user/scripts/mine.lua");
        write(&script, "-- one\n");
        let with_script = user_layer_fingerprint(portable, "2026.11", "windows-x64").unwrap();
        write(&script, "-- two\n");
        assert_ne!(
            with_script,
            user_layer_fingerprint(portable, "2026.11", "windows-x64").unwrap()
        );

        let before_backup = user_layer_fingerprint(portable, "2026.11", "windows-x64").unwrap();
        let backup = portable.join("user/backup/old/mpv.conf");
        write(&backup, "volume=1\n");
        assert_eq!(
            before_backup,
            user_layer_fingerprint(portable, "2026.11", "windows-x64").unwrap()
        );
    }
}
