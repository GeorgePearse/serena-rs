use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use walkdir::{DirEntry, WalkDir};

use crate::tool::{Tool, ToolRegistry};
use crate::tools::{resolve_path, state_file};

pub fn register(registry: &mut ToolRegistry) {
    registry.register(onboarding_tool());
    registry.register(prepare_for_new_conversation_tool());
    registry.register(check_onboarding_performed_tool());
}

#[derive(Default, Serialize, Deserialize)]
struct WorkflowState {
    projects: HashMap<String, StoredSummary>,
}

#[derive(Serialize, Deserialize, Clone)]
struct StoredSummary {
    summary: ProjectSummary,
    updated_at: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct ProjectSummary {
    root: String,
    generated_at: String,
    files_scanned: usize,
    scan_truncated: bool,
    top_directories: Vec<DirectorySummary>,
    dominant_languages: Vec<LanguageSummary>,
    sample_files: Vec<String>,
    todo_count: usize,
    readme_excerpt: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
struct DirectorySummary {
    name: String,
    file_count: usize,
    sample_files: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
struct LanguageSummary {
    language: String,
    extension: String,
    files: usize,
}

fn onboarding_tool() -> Tool {
    let schema = json!({
        "type": "object",
        "properties": {
            "project_root": {
                "type": "string",
                "description": "Project directory to analyse. Defaults to current working directory.",
            },
            "max_directories": {
                "type": "integer",
                "minimum": 1,
                "description": "Limit number of directories in the summary",
            },
            "max_languages": {
                "type": "integer",
                "minimum": 1,
                "description": "Limit number of languages in the summary",
            },
            "refresh": {
                "type": "boolean",
                "description": "Force regeneration even if cached",
                "default": false,
            }
        },
        "additionalProperties": false
    });

    #[derive(Deserialize)]
    struct Params {
        #[serde(default)]
        project_root: Option<String>,
        #[serde(default)]
        max_directories: Option<usize>,
        #[serde(default)]
        max_languages: Option<usize>,
        #[serde(default)]
        refresh: Option<bool>,
    }

    let handler = move |params| -> Result<Value> {
        let args: Params =
            serde_json::from_value(params).context("Invalid arguments for onboarding_tool")?;
        let root = match &args.project_root {
            Some(path) => resolve_path(path)?,
            None => std::env::current_dir()?,
        };

        if !root.is_dir() {
            anyhow::bail!("{} is not a directory", root.display());
        }

        let max_directories = args.max_directories.unwrap_or(6);
        let max_languages = args.max_languages.unwrap_or(6);
        let force_refresh = args.refresh.unwrap_or(false);

        let mut state = load_state()?;
        let key = root.to_string_lossy().to_string();

        let summary = if !force_refresh {
            state.projects.get(&key).cloned()
        } else {
            None
        };

        let (summary, cache_state) = if let Some(stored) = summary {
            (stored, "cached")
        } else {
            let summary = collect_project_summary(&root, max_directories, max_languages)?;
            let stored = StoredSummary {
                updated_at: now_string(),
                summary: summary.clone(),
            };
            state.projects.insert(key.clone(), stored.clone());
            save_state(&state)?;
            (stored, "fresh")
        };

        Ok(json!({
            "project_root": key,
            "source": cache_state,
            "updated_at": summary.updated_at,
            "summary": summary.summary,
        }))
    };

    Tool::new(
        "onboarding_tool",
        "Collect a high-level overview of the repository to kickstart onboarding",
        schema,
        Box::new(handler),
    )
}

fn prepare_for_new_conversation_tool() -> Tool {
    let schema = json!({
        "type": "object",
        "properties": {
            "project_root": {"type": "string"},
            "max_directories": {"type": "integer", "minimum": 1},
            "max_languages": {"type": "integer", "minimum": 1}
        },
        "additionalProperties": false
    });

    #[derive(Deserialize)]
    struct Params {
        #[serde(default)]
        project_root: Option<String>,
        #[serde(default)]
        max_directories: Option<usize>,
        #[serde(default)]
        max_languages: Option<usize>,
    }

    let handler = move |params| -> Result<Value> {
        let args: Params = serde_json::from_value(params)
            .context("Invalid arguments for prepare_for_new_conversation")?;
        let root = match &args.project_root {
            Some(path) => resolve_path(path)?,
            None => std::env::current_dir()?,
        };

        if !root.is_dir() {
            anyhow::bail!("{} is not a directory", root.display());
        }

        let max_directories = args.max_directories.unwrap_or(6);
        let max_languages = args.max_languages.unwrap_or(6);

        let mut state = load_state()?;
        let key = root.to_string_lossy().to_string();
        let summary = if let Some(stored) = state.projects.get(&key) {
            stored.summary.clone()
        } else {
            let summary = collect_project_summary(&root, max_directories, max_languages)?;
            let stored = StoredSummary {
                updated_at: now_string(),
                summary: summary.clone(),
            };
            state.projects.insert(key.clone(), stored.clone());
            save_state(&state)?;
            summary
        };

        let suggestions = build_conversation_suggestions(&summary);

        Ok(json!({
            "project_root": key,
            "summary": summary,
            "suggested_focus": suggestions,
        }))
    };

    Tool::new(
        "prepare_for_new_conversation",
        "Return onboarding highlights plus suggested focus areas for a new collaboration",
        schema,
        Box::new(handler),
    )
}

fn check_onboarding_performed_tool() -> Tool {
    let schema = json!({
        "type": "object",
        "properties": {
            "project_root": {"type": "string"}
        },
        "required": ["project_root"],
        "additionalProperties": false
    });

    #[derive(Deserialize)]
    struct Params {
        project_root: String,
    }

    let handler = move |params| -> Result<Value> {
        let args: Params = serde_json::from_value(params)
            .context("Invalid arguments for check_onboarding_performed")?;
        let root = resolve_path(&args.project_root)?;
        let key = root.to_string_lossy().to_string();
        let state = load_state()?;

        if let Some(stored) = state.projects.get(&key) {
            Ok(json!({
                "project_root": key,
                "onboarding_complete": true,
                "last_updated": stored.updated_at,
            }))
        } else {
            Ok(json!({
                "project_root": key,
                "onboarding_complete": false,
            }))
        }
    };

    Tool::new(
        "check_onboarding_performed",
        "Check whether onboarding metadata has already been generated for a project",
        schema,
        Box::new(handler),
    )
}

fn collect_project_summary(
    root: &Path,
    max_directories: usize,
    max_languages: usize,
) -> Result<ProjectSummary> {
    const MAX_SCAN_FILES: usize = 5_000;
    const MAX_SAMPLE_FILES: usize = 12;

    let mut files_scanned = 0usize;
    let mut scan_truncated = false;
    let mut dir_stats: HashMap<String, DirStats> = HashMap::new();
    let mut language_stats: HashMap<String, usize> = HashMap::new();
    let mut sample_files = Vec::new();
    let mut todo_count = 0usize;

    let walker = WalkDir::new(root)
        .follow_links(false)
        .max_depth(6)
        .into_iter()
        .filter_entry(|entry| allow_entry(entry));

    for entry in walker.filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }

        files_scanned += 1;
        if files_scanned > MAX_SCAN_FILES {
            scan_truncated = true;
            break;
        }

        if let Ok(relative) = entry.path().strip_prefix(root) {
            if sample_files.len() < MAX_SAMPLE_FILES {
                sample_files.push(relative.to_string_lossy().to_string());
            }

            let top = relative
                .components()
                .next()
                .and_then(|component| match component {
                    std::path::Component::Normal(name) => Some(name.to_string_lossy().to_string()),
                    _ => None,
                })
                .unwrap_or_else(|| String::from("<root>"));

            let stats = dir_stats.entry(top).or_insert_with(DirStats::default);
            stats.file_count += 1;
            if stats.sample_files.len() < 3 {
                stats
                    .sample_files
                    .push(relative.to_string_lossy().to_string());
            }
        }

        if let Some(ext) = entry.path().extension().and_then(|s| s.to_str()) {
            let ext_lower = ext.to_lowercase();
            *language_stats.entry(ext_lower).or_insert(0) += 1;
        }

        if todo_count < 200 {
            todo_count += count_todo_markers(entry.path())?;
        }
    }

    let mut directories = dir_stats
        .into_iter()
        .map(|(name, stats)| DirectorySummary {
            name,
            file_count: stats.file_count,
            sample_files: stats.sample_files,
        })
        .collect::<Vec<_>>();
    directories.sort_by(|a, b| b.file_count.cmp(&a.file_count));
    directories.truncate(max_directories);

    let mut languages = language_stats
        .into_iter()
        .map(|(ext, count)| LanguageSummary {
            language: language_from_extension(&ext),
            extension: ext,
            files: count,
        })
        .collect::<Vec<_>>();
    languages.sort_by(|a, b| b.files.cmp(&a.files));
    languages.truncate(max_languages);

    let readme_excerpt = read_readme_excerpt(root)?;

    Ok(ProjectSummary {
        root: root.to_string_lossy().to_string(),
        generated_at: now_string(),
        files_scanned,
        scan_truncated,
        top_directories: directories,
        dominant_languages: languages,
        sample_files,
        todo_count,
        readme_excerpt,
    })
}

#[derive(Default)]
struct DirStats {
    file_count: usize,
    sample_files: Vec<String>,
}

fn allow_entry(entry: &DirEntry) -> bool {
    if let Some(name) = entry.file_name().to_str() {
        const IGNORED: [&str; 9] = [
            ".git",
            "target",
            "node_modules",
            "venv",
            ".venv",
            "dist",
            "build",
            ".pytest_cache",
            "__pycache__",
        ];

        if entry.file_type().is_dir() && IGNORED.iter().any(|&skip| skip == name) {
            return false;
        }
        if name.starts_with('.') && entry.file_type().is_dir() {
            return false;
        }
    }
    true
}

fn count_todo_markers(path: &Path) -> Result<usize> {
    let metadata = fs::metadata(path)?;
    if metadata.len() > 512 * 1024 {
        return Ok(0);
    }

    let content = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::InvalidData => return Ok(0),
        Err(err) => {
            return Err(err).with_context(|| format!("Failed to read {}", path.display()));
        }
    };

