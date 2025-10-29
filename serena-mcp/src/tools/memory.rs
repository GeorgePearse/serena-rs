use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::tool::{Tool, ToolRegistry};
use crate::tools::state_file;

pub fn register(registry: &mut ToolRegistry) {
    registry.register(write_memory_tool());
    registry.register(read_memory_tool());
    registry.register(list_memories_tool());
    registry.register(delete_memory_tool());
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MemoryEntry {
    id: String,
    namespace: String,
    content: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    metadata: Value,
    created_at: String,
    #[serde(default)]
    updated_at: Option<String>,
}

impl MemoryEntry {
    fn matches(&self, filter: &MemoryFilter) -> bool {
        if let Some(id) = &filter.id {
            if &self.id != id {
                return false;
            }
        }

        if let Some(namespace) = &filter.namespace {
            if &self.namespace != namespace {
                return false;
            }
        }

        if let Some(tag) = &filter.tag {
            if !self.tags.iter().any(|t| t == tag) {
                return false;
            }
        }

        if let Some(query) = &filter.query {
            let needle = query.to_lowercase();
            let content_match = self.content.to_lowercase().contains(&needle);
            let metadata_match = self
                .metadata
                .as_object()
                .and_then(|obj| {
                    Some(
                        obj.values()
                            .any(|value| value.to_string().to_lowercase().contains(&needle)),
                    )
                })
                .unwrap_or(false);

            if !content_match && !metadata_match {
                return false;
            }
        }

        true
    }
}

struct MemoryStore {
    path: PathBuf,
}

impl MemoryStore {
    fn new() -> Result<Self> {
        let path = state_file("memories.json")?;
        if !path.exists() {
            fs::write(&path, b"[]")
                .with_context(|| format!("Failed to initialise memory store at {path:?}"))?;
        }
        Ok(Self { path })
    }

    fn load(&self) -> Result<Vec<MemoryEntry>> {
        let bytes = fs::read(&self.path)
            .with_context(|| format!("Failed to read memory store at {}", self.path.display()))?;
        if bytes.is_empty() {
            return Ok(Vec::new());
        }
        let entries = serde_json::from_slice(&bytes)
            .with_context(|| format!("Failed to parse memory store at {}", self.path.display()))?;
        Ok(entries)
    }

    fn save(&self, entries: &[MemoryEntry]) -> Result<()> {
        let payload =
            serde_json::to_vec_pretty(entries).context("Failed to serialise memory store")?;
        fs::write(&self.path, payload)
            .with_context(|| format!("Failed to write memory store at {}", self.path.display()))
    }
}

#[derive(Debug, Default)]
struct MemoryFilter {
    id: Option<String>,
    namespace: Option<String>,
    tag: Option<String>,
    query: Option<String>,
}

fn write_memory_tool() -> Tool {
    let schema = json!({
        "type": "object",
        "properties": {
            "content": {"type": "string", "description": "Free-form content to store."},
            "namespace": {"type": "string", "description": "Logical namespace for grouping memories.", "default": "default"},
            "tags": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Optional tags for later retrieval"
            },
            "metadata": {
                "type": "object",
                "description": "Arbitrary key/value metadata to persist alongside the memory",
            },
            "id": {
                "type": "string",
                "description": "Override the generated identifier or update an existing entry",
            }
        },
        "required": ["content"],
        "additionalProperties": false
    });

    #[derive(Deserialize)]
    struct Params {
        content: String,
        #[serde(default)]
        namespace: Option<String>,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default)]
        metadata: Option<Value>,
        #[serde(default)]
        id: Option<String>,
    }

    let handler = move |params| -> Result<Value> {
        let args: Params =
            serde_json::from_value(params).context("Invalid arguments for write_memory")?;
        let store = MemoryStore::new()?;
        let mut entries = store.load()?;

        let namespace = args.namespace.unwrap_or_else(|| "default".to_string());
        let metadata = args
            .metadata
            .unwrap_or_else(|| Value::Object(Default::default()));
        let timestamp = now_string();

        let (entry, action) = if let Some(id) = args.id {
            match entries.iter_mut().find(|entry| entry.id == id) {
                Some(existing) => {
                    existing.content = args.content;
                    existing.namespace = namespace;
                    existing.tags = args.tags;
                    existing.metadata = metadata;
                    existing.updated_at = Some(timestamp.clone());
                    (existing.clone(), "updated")
                }
                None => {
                    let entry = MemoryEntry {
                        id,
                        namespace,
                        content: args.content,
                        tags: args.tags,
                        metadata,
                        created_at: timestamp.clone(),
                        updated_at: Some(timestamp.clone()),
                    };
                    entries.push(entry.clone());
                    (entry, "created")
                }
            }
        } else {
            let entry = MemoryEntry {
                id: generate_id(),
                namespace,
                content: args.content,
                tags: args.tags,
                metadata,
                created_at: timestamp.clone(),
                updated_at: None,
            };
            entries.push(entry.clone());
            (entry, "created")
        };

        store.save(&entries)?;
        Ok(json!({
            "memory": entry,
            "action": action,
        }))
    };

    Tool::new(
        "write_memory",
        "Persist an item in the built-in memory store",
        schema,
        Box::new(handler),
    )
}

