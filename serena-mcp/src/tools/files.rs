use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use regex::RegexBuilder;
use serde::Deserialize;
use serde_json::{Value, json};
use walkdir::WalkDir;

use crate::tool::{Tool, ToolRegistry};
use crate::tools::resolve_path;

pub fn register(registry: &mut ToolRegistry) {
    registry.register(read_file_tool());
    registry.register(list_dir_tool());
    registry.register(write_file_tool());
    registry.register(search_pattern_tool());
}

#[derive(Debug, Deserialize)]
struct ReadFileParams {
    path: String,
    #[serde(default)]
    max_bytes: Option<usize>,
}

fn read_file_tool() -> Tool {
    let schema = json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Absolute or relative filesystem path to read",
            },
            "max_bytes": {
                "type": "integer",
                "minimum": 1,
                "description": "Optional soft limit. If the file is larger, content is truncated.",
            }
        },
        "required": ["path"],
        "additionalProperties": false
    });

    let handler = move |params| -> Result<_> {
        let args: ReadFileParams =
            serde_json::from_value(params).context("Invalid arguments for read_file")?;
        let path = resolve_path(&args.path)?;
        let display_path = path.to_string_lossy().to_string();
        let content =
            fs::read_to_string(&path).with_context(|| format!("Failed to read {display_path}"))?;

        let (content, truncated) = match args.max_bytes {
            Some(limit) if content.len() > limit => {
                let mut slice = content[..limit].to_string();
                slice.push_str("…");
                (slice, true)
            }
            _ => (content, false),
        };

        Ok(json!({
            "path": display_path,
            "content": content,
            "truncated": truncated,
        }))
    };

    Tool::new(
        "read_file",
        "Read file contents into a UTF-8 string",
        schema,
        Box::new(handler),
    )
}

#[derive(Debug, Deserialize)]
struct ListDirParams {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    max_entries: Option<usize>,
    #[serde(default)]
    include_hidden: Option<bool>,
}

fn list_dir_tool() -> Tool {
    let schema = json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Directory path to enumerate; defaults to current directory",
            },
            "max_entries": {
                "type": "integer",
                "minimum": 1,
                "description": "Optional maximum number of entries to return",
            },
            "include_hidden": {
                "type": "boolean",
                "description": "Whether to include dotfiles and dot-directories",
                "default": false,
            }
        },
        "additionalProperties": false
    });

    let handler = move |params| -> Result<_> {
        let args: ListDirParams =
            serde_json::from_value(params).context("Invalid arguments for list_dir")?;
        let dir_path = match args.path {
            Some(path) => resolve_path(&path)?,
            None => std::env::current_dir()?,
        };
        let dir_display = dir_path.to_string_lossy().to_string();
        let max_entries = args.max_entries.unwrap_or(usize::MAX);
        let include_hidden = args.include_hidden.unwrap_or(false);

        let mut entries = Vec::new();
        let read_dir = fs::read_dir(&dir_path)
            .with_context(|| format!("Failed to list directory {dir_display}"))?;

        for entry in read_dir {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !include_hidden && name.starts_with('.') {
                continue;
            }

            let metadata = entry.metadata()?;
            let file_type = metadata.file_type();
            let entry_type = if file_type.is_dir() {
                "directory"
            } else if file_type.is_file() {
                "file"
            } else if file_type.is_symlink() {
                "symlink"
            } else {
                "other"
            };

            entries.push(json!({
                "name": name,
                "type": entry_type,
            }));

            if entries.len() >= max_entries {
                break;
            }
        }

        Ok(json!({
            "path": dir_display,
            "entries": entries,
        }))
    };

    Tool::new(
        "list_dir",
        "List directory entries with basic metadata",
        schema,
        Box::new(handler),
    )
}

#[derive(Debug, Deserialize)]
struct WriteFileParams {
    path: String,
    content: String,
    #[serde(default)]
    append: bool,
    #[serde(default)]
    create_dirs: bool,
    #[serde(default)]
    ensure_trailing_newline: bool,
}

