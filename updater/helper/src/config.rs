//! Semantic parsing and patching of the small set of user-editable mpv
//! configuration files.
//!
//! The updater never regenerates these files from scratch.  It keeps the
//! official file byte-for-byte except for the lines a user patch touches, so
//! comments, ordering and platform-specific options stay exactly as shipped.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::patch::{PatchDoc, PatchEntry, PatchOp};
use crate::util::{Failure, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigKind {
    /// `mpv.conf` and `platform.conf`: `key=value`, no sections.
    Mpv,
    /// `input.conf`: `key=command`, optional trailing uosc `#!` label.
    Input,
    /// `profiles.conf`: `[profile]` sections containing `key=value`.
    Profiles,
    /// `script-opts/*.conf`: `key=value`, no sections.
    ScriptOpts,
}

impl ConfigKind {
    pub fn allows_sections(self) -> bool {
        matches!(self, ConfigKind::Profiles)
    }

    pub fn key_matches(self, left: &str, right: &str) -> bool {
        if self == ConfigKind::Input {
            left.eq_ignore_ascii_case(right)
        } else {
            left == right
        }
    }

    pub fn validate_key(self, key: &str) -> bool {
        if key.is_empty() || key.len() > 200 {
            return false;
        }
        match self {
            ConfigKind::Input => key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'-')),
            _ => key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ConfigKind::Mpv => "mpv",
            ConfigKind::Input => "input",
            ConfigKind::Profiles => "profiles",
            ConfigKind::ScriptOpts => "script-opts",
        }
    }
}

/// Map a path relative to `portable_config/` to the config kind the updater is
/// allowed to patch.  Everything else (menu.conf, scripts, fonts, ...) is not
/// a patch target.
pub fn managed_kind(relative: &str) -> Option<ConfigKind> {
    let relative = relative.replace('\\', "/");
    match relative.as_str() {
        "mpv.conf" | "platform.conf" => Some(ConfigKind::Mpv),
        "input.conf" => Some(ConfigKind::Input),
        "profiles.conf" => Some(ConfigKind::Profiles),
        _ => {
            let rest = relative.strip_prefix("script-opts/")?;
            if rest.is_empty()
                || rest.contains('/')
                || rest.starts_with('.')
                || !rest.ends_with(".conf")
                || rest.len() > 200
            {
                return None;
            }
            if !rest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            {
                return None;
            }
            Some(ConfigKind::ScriptOpts)
        }
    }
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub indent: String,
    pub key: String,
    /// Exact text between the key and the value: `" "`, `"="`, `" = "`, ...
    pub separator: String,
    /// Trimmed value, without an input.conf label.
    pub value: String,
    /// input.conf only: the trailing `#!` label including its leading space.
    pub label: Option<String>,
}

impl Entry {
    pub fn full_value(&self) -> String {
        match &self.label {
            Some(label) => format!("{}{}", self.value, label),
            None => self.value.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ParsedLine {
    Blank,
    Comment,
    Section { name: String },
    Entry(Entry),
    Other,
}

#[derive(Debug, Clone)]
pub struct Line {
    pub raw: String,
    pub parsed: ParsedLine,
}

#[derive(Debug, Clone)]
pub struct ConfigDoc {
    pub kind: ConfigKind,
    pub lines: Vec<Line>,
    pub line_ending: String,
    pub trailing_newline: bool,
}

#[derive(Debug, Clone)]
pub struct EffectiveEntry {
    pub section: Option<String>,
    pub key: String,
    pub value: String,
    pub label: Option<String>,
    pub line_index: usize,
}

impl EffectiveEntry {
    pub fn full_value(&self) -> String {
        match &self.label {
            Some(label) => format!("{}{}", self.value, label),
            None => self.value.clone(),
        }
    }
}

impl ConfigDoc {
    pub fn parse(text: &str, kind: ConfigKind) -> Result<Self> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        if text.contains('\0') {
            return Err(Failure::failed(
                "配置文件包含无效字符。",
                "config contains a NUL byte",
            ));
        }
        let line_ending = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let trailing_newline = text.ends_with('\n');
        let mut raw_lines: Vec<&str> = text.split('\n').collect();
        if trailing_newline {
            raw_lines.pop();
        }
        let mut lines = Vec::with_capacity(raw_lines.len());
        for raw in raw_lines {
            let raw = raw.strip_suffix('\r').unwrap_or(raw);
            lines.push(parse_line(raw, kind));
        }
        Ok(Self {
            kind,
            lines,
            line_ending: line_ending.to_string(),
            trailing_newline,
        })
    }

    pub fn read(path: &Path, kind: ConfigKind) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|error| {
            Failure::failed("读取配置文件失败。", format!("{}: {error}", path.display()))
        })?;
        let text = String::from_utf8(bytes).map_err(|error| {
            Failure::failed(
                "配置文件不是有效的 UTF-8。",
                format!("{}: {error}", path.display()),
            )
        })?;
        Self::parse(&text, kind)
    }