    let mut total = 0usize;
    total += content.matches("TODO").count();
    total += content.matches("FIXME").count();
    Ok(total)
}

fn read_readme_excerpt(root: &Path) -> Result<Option<String>> {
    const MAX_BYTES: usize = 1_200;
    let candidates = ["README.md", "README", "readme.md", "Readme.md"];

    for candidate in candidates {
        let path = root.join(candidate);
        if path.exists() && path.is_file() {
            let mut content = fs::read_to_string(&path)
                .with_context(|| format!("Failed to read {}", path.display()))?;
            if content.len() > MAX_BYTES {
                content.truncate(MAX_BYTES);
                content.push_str("…");
            }
            return Ok(Some(content));
        }
    }

    Ok(None)
}

fn language_from_extension(ext: &str) -> String {
    match ext {
        "rs" => "Rust".to_string(),
        "py" => "Python".to_string(),
        "ts" | "tsx" => "TypeScript".to_string(),
        "js" | "jsx" | "mjs" | "cjs" => "JavaScript".to_string(),
        "go" => "Go".to_string(),
        "java" | "kt" | "kts" => "JVM".to_string(),
        "cs" => "C#".to_string(),
        "swift" => "Swift".to_string(),
        "rb" => "Ruby".to_string(),
        "php" => "PHP".to_string(),
        "lua" => "Lua".to_string(),
        "cpp" | "cc" | "cxx" | "h" | "hpp" => "C/C++".to_string(),
        "c" => "C".to_string(),
        "dart" => "Dart".to_string(),
        "scala" => "Scala".to_string(),
        "hs" => "Haskell".to_string(),
        "ml" | "mli" => "OCaml".to_string(),
        "ex" | "exs" => "Elixir".to_string(),
        "zig" => "Zig".to_string(),
        "sh" => "Shell".to_string(),
        "json" => "JSON".to_string(),
        "yml" | "yaml" => "YAML".to_string(),
        other => other.to_uppercase(),
    }
}