fn read_memory_tool() -> Tool {
    let schema = json!({
        "type": "object",
        "properties": {
            "id": {"type": "string"},
            "namespace": {"type": "string"},
            "tag": {"type": "string"},
            "query": {
                "type": "string",
                "description": "Substring to search within content or metadata",
            },
            "limit": {"type": "integer", "minimum": 1, "description": "Maximum number of memories to return"}
        },
        "additionalProperties": false
    });

    #[derive(Deserialize)]
    struct Params {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        namespace: Option<String>,
        #[serde(default)]
        tag: Option<String>,
        #[serde(default)]
        query: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
    }

    let handler = move |params| -> Result<Value> {
        let args: Params =
            serde_json::from_value(params).context("Invalid arguments for read_memory")?;
        let store = MemoryStore::new()?;
        let entries = store.load()?;

        let filter = MemoryFilter {
            id: args.id,
            namespace: args.namespace,
            tag: args.tag,
            query: args.query,
        };
        let limit = args.limit.unwrap_or(20);

        let filtered = entries
            .into_iter()
            .filter(|entry| entry.matches(&filter))
            .take(limit)
            .collect::<Vec<_>>();

        Ok(json!({
            "count": filtered.len(),
            "memories": filtered,
        }))
    };

    Tool::new(
        "read_memory",
        "Retrieve memories by id, namespace, tag, or fuzzy content search",
        schema,
        Box::new(handler),
    )
}

fn list_memories_tool() -> Tool {
    let schema = json!({
        "type": "object",
        "properties": {
            "namespace": {"type": "string"},
            "limit": {"type": "integer", "minimum": 1},
            "offset": {"type": "integer", "minimum": 0}
        },
        "additionalProperties": false
    });

    #[derive(Deserialize)]
    struct Params {
        #[serde(default)]
        namespace: Option<String>,
        #[serde(default)]
        limit: Option<usize>,
        #[serde(default)]
        offset: Option<usize>,
    }

    let handler = move |params| -> Result<Value> {
        let args: Params =
            serde_json::from_value(params).context("Invalid arguments for list_memories")?;
        let store = MemoryStore::new()?;
        let mut entries = store.load()?;

        if let Some(namespace) = args.namespace {
            entries.retain(|entry| entry.namespace == namespace);
        }

        entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        let offset = args.offset.unwrap_or(0);
        let limit = args.limit.unwrap_or(50);
        let slice = entries
            .into_iter()
            .skip(offset)
            .take(limit)
            .collect::<Vec<_>>();

        Ok(json!({
            "count": slice.len(),
            "memories": slice,
        }))
    };

    Tool::new(
        "list_memories",
        "List recent memories, optionally scoped to a namespace",
        schema,
        Box::new(handler),
    )
}

fn delete_memory_tool() -> Tool {
    let schema = json!({
        "type": "object",
        "properties": {
            "id": {
                "type": "string",
                "description": "Identifier of the memory to remove",
            }
        },
        "required": ["id"],
        "additionalProperties": false
    });

    #[derive(Deserialize)]
    struct Params {
        id: String,
    }

    let handler = move |params| -> Result<Value> {
        let args: Params =
            serde_json::from_value(params).context("Invalid arguments for delete_memory")?;
        let store = MemoryStore::new()?;
        let mut entries = store.load()?;
        let original_len = entries.len();
        entries.retain(|entry| entry.id != args.id);
        let removed = entries.len() != original_len;

        store.save(&entries)?;

        Ok(json!({
            "id": args.id,
            "deleted": removed,
        }))
    };

    Tool::new(
        "delete_memory",
        "Delete a stored memory entry by id",
        schema,
        Box::new(handler),
    )
}

fn now_string() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn generate_id() -> String {
    let timestamp = OffsetDateTime::now_utc().unix_timestamp_nanos();
    format!("mem-{}", timestamp)
}