    pub fn render(&self) -> String {
        let mut output = String::new();
        for (index, line) in self.lines.iter().enumerate() {
            if index > 0 {
                output.push_str(&self.line_ending);
            }
            output.push_str(&line.raw);
        }
        if (self.trailing_newline || !self.lines.is_empty()) && !self.lines.is_empty() {
            output.push_str(&self.line_ending);
        }
        output
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        crate::util::atomic_write(path, self.render().as_bytes())
    }

    pub fn effective(&self) -> Vec<EffectiveEntry> {
        let mut result: Vec<EffectiveEntry> = Vec::new();
        let mut index_by_key: HashMap<(Option<String>, String), usize> = HashMap::new();
        let mut section: Option<String> = None;
        for (line_index, line) in self.lines.iter().enumerate() {
            match &line.parsed {
                ParsedLine::Section { name } => {
                    section = Some(name.clone());
                }
                ParsedLine::Entry(entry) => {
                    let key = normalize_key(self.kind, &entry.key);
                    let lookup = (section.clone(), key.clone());
                    let item = EffectiveEntry {
                        section: section.clone(),
                        key: entry.key.clone(),
                        value: entry.value.clone(),
                        label: entry.label.clone(),
                        line_index,
                    };
                    if let Some(existing) = index_by_key.get(&lookup) {
                        result[*existing] = item;
                    } else {
                        index_by_key.insert(lookup, result.len());
                        result.push(item);
                    }
                }
                _ => {}
            }
        }
        result
    }

    /// Apply a patch.  Returns non-fatal warnings; malformed or impossible
    /// operations are fatal so that a user's intent is never silently dropped.
    pub fn apply(&mut self, patch: &PatchDoc) -> Result<Vec<String>> {
        let mut warnings = Vec::new();
        for entry in &patch.entries {
            if entry.section.is_some() && !self.kind.allows_sections() {
                return Err(Failure::blocked(
                    "用户 patch 使用了不支持的配置段。",
                    format!(
                        "{}:{}: [{}] is not valid for {}",
                        patch.source,
                        entry.line,
                        entry.section.as_deref().unwrap_or(""),
                        self.kind.name()
                    ),
                ));
            }
            if !self.kind.validate_key(&entry.key) {
                return Err(Failure::blocked(
                    "用户 patch 包含无效的配置键。",
                    format!("{}:{}: key {:?}", patch.source, entry.line, entry.key),
                ));
            }
            match entry.op {
                PatchOp::Set => self.set(entry, &mut warnings)?,
                PatchOp::Remove => self.remove(entry, &mut warnings),
                PatchOp::Append => self.append(entry, &mut warnings)?,
            }
        }
        Ok(warnings)
    }

