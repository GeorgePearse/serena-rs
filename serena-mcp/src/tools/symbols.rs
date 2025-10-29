use std::collections::HashSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use once_cell::sync::Lazy;
use regex::{Regex, RegexBuilder};
use serde::Deserialize;
use serde_json::{Value, json};
use walkdir::WalkDir;

use crate::tool::{Tool, ToolRegistry};
use crate::tools::resolve_path;

pub fn register(registry: &mut ToolRegistry) {
    registry.register(find_symbol_tool());
    registry.register(find_referencing_symbols_tool());
    registry.register(get_symbols_overview_tool());
    registry.register(rename_symbol_tool());
    registry.register(replace_symbol_body_tool());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Language {
    Python,
    Rust,
    Typescript,
    Javascript,
    Go,
    Java,
    Csharp,
    Generic,
}

impl Language {
    fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_string_lossy().to_lowercase();
        let lang = match ext.as_str() {
            "py" => Self::Python,
            "rs" => Self::Rust,
            "ts" | "tsx" => Self::Typescript,
            "js" | "jsx" | "mjs" | "cjs" => Self::Javascript,
            "go" => Self::Go,
            "java" | "kt" | "kts" | "scala" => Self::Java,
            "cs" => Self::Csharp,
            "swift" | "cpp" | "cc" | "cxx" | "h" | "hpp" | "hh" | "rb" | "php" | "lua" | "zig"
            | "rsx" | "c" | "dart" | "el" | "erl" | "ex" | "exs" | "hs" | "ml" | "nim" | "sh" => {
                Self::Generic
            }
            _ => return None,
        };
        Some(lang)
    }

    fn as_str(&self) -> &'static str {
        match self {
            Language::Python => "python",
            Language::Rust => "rust",
            Language::Typescript => "typescript",
            Language::Javascript => "javascript",
            Language::Go => "go",
            Language::Java => "java",
            Language::Csharp => "csharp",
            Language::Generic => "generic",
        }
    }
}

#[derive(Debug, Clone)]
struct FileSymbol {
    name: String,
    kind: String,
    signature: String,
    line: usize,
    column: usize,
    body: BodyStyle,
}

#[derive(Debug, Clone)]
enum BodyStyle {
    Braces {
        start: usize,
        end: usize,
        base_indent: String,
        inner_indent: String,
    },
    Indented {
        start: usize,
        end: usize,
        base_indent: String,
        indent_unit: String,
    },
    None,
}

struct ParsedFile {
    language: Language,
    content: String,
    lines: FileLines,
    symbols: Vec<FileSymbol>,
}

impl ParsedFile {
    fn from_path(path: &Path) -> Result<Option<Self>> {
        let language = match Language::from_path(path) {
            Some(lang) => lang,
            None => return Ok(None),
        };

        let metadata = fs::metadata(path)
            .with_context(|| format!("Failed to read metadata for {}", path.display()))?;
        if metadata.len() > 2 * 1024 * 1024 {
            // Skip very large files to keep the tool responsive.
            return Ok(None);
        }

        let content = match fs::read_to_string(path) {
            Ok(content) => content,
            Err(err) if err.kind() == std::io::ErrorKind::InvalidData => return Ok(None),
            Err(err) => {
                return Err(err).with_context(|| format!("Failed to read {}", path.display()));
            }
        };

        let lines = FileLines::new(&content);
        let symbols = extract_symbols(&content, &lines, language);

        Ok(Some(Self {
            language,
            content,
            lines,
            symbols,
        }))
    }
}

struct FileLines {
    records: Vec<LineRecord>,
    starts: Vec<usize>,
}

impl FileLines {
    fn new(content: &str) -> Self {
        let mut records = Vec::new();
        let mut starts = Vec::new();

        if content.is_empty() {
            return Self { records, starts };
        }

        let mut offset = 0;
        for piece in content.split_inclusive('\n') {
            let len = piece.len();
            let text = piece.trim_end_matches('\n').to_string();
            records.push(LineRecord {
                start: offset,
                end: offset + len,
                text,
            });
            starts.push(offset);
            offset += len;
        }

        if !content.ends_with('\n') {
            if let Some(last) = records.last_mut() {
                last.end = content.len();
            }
        }

        Self { records, starts }
    }

    fn len(&self) -> usize {
        self.records.len()
    }

