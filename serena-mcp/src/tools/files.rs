use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;

use super::register_stub;
use crate::tool::{Tool, ToolRegistry};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(read_file_tool());
    registry.register(list_dir_tool());

    register_stub(registry, "write_file", "Stubbed file write tool");
    register_stub(registry, "search_pattern", "Stubbed project search tool");
}

#[derive(Debug, Deserialize)]
struct ReadFileParams {
    path: String,
}

fn read_file_tool() -> Tool {
    let schema = json!({
        "type": "object",
        "properties": {
            "path": {
                "type": "string",
                "description": "Absolute or relative filesystem path to read",
            }
        },
        "required": ["path"],
        "additionalProperties": false
    });

    let handler = move |params| -> Result<_> {
        let args: ReadFileParams =
            serde_json::from_value(params).context("Invalid arguments for read_file")?;
        let path = PathBuf::from(args.path);
        let display_path = path.to_string_lossy().to_string();
        let content =
            fs::read_to_string(&path).with_context(|| format!("Failed to read {display_path}"))?;
        Ok(json!({
            "path": display_path,
            "content": content,
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
            }
        },
        "additionalProperties": false
    });

    let handler = move |params| -> Result<_> {
        let args: ListDirParams =
            serde_json::from_value(params).context("Invalid arguments for list_dir")?;
        let dir_path = PathBuf::from(args.path.unwrap_or_else(|| ".".to_string()));
        let dir_display = dir_path.to_string_lossy().to_string();
        let max_entries = args.max_entries.unwrap_or(usize::MAX);

        let mut entries = Vec::new();
        for entry in fs::read_dir(&dir_path)
            .with_context(|| format!("Failed to list directory {:?}", dir_path))?
        {
            let entry = entry?;
            let metadata = entry.metadata()?;
            let name = entry.file_name().to_string_lossy().to_string();
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