    fn set(&mut self, entry: &PatchEntry, _warnings: &mut Vec<String>) -> Result<()> {
        let section = entry.section.as_deref();
        let bounds = self.scope_bounds(section);
        let mut found: Option<usize> = None;
        if let Some((start, end)) = bounds {
            for index in start..end {
                if let ParsedLine::Entry(existing) = &self.lines[index].parsed {
                    if self.kind.key_matches(&existing.key, &entry.key) {
                        found = Some(index);
                    }
                }
            }
        }
        if let Some(index) = found {
            if let ParsedLine::Entry(existing) = &self.lines[index].parsed {
                let (value, label) = split_value_for_kind(self.kind, &entry.value);
                let final_label = if self.kind == ConfigKind::Input {
                    match label {
                        Some(label) => Some(label),
                        None => existing.label.clone(),
                    }
                } else {
                    None
                };
                let key = existing.key.clone();
                let indent = existing.indent.clone();
                let separator = existing.separator.clone();
                let raw = format!(
                    "{}{}{}{}{}",
                    indent,
                    key,
                    separator,
                    value,
                    final_label.as_deref().unwrap_or("")
                );
                self.lines[index] = Line {
                    raw,
                    parsed: ParsedLine::Entry(Entry {
                        indent,
                        key,
                        separator,
                        value,
                        label: final_label,
                    }),
                };
            }
            return Ok(());
        }
        self.insert_entry(entry, false)
    }

    fn append(&mut self, entry: &PatchEntry, _warnings: &mut Vec<String>) -> Result<()> {
        self.insert_entry(entry, true)
    }

    fn insert_entry(&mut self, entry: &PatchEntry, _append: bool) -> Result<()> {
        let section = entry.section.as_deref();
        let (value, label) = split_value_for_kind(self.kind, &entry.value);
        let separator = if self.kind == ConfigKind::Input {
            " "
        } else {
            "="
        };
        let new_line = Line {
            raw: format!(
                "{}{}{}{}",
                entry.key,
                separator,
                value,
                label.as_deref().unwrap_or("")
            ),
            parsed: ParsedLine::Entry(Entry {
                indent: String::new(),
                key: entry.key.clone(),
                separator: separator.to_string(),
                value,
                label,
            }),
        };
        if let Some((start, end)) = self.scope_bounds(section) {
            let insert_at = insertion_index(&self.lines, start, end);
            self.lines.insert(insert_at, new_line);
            return Ok(());
        }
        if section.is_none() || !self.kind.allows_sections() {
            // No section requested and no existing scope: append at EOF.
            self.append_line_at_end(new_line);
            return Ok(());
        }
        // A profile section referenced by the patch does not exist yet.  Create
        // it at the end of the file, which is how a user would add a profile.
        if !self.lines.is_empty()
            && !matches!(
                self.lines.last().map(|line| &line.parsed),
                Some(ParsedLine::Blank)
            )
        {
            self.lines.push(Line {
                raw: String::new(),
                parsed: ParsedLine::Blank,
            });
        }
        let name = section.unwrap_or_default();
        self.lines.push(Line {
            raw: format!("[{name}]"),
            parsed: ParsedLine::Section {
                name: name.to_string(),
            },
        });
        self.lines.push(new_line);
        Ok(())
    }

    fn append_line_at_end(&mut self, line: Line) {
        if let Some(last) = self.lines.last() {
            if !matches!(last.parsed, ParsedLine::Blank) {
                self.lines.push(Line {
                    raw: String::new(),
                    parsed: ParsedLine::Blank,
                });
            }
        }
        self.lines.push(line);
    }

    fn remove(&mut self, entry: &PatchEntry, warnings: &mut Vec<String>) {
        let section = entry.section.as_deref();
        let Some((start, end)) = self.scope_bounds(section) else {
            warnings.push(format!(
                "{}: [{}] 配置段不存在，无法删除 {}",
                entry.key,
                section.unwrap_or("global"),
                entry.key
            ));
            return;
        };
        let mut indices = Vec::new();
        for index in start..end {
            if let ParsedLine::Entry(existing) = &self.lines[index].parsed {
                if self.kind.key_matches(&existing.key, &entry.key) {
                    indices.push(index);
                }
            }
        }
        if indices.is_empty() {
            warnings.push(format!(
                "{}: 要删除的键不存在（[{}]）",
                entry.key,
                section.unwrap_or("global")
            ));
            return;
        }
        for index in indices.into_iter().rev() {
            self.lines.remove(index);
        }
    }