    fn line_index(&self, offset: usize) -> usize {
        match self.starts.binary_search(&offset) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        }
    }

    fn text(&self, index: usize) -> &str {
        &self.records[index].text
    }

    fn bounds(&self, index: usize) -> (usize, usize) {
        let record = &self.records[index];
        (record.start, record.end)
    }
}

struct LineRecord {
    start: usize,
    end: usize,
    text: String,
}

#[derive(Clone, Copy)]
struct BracePattern {
    regex: &'static Lazy<Regex>,
    kind: &'static str,
}

const fn brace_pattern(regex: &'static Lazy<Regex>, kind: &'static str) -> BracePattern {
    BracePattern { regex, kind }
}

static RUST_FN_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)^(?P<indent>\s*)(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?fn\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)").unwrap()
});

static RUST_STRUCT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?m)^(?P<indent>\s*)(?:pub(?:\([^)]*\))?\s+)?struct\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
    )
    .unwrap()
});

static RUST_ENUM_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?m)^(?P<indent>\s*)(?:pub(?:\([^)]*\))?\s+)?enum\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
    )
    .unwrap()
});

static RUST_TRAIT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?m)^(?P<indent>\s*)(?:pub(?:\([^)]*\))?\s+)?trait\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
    )
    .unwrap()
});

static RUST_IMPL_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)^(?P<indent>\s*)impl(?:<[^>]+>)?\s+(?P<name>[A-Za-z_][A-Za-z0-9_:<>]*)")
        .unwrap()
});

static JS_FUNCTION_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)^(?P<indent>\s*)(?:export\s+)?(?:async\s+)?function\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)").unwrap()
});

static JS_CLASS_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)^(?P<indent>\s*)(?:export\s+)?class\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)")
        .unwrap()
});

static ARROW_FN_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)^(?P<indent>\s*)(?:export\s+)?(?:const|let|var)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*=\s*(?:async\s+)?\(?[^\n]*=>").unwrap()
});

static GO_FUNC_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)^(?P<indent>\s*)func\s+(?:\([^)]+\)\s*)?(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\(")
        .unwrap()
});

static JAVA_CLASS_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)^(?P<indent>\s*)(?:public|protected|private|abstract|final|static|sealed|class|interface|record|enum|\s)+\s*(?:class|interface|record|enum)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)").unwrap()
});

static JAVA_METHOD_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)^(?P<indent>\s*)(?:public|protected|private|static|final|synchronized|abstract|default|async|override|mutating|class|\s)+[A-Za-z0-9_<>,\[\]]+\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap()
});

static GENERIC_FUNC_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)^(?P<indent>\s*)(?:pub\s+|export\s+|public\s+|private\s+|protected\s+|static\s+|final\s+|async\s+|fn\s+|function\s+|def\s+)*(?:fn|function)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)").unwrap()
});

static GENERIC_CLASS_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)^(?P<indent>\s*)(?:export\s+|public\s+|private\s+|protected\s+|abstract\s+|final\s+)*(?:class|struct|enum|trait)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)").unwrap()
});

static PY_DEF_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)^(?P<indent>\s*)(?:async\s+)?def\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\(")
        .unwrap()
});

static PY_CLASS_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?m)^(?P<indent>\s*)class\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)").unwrap()
});

static RUST_PATTERNS: &[BracePattern] = &[
    brace_pattern(&RUST_FN_RE, "function"),
    brace_pattern(&RUST_STRUCT_RE, "struct"),
    brace_pattern(&RUST_ENUM_RE, "enum"),
    brace_pattern(&RUST_TRAIT_RE, "trait"),
    brace_pattern(&RUST_IMPL_RE, "impl"),
];

static JS_PATTERNS: &[BracePattern] = &[
    brace_pattern(&JS_FUNCTION_RE, "function"),
    brace_pattern(&JS_CLASS_RE, "class"),
    brace_pattern(&ARROW_FN_RE, "function"),
];

static GO_PATTERNS: &[BracePattern] = &[brace_pattern(&GO_FUNC_RE, "function")];

static JAVA_PATTERNS: &[BracePattern] = &[
    brace_pattern(&JAVA_CLASS_RE, "class"),
    brace_pattern(&JAVA_METHOD_RE, "method"),
];

static GENERIC_PATTERNS: &[BracePattern] = &[
    brace_pattern(&GENERIC_FUNC_RE, "function"),
    brace_pattern(&GENERIC_CLASS_RE, "type"),
];