fn build_conversation_suggestions(summary: &ProjectSummary) -> Vec<Value> {
    let mut suggestions = Vec::new();

    if summary.todo_count > 0 {
        suggestions.push(json!({
            "type": "todo_review",
            "message": format!("Review approximately {} TODO/FIXME markers before modifying code", summary.todo_count),
        }));
    }

    if summary.scan_truncated {
        suggestions.push(json!({
            "type": "large_project",
            "message": "Project is large; consider narrowing scope or running targeted symbol searches.",
        }));
    }

    if let Some(primary) = summary.dominant_languages.first() {
        suggestions.push(json!({
            "type": "language_focus",
            "message": format!("Primary language detected: {} (.{})", primary.language, primary.extension),
        }));
    }

    if !summary.top_directories.is_empty() {
        let names: Vec<&str> = summary
            .top_directories
            .iter()
            .map(|d| d.name.as_str())
            .collect();
        suggestions.push(json!({
            "type": "directory_orientation",
            "message": format!("Key areas to inspect: {}", names.join(", ")),
        }));
    }

    if summary.readme_excerpt.is_none() {
        suggestions.push(json!({
            "type": "documentation",
            "message": "No README detected at the project root; confirm setup expectations with maintainers.",
        }));
    }

    suggestions
}

fn load_state() -> Result<WorkflowState> {
    let path = state_file("workflow_state.json")?;
    if !path.exists() {
        return Ok(WorkflowState::default());
    }

    let bytes = fs::read(&path)
        .with_context(|| format!("Failed to read workflow state at {}", path.display()))?;
    if bytes.is_empty() {
        return Ok(WorkflowState::default());
    }

    let state = serde_json::from_slice(&bytes)
        .with_context(|| format!("Failed to parse workflow state at {}", path.display()))?;
    Ok(state)
}

fn save_state(state: &WorkflowState) -> Result<()> {
    let path = state_file("workflow_state.json")?;
    let payload = serde_json::to_vec_pretty(state).context("Failed to serialise workflow state")?;
    fs::write(&path, payload)
        .with_context(|| format!("Failed to write workflow state to {}", path.display()))
}

fn now_string() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}