    /// Half-open line range of a scope, excluding the `[section]` header.
    fn scope_bounds(&self, section: Option<&str>) -> Option<(usize, usize)> {
        match section {
            None => {
                let end = self
                    .lines
                    .iter()
                    .position(|line| matches!(line.parsed, ParsedLine::Section { .. }))
                    .unwrap_or(self.lines.len());
                Some((0, end))
            }
            Some(name) => {
                let header = self.lines.iter().position(|line| {
                    matches!(&line.parsed, ParsedLine::Section { name: found } if found == name)
                })?;
                let end = self.lines[header + 1..]
                    .iter()
                    .position(|line| matches!(line.parsed, ParsedLine::Section { .. }))
                    .map(|offset| header + 1 + offset)
                    .unwrap_or(self.lines.len());
                Some((header + 1, end))
            }
        }
    }
}

fn parse_line(raw: &str, kind: ConfigKind) -> Line {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Line {
            raw: raw.to_string(),
            parsed: ParsedLine::Blank,
        };
    }
    if trimmed.starts_with('#') {
        return Line {
            raw: raw.to_string(),
            parsed: ParsedLine::Comment,
        };
    }
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        let name = &trimmed[1..trimmed.len() - 1];
        if !name.is_empty() && !name.contains('[') && !name.contains(']') {
            return Line {
                raw: raw.to_string(),
                parsed: ParsedLine::Section {
                    name: name.to_string(),
                },
            };
        }
        return Line {
            raw: raw.to_string(),
            parsed: ParsedLine::Other,
        };
    }
    let indent_len = raw.len() - raw.trim_start().len();
    let indent = raw[..indent_len].to_string();
    let body = &raw[indent_len..];
    let Some((key, separator, raw_value)) = split_assignment(body, kind) else {
        return Line {
            raw: raw.to_string(),
            parsed: ParsedLine::Other,
        };
    };
    let (value, label) = split_value_for_kind(kind, raw_value);
    Line {
        raw: raw.to_string(),
        parsed: ParsedLine::Entry(Entry {
            indent,
            key: key.to_string(),
            separator,
            value,
            label,
        }),
    }
}

/// Split `key=value` (ordinary configs) or `key command` / `key=command`
/// (input.conf) into key, the exact separator text, and the untrimmed value.
fn split_assignment(body: &str, kind: ConfigKind) -> Option<(&str, String, &str)> {
    let bytes = body.as_bytes();
    if kind == ConfigKind::Input {
        // input.conf is whitespace-separated (`MBTN_LEFT cycle pause`), but
        // `key=command` is also accepted by mpv and by our own patches.
        let end = bytes
            .iter()
            .position(|b| b.is_ascii_whitespace() || *b == b'=')?;
        let key = &body[..end];
        if !kind.validate_key(key) {
            return None;
        }
        let mut value_start = end;
        if bytes[end] == b'=' {
            value_start += 1;
        }
        while value_start < bytes.len() && bytes[value_start].is_ascii_whitespace() {
            value_start += 1;
        }
        if value_start >= bytes.len() {
            return None;
        }
        Some((
            key,
            body[end..value_start].to_string(),
            &body[value_start..],
        ))
    } else {
        let eq = bytes.iter().position(|b| *b == b'=')?;
        let key = body[..eq].trim_end();
        if !kind.validate_key(key) {
            return None;
        }
        let mut value_start = eq + 1;
        while value_start < bytes.len() && bytes[value_start].is_ascii_whitespace() {
            value_start += 1;
        }
        Some((
            key,
            body[key.len()..value_start].to_string(),
            &body[value_start..],
        ))
    }
}