fn brace_patterns(language: Language) -> &'static [BracePattern] {
    match language {
        Language::Rust => RUST_PATTERNS,
        Language::Typescript | Language::Javascript => JS_PATTERNS,
        Language::Go => GO_PATTERNS,
        Language::Java | Language::Csharp => JAVA_PATTERNS,
        Language::Generic => GENERIC_PATTERNS,
        // Fallback for Python handled separately
        Language::Python => &[],
    }
}

fn extract_symbols(content: &str, lines: &FileLines, language: Language) -> Vec<FileSymbol> {
    match language {
        Language::Python => parse_python_symbols(content, lines),
        _ => parse_brace_symbols(content, lines, language),
    }
}

fn parse_python_symbols(content: &str, lines: &FileLines) -> Vec<FileSymbol> {
    let mut symbols = Vec::new();

    for caps in PY_DEF_RE.captures_iter(content) {
        if let Some(symbol) = build_python_symbol(&caps, lines, "function") {
            symbols.push(symbol);
        }
    }

    for caps in PY_CLASS_RE.captures_iter(content) {
        if let Some(symbol) = build_python_symbol(&caps, lines, "class") {
            symbols.push(symbol);
        }
    }

    symbols.sort_by_key(|s| s.line);
    symbols
}

fn build_python_symbol(
    caps: &regex::Captures<'_>,
    lines: &FileLines,
    kind: &str,
) -> Option<FileSymbol> {
    let name = caps.name("name")?.as_str().to_string();
    let indent = caps.name("indent").map(|m| m.as_str()).unwrap_or("");
    let line_idx = lines.line_index(caps.get(0)?.start());
    let line_text = lines.text(line_idx).trim_end().to_string();
    let column = indent.len() + 1;
    let body = locate_python_body(lines, line_idx, indent);

    Some(FileSymbol {
        name,
        kind: kind.to_string(),
        signature: line_text,
        line: line_idx + 1,
        column,
        body,
    })
}

fn locate_python_body(lines: &FileLines, def_line: usize, base_indent: &str) -> BodyStyle {
    let base_indent_len = base_indent.len();
    let mut start_line: Option<usize> = None;
    let mut end_line: Option<usize> = None;

    for idx in (def_line + 1)..lines.len() {
        let text = lines.text(idx);
        if text.trim().is_empty() {
            if start_line.is_some() {
                end_line = Some(idx);
            }
            continue;
        }

        let indent = leading_whitespace(text);
        if indent.len() <= base_indent_len {
            break;
        }

        if start_line.is_none() {
            start_line = Some(idx);
        }
        end_line = Some(idx);
    }

    match (start_line, end_line) {
        (Some(start), Some(end)) => {
            let (start_offset, _) = lines.bounds(start);
            let (_, end_offset) = lines.bounds(end);
            let inner_indent = leading_whitespace(lines.text(start));
            let indent_unit = derive_indent_unit(inner_indent, base_indent);

            BodyStyle::Indented {
                start: start_offset,
                end: end_offset,
                base_indent: base_indent.to_string(),
                indent_unit,
            }
        }
        _ => BodyStyle::None,
    }
}

fn parse_brace_symbols(content: &str, lines: &FileLines, language: Language) -> Vec<FileSymbol> {
    let mut symbols = Vec::new();
    let patterns = brace_patterns(language);

    for pattern in patterns {
        for caps in pattern.regex.captures_iter(content) {
            let name_match = match caps.name("name") {
                Some(value) => value.as_str(),
                None => continue,
            };
            let name = name_match.to_string();
            let match_range = caps.get(0).unwrap();
            let line_idx = lines.line_index(match_range.start());
            let line_text = lines.text(line_idx).trim_end().to_string();
            let indent = caps
                .name("indent")
                .map(|m| m.as_str())
                .unwrap_or_else(|| leading_whitespace(lines.text(line_idx)));
            let column = indent.len() + 1;
            let line_end = lines.bounds(line_idx).1;
            let body = locate_brace_body(content, line_end, indent);

            symbols.push(FileSymbol {
                name,
                kind: pattern.kind.to_string(),
                signature: line_text,
                line: line_idx + 1,
                column,
                body,
            });
        }
    }

    symbols.sort_by_key(|s| s.line);
    symbols
}

fn locate_brace_body(content: &str, search_start: usize, indent: &str) -> BodyStyle {
    if let Some((start, end)) = find_brace_block(content, search_start) {
        let inner_indent = compute_inner_indent(content, start, end, indent);
        BodyStyle::Braces {
            start,
            end,
            base_indent: indent.to_string(),
            inner_indent,
        }
    } else {
        BodyStyle::None
    }
}

