mod files;
mod memory;
mod symbols;
mod workflow;

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::tool::ToolRegistry;

/// Build a tool registry populated with the implemented tool families.
pub fn build_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();

    files::register(&mut registry);
    memory::register(&mut registry);
    symbols::register(&mut registry);
    workflow::register(&mut registry);

    registry
}

/// Resolve the directory used to persist mutable tool state.
pub(crate) fn state_dir() -> Result<PathBuf> {
    if let Ok(dir) = env::var("SERENA_STATE_DIR") {
        let path = PathBuf::from(dir);
        fs::create_dir_all(&path)
            .with_context(|| format!("Failed to create state dir at {path:?}"))?;
        return Ok(path);
    }

    let home = env::var("HOME").context("HOME environment variable is not set")?;
    let path = Path::new(&home).join(".serena-mcp");
    fs::create_dir_all(&path).with_context(|| format!("Failed to create state dir at {path:?}"))?;
    Ok(path)
}

/// Convenience helper for working with stable state files.
pub(crate) fn state_file(name: &str) -> Result<PathBuf> {
    Ok(state_dir()?.join(name))
}

/// Expand `~` and resolve relative paths against the current directory.
pub(crate) fn resolve_path(path: &str) -> Result<PathBuf> {
    if path.trim().is_empty() {
        anyhow::bail!("Path cannot be empty");
    }

    if path.starts_with("~/") {
        let home = env::var("HOME").context("HOME environment variable is not set")?;
        return Ok(PathBuf::from(home).join(path.trim_start_matches("~/")));
    }

    let candidate = PathBuf::from(path);
    if candidate.is_absolute() {
        Ok(candidate)
    } else {
        Ok(env::current_dir()?.join(candidate))
    }
}