/// Split a raw value into its value and an optional input.conf uosc `#!` label.
fn split_value_for_kind(kind: ConfigKind, raw: &str) -> (String, Option<String>) {
    if kind != ConfigKind::Input {
        return (raw.trim_end().to_string(), None);
    }
    let bytes = raw.as_bytes();
    let mut label_at = None;
    for index in 0..bytes.len() {
        if bytes[index] == b'#'
            && bytes.get(index + 1) == Some(&b'!')
            && (index == 0 || bytes[index - 1].is_ascii_whitespace())
        {
            label_at = Some(index);
            break;
        }
    }
    match label_at {
        Some(0) => (String::new(), Some(raw.trim_end().to_string())),
        Some(index) => {
            let value = raw[..index].trim_end().to_string();
            let label = format!(" {}", raw[index..].trim_end());
            (value, Some(label))
        }
        None => (raw.trim_end().to_string(), None),
    }
}

fn normalize_key(kind: ConfigKind, key: &str) -> String {
    if kind == ConfigKind::Input {
        key.to_ascii_lowercase()
    } else {
        key.to_string()
    }
}

fn insertion_index(lines: &[Line], start: usize, end: usize) -> usize {
    let mut last_content = None;
    for (index, line) in lines.iter().enumerate().take(end).skip(start) {
        match line.parsed {
            ParsedLine::Blank | ParsedLine::Comment => {}
            _ => last_content = Some(index),
        }
    }
    match last_content {
        Some(index) => index + 1,
        None => start,
    }
}

/// Compute the semantic difference between what the updater generated last
/// time and what the user currently has on disk.  The result is expressed as
/// patch entries against the official configuration, never as a copied file.
pub fn capture_diff(current: &ConfigDoc, generated: &ConfigDoc) -> Vec<PatchEntry> {
    let current_effective = current.effective();
    let generated_effective = generated.effective();
    let mut generated_by_key: HashMap<(Option<String>, String), &EffectiveEntry> = HashMap::new();
    for entry in &generated_effective {
        generated_by_key.insert(
            (
                entry.section.clone(),
                normalize_key(current.kind, &entry.key),
            ),
            entry,
        );
    }
    let mut seen: HashMap<(Option<String>, String), ()> = HashMap::new();
    let mut entries: Vec<PatchEntry> = Vec::new();
    for entry in &current_effective {
        let lookup = (
            entry.section.clone(),
            normalize_key(current.kind, &entry.key),
        );
        seen.insert(lookup.clone(), ());
        let previous = generated_by_key.get(&lookup);
        let value = match previous {
            Some(previous) if previous.full_value() == entry.full_value() => continue,
            Some(previous) => {
                if current.kind == ConfigKind::Input && previous.label != entry.label {
                    entry.full_value()
                } else {
                    entry.value.clone()
                }
            }
            None => {
                if current.kind == ConfigKind::Input && entry.label.is_some() {
                    entry.full_value()
                } else {
                    entry.value.clone()
                }
            }
        };
        entries.push(PatchEntry {
            section: entry.section.clone(),
            key: entry.key.clone(),
            op: PatchOp::Set,
            value,
            line: 0,
        });
    }
    for entry in &generated_effective {
        let lookup = (
            entry.section.clone(),
            normalize_key(current.kind, &entry.key),
        );
        if seen.contains_key(&lookup) {
            continue;
        }
        entries.push(PatchEntry {
            section: entry.section.clone(),
            key: entry.key.clone(),
            op: PatchOp::Remove,
            value: String::new(),
            line: 0,
        });
    }
    entries
}