fn find_brace_block(content: &str, mut index: usize) -> Option<(usize, usize)> {
    let bytes = content.as_bytes();
    let len = bytes.len();

    while index < len {
        match bytes[index] {
            b'{' => {
                let mut depth = 1;
                let mut cursor = index + 1;
                while cursor < len {
                    match bytes[cursor] {
                        b'{' => depth += 1,
                        b'}' => {
                            depth -= 1;
                            if depth == 0 {
                                return Some((index + 1, cursor));
                            }
                        }
                        b'"' | b'\'' | b'`' => {
                            cursor = skip_string(bytes, cursor);
                            continue;
                        }
                        _ => {}
                    }
                    cursor += 1;
                }
                break;
            }
            b';' => return None,
            b'"' | b'\'' | b'`' => {
                index = skip_string(bytes, index);
                continue;
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn skip_string(bytes: &[u8], mut index: usize) -> usize {
    let quote = bytes[index];
    index += 1;
    while index < bytes.len() {
        let b = bytes[index];
        if b == b'\\' {
            index += 2;
            continue;
        }
        if b == quote {
            return index + 1;
        }
        index += 1;
    }
    bytes.len()
}

fn compute_inner_indent(content: &str, start: usize, end: usize, base_indent: &str) -> String {
    let slice = &content[start..end];
    for line in slice.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let indent = leading_whitespace(line);
        if !indent.is_empty() {
            return indent.to_string();
        }
    }
    format!("{base_indent}    ")
}

fn derive_indent_unit(inner_indent: &str, base_indent: &str) -> String {
    if inner_indent.len() > base_indent.len() {
        inner_indent[base_indent.len()..].to_string()
    } else if !inner_indent.is_empty() {
        inner_indent.to_string()
    } else {
        "    ".to_string()
    }
}

fn leading_whitespace(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut end = 0;
    while end < bytes.len() && (bytes[end] == b' ' || bytes[end] == b'\t') {
        end += 1;
    }
    &s[..end]
}

fn find_symbol_tool() -> Tool {
    let schema = json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "Symbol name or pattern to search for",
            },
            "path": {
                "type": "string",
                "description": "File or directory to inspect. Defaults to current working directory.",
            },
            "match_substring": {
                "type": "boolean",
                "description": "Allow substring matches instead of exact matches",
                "default": true,
            },
            "case_sensitive": {
                "type": "boolean",
                "description": "Whether matching is case sensitive",
                "default": false,
            },
            "include_body": {
                "type": "boolean",
                "description": "Include symbol body text when available",
                "default": false,
            },
            "kinds": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Restrict to specific symbol kinds (e.g. function, class)",
            },
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "description": "Maximum number of results to return",
            }
        },
        "required": ["name"],
        "additionalProperties": false
    });

    #[derive(Deserialize)]
    struct Params {
        name: String,
        #[serde(default)]
        path: Option<String>,
        #[serde(default = "default_true")]
        match_substring: bool,
        #[serde(default)]
        case_sensitive: Option<bool>,
        #[serde(default)]
        include_body: Option<bool>,
        #[serde(default)]
        kinds: Option<Vec<String>>,
        #[serde(default)]
        max_results: Option<usize>,
    }

    let handler = move |params| -> Result<Value> {
        let args: Params =
            serde_json::from_value(params).context("Invalid arguments for find_symbol")?;
        let root = match &args.path {
            Some(path) => resolve_path(path)?,
            None => std::env::current_dir()?,
        };

        let case_sensitive = args.case_sensitive.unwrap_or(false);
        let include_body = args.include_body.unwrap_or(false);
        let max_results = args.max_results.unwrap_or(50);
        let kind_filter: Option<HashSet<String>> = args
            .kinds
            .as_ref()
            .map(|kinds| kinds.iter().map(|s| s.to_lowercase()).collect());

        let mut matches = Vec::new();

        if root.is_file() {
            collect_symbols_for_file(
                &root,
                &args.name,
                args.match_substring,
                case_sensitive,
                include_body,
                kind_filter.as_ref(),
                max_results,
                &mut matches,
            )?;
        } else {
            for entry in WalkDir::new(&root)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                collect_symbols_for_file(
                    entry.path(),
                    &args.name,
                    args.match_substring,
                    case_sensitive,
                    include_body,
                    kind_filter.as_ref(),
                    max_results,
                    &mut matches,
                )?;

                if matches.len() >= max_results {
                    break;
                }
            }
        }

        let truncated = matches.len() >= max_results;
        Ok(json!({
            "query": args.name,
            "count": matches.len(),
            "truncated": truncated,
            "matches": matches,
        }))
    };

    Tool::new(
        "find_symbol",
        "Search for symbol definitions across the project",
        schema,
        Box::new(handler),
    )
}

