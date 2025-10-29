use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Handler signature for incoming tool calls.
pub type ToolHandler = Box<dyn Fn(Value) -> Result<Value> + Send + Sync>;

/// Lightweight tool description mirroring FastMCP metadata.
pub struct Tool {
    name: String,
    description: String,
    parameters: Value,
    handler: ToolHandler,
}

impl Tool {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: Value,
        handler: ToolHandler,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
            handler,
        }
    }

    pub fn call(&self, params: Value) -> Result<Value> {
        (self.handler)(params)
    }

    pub fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: self.parameters.clone(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

/// Public JSON description returned via the registry list endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// Registry storing all available tools.
pub struct ToolRegistry {
    tools: HashMap<String, Tool>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Tool) {
        let name = tool.name().to_owned();
        self.tools.insert(name, tool);
    }

    pub fn descriptors(&self) -> Vec<ToolDescriptor> {
        self.tools.values().map(|tool| tool.descriptor()).collect()
    }

    pub fn call(&self, name: &str, params: Value) -> Result<Value> {
        match self.tools.get(name) {
            Some(tool) => tool.call(params),
            None => anyhow::bail!("Unknown tool: {name}"),
        }
    }
}