/// Group captured entries by scope while preserving file order, which keeps the
/// generated patch readable.
pub fn group_by_section(entries: &[PatchEntry]) -> BTreeMap<Option<String>, Vec<PatchEntry>> {
    let mut groups: BTreeMap<Option<String>, Vec<PatchEntry>> = BTreeMap::new();
    for entry in entries {
        groups
            .entry(entry.section.clone())
            .or_default()
            .push(entry.clone());
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str, kind: ConfigKind) -> ConfigDoc {
        ConfigDoc::parse(text, kind).unwrap()
    }

    fn patch(text: &str, kind: ConfigKind) -> PatchDoc {
        PatchDoc::parse(text, "test.patch", kind).unwrap()
    }

    #[test]
    fn mpv_set_preserves_comments_and_spacing() {
        let mut config = doc(
            "# official\nvolume = 100\nosc=yes\n\n# trailing\n",
            ConfigKind::Mpv,
        );
        let warnings = config
            .apply(&patch("schema=1\n[global]\nvolume = 70\n", ConfigKind::Mpv))
            .unwrap();
        assert!(warnings.is_empty());
        assert_eq!(
            config.render(),
            "# official\nvolume = 70\nosc=yes\n\n# trailing\n"
        );
    }

    #[test]
    fn mpv_set_adds_missing_key_to_global_scope() {
        let mut config = doc("volume=100\n\n[unused]\n", ConfigKind::Mpv);
        config
            .apply(&patch(
                "schema=1\n[global]\nkeepaspect-window=no\n",
                ConfigKind::Mpv,
            ))
            .unwrap();
        assert_eq!(
            config.render(),
            "volume=100\nkeepaspect-window=no\n\n[unused]\n"
        );
    }

    #[test]
    fn remove_is_scope_aware_in_profiles() {
        let mut config = doc(
            "volume=100\n\n[gpu-hq]\nvolume=80\nscale=ewa_lanczos\n\n[low]\nvolume=40\n",
            ConfigKind::Profiles,
        );
        config
            .apply(&patch(
                "schema=1\n[gpu-hq]\n-volume\n",
                ConfigKind::Profiles,
            ))
            .unwrap();
        assert_eq!(
            config.render(),
            "volume=100\n\n[gpu-hq]\nscale=ewa_lanczos\n\n[low]\nvolume=40\n"
        );
    }

    #[test]
    fn profiles_do_not_touch_same_name_key_in_other_section() {
        let mut config = doc(
            "[gpu-hq]\nscale=bilinear\n[other]\nscale=nearest\n",
            ConfigKind::Profiles,
        );
        config
            .apply(&patch(
                "schema=1\n[gpu-hq]\nscale=ewa_lanczos\n",
                ConfigKind::Profiles,
            ))
            .unwrap();
        assert_eq!(
            config.render(),
            "[gpu-hq]\nscale=ewa_lanczos\n[other]\nscale=nearest\n"
        );
    }

    #[test]
    fn input_set_preserves_uosc_label() {
        let mut config = doc("MBTN_LEFT cycle pause #! 播放 > 暂停\n", ConfigKind::Input);
        config
            .apply(&patch(
                "schema=1\n[global]\nMBTN_LEFT=cycle mute\n",
                ConfigKind::Input,
            ))
            .unwrap();
        assert_eq!(config.render(), "MBTN_LEFT cycle mute #! 播放 > 暂停\n");
    }

    #[test]
    fn input_set_can_replace_label() {
        let mut config = doc("a cycle pause #! old\n", ConfigKind::Input);
        config
            .apply(&patch(
                "schema=1\na=cycle mute #! new label\n",
                ConfigKind::Input,
            ))
            .unwrap();
        assert_eq!(config.render(), "a cycle mute #! new label\n");
    }

    #[test]
    fn input_keys_are_case_insensitive() {
        let mut config = doc("MBTN_LEFT cycle pause\n", ConfigKind::Input);
        config
            .apply(&patch(
                "schema=1\nmbtn_left=cycle mute\n",
                ConfigKind::Input,
            ))
            .unwrap();
        assert_eq!(config.render(), "MBTN_LEFT cycle mute\n");
    }

    #[test]
    fn remove_input_binding() {
        let mut config = doc("a cycle pause\nb cycle fullscreen\n", ConfigKind::Input);
        let warnings = config
            .apply(&patch("schema=1\n-b\n", ConfigKind::Input))
            .unwrap();
        assert!(warnings.is_empty());
        assert_eq!(config.render(), "a cycle pause\n");
    }

    #[test]
    fn append_keeps_both_and_last_wins() {
        let mut config = doc("volume=100\n", ConfigKind::Mpv);
        config
            .apply(&patch("schema=1\n+volume=60\n", ConfigKind::Mpv))
            .unwrap();
        assert_eq!(config.render(), "volume=100\nvolume=60\n");
    }

    #[test]
    fn missing_key_remove_warns() {
        let mut config = doc("volume=100\n", ConfigKind::Mpv);
        let warnings = config
            .apply(&patch("schema=1\n-missing\n", ConfigKind::Mpv))
            .unwrap();
        assert_eq!(warnings.len(), 1);
        assert_eq!(config.render(), "volume=100\n");
    }

    #[test]
    fn creates_missing_profile_section() {
        let mut config = doc("[low]\nvolume=20\n", ConfigKind::Profiles);
        config
            .apply(&patch(
                "schema=1\n[new-profile]\nvolume=50\n",
                ConfigKind::Profiles,
            ))
            .unwrap();
        assert_eq!(
            config.render(),
            "[low]\nvolume=20\n\n[new-profile]\nvolume=50\n"
        );
    }

    #[test]
    fn rejects_section_for_mpv_conf() {
        let mut config = doc("volume=100\n", ConfigKind::Mpv);
        let error = config
            .apply(&patch("schema=1\n[x]\nvolume=50\n", ConfigKind::Mpv))
            .unwrap_err();
        assert_eq!(error.kind, crate::util::ErrorKind::Blocked);
    }

    #[test]
    fn capture_set_and_remove() {
        let generated = doc(
            "volume=100\nosc=yes\n\n[gpu-hq]\nscale=bilinear\n",
            ConfigKind::Profiles,
        );
        let current = doc(
            "volume=70\n\n[gpu-hq]\nscale=ewa_lanczos\n",
            ConfigKind::Profiles,
        );
        let entries = capture_diff(&current, &generated);
        assert!(entries.iter().any(|entry| {
            entry.section.is_none() && entry.key == "volume" && entry.value == "70"
        }));
        assert!(entries.iter().any(|entry| {
            entry.section.is_none() && entry.key == "osc" && entry.op == PatchOp::Remove
        }));
        assert!(entries.iter().any(|entry| {
            entry.section.as_deref() == Some("gpu-hq")
                && entry.key == "scale"
                && entry.value == "ewa_lanczos"
        }));
    }

    #[test]
    fn capture_input_label_change() {
        let generated = doc("a cycle pause #! old\n", ConfigKind::Input);
        let current = doc("a cycle mute #! new\n", ConfigKind::Input);
        let entries = capture_diff(&current, &generated);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].value, "cycle mute #! new");
    }

    #[test]
    fn capture_is_empty_when_semantics_match() {
        let generated = doc("# a\nvolume=100\n", ConfigKind::Mpv);
        let current = doc("# b\nvolume = 100\n", ConfigKind::Mpv);
        assert!(capture_diff(&current, &generated).is_empty());
    }

    #[test]
    fn malformed_lines_are_preserved_not_reinterpreted() {
        let text = "not an assignment\nvolume=100\n";
        let config = doc(text, ConfigKind::Mpv);
        assert_eq!(config.render(), text);
        assert!(matches!(config.lines[0].parsed, ParsedLine::Other));
    }

    #[test]
    fn script_opts_keys_allow_dots() {
        let mut config = doc("font.size=20\n", ConfigKind::ScriptOpts);
        config
            .apply(&patch("schema=1\nfont.size=30\n", ConfigKind::ScriptOpts))
            .unwrap();
        assert_eq!(config.render(), "font.size=30\n");
    }
}