fn default_true() -> bool {
    true
}

fn collect_symbols_for_file(
    path: &Path,
    query: &str,
    match_substring: bool,
    case_sensitive: bool,
    include_body: bool,
    kind_filter: Option<&HashSet<String>>,
    max_results: usize,
    matches: &mut Vec<Value>,
) -> Result<()> {
    if matches.len() >= max_results {
        return Ok(());
    }

    let Some(parsed) = ParsedFile::from_path(path)? else {
        return Ok(());
    };

    for symbol in parsed.symbols.iter() {
        if matches.len() >= max_results {
            break;
        }

        if let Some(filter) = kind_filter {
            if !filter.contains(&symbol.kind.to_lowercase()) {
                continue;
            }
        }

        if !symbol_name_matches(&symbol.name, query, match_substring, case_sensitive) {
            continue;
        }

        let mut entry = json!({
            "name": symbol.name,
            "kind": symbol.kind,
            "path": path.to_string_lossy(),
            "line": symbol.line,
            "column": symbol.column,
            "signature": symbol.signature,
            "language": parsed.language.as_str(),
        });

        if include_body {
            if let Some(body) = extract_body(&parsed.content, &symbol.body) {
                entry["body"] = json!(body);
            }
        }

        matches.push(entry);
    }

    Ok(())
}

fn symbol_name_matches(symbol: &str, query: &str, substring: bool, case_sensitive: bool) -> bool {
    if case_sensitive {
        if substring {
            symbol.contains(query)
        } else {
            symbol == query
        }
    } else {
        let symbol_lower = symbol.to_lowercase();
        let query_lower = query.to_lowercase();
        if substring {
            symbol_lower.contains(&query_lower)
        } else {
            symbol_lower == query_lower
        }
    }
}

fn extract_body(content: &str, body: &BodyStyle) -> Option<String> {
    match body {
        BodyStyle::Braces { start, end, .. } => {
            if *start >= *end || *end > content.len() {
                None
            } else {
                Some(content[*start..*end].trim_matches('\n').to_string())
            }
        }
        BodyStyle::Indented { start, end, .. } => {
            if *start >= *end || *end > content.len() {
                None
            } else {
                Some(content[*start..*end].trim_matches('\n').to_string())
            }
        }
        BodyStyle::None => None,
    }
}

fn find_referencing_symbols_tool() -> Tool {
    let schema = json!({
        "type": "object",
        "properties": {
            "name": {"type": "string", "description": "Target symbol name to search for"},
            "path": {"type": "string", "description": "Directory or file to search"},
            "case_sensitive": {"type": "boolean", "default": false},
            "max_results": {"type": "integer", "minimum": 1},
            "context_lines": {"type": "integer", "minimum": 0},
            "include_hidden": {"type": "boolean", "default": false}
        },
        "required": ["name"],
        "additionalProperties": false
    });

    #[derive(Deserialize)]
    struct Params {
        name: String,
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        case_sensitive: Option<bool>,
        #[serde(default)]
        max_results: Option<usize>,
        #[serde(default)]
        context_lines: Option<usize>,
        #[serde(default)]
        include_hidden: Option<bool>,
    }

    let handler = move |params| -> Result<Value> {
        let args: Params = serde_json::from_value(params)
            .context("Invalid arguments for find_referencing_symbols")?;
        let root = match &args.path {
            Some(path) => resolve_path(path)?,
            None => std::env::current_dir()?,
        };

        let case_sensitive = args.case_sensitive.unwrap_or(false);
        let max_results = args.max_results.unwrap_or(50);
        let context_lines = args.context_lines.unwrap_or(2);
        let include_hidden = args.include_hidden.unwrap_or(false);

        let mut matches = Vec::new();

        let symbol_pattern = RegexBuilder::new(&format!("\\b{}\\b", regex::escape(&args.name)))
            .case_insensitive(!case_sensitive)
            .build()
            .with_context(|| format!("Failed to compile search pattern for '{}'", args.name))?;

        if root.is_file() {
            scan_file_for_references(
                &root,
                &symbol_pattern,
                context_lines,
                max_results,
                &mut matches,
            )?;
        } else {
            for entry in WalkDir::new(&root)
                .follow_links(false)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if !entry.file_type().is_file() {
                    continue;
                }
                if !include_hidden && is_hidden_path(entry.path()) {
                    continue;
                }
                scan_file_for_references(
                    entry.path(),
                    &symbol_pattern,
                    context_lines,
                    max_results,
                    &mut matches,
                )?;
                if matches.len() >= max_results {
                    break;
                }
            }
        }

        Ok(json!({
            "symbol": args.name,
            "count": matches.len(),
            "matches": matches,
        }))
    };

    Tool::new(
        "find_referencing_symbols",
        "Locate references to a symbol by searching for exact word matches",
        schema,
        Box::new(handler),
    )
}

