//! Versioned semantic patch files.
//!
//! A patch describes intent ("volume is 70", "remove this binding") rather than
//! a historical copy of a configuration file.  It is applied to the official
//! configuration of whatever version is being installed.

use std::path::Path;

use crate::config::ConfigKind;
use crate::util::{relative_path, Failure, Result};

pub const PATCH_SCHEMA: u32 = 1;
const MAX_PATCH_BYTES: usize = 1024 * 1024;
const MAX_ENTRIES: usize = 5000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchOp {
    Set,
    Remove,
    Append,
}

#[derive(Debug, Clone)]
pub struct PatchEntry {
    pub section: Option<String>,
    pub key: String,
    pub op: PatchOp,
    pub value: String,
    /// 1-based line in the patch file; 0 for entries generated at runtime.
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct PatchDoc {
    pub schema: u32,
    pub target: Option<String>,
    pub source: String,
    pub entries: Vec<PatchEntry>,
}

impl PatchDoc {
    pub fn parse(text: &str, source: &str, kind: ConfigKind) -> Result<Self> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        if text.len() > MAX_PATCH_BYTES {
            return Err(Failure::blocked(
                "用户 patch 文件过大。",
                format!("{source}: {} bytes", text.len()),
            ));
        }
        let mut schema: Option<u32> = None;
        let mut target: Option<String> = None;
        let mut section: Option<String> = None;
        let mut entries: Vec<PatchEntry> = Vec::new();
        let mut seen_content = false;
        for (index, raw) in text.split('\n').enumerate() {
            let line_number = index + 1;
            let raw = raw.strip_suffix('\r').unwrap_or(raw);
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if trimmed.starts_with('[') {
                if !trimmed.ends_with(']') {
                    return Err(Failure::blocked(
                        "用户 patch 格式错误。",
                        format!("{source}:{line_number}: unterminated section header"),
                    ));
                }
                let name = &trimmed[1..trimmed.len() - 1];
                if name.is_empty() || name.contains('[') || name.contains(']') {
                    return Err(Failure::blocked(
                        "用户 patch 格式错误。",
                        format!("{source}:{line_number}: invalid section name"),
                    ));
                }
                section = if name.eq_ignore_ascii_case("global") {
                    None
                } else {
                    Some(name.to_string())
                };
                seen_content = true;
                continue;
            }
            if let Some(value) = trimmed.strip_prefix("schema=") {
                if schema.is_some() {
                    return Err(Failure::blocked(
                        "用户 patch 重复声明 schema。",
                        format!("{source}:{line_number}"),
                    ));
                }
                if seen_content || !entries.is_empty() {
                    return Err(Failure::blocked(
                        "用户 patch 的 schema 必须位于其他内容之前。",
                        format!("{source}:{line_number}"),
                    ));
                }
                let parsed = value.trim().parse::<u32>().map_err(|_| {
                    Failure::blocked(
                        "用户 patch 的 schema 无效。",
                        format!("{source}:{line_number}: {value:?}"),
                    )
                })?;
                if parsed != PATCH_SCHEMA {
                    return Err(Failure::blocked(
                        "用户 patch 的 schema 版本不受支持。",
                        format!("{source}:{line_number}: schema={parsed}"),
                    ));
                }
                schema = Some(parsed);
                continue;
            }
            if let Some(value) = trimmed.strip_prefix("target=") {
                if seen_content || !entries.is_empty() {
                    return Err(Failure::blocked(
                        "用户 patch 的 target 必须位于操作之前。",
                        format!("{source}:{line_number}"),
                    ));
                }
                let value = value.trim().replace('\\', "/");
                relative_path(&value).map_err(|error| {
                    Failure::blocked(
                        "用户 patch 的 target 路径无效。",
                        format!("{source}:{line_number}: {}", error.detail),
                    )
                })?;
                target = Some(value);
                continue;
            }
            if schema.is_none() {
                return Err(Failure::blocked(
                    "用户 patch 缺少 schema=1。",
                    format!("{source}:{line_number}"),
                ));
            }
            seen_content = true;
            let (op, rest) = if let Some(rest) = trimmed.strip_prefix('+') {
                (PatchOp::Append, rest)
            } else if let Some(rest) = trimmed.strip_prefix('-') {
                (PatchOp::Remove, rest)
            } else if let Some(rest) = trimmed.strip_prefix('!') {
                (PatchOp::Remove, rest)
            } else {
                (PatchOp::Set, trimmed)
            };
            if op == PatchOp::Remove {
                let key = rest.trim();
                if key.is_empty() || key.contains('=') {
                    return Err(Failure::blocked(
                        "用户 patch 的删除语法错误。",
                        format!("{source}:{line_number}: {trimmed:?}"),
                    ));
                }
                if !kind.validate_key(key) {
                    return Err(Failure::blocked(
                        "用户 patch 包含无效的配置键。",
                        format!("{source}:{line_number}: {key:?}"),
                    ));
                }
                entries.push(PatchEntry {
                    section: section.clone(),
                    key: key.to_string(),
                    op,
                    value: String::new(),
                    line: line_number,
                });
                continue;
            }
            let Some(eq) = rest.find('=') else {
                return Err(Failure::blocked(
                    "用户 patch 无法识别。",
                    format!("{source}:{line_number}: {trimmed:?}"),
                ));
            };
            let key = rest[..eq].trim();
            let value = rest[eq + 1..].trim();
            if !kind.validate_key(key) {
                return Err(Failure::blocked(
                    "用户 patch 包含无效的配置键。",
                    format!("{source}:{line_number}: {key:?}"),
                ));
            }
            if kind == ConfigKind::Input && value.is_empty() {
                return Err(Failure::blocked(
                    "input.conf 的用户 patch 缺少命令。",
                    format!("{source}:{line_number}: {key:?}"),
                ));
            }
            entries.push(PatchEntry {
                section: section.clone(),
                key: key.to_string(),
                op,
                value: value.to_string(),
                line: line_number,
            });
        }
        let Some(schema) = schema else {
            return Err(Failure::blocked(
                "用户 patch 缺少 schema=1。",
                source.to_string(),
            ));
        };
        if entries.len() > MAX_ENTRIES {
            return Err(Failure::blocked(
                "用户 patch 条目过多。",
                format!("{source}: {} entries", entries.len()),
            ));
        }
        Ok(Self {
            schema,
            target,
            source: source.to_string(),
            entries,
        })
    }

