//! Persistent installation state: the official baseline, the generated output
//! of the last install, and the user layer inventory.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{managed_kind, ConfigKind};
use crate::util::{
    ensure_dir, io_context, is_sha256, read_json, sha256_file, utc_now_iso, write_json, Failure,
    Result,
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
        if !directory.is_dir() {
            continue;
        }
        collect_patch_targets(&directory, &directory, &mut result)?;
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
        let patch = patch_dir(portable).join(format!("{relative}.patch"));
        if !patch.is_file() {
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