fn is_hidden_path(path: &Path) -> bool {
    path.components().any(|component| match component {
        std::path::Component::Normal(name) => name.to_string_lossy().starts_with('.'),
        _ => false,
    })
}

fn scan_file_for_references(
    path: &Path,
    pattern: &Regex,
    context_lines: usize,
    max_results: usize,
    matches: &mut Vec<Value>,
) -> Result<()> {
    if matches.len() >= max_results {
        return Ok(());
    }

    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::InvalidData => return Ok(()),
        Err(err) => {
            return Err(err).with_context(|| format!("Failed to read {}", path.display()));
        }
    };

    let lines: Vec<&str> = content.lines().collect();

    for (idx, line) in lines.iter().enumerate() {
        for capture in pattern.find_iter(line) {
            let column = line[..capture.start()].chars().count() + 1;
            let preview = line.trim_end().to_string();
            let mut context = Vec::new();

            if context_lines > 0 {
                let start = idx.saturating_sub(context_lines);
                let end = usize::min(idx + context_lines, lines.len().saturating_sub(1));
                for ctx_idx in start..=end {
                    if ctx_idx == idx {
                        continue;
                    }
                    context.push(json!({
                        "line": ctx_idx + 1,
                        "text": lines[ctx_idx].trim_end(),
                    }));
                }
            }

            matches.push(json!({
                "path": path.to_string_lossy(),
                "line": idx + 1,
                "column": column,
                "preview": preview,
                "context": context,
            }));

            if matches.len() >= max_results {
                return Ok(());
            }
        }
    }

    Ok(())
}

fn get_symbols_overview_tool() -> Tool {
    let schema = json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "File or directory to summarise",
            },
            "max_files": {
                "type": "integer",
                "minimum": 1,
                "description": "Limit number of files when summarising a directory",
            }
        },
        "required": ["path"],
        "additionalProperties": false
    });

    #[derive(Deserialize)]
    struct Params {
        path: String,
        #[serde(default)]
        max_files: Option<usize>,
    }

    let handler = move |params| -> Result<Value> {
        let args: Params =
            serde_json::from_value(params).context("Invalid arguments for get_symbols_overview")?;
        let path = resolve_path(&args.path)?;

        if path.is_file() {
            let parsed =
                ParsedFile::from_path(&path)?.context("Path is not a recognised source file")?;
            let symbols = parsed
                .symbols
                .iter()
                .map(|symbol| {
                    json!({
                        "name": symbol.name,
                        "kind": symbol.kind,
                        "line": symbol.line,
                        "signature": symbol.signature,
                    })
                })
                .collect::<Vec<_>>();

            Ok(json!({
                "path": path.to_string_lossy(),
                "language": parsed.language.as_str(),
                "symbol_count": symbols.len(),
                "symbols": symbols,
            }))
        } else {
            let max_files = args.max_files.unwrap_or(20);
            let mut summaries = Vec::new();
            let mut total_symbols = 0usize;

            for entry in WalkDir::new(&path)
                .max_depth(4)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                if summaries.len() >= max_files {
                    break;
                }
                if let Some(parsed) = ParsedFile::from_path(entry.path())? {
                    let count = parsed.symbols.len();
                    total_symbols += count;
                    summaries.push(json!({
                        "path": entry.path().strip_prefix(&path).unwrap_or(entry.path()).to_string_lossy(),
                        "language": parsed.language.as_str(),
                        "symbol_count": count,
                        "top_symbols": parsed.symbols.iter().take(5).map(|symbol| json!({
                            "name": symbol.name,
                            "kind": symbol.kind,
                            "line": symbol.line,
                        })).collect::<Vec<_>>(),
                    }));
                }
            }

            Ok(json!({
                "path": path.to_string_lossy(),
                "files_summarised": summaries.len(),
                "total_symbols": total_symbols,
                "files": summaries,
            }))
        }
    };

    Tool::new(
        "get_symbols_overview",
        "Summarise the symbols declared in a file or directory",
        schema,
        Box::new(handler),
    )
}

