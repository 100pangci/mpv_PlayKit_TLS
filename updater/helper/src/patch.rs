//! Versioned semantic patch files.
//!
//! A patch describes intent ("volume is 70", "remove this binding", "insert a
//! menu item after this command") rather than a historical copy of a
//! configuration file.  It is applied to the official configuration of
//! whatever version is being installed.
//!
//! Schema 1 contains only key operations.  Schema 2 keeps those operations and
//! adds line operations for text that is not a configuration entry: comments,
//! blank lines, uosc menu entries, and any other raw line.  Line operations
//! locate their target with semantic selectors (menu path, command, key,
//! section, or a unique line), never with line numbers.

use std::path::Path;

use crate::config::ConfigKind;
use crate::util::{relative_path, Failure, Result};

pub const PATCH_SCHEMA: u32 = 2;
pub const MIN_PATCH_SCHEMA: u32 = 1;
const MAX_PATCH_BYTES: usize = 1024 * 1024;
const MAX_ENTRIES: usize = 5000;
const MAX_BLOCK_LINES: usize = 5000;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorKind {
    /// uosc menu label path, e.g. `更新 > 立即检查更新`.
    Menu,
    /// input.conf command before the `#!` label.
    Command,
    /// Configuration key inside the current section.
    Key,
    /// `[section]` header.
    Section,
    /// Exact trimmed raw line.
    Line,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    pub kind: SelectorKind,
    pub text: String,
    /// 1-based occurrence for duplicate selectors.
    pub occurrence: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchMode {
    Exact,
    Trim,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineOpKind {
    InsertAfter,
    InsertBefore,
    Replace,
    Delete,
    Append,
    Prepend,
    Splice,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineOp {
    pub kind: LineOpKind,
    pub selector: Option<Selector>,
    pub section: Option<String>,
    /// New content for insert/replace/append/prepend/splice.
    pub lines: Vec<String>,
    /// Old content for splice.
    pub old_lines: Vec<String>,
    /// 1-based occurrence of the old block for splice.
    pub occurrence: Option<usize>,
    pub match_mode: MatchMode,
    pub optional: bool,
    /// 1-based line in the patch file; 0 for operations generated at runtime.
    pub line: usize,
}

#[derive(Debug, Clone)]
pub enum PatchItem {
    Key(PatchEntry),
    Line(LineOp),
}

impl PatchItem {
    pub fn section(&self) -> Option<String> {
        match self {
            Self::Key(entry) => entry.section.clone(),
            Self::Line(op) => op.section.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PatchDoc {
    pub schema: u32,
    pub target: Option<String>,
    pub source: String,
    pub items: Vec<PatchItem>,
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
        if text.contains('\0') {
            return Err(Failure::blocked(
                "用户 patch 包含无效字符。",
                format!("{source}: NUL byte"),
            ));
        }
        let mut schema: Option<u32> = None;
        let mut target: Option<String> = None;
        let mut section: Option<String> = None;
        let mut items: Vec<PatchItem> = Vec::new();
        let mut seen_content = false;
        let lines: Vec<&str> = text
            .split('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line))
            .collect();
        let mut index = 0usize;

        while index < lines.len() {
            let raw = lines[index];
            let line_number = index + 1;
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                index += 1;
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
                index += 1;
                continue;
            }
            if let Some(value) = trimmed.strip_prefix("schema=") {
                if schema.is_some() {
                    return Err(Failure::blocked(
                        "用户 patch 重复声明 schema。",
                        format!("{source}:{line_number}"),
                    ));
                }
                if seen_content || !items.is_empty() {
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
                if !(MIN_PATCH_SCHEMA..=PATCH_SCHEMA).contains(&parsed) {
                    return Err(Failure::blocked(
                        "用户 patch 的 schema 版本不受支持。",
                        format!("{source}:{line_number}: schema={parsed}"),
                    ));
                }
                schema = Some(parsed);
                index += 1;
                continue;
            }
            if let Some(value) = trimmed.strip_prefix("target=") {
                if seen_content || !items.is_empty() {
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
                index += 1;
                continue;
            }
            let Some(schema_value) = schema else {
                return Err(Failure::blocked(
                    "用户 patch 缺少 schema=1。",
                    format!("{source}:{line_number}"),
                ));
            };
            seen_content = true;

            if trimmed.starts_with('@') {
                if schema_value < 2 {
                    return Err(Failure::blocked(
                        "用户 patch 的行操作需要 schema=2。",
                        format!("{source}:{line_number}: {trimmed:?}"),
                    ));
                }
                index += 1;
                let (optional, directive) = parse_optional(trimmed);
                if let Some(rest) = strip_directive(directive, "after") {
                    let selector = parse_selector(rest, kind, source, line_number)?;
                    let block = parse_block(&lines, &mut index, source, line_number)?;
                    require_nonempty(&block, source, line_number, "@after")?;
                    items.push(PatchItem::Line(LineOp {
                        kind: LineOpKind::InsertAfter,
                        selector: Some(selector),
                        section: section.clone(),
                        lines: block,
                        old_lines: Vec::new(),
                        occurrence: None,
                        match_mode: MatchMode::Exact,
                        optional,
                        line: line_number,
                    }));
                    continue;
                }
                if let Some(rest) = strip_directive(directive, "before") {
                    let selector = parse_selector(rest, kind, source, line_number)?;
                    let block = parse_block(&lines, &mut index, source, line_number)?;
                    require_nonempty(&block, source, line_number, "@before")?;
                    items.push(PatchItem::Line(LineOp {
                        kind: LineOpKind::InsertBefore,
                        selector: Some(selector),
                        section: section.clone(),
                        lines: block,
                        old_lines: Vec::new(),
                        occurrence: None,
                        match_mode: MatchMode::Exact,
                        optional,
                        line: line_number,
                    }));
                    continue;
                }
                if let Some(rest) = strip_directive(directive, "replace") {
                    let selector = parse_selector(rest, kind, source, line_number)?;
                    let block = parse_block(&lines, &mut index, source, line_number)?;
                    require_nonempty(&block, source, line_number, "@replace")?;
                    items.push(PatchItem::Line(LineOp {
                        kind: LineOpKind::Replace,
                        selector: Some(selector),
                        section: section.clone(),
                        lines: block,
                        old_lines: Vec::new(),
                        occurrence: None,
                        match_mode: MatchMode::Exact,
                        optional,
                        line: line_number,
                    }));
                    continue;
                }
                if let Some(rest) = strip_directive(directive, "delete") {
                    let selector = parse_selector(rest, kind, source, line_number)?;
                    items.push(PatchItem::Line(LineOp {
                        kind: LineOpKind::Delete,
                        selector: Some(selector),
                        section: section.clone(),
                        lines: Vec::new(),
                        old_lines: Vec::new(),
                        occurrence: None,
                        match_mode: MatchMode::Exact,
                        optional,
                        line: line_number,
                    }));
                    continue;
                }
                if let Some(rest) = strip_directive(directive, "append") {
                    if !rest.trim().is_empty() {
                        return Err(Failure::blocked(
                            "用户 patch 的 @append 不接受选择器。",
                            format!("{source}:{line_number}: {rest:?}"),
                        ));
                    }
                    let block = parse_block(&lines, &mut index, source, line_number)?;
                    require_nonempty(&block, source, line_number, "@append")?;
                    items.push(PatchItem::Line(LineOp {
                        kind: LineOpKind::Append,
                        selector: None,
                        section: section.clone(),
                        lines: block,
                        old_lines: Vec::new(),
                        occurrence: None,
                        match_mode: MatchMode::Exact,
                        optional,
                        line: line_number,
                    }));
                    continue;
                }
                if let Some(rest) = strip_directive(directive, "prepend") {
                    if !rest.trim().is_empty() {
                        return Err(Failure::blocked(
                            "用户 patch 的 @prepend 不接受选择器。",
                            format!("{source}:{line_number}: {rest:?}"),
                        ));
                    }
                    let block = parse_block(&lines, &mut index, source, line_number)?;
                    require_nonempty(&block, source, line_number, "@prepend")?;
                    items.push(PatchItem::Line(LineOp {
                        kind: LineOpKind::Prepend,
                        selector: None,
                        section: section.clone(),
                        lines: block,
                        old_lines: Vec::new(),
                        occurrence: None,
                        match_mode: MatchMode::Exact,
                        optional,
                        line: line_number,
                    }));
                    continue;
                }
                if let Some(rest) = strip_directive(directive, "splice") {
                    let (remove, insert, occurrence, match_mode) =
                        parse_splice_args(rest, source, line_number)?;
                    if remove == 0 {
                        return Err(Failure::blocked(
                            "用户 patch 的 @splice 需要 remove>=1；插入请使用 @after/@before。",
                            format!("{source}:{line_number}"),
                        ));
                    }
                    let block = parse_block(&lines, &mut index, source, line_number)?;
                    let expected = remove.checked_add(insert).ok_or_else(|| {
                        Failure::blocked(
                            "用户 patch 的 @splice remove/insert 数值过大。",
                            format!("{source}:{line_number}"),
                        )
                    })?;
                    if block.len() != expected {
                        return Err(Failure::blocked(
                            "用户 patch 的 @splice 行数与 remove/insert 不一致。",
                            format!(
                                "{source}:{line_number}: expected {expected}, got {}",
                                block.len()
                            ),
                        ));
                    }
                    let old_lines = block[..remove].to_vec();
                    let new_lines = block[remove..].to_vec();
                    items.push(PatchItem::Line(LineOp {
                        kind: LineOpKind::Splice,
                        selector: None,
                        section: section.clone(),
                        lines: new_lines,
                        old_lines,
                        occurrence,
                        match_mode,
                        optional,
                        line: line_number,
                    }));
                    continue;
                }
                return Err(Failure::blocked(
                    "用户 patch 无法识别的行操作。",
                    format!("{source}:{line_number}: {trimmed:?}"),
                ));
            }

            if trimmed.starts_with('|') {
                return Err(Failure::blocked(
                    "用户 patch 在行块之外遇到了 | 行。",
                    format!("{source}:{line_number}"),
                ));
            }

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
                items.push(PatchItem::Key(PatchEntry {
                    section: section.clone(),
                    key: key.to_string(),
                    op,
                    value: String::new(),
                    line: line_number,
                }));
                index += 1;
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
            items.push(PatchItem::Key(PatchEntry {
                section: section.clone(),
                key: key.to_string(),
                op,
                value: value.to_string(),
                line: line_number,
            }));
            index += 1;
        }

        let Some(schema) = schema else {
            return Err(Failure::blocked(
                "用户 patch 缺少 schema=1。",
                source.to_string(),
            ));
        };
        if items.len() > MAX_ENTRIES {
            return Err(Failure::blocked(
                "用户 patch 条目过多。",
                format!("{source}: {} entries", items.len()),
            ));
        }
        Ok(Self {
            schema,
            target,
            source: source.to_string(),
            items,
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
        output.push_str(&format!(
            "# PlayKit 用户配置 patch（schema {}）\n",
            self.schema
        ));
        output.push_str("# 由 playkit-updater 维护；这里只保存配置意图，不保存旧配置副本。\n");
        output.push_str(&format!("schema={}\n", self.schema));
        if let Some(target) = &self.target {
            output.push_str(&format!("target={target}\n"));
        }
        let mut current_section: Option<Option<String>> = None;
        for item in &self.items {
            let section = item.section();
            if current_section.as_ref() != Some(&section) {
                if let Some(name) = &section {
                    output.push('\n');
                    output.push_str(&format!("[{name}]\n"));
                } else if current_section.is_some() {
                    output.push('\n');
                    output.push_str("[global]\n");
                }
                current_section = Some(section);
            }
            match item {
                PatchItem::Key(entry) => match entry.op {
                    PatchOp::Set => output.push_str(&format!("{}={}\n", entry.key, entry.value)),
                    PatchOp::Append => {
                        output.push_str(&format!("+{}={}\n", entry.key, entry.value))
                    }
                    PatchOp::Remove => output.push_str(&format!("-{}\n", entry.key)),
                },
                PatchItem::Line(op) => render_line_op(&mut output, op),
            }
        }
        output
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn has_line_ops(&self) -> bool {
        self.items
            .iter()
            .any(|item| matches!(item, PatchItem::Line(_)))
    }

    pub fn key_items(&self) -> impl Iterator<Item = &PatchEntry> {
        self.items.iter().filter_map(|item| match item {
            PatchItem::Key(entry) => Some(entry),
            PatchItem::Line(_) => None,
        })
    }

    pub fn line_items(&self) -> impl Iterator<Item = &LineOp> {
        self.items.iter().filter_map(|item| match item {
            PatchItem::Key(_) => None,
            PatchItem::Line(op) => Some(op),
        })
    }
}

fn parse_optional(line: &str) -> (bool, &str) {
    if let Some(rest) = line.strip_prefix("@optional") {
        let rest = rest.trim_start();
        let rest = rest.strip_prefix('@').unwrap_or(rest);
        return (true, rest);
    }
    (false, line.strip_prefix('@').unwrap_or(line))
}

fn strip_directive<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(name)?;
    if rest.is_empty() || rest.starts_with(char::is_whitespace) {
        Some(rest.trim())
    } else {
        None
    }
}

fn parse_block(
    lines: &[&str],
    index: &mut usize,
    source: &str,
    start_line: usize,
) -> Result<Vec<String>> {
    let mut result = Vec::new();
    while *index < lines.len() {
        let raw = lines[*index];
        if raw == "@end" {
            *index += 1;
            return Ok(result);
        }
        let Some(content) = raw.strip_prefix('|') else {
            return Err(Failure::blocked(
                "用户 patch 的行块内容必须以 | 开头，并以 @end 结束。",
                format!("{source}:{}: {raw:?}", *index + 1),
            ));
        };
        result.push(content.to_string());
        if result.len() > MAX_BLOCK_LINES {
            return Err(Failure::blocked(
                "用户 patch 的行块过大。",
                format!("{source}:{start_line}"),
            ));
        }
        *index += 1;
    }
    Err(Failure::blocked(
        "用户 patch 的行块缺少 @end。",
        format!("{source}:{start_line}"),
    ))
}

fn require_nonempty(block: &[String], source: &str, line: usize, directive: &str) -> Result<()> {
    if block.is_empty() {
        return Err(Failure::blocked(
            "用户 patch 的行操作缺少内容。",
            format!("{source}:{line}: {directive}"),
        ));
    }
    Ok(())
}

fn parse_splice_args(
    text: &str,
    source: &str,
    line: usize,
) -> Result<(usize, usize, Option<usize>, MatchMode)> {
    let mut remove: Option<usize> = None;
    let mut insert: Option<usize> = None;
    let mut occurrence: Option<usize> = None;
    let mut match_mode = MatchMode::Exact;
    for part in text.split_whitespace() {
        let Some((key, value)) = part.split_once('=') else {
            return Err(Failure::blocked(
                "用户 patch 的 @splice 参数格式错误。",
                format!("{source}:{line}: {part:?}"),
            ));
        };
        match key {
            "remove" => {
                if remove.is_some() {
                    return Err(Failure::blocked(
                        "用户 patch 的 @splice 重复参数。",
                        format!("{source}:{line}: {key}"),
                    ));
                }
                remove = Some(value.parse::<usize>().map_err(|_| {
                    Failure::blocked(
                        "用户 patch 的 @splice remove 无效。",
                        format!("{source}:{line}: {value:?}"),
                    )
                })?);
            }
            "insert" => {
                if insert.is_some() {
                    return Err(Failure::blocked(
                        "用户 patch 的 @splice 重复参数。",
                        format!("{source}:{line}: {key}"),
                    ));
                }
                insert = Some(value.parse::<usize>().map_err(|_| {
                    Failure::blocked(
                        "用户 patch 的 @splice insert 无效。",
                        format!("{source}:{line}: {value:?}"),
                    )
                })?);
            }
            "occurrence" => {
                let parsed = value.parse::<usize>().map_err(|_| {
                    Failure::blocked(
                        "用户 patch 的 @splice occurrence 无效。",
                        format!("{source}:{line}: {value:?}"),
                    )
                })?;
                if parsed == 0 {
                    return Err(Failure::blocked(
                        "用户 patch 的 @splice occurrence 必须从 1 开始。",
                        format!("{source}:{line}"),
                    ));
                }
                occurrence = Some(parsed);
            }
            "match" => {
                match_mode = match value {
                    "exact" => MatchMode::Exact,
                    "trim" => MatchMode::Trim,
                    _ => {
                        return Err(Failure::blocked(
                            "用户 patch 的 @splice match 必须是 exact 或 trim。",
                            format!("{source}:{line}: {value:?}"),
                        ));
                    }
                }
            }
            _ => {
                return Err(Failure::blocked(
                    "用户 patch 的 @splice 包含未知参数。",
                    format!("{source}:{line}: {key}"),
                ));
            }
        }
    }
    let Some(remove) = remove else {
        return Err(Failure::blocked(
            "用户 patch 的 @splice 缺少 remove。",
            format!("{source}:{line}"),
        ));
    };
    let Some(insert) = insert else {
        return Err(Failure::blocked(
            "用户 patch 的 @splice 缺少 insert。",
            format!("{source}:{line}"),
        ));
    };
    Ok((remove, insert, occurrence, match_mode))
}

fn parse_selector(text: &str, kind: ConfigKind, source: &str, line: usize) -> Result<Selector> {
    let text = text.trim();
    let Some((kind_part, value)) = text.split_once(':') else {
        return Err(Failure::blocked(
            "用户 patch 的选择器缺少冒号。",
            format!("{source}:{line}: {text:?}"),
        ));
    };
    let (name, occurrence) = parse_occurrence(kind_part, source, line)?;
    let selector_kind = match name {
        "menu" => SelectorKind::Menu,
        "command" => SelectorKind::Command,
        "key" => SelectorKind::Key,
        "section" => SelectorKind::Section,
        "line" => SelectorKind::Line,
        _ => {
            return Err(Failure::blocked(
                "用户 patch 的选择器类型无效。",
                format!("{source}:{line}: {name:?}"),
            ));
        }
    };
    let value = value.trim();
    if value.is_empty() {
        return Err(Failure::blocked(
            "用户 patch 的选择器内容为空。",
            format!("{source}:{line}: {text:?}"),
        ));
    }
    if value.contains('\n') || value.contains('\r') {
        return Err(Failure::blocked(
            "用户 patch 的选择器包含换行。",
            format!("{source}:{line}"),
        ));
    }
    match selector_kind {
        SelectorKind::Menu | SelectorKind::Command => {
            if kind != ConfigKind::Input {
                return Err(Failure::blocked(
                    "menu/command 选择器只能用于 input.conf。",
                    format!("{source}:{line}: {text:?}"),
                ));
            }
            if value.contains('#') {
                return Err(Failure::blocked(
                    "menu/command 选择器不能包含 #。",
                    format!("{source}:{line}: {text:?}"),
                ));
            }
        }
        SelectorKind::Section => {
            if kind != ConfigKind::Profiles {
                return Err(Failure::blocked(
                    "section 选择器只能用于 profiles.conf。",
                    format!("{source}:{line}: {text:?}"),
                ));
            }
            if value.contains('[') || value.contains(']') {
                return Err(Failure::blocked(
                    "section 选择器包含无效字符。",
                    format!("{source}:{line}: {text:?}"),
                ));
            }
        }
        SelectorKind::Key => {
            if !kind.validate_key(value) {
                return Err(Failure::blocked(
                    "用户 patch 的 key 选择器包含无效的配置键。",
                    format!("{source}:{line}: {value:?}"),
                ));
            }
        }
        SelectorKind::Line => {}
    }
    Ok(Selector {
        kind: selector_kind,
        text: value.to_string(),
        occurrence,
    })
}

fn parse_occurrence<'a>(
    text: &'a str,
    source: &str,
    line: usize,
) -> Result<(&'a str, Option<usize>)> {
    if let Some(open) = text.find('[') {
        if !text.ends_with(']') {
            return Err(Failure::blocked(
                "用户 patch 的选择器序号格式错误。",
                format!("{source}:{line}: {text:?}"),
            ));
        }
        let name = &text[..open];
        let value = &text[open + 1..text.len() - 1];
        let occurrence = value.parse::<usize>().map_err(|_| {
            Failure::blocked(
                "用户 patch 的选择器序号无效。",
                format!("{source}:{line}: {value:?}"),
            )
        })?;
        if occurrence == 0 {
            return Err(Failure::blocked(
                "用户 patch 的选择器序号必须从 1 开始。",
                format!("{source}:{line}: {text:?}"),
            ));
        }
        Ok((name, Some(occurrence)))
    } else {
        Ok((text, None))
    }
}

fn render_line_op(output: &mut String, op: &LineOp) {
    if op.optional {
        output.push_str("@optional ");
    }
    match op.kind {
        LineOpKind::InsertAfter => {
            output.push_str("@after ");
            output.push_str(&render_selector(
                op.selector.as_ref().expect("after selector"),
            ));
            output.push('\n');
            render_block(output, &op.lines);
        }
        LineOpKind::InsertBefore => {
            output.push_str("@before ");
            output.push_str(&render_selector(
                op.selector.as_ref().expect("before selector"),
            ));
            output.push('\n');
            render_block(output, &op.lines);
        }
        LineOpKind::Replace => {
            output.push_str("@replace ");
            output.push_str(&render_selector(
                op.selector.as_ref().expect("replace selector"),
            ));
            output.push('\n');
            render_block(output, &op.lines);
        }
        LineOpKind::Delete => {
            output.push_str("@delete ");
            output.push_str(&render_selector(
                op.selector.as_ref().expect("delete selector"),
            ));
            output.push('\n');
        }
        LineOpKind::Append => {
            output.push_str("@append\n");
            render_block(output, &op.lines);
        }
        LineOpKind::Prepend => {
            output.push_str("@prepend\n");
            render_block(output, &op.lines);
        }
        LineOpKind::Splice => {
            output.push_str(&format!(
                "@splice remove={} insert={}",
                op.old_lines.len(),
                op.lines.len()
            ));
            if op.match_mode == MatchMode::Trim {
                output.push_str(" match=trim");
            }
            if let Some(occurrence) = op.occurrence {
                output.push_str(&format!(" occurrence={occurrence}"));
            }
            output.push('\n');
            for line in &op.old_lines {
                output.push('|');
                output.push_str(line);
                output.push('\n');
            }
            for line in &op.lines {
                output.push('|');
                output.push_str(line);
                output.push('\n');
            }
            output.push_str("@end\n");
        }
    }
}

fn render_block(output: &mut String, lines: &[String]) {
    for line in lines {
        output.push('|');
        output.push_str(line);
        output.push('\n');
    }
    output.push_str("@end\n");
}

fn render_selector(selector: &Selector) -> String {
    let kind = match selector.kind {
        SelectorKind::Menu => "menu",
        SelectorKind::Command => "command",
        SelectorKind::Key => "key",
        SelectorKind::Section => "section",
        SelectorKind::Line => "line",
    };
    match selector.occurrence {
        Some(occurrence) => format!("{kind}[{occurrence}]:{}", selector.text),
        None => format!("{kind}:{}", selector.text),
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
        let entries: Vec<&PatchEntry> = doc.key_items().collect();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].section.as_deref(), Some("gpu-hq"));
        assert_eq!(entries[1].op, PatchOp::Remove);
        assert_eq!(entries[2].op, PatchOp::Append);
    }

    #[test]
    fn global_section_maps_to_none() {
        let doc =
            PatchDoc::parse("schema=1\n[global]\nvolume=70\n", "test", ConfigKind::Mpv).unwrap();
        assert!(doc.key_items().next().unwrap().section.is_none());
    }

    #[test]
    fn missing_schema_is_rejected() {
        let error = PatchDoc::parse("volume=70\n", "test", ConfigKind::Mpv).unwrap_err();
        assert_eq!(error.kind, crate::util::ErrorKind::Blocked);
    }

    #[test]
    fn wrong_schema_is_rejected() {
        assert!(PatchDoc::parse("schema=3\nvolume=70\n", "test", ConfigKind::Mpv).is_err());
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
        assert_eq!(
            doc.key_items().next().unwrap().value,
            "cycle pause #! 播放 > 暂停"
        );
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
        assert_eq!(again.key_items().count(), 3);
        let entries: Vec<&PatchEntry> = again.key_items().collect();
        assert_eq!(entries[2].section.as_deref(), Some("gpu-hq"));
    }

    #[test]
    fn duplicate_entries_are_allowed() {
        let doc =
            PatchDoc::parse("schema=1\nvolume=70\nvolume=80\n", "test", ConfigKind::Mpv).unwrap();
        assert_eq!(doc.key_items().count(), 2);
    }

    #[test]
    fn empty_input_value_is_rejected() {
        assert!(PatchDoc::parse("schema=1\na=\n", "test", ConfigKind::Input).is_err());
    }

    #[test]
    fn schema_one_rejects_line_operations() {
        let error = PatchDoc::parse(
            "schema=1\n@after menu:更新 > 立即检查更新\n|x\n@end\n",
            "test",
            ConfigKind::Input,
        )
        .unwrap_err();
        assert_eq!(error.kind, crate::util::ErrorKind::Blocked);
    }

    #[test]
    fn parses_menu_insert_block() {
        let doc = PatchDoc::parse(
            "schema=2\ntarget=input.conf\n\n@after menu:更新 > 立即检查更新\n|# script-binding my-ui/refresh #! 更新 > 刷新订阅\n@end\n",
            "test",
            ConfigKind::Input,
        )
        .unwrap();
        let op = doc.line_items().next().unwrap();
        assert_eq!(op.kind, LineOpKind::InsertAfter);
        assert_eq!(op.selector.as_ref().unwrap().kind, SelectorKind::Menu);
        assert_eq!(op.lines.len(), 1);
    }

    #[test]
    fn parses_optional_and_occurrence() {
        let doc = PatchDoc::parse(
            "schema=2\n@optional after command[2]:script-binding update-check/check-now\n|x\n@end\n",
            "test",
            ConfigKind::Input,
        )
        .unwrap();
        let op = doc.line_items().next().unwrap();
        assert!(op.optional);
        assert_eq!(op.selector.as_ref().unwrap().occurrence, Some(2));
    }

    #[test]
    fn parses_splice_counts() {
        let doc = PatchDoc::parse(
            "schema=2\n@splice remove=2 insert=1 match=trim\n|old 1\n|old 2\n|new\n@end\n",
            "test",
            ConfigKind::Mpv,
        )
        .unwrap();
        let op = doc.line_items().next().unwrap();
        assert_eq!(op.kind, LineOpKind::Splice);
        assert_eq!(op.old_lines, vec!["old 1", "old 2"]);
        assert_eq!(op.lines, vec!["new"]);
        assert_eq!(op.match_mode, MatchMode::Trim);
    }

    #[test]
    fn block_prefix_is_removed_once() {
        let doc = PatchDoc::parse(
            "schema=2\n@append\n||already piped\n|\n@end\n",
            "test",
            ConfigKind::Mpv,
        )
        .unwrap();
        let op = doc.line_items().next().unwrap();
        assert_eq!(op.lines, vec!["|already piped", ""]);
    }

    #[test]
    fn unterminated_block_is_rejected() {
        assert!(PatchDoc::parse(
            "schema=2\n@after menu:更新 > 立即检查更新\n|x\n",
            "test",
            ConfigKind::Input
        )
        .is_err());
    }

    #[test]
    fn selector_kind_is_checked_against_target() {
        assert!(PatchDoc::parse(
            "schema=2\n@after menu:更新\n|x\n@end\n",
            "test",
            ConfigKind::Mpv
        )
        .is_err());
        assert!(PatchDoc::parse(
            "schema=2\n@after section:gpu-hq\n|x\n@end\n",
            "test",
            ConfigKind::Mpv
        )
        .is_err());
    }

    #[test]
    fn render_roundtrips_line_operations() {
        let doc = PatchDoc::parse(
            "schema=2\ntarget=input.conf\n@after menu:更新 > 立即检查更新\n|# script-binding my-ui/refresh #! 更新 > 刷新订阅\n@end\n@splice remove=1 insert=1\n|old\n|new\n@end\n",
            "test",
            ConfigKind::Input,
        )
        .unwrap();
        let text = doc.render();
        let again = PatchDoc::parse(&text, "test", ConfigKind::Input).unwrap();
        assert_eq!(again.line_items().count(), 2);
        assert_eq!(
            again.line_items().next().unwrap().lines,
            vec!["# script-binding my-ui/refresh #! 更新 > 刷新订阅"]
        );
    }
}