fn write_file_tool() -> Tool {
    let schema = json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Destination file path (will be resolved relative to the current working directory)",
            },
            "content": {
                "type": "string",
                "description": "Content to write to the file",
            },
            "append": {
                "type": "boolean",
                "description": "Append instead of replacing existing file contents",
                "default": false,
            },
            "create_dirs": {
                "type": "boolean",
                "description": "Create parent directories when they do not exist",
                "default": false,
            },
            "ensure_trailing_newline": {
                "type": "boolean",
                "description": "Guarantee that the file ends with a newline",
                "default": false,
            }
        },
        "required": ["path", "content"],
        "additionalProperties": false
    });

    let handler = move |params| -> Result<Value> {
        let args: WriteFileParams =
            serde_json::from_value(params).context("Invalid arguments for write_file")?;
        let path = resolve_path(&args.path)?;
        if args.create_dirs {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create parent directories for {path:?}"))?;
            }
        }

        let mut content = args.content;
        if args.ensure_trailing_newline && !content.ends_with('\n') {
            content.push('\n');
        }

        let mut options = OpenOptions::new();
        options.create(true).write(true);
        if args.append {
            options.append(true);
        } else {
            options.truncate(true);
        }

        let mut file = options
            .open(&path)
            .with_context(|| format!("Failed to open {}", path.to_string_lossy()))?;
        file.write_all(content.as_bytes())
            .with_context(|| format!("Failed writing to {}", path.to_string_lossy()))?;

        Ok(json!({
            "path": path.to_string_lossy(),
            "bytes_written": content.len(),
            "operation": if args.append { "append" } else { "overwrite" },
        }))
    };

    Tool::new(
        "write_file",
        "Write or append content to a file on disk",
        schema,
        Box::new(handler),
    )
}

#[derive(Debug, Deserialize)]
struct SearchPatternParams {
    pattern: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    regex: bool,
    #[serde(default)]
    case_sensitive: Option<bool>,
    #[serde(default)]
    max_results: Option<usize>,
    #[serde(default)]
    context_lines: Option<usize>,
    #[serde(default)]
    include_hidden: Option<bool>,
}

fn search_pattern_tool() -> Tool {
    let schema = json!({
        "type": "object",
        "properties": {
            "pattern": {
                "type": "string",
                "description": "Needle to look for. If `regex` is true it is treated as a regular expression.",
            },
            "path": {
                "type": "string",
                "description": "Directory or file to search. Defaults to current working directory.",
            },
            "regex": {
                "type": "boolean",
                "description": "Interpret pattern as a Rust regular expression",
                "default": false,
            },
            "case_sensitive": {
                "type": "boolean",
                "description": "Control case sensitivity (default true)",
            },
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "description": "Stop searching after this many matches (default 50)",
            },
            "context_lines": {
                "type": "integer",
                "minimum": 0,
                "description": "Number of surrounding lines to include for each match (default 2)",
            },
            "include_hidden": {
                "type": "boolean",
                "description": "Search files inside hidden directories (dot-prefixed)",
                "default": false,
            }
        },
        "required": ["pattern"],
        "additionalProperties": false
    });

    let handler = move |params| -> Result<Value> {
        let args: SearchPatternParams =
            serde_json::from_value(params).context("Invalid arguments for search_pattern")?;
        let root = match &args.path {
            Some(path) => resolve_path(path)?,
            None => std::env::current_dir()?,
        };

        let max_results = args.max_results.unwrap_or(50);
        let context_lines = args.context_lines.unwrap_or(2);
        let case_sensitive = args.case_sensitive.unwrap_or(true);
        let include_hidden = args.include_hidden.unwrap_or(false);

        let mut results = Vec::new();

        if root.is_file() {
            search_in_file(
                &root,
                &args.pattern,
                SearchOptions {
                    regex: args.regex,
                    case_sensitive,
                    context_lines,
                    max_results,
                },
                &mut results,
            )?;
        } else {
            for entry in WalkDir::new(&root)
                .follow_links(false)
                .into_iter()
                .filter_entry(|e| include_hidden || !is_hidden_path(e.path()))
                .filter_map(|e| e.ok())
            {
                if !entry.file_type().is_file() {
                    continue;
                }

                search_in_file(
                    entry.path(),
                    &args.pattern,
                    SearchOptions {
                        regex: args.regex,
                        case_sensitive,
                        context_lines,
                        max_results,
                    },
                    &mut results,
                )?;

                if results.len() >= max_results {
                    break;
                }
            }
        }

        let truncated = results.len() >= max_results;
        Ok(json!({
            "root": root.to_string_lossy(),
            "pattern": args.pattern,
            "regex": args.regex,
            "case_sensitive": case_sensitive,
            "matches": results,
            "truncated": truncated,
        }))
    };

    Tool::new(
        "search_pattern",
        "Search for a literal string or regular expression across the project",
        schema,
        Box::new(handler),
    )
}