fn rename_symbol_tool() -> Tool {
    let schema = json!({
        "type": "object",
        "properties": {
            "path": {"type": "string"},
            "old_name": {"type": "string"},
            "new_name": {"type": "string"},
            "case_sensitive": {"type": "boolean", "default": true},
            "occurrence": {"type": "integer", "minimum": 1, "description": "Only rename the nth occurrence (1-based)"}
        },
        "required": ["path", "old_name", "new_name"],
        "additionalProperties": false
    });

    #[derive(Deserialize)]
    struct Params {
        path: String,
        old_name: String,
        new_name: String,
        #[serde(default)]
        case_sensitive: Option<bool>,
        #[serde(default)]
        occurrence: Option<usize>,
    }

    let handler = move |params| -> Result<Value> {
        let args: Params =
            serde_json::from_value(params).context("Invalid arguments for rename_symbol")?;
        let path = resolve_path(&args.path)?;
        let mut content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;

        let case_sensitive = args.case_sensitive.unwrap_or(true);
        let pattern = RegexBuilder::new(&format!("\\b{}\\b", regex::escape(&args.old_name)))
            .case_insensitive(!case_sensitive)
            .build()
            .with_context(|| format!("Failed to compile rename pattern for '{}'", args.old_name))?;

        let mut replacements = 0usize;

        if let Some(target) = args.occurrence {
            let mut new_content = String::with_capacity(content.len());
            let mut last = 0;
            for (idx, mat) in pattern.find_iter(&content).enumerate() {
                if idx + 1 == target {
                    new_content.push_str(&content[last..mat.start()]);
                    new_content.push_str(&args.new_name);
                    last = mat.end();
                    replacements = 1;
                    break;
                }
            }

            if replacements > 0 {
                new_content.push_str(&content[last..]);
                content = new_content;
            }
        } else {
            replacements = pattern.find_iter(&content).count();
            if replacements > 0 {
                content = pattern
                    .replace_all(&content, args.new_name.as_str())
                    .to_string();
            }
        }

        if replacements > 0 {
            fs::write(&path, &content)
                .with_context(|| format!("Failed to write {}", path.display()))?;
        }

        Ok(json!({
            "path": path.to_string_lossy(),
            "replacements": replacements,
        }))
    };

    Tool::new(
        "rename_symbol",
        "Rename symbol occurrences within a single file using word-boundary matching",
        schema,
        Box::new(handler),
    )
}

