//! Semantic parsing and patching of the small set of user-editable mpv
//! configuration files.
//!
//! The updater never regenerates these files from scratch.  It keeps the
//! official file byte-for-byte except for the lines a user patch touches, so
//! comments, ordering and platform-specific options stay exactly as shipped.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::patch::{
    LineOp, LineOpKind, MatchMode, PatchDoc, PatchEntry, PatchItem, PatchOp, Selector, SelectorKind,
};
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

/// The result of comparing the user's current configuration with the last
/// generated snapshot.  Key operations describe semantic changes; line
/// operations describe comments, menu entries and other raw text.
#[derive(Debug, Clone, Default)]
pub struct CapturedPatch {
    pub keys: Vec<PatchEntry>,
    pub lines: Vec<LineOp>,
}

impl CapturedPatch {
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty() && self.lines.is_empty()
    }

    pub fn len(&self) -> usize {
        self.keys.len() + self.lines.len()
    }

    pub fn items(&self) -> Vec<PatchItem> {
        let mut items: Vec<PatchItem> = self.keys.iter().cloned().map(PatchItem::Key).collect();
        items.extend(self.lines.iter().cloned().map(PatchItem::Line));
        items
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
        for item in &patch.items {
            match item {
                PatchItem::Key(entry) => {
                    self.validate_key_entry(entry, patch)?;
                    match entry.op {
                        PatchOp::Set => self.set(entry, &mut warnings)?,
                        PatchOp::Remove => self.remove(entry, &mut warnings),
                        PatchOp::Append => self.append(entry, &mut warnings)?,
                    }
                }
                PatchItem::Line(op) => {
                    self.validate_line_op(op, patch)?;
                    self.apply_line_op(op, patch, &mut warnings)?;
                }
            }
        }
        Ok(warnings)
    }

    fn validate_key_entry(&self, entry: &PatchEntry, patch: &PatchDoc) -> Result<()> {
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
        Ok(())
    }

    fn validate_line_op(&self, op: &LineOp, patch: &PatchDoc) -> Result<()> {
        if op.section.is_some() && !self.kind.allows_sections() {
            return Err(Failure::blocked(
                "用户 patch 的行操作使用了不支持的配置段。",
                format!(
                    "{}:{}: [{}] is not valid for {}",
                    patch.source,
                    op.line,
                    op.section.as_deref().unwrap_or(""),
                    self.kind.name()
                ),
            ));
        }
        Ok(())
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

    fn apply_line_op(
        &mut self,
        op: &LineOp,
        patch: &PatchDoc,
        warnings: &mut Vec<String>,
    ) -> Result<()> {
        match op.kind {
            LineOpKind::InsertAfter => {
                let index = match self.resolve_one(op, patch, warnings)? {
                    Some(index) => index,
                    None => return Ok(()),
                };
                if block_at(&self.lines, index + 1, &op.lines) {
                    return Ok(());
                }
                self.insert_lines(index + 1, &op.lines);
            }
            LineOpKind::InsertBefore => {
                let index = match self.resolve_one(op, patch, warnings)? {
                    Some(index) => index,
                    None => return Ok(()),
                };
                if index >= op.lines.len()
                    && block_at(&self.lines, index - op.lines.len(), &op.lines)
                {
                    return Ok(());
                }
                self.insert_lines(index, &op.lines);
            }
            LineOpKind::Replace => {
                let index = match self.resolve_one(op, patch, warnings)? {
                    Some(index) => index,
                    None => return Ok(()),
                };
                if block_at(&self.lines, index, &op.lines) {
                    return Ok(());
                }
                self.lines.remove(index);
                self.insert_lines(index, &op.lines);
            }
            LineOpKind::Delete => {
                let Some(index) = self.resolve_one(op, patch, warnings)? else {
                    return Ok(());
                };
                self.lines.remove(index);
            }
            LineOpKind::Append => {
                let Some((start, end)) = self.line_scope(op, patch)? else {
                    warnings.push(format!(
                        "{}:{}: 配置段不存在，@append 已跳过",
                        patch.source, op.line
                    ));
                    return Ok(());
                };
                let mut last = end;
                while last > start && matches!(self.lines[last - 1].parsed, ParsedLine::Blank) {
                    last -= 1;
                }
                if last >= op.lines.len() && block_at(&self.lines, last - op.lines.len(), &op.lines)
                {
                    return Ok(());
                }
                self.insert_lines(last, &op.lines);
            }
            LineOpKind::Prepend => {
                let Some((start, end)) = self.line_scope(op, patch)? else {
                    warnings.push(format!(
                        "{}:{}: 配置段不存在，@prepend 已跳过",
                        patch.source, op.line
                    ));
                    return Ok(());
                };
                let mut first = start;
                while first < end && matches!(self.lines[first].parsed, ParsedLine::Blank) {
                    first += 1;
                }
                if block_at(&self.lines, first, &op.lines) {
                    return Ok(());
                }
                self.insert_lines(first, &op.lines);
            }
            LineOpKind::Splice => {
                let (start, end) = self.selector_scope(op, patch)?;
                let matches = find_block(&self.lines, start, end, &op.old_lines, op.match_mode);
                let selected = match op.occurrence {
                    Some(occurrence) => matches.get(occurrence - 1).copied(),
                    None => {
                        if matches.len() > 1 {
                            return Err(Failure::blocked(
                                "用户 patch 的 @splice 匹配到多处相同内容。",
                                format!(
                                    "{}:{}: {} matches; add occurrence=",
                                    patch.source,
                                    op.line,
                                    matches.len()
                                ),
                            ));
                        }
                        matches.first().copied()
                    }
                };
                if let Some(index) = selected {
                    if op.old_lines == op.lines {
                        return Ok(());
                    }
                    // Repeated repair must not replace the next identical old
                    // block.  If the new block already appears at or before the
                    // selected old block, this splice has already run.
                    let already_applied =
                        find_block(&self.lines, start, end, &op.lines, op.match_mode)
                            .iter()
                            .any(|found| *found <= index);
                    if already_applied {
                        return Ok(());
                    }
                    self.replace_lines(index, op.old_lines.len(), &op.lines);
                    return Ok(());
                }
                if op.lines.is_empty() {
                    // A deletion is idempotent: once the old block is gone,
                    // there is nothing left to do.
                    warnings.push(format!(
                        "{}:{}: @splice 目标已不存在或已删除，已跳过",
                        patch.source, op.line
                    ));
                    return Ok(());
                }
                if find_block(&self.lines, start, end, &op.lines, op.match_mode).is_empty() {
                    if op.optional {
                        warnings.push(format!(
                            "{}:{}: @splice 的目标已不存在，已跳过",
                            patch.source, op.line
                        ));
                    } else {
                        return Err(Failure::blocked(
                            "用户 patch 的 @splice 目标不存在。",
                            format!("{}:{}", patch.source, op.line),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn resolve_one(
        &self,
        op: &LineOp,
        patch: &PatchDoc,
        warnings: &mut Vec<String>,
    ) -> Result<Option<usize>> {
        let selector = op.selector.as_ref().ok_or_else(|| {
            Failure::internal(format!("line operation at {} has no selector", op.line))
        })?;
        let (start, end) = self.selector_scope(op, patch)?;
        let matches = self.selector_matches(selector, start, end);
        let selected = match selector.occurrence {
            Some(occurrence) => matches.get(occurrence - 1).copied(),
            None => {
                if matches.len() > 1 {
                    return Err(Failure::blocked(
                        "用户 patch 的选择器匹配到多处内容。",
                        format!(
                            "{}:{}: {} matches; add [n] to the selector",
                            patch.source,
                            op.line,
                            matches.len()
                        ),
                    ));
                }
                matches.first().copied()
            }
        };
        if selected.is_none() {
            if op.optional || op.kind == LineOpKind::Delete {
                warnings.push(format!("{}:{}: 目标不存在，已跳过", patch.source, op.line));
                return Ok(None);
            }
            return Err(Failure::blocked(
                "用户 patch 的选择器目标不存在。",
                format!("{}:{}", patch.source, op.line),
            ));
        }
        Ok(selected)
    }

    fn line_scope(&self, op: &LineOp, patch: &PatchDoc) -> Result<Option<(usize, usize)>> {
        let bounds = self.scope_bounds(op.section.as_deref());
        if bounds.is_none() && !op.optional {
            return Err(Failure::blocked(
                "用户 patch 的配置段不存在。",
                format!("{}:{}", patch.source, op.line),
            ));
        }
        Ok(bounds)
    }

    fn selector_scope(&self, op: &LineOp, patch: &PatchDoc) -> Result<(usize, usize)> {
        if op.section.is_some() {
            return match self.scope_bounds(op.section.as_deref()) {
                Some(bounds) => Ok(bounds),
                None if op.optional => Ok((0, 0)),
                None => Err(Failure::blocked(
                    "用户 patch 的配置段不存在。",
                    format!("{}:{}", patch.source, op.line),
                )),
            };
        }
        let key_selector = matches!(
            op.selector.as_ref().map(|selector| selector.kind),
            Some(SelectorKind::Key)
        );
        match self.kind {
            // A key selector without a section means the global scope.  Line,
            // menu and command selectors are semantic identities, so search the
            // whole file.
            ConfigKind::Profiles if key_selector => {
                Ok(self.scope_bounds(None).unwrap_or((0, self.lines.len())))
            }
            _ => Ok((0, self.lines.len())),
        }
    }

    fn selector_matches(&self, selector: &Selector, start: usize, end: usize) -> Vec<usize> {
        let mut result = Vec::new();
        for index in start..end {
            let matched = match selector.kind {
                SelectorKind::Menu => {
                    self.kind == ConfigKind::Input
                        && menu_label(&self.lines[index]).as_deref() == Some(selector.text.as_str())
                }
                SelectorKind::Command => {
                    self.kind == ConfigKind::Input
                        && line_command(&self.lines[index], self.kind).as_deref()
                            == Some(selector.text.trim())
                }
                SelectorKind::Key => match &self.lines[index].parsed {
                    ParsedLine::Entry(entry) => self.kind.key_matches(&entry.key, &selector.text),
                    _ => false,
                },
                SelectorKind::Section => match &self.lines[index].parsed {
                    ParsedLine::Section { name } => name == &selector.text,
                    _ => false,
                },
                SelectorKind::Line => self.lines[index].raw.trim() == selector.text,
            };
            if matched {
                result.push(index);
            }
        }
        result
    }

    fn insert_lines(&mut self, at: usize, lines: &[String]) {
        let parsed: Vec<Line> = lines.iter().map(|raw| parse_line(raw, self.kind)).collect();
        self.lines.splice(at..at, parsed);
    }

    fn replace_lines(&mut self, at: usize, count: usize, lines: &[String]) {
        let parsed: Vec<Line> = lines.iter().map(|raw| parse_line(raw, self.kind)).collect();
        self.lines.splice(at..at + count, parsed);
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

/// Extract a uosc menu path from either a real binding (`key command #! label`)
/// or a menu-only comment (`# command #! label`).  The label is the stable
/// identity of the menu item; the command is a separate selector.
fn menu_label(line: &Line) -> Option<String> {
    let label = match &line.parsed {
        ParsedLine::Entry(entry) => entry.label.clone(),
        ParsedLine::Comment => {
            let trimmed = line.raw.trim();
            if let Some(rest) = trimmed.strip_prefix("#!") {
                Some(rest.to_string())
            } else if let Some(rest) = trimmed.strip_prefix("#menu:") {
                Some(rest.to_string())
            } else {
                find_label_marker(trimmed)
            }
        }
        _ => None,
    }?;
    let label = label.trim();
    let label = label.strip_prefix("#!").unwrap_or(label);
    let label = label.strip_prefix("#menu:").unwrap_or(label);
    let label = label.trim();
    if label.is_empty() {
        return None;
    }
    let parts: Vec<&str> = label
        .split('>')
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect();
    if parts.is_empty() {
        return None;
    }
    Some(parts.join(" > "))
}

/// Extract the stable command from a real binding or a menu-only comment.
/// The update menu uses comment lines (`# script-binding ... #! label`) so mpv
/// ignores them; those lines must still be anchorable by command.
fn line_command(line: &Line, kind: ConfigKind) -> Option<String> {
    if kind != ConfigKind::Input {
        return None;
    }
    match &line.parsed {
        ParsedLine::Entry(entry) => {
            let command = entry.value.trim();
            (!command.is_empty()).then(|| command.to_string())
        }
        ParsedLine::Comment => {
            let body = line.raw.trim().strip_prefix('#')?;
            let body = body.trim_start();
            if body.starts_with('!') || body.starts_with("menu:") {
                return None;
            }
            let command = match label_marker_index(body) {
                Some(index) => body[..index].trim(),
                None => body.trim(),
            };
            (!command.is_empty()).then(|| command.to_string())
        }
        _ => None,
    }
}

fn find_label_marker(text: &str) -> Option<String> {
    label_marker_index(text).map(|index| text[index..].to_string())
}

fn label_marker_index(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index + 1 < bytes.len() {
        if bytes[index] == b'#'
            && bytes[index + 1] == b'!'
            && (index == 0 || bytes[index - 1].is_ascii_whitespace())
        {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn block_at(lines: &[Line], start: usize, block: &[String]) -> bool {
    if start + block.len() > lines.len() {
        return false;
    }
    lines[start..start + block.len()]
        .iter()
        .zip(block)
        .all(|(line, expected)| line.raw == *expected)
}

fn find_block(
    lines: &[Line],
    start: usize,
    end: usize,
    block: &[String],
    mode: MatchMode,
) -> Vec<usize> {
    if block.is_empty() || block.len() > end.saturating_sub(start) {
        return Vec::new();
    }
    let mut result = Vec::new();
    for index in start..=end - block.len() {
        let matched = lines[index..index + block.len()].iter().zip(block).all(
            |(line, expected)| match mode {
                MatchMode::Exact => line.raw == *expected,
                MatchMode::Trim => line.raw.trim() == expected.trim(),
            },
        );
        if matched {
            result.push(index);
        }
    }
    result
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

/// Capture both semantic key changes and raw line changes.  Raw line changes
/// are only produced for text that is not a configuration entry or section
/// header; those are represented by key operations.  A hunk that mixes the
/// two would be ambiguous, so capture stops instead of guessing.
pub fn capture_patch(current: &ConfigDoc, generated: &ConfigDoc) -> Result<CapturedPatch> {
    let keys = capture_diff(current, generated);
    let lines = capture_line_diff(current, generated)?;
    Ok(CapturedPatch { keys, lines })
}

fn capture_line_diff(current: &ConfigDoc, generated: &ConfigDoc) -> Result<Vec<LineOp>> {
    let old = &generated.lines;
    let new = &current.lines;
    let rows = old.len().saturating_add(1);
    let columns = new.len().saturating_add(1);
    if rows.saturating_mul(columns) > 4_000_000 {
        return Err(Failure::blocked(
            "配置文件过大，无法安全地自动捕获行级修改。",
            format!(
                "{}: {} x {} lines",
                generated.kind.name(),
                old.len(),
                new.len()
            ),
        ));
    }
    // Compare entries by their semantic value, not by spacing.  A user who
    // only changes `volume = 100` to `volume=100` has not changed intent, and
    // that formatting difference must not make an adjacent comment edit look
    // like a mixed structural/text change.
    let old_tokens: Vec<String> = old
        .iter()
        .map(|line| diff_token(line, generated.kind))
        .collect();
    let new_tokens: Vec<String> = new
        .iter()
        .map(|line| diff_token(line, current.kind))
        .collect();
    let mut dp = vec![vec![0u32; columns]; rows];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            dp[i][j] = if old_tokens[i] == new_tokens[j] {
                dp[i + 1][j + 1].saturating_add(1)
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut hunks: Vec<(usize, usize, usize, usize)> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old_tokens[i] == new_tokens[j] {
            i += 1;
            j += 1;
            continue;
        }
        let start_i = i;
        let start_j = j;
        while i < old.len() || j < new.len() {
            if i < old.len() && j < new.len() && old_tokens[i] == new_tokens[j] {
                break;
            }
            if i < old.len() && j < new.len() {
                if dp[i + 1][j] >= dp[i][j + 1] {
                    i += 1;
                } else {
                    j += 1;
                }
            } else if i < old.len() {
                i += 1;
            } else {
                j += 1;
            }
        }
        hunks.push((start_i, i, start_j, j));
    }

    let mut result = Vec::new();
    for (old_start, old_end, new_start, new_end) in hunks {
        let old_lines = &old[old_start..old_end];
        let new_lines = &new[new_start..new_end];
        let old_structural = old_lines.iter().any(is_structural);
        let new_structural = new_lines.iter().any(is_structural);
        if old_structural || new_structural {
            let old_text = old_lines.iter().any(is_significant_text);
            let new_text = new_lines.iter().any(is_significant_text);
            if old_text || new_text {
                return Err(Failure::blocked(
                    "无法自动捕获同一处同时修改配置键和注释的编辑，更新已停止。",
                    format!(
                        "{}: old lines {}-{}, new lines {}-{}; move the comment edit away from the key edit or save a manual patch",
                        current.kind.name(),
                        old_start + 1,
                        old_end,
                        new_start + 1,
                        new_end
                    ),
                ));
            }
            continue;
        }
        if old_lines.is_empty() {
            result.push(insertion_op(
                old,
                new,
                old_start,
                new_start,
                new_end,
                current.kind,
            ));
        } else {
            let occurrence =
                count_block_occurrences(old, 0, old_start, old_lines, MatchMode::Exact)
                    .saturating_add(1);
            result.push(LineOp {
                kind: LineOpKind::Splice,
                selector: None,
                section: None,
                lines: new_lines.iter().map(|line| line.raw.clone()).collect(),
                old_lines: old_lines.iter().map(|line| line.raw.clone()).collect(),
                occurrence: Some(occurrence),
                match_mode: MatchMode::Exact,
                optional: false,
                line: 0,
            });
        }
    }
    Ok(result)
}

fn diff_token(line: &Line, kind: ConfigKind) -> String {
    match &line.parsed {
        ParsedLine::Entry(entry) => format!(
            "entry\0{}\0{}\0{}",
            normalize_key(kind, &entry.key),
            entry.value,
            entry.label.as_deref().unwrap_or("")
        ),
        ParsedLine::Section { name } => format!("section\0{name}"),
        _ => format!("text\0{}", line.raw),
    }
}

fn is_structural(line: &Line) -> bool {
    matches!(
        line.parsed,
        ParsedLine::Entry(_) | ParsedLine::Section { .. }
    )
}

fn is_significant_text(line: &Line) -> bool {
    matches!(line.parsed, ParsedLine::Comment | ParsedLine::Other)
}

fn insertion_op(
    old: &[Line],
    new: &[Line],
    old_index: usize,
    new_start: usize,
    new_end: usize,
    kind: ConfigKind,
) -> LineOp {
    let lines: Vec<String> = new[new_start..new_end]
        .iter()
        .map(|line| line.raw.clone())
        .collect();
    let anchor = if old_index > 0 && !old[old_index - 1].raw.trim().is_empty() {
        Some((old_index - 1, false))
    } else if old_index < old.len() && !old[old_index].raw.trim().is_empty() {
        Some((old_index, true))
    } else {
        None
    };
    match anchor {
        Some((index, before)) => {
            let selector = anchor_selector(old, index, kind);
            LineOp {
                kind: if before {
                    LineOpKind::InsertBefore
                } else {
                    LineOpKind::InsertAfter
                },
                selector: Some(selector),
                section: None,
                lines,
                old_lines: Vec::new(),
                occurrence: None,
                match_mode: MatchMode::Exact,
                optional: false,
                line: 0,
            }
        }
        None => LineOp {
            kind: if old.is_empty() {
                LineOpKind::Prepend
            } else {
                LineOpKind::Append
            },
            selector: None,
            section: None,
            lines,
            old_lines: Vec::new(),
            occurrence: None,
            match_mode: MatchMode::Exact,
            optional: false,
            line: 0,
        },
    }
}

/// Prefer a semantic identity over a raw line for an insertion anchor.  Menu
/// labels and input commands survive spacing and formatting changes; the raw
/// line is the last resort for comments and ordinary configuration text.
fn anchor_selector(old: &[Line], index: usize, kind: ConfigKind) -> Selector {
    if kind == ConfigKind::Input {
        if let Some(command) = line_command(&old[index], kind) {
            if !command.contains('#') {
                let occurrence = old[..=index]
                    .iter()
                    .filter(|line| line_command(line, kind).as_deref() == Some(command.as_str()))
                    .count();
                return Selector {
                    kind: SelectorKind::Command,
                    text: command,
                    occurrence: Some(occurrence),
                };
            }
        }
        if let Some(label) = menu_label(&old[index]) {
            if !label.contains('#') {
                let occurrence = old[..=index]
                    .iter()
                    .filter(|line| menu_label(line).as_deref() == Some(label.as_str()))
                    .count();
                return Selector {
                    kind: SelectorKind::Menu,
                    text: label,
                    occurrence: Some(occurrence),
                };
            }
        }
    }
    let text = old[index].raw.trim().to_string();
    let occurrence = old[..=index]
        .iter()
        .filter(|line| line.raw.trim() == text)
        .count();
    Selector {
        kind: SelectorKind::Line,
        text,
        occurrence: Some(occurrence),
    }
}

fn count_block_occurrences(
    lines: &[Line],
    start: usize,
    end: usize,
    block: &[Line],
    mode: MatchMode,
) -> usize {
    if block.is_empty() || end < start + block.len() {
        return 0;
    }
    let expected: Vec<String> = block.iter().map(|line| line.raw.clone()).collect();
    find_block(lines, start, end, &expected, mode).len()
}

/// Verify that applying the captured operations to the generated snapshot
/// reproduces the user's intent.  Formatting of key lines is allowed to differ;
/// effective keys and all non-structural text must match.
pub fn verify_capture(
    current: &ConfigDoc,
    generated: &ConfigDoc,
    captured: &CapturedPatch,
) -> Result<()> {
    let mut check = generated.clone();
    let patch = PatchDoc {
        schema: crate::patch::PATCH_SCHEMA,
        target: None,
        source: "capture".to_string(),
        items: captured.items(),
    };
    check.apply(&patch)?;
    if !semantically_equal(&check, current) {
        return Err(Failure::blocked(
            "自动捕获的配置修改无法完整重放，更新已停止。",
            format!(
                "{}: capture round-trip mismatch; save a manual patch for this edit",
                current.kind.name()
            ),
        ));
    }
    Ok(())
}

fn semantically_equal(left: &ConfigDoc, right: &ConfigDoc) -> bool {
    fn effective_map(
        doc: &ConfigDoc,
    ) -> BTreeMap<(Option<String>, String), (String, Option<String>)> {
        let mut result = BTreeMap::new();
        for entry in doc.effective() {
            result.insert(
                (entry.section.clone(), normalize_key(doc.kind, &entry.key)),
                (entry.value.clone(), entry.label.clone()),
            );
        }
        result
    }
    fn text_lines(doc: &ConfigDoc) -> Vec<String> {
        doc.lines
            .iter()
            .filter(|line| !is_structural(line) && !matches!(line.parsed, ParsedLine::Blank))
            .map(|line| line.raw.clone())
            .collect()
    }
    effective_map(left) == effective_map(right) && text_lines(left) == text_lines(right)
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
        let captured = capture_patch(&current, &generated).unwrap();
        assert_eq!(captured.lines.len(), 1);
        assert_eq!(captured.lines[0].kind, LineOpKind::Splice);
        verify_capture(&current, &generated, &captured).unwrap();
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

    #[test]
    fn inserts_menu_item_after_menu_label() {
        let mut config = doc(
            "# script-binding update-check/check-now #! 更新 > 立即检查更新\n# script-binding update-check/install-now #! 更新 > 下载并安装更新\n",
            ConfigKind::Input,
        );
        config
            .apply(&patch(
                "schema=2\n@after menu:更新 > 立即检查更新\n|# script-binding my-ui/refresh #! 更新 > 刷新订阅\n@end\n",
                ConfigKind::Input,
            ))
            .unwrap();
        assert_eq!(
            config.render(),
            "# script-binding update-check/check-now #! 更新 > 立即检查更新\n# script-binding my-ui/refresh #! 更新 > 刷新订阅\n# script-binding update-check/install-now #! 更新 > 下载并安装更新\n"
        );
    }

    #[test]
    fn menu_insert_is_idempotent() {
        let mut config = doc(
            "# script-binding update-check/check-now #! 更新 > 立即检查更新\n",
            ConfigKind::Input,
        );
        let patch = patch(
            "schema=2\n@after menu:更新 > 立即检查更新\n|# script-binding my-ui/refresh #! 更新 > 刷新订阅\n@end\n",
            ConfigKind::Input,
        );
        config.apply(&patch).unwrap();
        config.apply(&patch).unwrap();
        assert_eq!(config.lines.len(), 2);
    }

    #[test]
    fn command_anchor_matches_binding_not_label() {
        let mut config = doc(
            "a cycle pause #! 播放 > 暂停\nb cycle mute #! 播放 > 静音\n",
            ConfigKind::Input,
        );
        config
            .apply(&patch(
                "schema=2\n@after command:cycle pause\n|x script-binding my-ui/x #! 工具 > 自定义\n@end\n",
                ConfigKind::Input,
            ))
            .unwrap();
        assert_eq!(
            config.render(),
            "a cycle pause #! 播放 > 暂停\nx script-binding my-ui/x #! 工具 > 自定义\nb cycle mute #! 播放 > 静音\n"
        );
    }

    #[test]
    fn command_anchor_matches_menu_comment_line() {
        let mut config = doc(
            "#                script-binding update-check/check-now      #! 更新 > 立即检查更新\n",
            ConfigKind::Input,
        );
        config
            .apply(&patch(
                "schema=2\n@after command:script-binding update-check/check-now\n|#                script-binding my-ui/refresh #! 更新 > 刷新订阅\n@end\n",
                ConfigKind::Input,
            ))
            .unwrap();
        assert_eq!(
            config.render(),
            "#                script-binding update-check/check-now      #! 更新 > 立即检查更新\n#                script-binding my-ui/refresh #! 更新 > 刷新订阅\n"
        );
    }

    #[test]
    fn missing_anchor_is_fatal() {
        let mut config = doc("# a #! 菜单 > 一\n", ConfigKind::Input);
        let error = config
            .apply(&patch(
                "schema=2\n@after menu:菜单 > 二\n|x\n@end\n",
                ConfigKind::Input,
            ))
            .unwrap_err();
        assert_eq!(error.kind, crate::util::ErrorKind::Blocked);
    }

    #[test]
    fn optional_missing_anchor_is_skipped() {
        let mut config = doc("# a #! 菜单 > 一\n", ConfigKind::Input);
        let warnings = config
            .apply(&patch(
                "schema=2\n@optional after menu:菜单 > 二\n|x\n@end\n",
                ConfigKind::Input,
            ))
            .unwrap();
        assert_eq!(warnings.len(), 1);
        assert_eq!(config.lines.len(), 1);
    }

    #[test]
    fn ambiguous_anchor_requires_occurrence() {
        let mut config = doc("# a #! 菜单 > 一\n# b #! 菜单 > 一\n", ConfigKind::Input);
        assert!(config
            .apply(&patch(
                "schema=2\n@after menu:菜单 > 一\n|x\n@end\n",
                ConfigKind::Input,
            ))
            .is_err());
        config
            .apply(&patch(
                "schema=2\n@after menu[2]:菜单 > 一\n|x\n@end\n",
                ConfigKind::Input,
            ))
            .unwrap();
        assert_eq!(config.lines.len(), 3);
    }

    #[test]
    fn delete_missing_menu_item_warns() {
        let mut config = doc("# a #! 菜单 > 一\n", ConfigKind::Input);
        let warnings = config
            .apply(&patch(
                "schema=2\n@delete menu:菜单 > 二\n",
                ConfigKind::Input,
            ))
            .unwrap();
        assert_eq!(warnings.len(), 1);
        assert_eq!(config.lines.len(), 1);
    }

    #[test]
    fn splice_replaces_exact_block_and_is_idempotent() {
        let mut config = doc("# old 1\n# old 2\n", ConfigKind::Mpv);
        let patch = patch(
            "schema=2\n@splice remove=2 insert=1\n|# old 1\n|# old 2\n|# new\n@end\n",
            ConfigKind::Mpv,
        );
        config.apply(&patch).unwrap();
        assert_eq!(config.render(), "# new\n");
        config.apply(&patch).unwrap();
        assert_eq!(config.render(), "# new\n");
    }

    #[test]
    fn splice_deletion_is_idempotent() {
        let mut config = doc("# old\n", ConfigKind::Mpv);
        let patch = patch(
            "schema=2\n@splice remove=1 insert=0\n|# old\n@end\n",
            ConfigKind::Mpv,
        );
        config.apply(&patch).unwrap();
        assert_eq!(config.render(), "");
        let warnings = config.apply(&patch).unwrap();
        assert_eq!(warnings.len(), 1);
        assert_eq!(config.render(), "");
    }

    #[test]
    fn splice_with_duplicate_blocks_replaces_only_once() {
        let mut config = doc("# old\n# old\n", ConfigKind::Mpv);
        let patch = patch(
            "schema=2\n@splice remove=1 insert=1 occurrence=1\n|# old\n|# new\n@end\n",
            ConfigKind::Mpv,
        );
        config.apply(&patch).unwrap();
        assert_eq!(config.render(), "# new\n# old\n");
        config.apply(&patch).unwrap();
        assert_eq!(config.render(), "# new\n# old\n");
    }

    #[test]
    fn append_and_prepend_lines() {
        let mut config = doc("# a\n", ConfigKind::Mpv);
        config
            .apply(&patch(
                "schema=2\n@append\n|# end\n@end\n@prepend\n|# start\n@end\n",
                ConfigKind::Mpv,
            ))
            .unwrap();
        assert_eq!(config.render(), "# start\n# a\n# end\n");
    }

    #[test]
    fn capture_menu_insertion_roundtrips() {
        let generated = doc(
            "# script-binding update-check/check-now #! 更新 > 立即检查更新\n# script-binding update-check/install-now #! 更新 > 下载并安装更新\n",
            ConfigKind::Input,
        );
        let current = doc(
            "# script-binding update-check/check-now #! 更新 > 立即检查更新\n# script-binding my-ui/refresh #! 更新 > 刷新订阅\n# script-binding update-check/install-now #! 更新 > 下载并安装更新\n",
            ConfigKind::Input,
        );
        let captured = capture_patch(&current, &generated).unwrap();
        assert_eq!(captured.lines.len(), 1);
        assert_eq!(
            captured.lines[0].selector.as_ref().unwrap().kind,
            SelectorKind::Command
        );
        verify_capture(&current, &generated, &captured).unwrap();
        let mut replayed = generated.clone();
        replayed
            .apply(&PatchDoc {
                schema: 2,
                target: None,
                source: "capture".into(),
                items: captured.items(),
            })
            .unwrap();
        assert_eq!(replayed.render(), current.render());
    }

    #[test]
    fn capture_rejects_mixed_key_and_comment_edit() {
        let generated = doc("# official\nvolume=100\n", ConfigKind::Mpv);
        let current = doc("# official\nvolume=70\n# my note\n", ConfigKind::Mpv);
        let error = capture_patch(&current, &generated).unwrap_err();
        assert_eq!(error.kind, crate::util::ErrorKind::Blocked);
    }

    #[test]
    fn capture_key_edit_skips_structural_hunk() {
        let generated = doc("# official\nvolume=100\n", ConfigKind::Mpv);
        let current = doc("# official\nvolume=70\n", ConfigKind::Mpv);
        let captured = capture_patch(&current, &generated).unwrap();
        assert_eq!(captured.keys.len(), 1);
        assert!(captured.lines.is_empty());
        verify_capture(&current, &generated, &captured).unwrap();
    }
}