struct SearchOptions {
    regex: bool,
    case_sensitive: bool,
    context_lines: usize,
    max_results: usize,
}

fn search_in_file(
    path: &Path,
    pattern: &str,
    options: SearchOptions,
    matches: &mut Vec<Value>,
) -> Result<()> {
    if matches.len() >= options.max_results {
        return Ok(());
    }

    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::InvalidData {
                return Ok(()); // Skip non UTF-8 files
            }
            return Err(err).with_context(|| format!("Failed to read {}", path.display()));
        }
    };

    let lines: Vec<&str> = content.lines().collect();
    let mut local_matches = Vec::new();

    if options.regex {
        let regex = RegexBuilder::new(pattern)
            .case_insensitive(!options.case_sensitive)
            .build()
            .with_context(|| format!("Failed to compile regex pattern '{pattern}'"))?;

        for (line_idx, line) in lines.iter().enumerate() {
            for capture in regex.find_iter(line) {
                let column = line[..capture.start()].chars().count() + 1;
                local_matches.push(MatchInfo::new(
                    path,
                    line_idx,
                    column,
                    line,
                    &lines,
                    options.context_lines,
                ));

                if matches.len() + local_matches.len() >= options.max_results {
                    break;
                }
            }

            if matches.len() + local_matches.len() >= options.max_results {
                break;
            }
        }
    } else {
        let needle = if options.case_sensitive {
            pattern.to_string()
        } else {
            pattern.to_lowercase()
        };

        for (line_idx, line) in lines.iter().enumerate() {
            let haystack = if options.case_sensitive {
                (*line).to_string()
            } else {
                line.to_lowercase()
            };

            let mut search_start = 0;
            let mut remainder = haystack.as_str();
            while let Some(pos) = remainder.find(&needle) {
                let absolute_pos = search_start + pos;
                let column = line[..absolute_pos].chars().count() + 1;
                local_matches.push(MatchInfo::new(
                    path,
                    line_idx,
                    column,
                    line,
                    &lines,
                    options.context_lines,
                ));

                if matches.len() + local_matches.len() >= options.max_results {
                    break;
                }

                let advance = pos + needle.len();
                search_start += advance;
                remainder = &remainder[advance..];
            }

            if matches.len() + local_matches.len() >= options.max_results {
                break;
            }
        }
    }

    matches.extend(local_matches.into_iter().map(|m| m.into_value()));
    Ok(())
}

struct MatchInfo<'a> {
    path: PathBuf,
    line_idx: usize,
    column: usize,
    line: &'a str,
    context: Vec<(&'a str, usize)>,
}

impl<'a> MatchInfo<'a> {
    fn new(
        path: &Path,
        line_idx: usize,
        column: usize,
        line: &'a str,
        lines: &'a [&'a str],
        context_lines: usize,
    ) -> Self {
        let mut context = Vec::new();

        if context_lines > 0 {
            let start = line_idx.saturating_sub(context_lines);
            let end = usize::min(line_idx + context_lines, lines.len().saturating_sub(1));
            for idx in start..=end {
                if idx == line_idx {
                    continue;
                }
                context.push((lines[idx], idx));
            }
        }

        Self {
            path: path.to_path_buf(),
            line_idx,
            column,
            line,
            context,
        }
    }

    fn into_value(self) -> Value {
        let preview = self.line.trim_end().to_string();
        let context = self
            .context
            .into_iter()
            .map(|(line, idx)| {
                json!({
                    "line": idx + 1,
                    "text": line.trim_end(),
                })
            })
            .collect::<Vec<_>>();

        json!({
            "path": self.path.to_string_lossy(),
            "line": self.line_idx + 1,
            "column": self.column,
            "preview": preview,
            "context": context,
        })
    }
}

fn is_hidden_path(path: &Path) -> bool {
    path.components().any(|component| match component {
        std::path::Component::Normal(os_str) => os_str.to_string_lossy().starts_with('.'),
        _ => false,
    })
}