fn replace_symbol_body_tool() -> Tool {
    let schema = json!({
        "type": "object",
        "properties": {
            "path": {"type": "string"},
            "symbol": {"type": "string", "description": "Symbol name to update"},
            "new_body": {"type": "string", "description": "Replacement body content"},
            "occurrence": {"type": "integer", "minimum": 1},
            "case_sensitive": {"type": "boolean", "default": true},
            "start_line": {"type": "integer", "minimum": 1, "description": "Optional starting line override"},
            "end_line": {"type": "integer", "minimum": 1, "description": "Optional ending line override"}
        },
        "required": ["path", "symbol", "new_body"],
        "additionalProperties": false
    });

    #[derive(Deserialize)]
    struct Params {
        path: String,
        symbol: String,
        new_body: String,
        #[serde(default)]
        occurrence: Option<usize>,
        #[serde(default)]
        case_sensitive: Option<bool>,
        #[serde(default)]
        start_line: Option<usize>,
        #[serde(default)]
        end_line: Option<usize>,
    }

    let handler = move |params| -> Result<Value> {
        let args: Params =
            serde_json::from_value(params).context("Invalid arguments for replace_symbol_body")?;
        let path = resolve_path(&args.path)?;
        let mut parsed = ParsedFile::from_path(&path)?
            .with_context(|| format!("{} is not a supported source file", path.display()))?;

        let case_sensitive = args.case_sensitive.unwrap_or(true);

        if let (Some(start_line), Some(end_line)) = (args.start_line, args.end_line) {
            if start_line > end_line {
                anyhow::bail!("start_line must be <= end_line");
            }

            let start_index = start_line.saturating_sub(1);
            let end_index = end_line.saturating_sub(1);
            if start_index >= parsed.lines.len() {
                anyhow::bail!("start_line {start_line} is outside the file range");
            }
            if end_index >= parsed.lines.len() {
                anyhow::bail!("end_line {end_line} is outside the file range");
            }

            let (start_offset, _) = parsed.lines.bounds(start_index);
            let (_, end_offset) = parsed.lines.bounds(end_index);

            let replacement = ensure_trailing_newline(&args.new_body);
            parsed
                .content
                .replace_range(start_offset..end_offset, &replacement);

            fs::write(&path, &parsed.content)
                .with_context(|| format!("Failed to write {}", path.display()))?;

            return Ok(json!({
                "path": path.to_string_lossy(),
                "mode": "line_range",
                "start_line": start_line,
                "end_line": end_line,
            }));
        }

        let mut candidates: Vec<&FileSymbol> = parsed
            .symbols
            .iter()
            .filter(|symbol| symbol_name_matches(&symbol.name, &args.symbol, false, case_sensitive))
            .collect();

        if candidates.is_empty() {
            anyhow::bail!(
                "No symbol named '{}' found in {}",
                args.symbol,
                path.display()
            );
        }

        candidates.sort_by_key(|symbol| symbol.line);
        let target_index = match args.occurrence {
            Some(idx) => {
                if idx == 0 || idx > candidates.len() {
                    anyhow::bail!(
                        "Occurrence {idx} is out of bounds (only {} matches)",
                        candidates.len()
                    );
                }
                idx - 1
            }
            None => {
                if candidates.len() > 1 {
                    anyhow::bail!(
                        "Multiple symbols named '{}' found; specify `occurrence` to disambiguate",
                        args.symbol
                    );
                }
                0
            }
        };

        let target = candidates[target_index];
        let replacement = ensure_trailing_newline(&args.new_body);

        match &target.body {
            BodyStyle::Braces {
                start,
                end,
                base_indent,
                inner_indent,
            } => {
                let formatted = format_brace_body(&replacement, base_indent, inner_indent);
                parsed.content.replace_range(*start..*end, &formatted);
            }
            BodyStyle::Indented {
                start,
                end,
                base_indent,
                indent_unit,
            } => {
                let formatted = format_indented_body(&replacement, base_indent, indent_unit);
                parsed.content.replace_range(*start..*end, &formatted);
            }
            BodyStyle::None => anyhow::bail!(
                "Symbol '{}' does not have a replaceable body (maybe a declaration without implementation)",
                target.name
            ),
        }

        fs::write(&path, &parsed.content)
            .with_context(|| format!("Failed to write {}", path.display()))?;

        Ok(json!({
            "path": path.to_string_lossy(),
            "symbol": target.name,
            "occurrence": target_index + 1,
        }))
    };

    Tool::new(
        "replace_symbol_body",
        "Replace the implementation of a symbol, preserving surrounding formatting",
        schema,
        Box::new(handler),
    )
}

fn ensure_trailing_newline(body: &str) -> String {
    if body.ends_with('\n') {
        body.to_string()
    } else {
        let mut owned = body.to_string();
        owned.push('\n');
        owned
    }
}

fn format_brace_body(body: &str, base_indent: &str, inner_indent: &str) -> String {
    let trimmed = body.trim_matches('\n');
    if trimmed.is_empty() {
        format!("\n{base_indent}")
    } else {
        let mut lines = Vec::new();
        for line in trimmed.lines() {
            let line = line.trim_end();
            if line.is_empty() {
                lines.push(format!("{inner_indent}"));
            } else {
                lines.push(format!("{inner_indent}{line}"));
            }
        }
        format!("\n{}\n{base_indent}", lines.join("\n"))
    }
}

fn format_indented_body(body: &str, base_indent: &str, indent_unit: &str) -> String {
    let trimmed = body.trim_matches('\n');
    let indent = format!("{base_indent}{indent_unit}");

    if trimmed.is_empty() {
        format!("{indent}pass\n")
    } else {
        let mut lines = Vec::new();
        for line in trimmed.lines() {
            let clean = line.trim_end();
            if clean.is_empty() {
                lines.push(indent.clone());
            } else {
                lines.push(format!("{indent}{clean}"));
            }
        }
        format!("{}\n", lines.join("\n"))
    }
}
