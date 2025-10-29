mod files;
mod memory;
mod symbols;
mod workflow;

use serde_json::json;

use crate::tool::{Tool, ToolHandler, ToolRegistry};

/// Build a registry populated with stub implementations for the core tool families.
pub fn build_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();

    files::register(&mut registry);
    memory::register(&mut registry);
    symbols::register(&mut registry);
    workflow::register(&mut registry);

    registry
}

fn stub_handler(message: &str) -> ToolHandler {
    let message = message.to_owned();
    Box::new(move |_params| Ok(json!({ "status": "not_implemented", "message": message })))
}

fn simple_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "payload": { "type": "object" }
        },
        "additionalProperties": true
    })
}

fn register_stub(registry: &mut ToolRegistry, name: &str, description: &str) {
    let tool = Tool::new(
        name,
        description,
        simple_schema(),
        stub_handler(description),
    );
    registry.register(tool);
}