    pub fn load(path: &Path, expected_target: &str, kind: ConfigKind) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|error| {
            Failure::blocked(
                "读取用户 patch 失败。",
                format!("{}: {error}", path.display()),
            )
        })?;
        let text = String::from_utf8(bytes).map_err(|error| {
            Failure::blocked(
                "用户 patch 不是有效的 UTF-8。",
                format!("{}: {error}", path.display()),
            )
        })?;
        let doc = Self::parse(&text, &path.display().to_string(), kind)?;
        if let Some(target) = &doc.target {
            if target != expected_target {
                return Err(Failure::blocked(
                    "用户 patch 的 target 与文件名不一致。",
                    format!(
                        "{}: target={target}, expected {expected_target}",
                        path.display()
                    ),
                ));
            }
        }
        Ok(doc)
    }

    pub fn render(&self) -> String {
        let mut output = String::new();
        output.push_str("# PlayKit 用户配置 patch（schema 1）\n");
        output.push_str("# 由 playkit-updater 维护；这里只保存配置意图，不保存旧配置副本。\n");
        output.push_str("schema=1\n");
        if let Some(target) = &self.target {
            output.push_str(&format!("target={target}\n"));
        }
        let mut current_section: Option<Option<String>> = None;
        for entry in &self.entries {
            if current_section.as_ref() != Some(&entry.section) {
                output.push('\n');
                match &entry.section {
                    Some(name) => output.push_str(&format!("[{name}]\n")),
                    None => output.push_str("[global]\n"),
                }
                current_section = Some(entry.section.clone());
            }
            match entry.op {
                PatchOp::Set => output.push_str(&format!("{}={}\n", entry.key, entry.value)),
                PatchOp::Append => output.push_str(&format!("+{}={}\n", entry.key, entry.value)),
                PatchOp::Remove => output.push_str(&format!("-{}\n", entry.key)),
            }
        }
        output
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_operations_and_sections() {
        let doc = PatchDoc::parse(
            "# comment\nschema=1\ntarget=profiles.conf\n\n[gpu-hq]\nscale=ewa_lanczos\n-volume\n+extra=1\n",
            "test",
            ConfigKind::Profiles,
        )
        .unwrap();
        assert_eq!(doc.entries.len(), 3);
        assert_eq!(doc.entries[0].section.as_deref(), Some("gpu-hq"));
        assert_eq!(doc.entries[1].op, PatchOp::Remove);
        assert_eq!(doc.entries[2].op, PatchOp::Append);
    }

    #[test]
    fn global_section_maps_to_none() {
        let doc =
            PatchDoc::parse("schema=1\n[global]\nvolume=70\n", "test", ConfigKind::Mpv).unwrap();
        assert!(doc.entries[0].section.is_none());
    }

    #[test]
    fn missing_schema_is_rejected() {
        let error = PatchDoc::parse("volume=70\n", "test", ConfigKind::Mpv).unwrap_err();
        assert_eq!(error.kind, crate::util::ErrorKind::Blocked);
    }

    #[test]
    fn wrong_schema_is_rejected() {
        assert!(PatchDoc::parse("schema=2\nvolume=70\n", "test", ConfigKind::Mpv).is_err());
    }

    #[test]
    fn malformed_line_is_rejected() {
        assert!(PatchDoc::parse("schema=1\njust some text\n", "test", ConfigKind::Mpv).is_err());
    }

    #[test]
    fn traversal_target_is_rejected() {
        assert!(PatchDoc::parse(
            "schema=1\ntarget=../mpv.conf\nvolume=70\n",
            "test",
            ConfigKind::Mpv
        )
        .is_err());
    }

    #[test]
    fn input_value_may_contain_equals_and_label() {
        let doc = PatchDoc::parse(
            "schema=1\na=cycle pause #! 播放 > 暂停\n",
            "test",
            ConfigKind::Input,
        )
        .unwrap();
        assert_eq!(doc.entries[0].value, "cycle pause #! 播放 > 暂停");
    }

    #[test]
    fn render_roundtrips() {
        let doc = PatchDoc::parse(
            "schema=1\ntarget=mpv.conf\n[global]\nvolume=70\n-mute\n[gpu-hq]\nscale=ewa_lanczos\n",
            "test",
            ConfigKind::Profiles,
        )
        .unwrap();
        let text = doc.render();
        let again = PatchDoc::parse(&text, "test", ConfigKind::Profiles).unwrap();
        assert_eq!(again.entries.len(), 3);
        assert_eq!(again.entries[2].section.as_deref(), Some("gpu-hq"));
    }

    #[test]
    fn duplicate_entries_are_allowed() {
        let doc =
            PatchDoc::parse("schema=1\nvolume=70\nvolume=80\n", "test", ConfigKind::Mpv).unwrap();
        assert_eq!(doc.entries.len(), 2);
    }

    #[test]
    fn empty_input_value_is_rejected() {
        assert!(PatchDoc::parse("schema=1\na=\n", "test", ConfigKind::Input).is_err());
    }
}
